//! The fake GitHub's HTTP server, [`FakeGitHub`] (TST-3; T20): the world of `testkit::github` served
//! on the loopback with GitHub's headers, misbehaving on request, recording every request, failing
//! the test on a broken rule, and telling the test clock when it's settled (TST-4).
//!
//! - **Starting.** [`FakeGitHub::start`] on a `TestClock` (it adds itself as a `SettleProbe`);
//!   [`FakeGitHub::start_real`] on the system clock, for real processes (TST-9). [`FakeGitHub::url`],
//!   [`FakeGitHub::client`].
//! - **The world.** [`FakeGitHub::world`] changes it now or schedules changes. Before each request
//!   and each `world` call, GitHub's clock advances to the clock's wall time plus the skew
//!   ([`FakeGitHub::set_skew`], [`FakeGitHub::set_skew_behind`]), and never back (TST-3).
//!   [`FakeGitHub::set_login`], [`FakeGitHub::set_rate`], [`FakeGitHub::set_rate_lag`],
//!   [`FakeGitHub::without_date_header`].
//! - **Faults.** [`FakeGitHub::fault`] and [`FakeGitHub::fault_aimed`] with a [`Fault`], an [`Aim`]
//!   and [`Times`]; [`FakeGitHub::clear_faults`]. A refused connection is
//!   [`FakeGitHub::refuse`] until [`FakeGitHub::listen`]. A redirect's target counts its hits
//!   ([`FakeGitHub::redirect_hits`]). The faults that change an answer are the world's: `mergeable`
//!   unknown (`World::unknown_for`), the old head served (`World::push_lagged`), a check read out of
//!   order (`World::misread`), and, under an aim and a count, an error on a PR's node or lookup
//!   ([`FakeGitHub::pr_error`]) and inside it ([`FakeGitHub::field_error`]).
//! - **Holding.** [`FakeGitHub::hold_next`] returns a [`Hold`]: the next request waits until it's
//!   released, then is answered from the world as it is then. [`FakeGitHub::held`] and
//!   [`FakeGitHub::until_held`] count what's held by script (holds and hangs).
//! - **Tokens.** [`FakeGitHub::accept_only`] names the tokens it accepts (any by default): another
//!   gets the 401.
//! - **The log.** [`FakeGitHub::requests`], [`FakeGitHub::points`].
//!   [`FakeGitHub::broken`] and [`FakeGitHub::take_broken`] read the broken rules.
//! - **The rules.** A mutation, a query the schema refuses or the fake can't answer, a request
//!   while another is in flight, a request under 1 s after the last ended (POL-1, POL-2). Each is
//!   recorded and panics at [`FakeGitHub::check`] and at drop. [`FakeGitHub::new_daemon`] starts
//!   the per-daemon checks afresh. Under `start_real` they're held per `x-circitor-test-pid`.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::clock::{Activity, Excused, SettleProbe, SharedClock, TestClock, Time};
use crate::github::client::Client;
use crate::testkit::MARKER_LOGIN;
use crate::testkit::github::{PrId, World};

mod answer;
mod fault;
mod log;
mod serve;

pub use fault::{Aim, Fault, LimitStatus, Times};
pub use log::Request;

use fault::Entry;
use log::Rules;
use serve::Conn;

/// The path a [`Fault::Redirect`] points at; its hits are counted, never answered as GraphQL.
const MOVED_PATH: &str = "/moved";

/// An answer-changing fault the world plays for a while (`pr_error`, `field_error`).
#[derive(Clone)]
enum PrFault {
    /// `World::node_error`.
    Node { pr: PrId, type_: String },
    /// `World::field_error`.
    Field {
        pr: PrId,
        field: String,
        type_: String,
    },
}

/// A `hold_next` waiting for its request.
struct HoldSlot {
    release: oneshot::Receiver<()>,
    arrived: watch::Sender<bool>,
}

/// Everything behind the lock, which is never held across an `.await`.
struct State {
    world: World,
    /// GitHub's clock ahead of the wall time (negative: behind), in milliseconds.
    skew_ms: i64,
    date_header: bool,
    faults: Vec<Entry<Fault>>,
    pr_faults: Vec<Entry<PrFault>>,
    holds: VecDeque<HoldSlot>,
    /// `None`: any token.
    tokens: Option<Vec<String>>,
    /// Answers report the rate this many requests old (docs/02 §2).
    lag: usize,
    /// The true rate after each request answered since the lag was set, oldest first.
    rates: VecDeque<(u32, Time)>,
    log: Vec<Request>,
    broken: Vec<String>,
    rules: Rules,
}

/// What the server's tasks share.
struct Inner {
    clock: SharedClock,
    /// The test clock's count of work in flight; `None` under `start_real` (no excuse).
    activity: Option<Activity>,
    /// `start_real`: the rules are held per process.
    real: bool,
    addr: SocketAddr,
    state: Mutex<State>,
    /// Connections accepted and not answered, closed or held by script (TST-4).
    unsettled: Arc<AtomicUsize>,
    /// Requests held by script: holds and hangs.
    held: watch::Sender<usize>,
    /// Bumped by `clear_faults`: a hang ends.
    cleared: watch::Sender<u64>,
    conns: Mutex<Vec<Weak<Conn>>>,
    redirect_hits: AtomicU32,
    accept: Mutex<Option<JoinHandle<()>>>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding it leaves the state as it was at that line: still worth reading.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// TST-3: GitHub's clock follows the wall time forward, never back, with the skew.
    fn sync_world(&self, st: &mut State) {
        let wall = self.clock.wall();
        let target = if st.skew_ms >= 0 {
            wall.plus(Duration::from_millis(st.skew_ms.unsigned_abs()))
        } else {
            wall.minus(Duration::from_millis(st.skew_ms.unsigned_abs()))
        };
        st.world.advance_to(target);
    }

    /// Closes every connection open now.
    fn kill_all(&self) {
        let conns: Vec<Arc<Conn>> = self
            .conns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for c in conns {
            c.kill();
        }
    }

    /// Connections not yet closed.
    fn open_conns(&self) -> usize {
        let conns = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        conns.iter().filter(|c| c.strong_count() > 0).count()
    }

    fn add_conn(&self, conn: &Arc<Conn>) {
        let mut conns = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        conns.retain(|c| c.strong_count() > 0);
        conns.push(Arc::downgrade(conn));
    }

    /// A request held by script: counted in `held`, and excused from the work in flight (TST-4).
    fn hold_guard(self: &Arc<Self>) -> HeldGuard {
        self.held.send_modify(|n| *n += 1);
        HeldGuard {
            inner: self.clone(),
            _excused: self.activity.as_ref().map(Activity::excuse),
        }
    }
}

// TST-4: settled when every connection accepted is answered, closed, or held by script.
impl SettleProbe for Inner {
    fn settled(&self) -> bool {
        self.unsettled.load(Ordering::SeqCst) == 0
    }
}

/// Held for as long as a request is held by script.
struct HeldGuard {
    inner: Arc<Inner>,
    _excused: Option<Excused>,
}

impl Drop for HeldGuard {
    fn drop(&mut self) {
        self.inner.held.send_modify(|n| *n = n.saturating_sub(1));
    }
}

/// A connection or a request not yet answered, closed or held: unsettled while it lives (TST-4).
struct Pending(Arc<AtomicUsize>);

impl Pending {
    fn new(count: &Arc<AtomicUsize>) -> Pending {
        count.fetch_add(1, Ordering::SeqCst);
        Pending(count.clone())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The handle `hold_next` returns: the held request is answered once it's released or dropped.
pub struct Hold {
    release: Option<oneshot::Sender<()>>,
    arrived: watch::Receiver<bool>,
}

impl Hold {
    /// Lets the held request be answered, from the world as it is now.
    pub fn release(mut self) {
        if let Some(tx) = self.release.take() {
            let _ = tx.send(());
        }
    }

    /// Whether the request has arrived and is being held.
    pub fn is_holding(&self) -> bool {
        *self.arrived.borrow()
    }

    /// Waits, in real time, until the request has arrived and is held.
    pub async fn arrived(&mut self) {
        let _ = self.arrived.wait_for(|v| *v).await;
    }
}

/// The fake GitHub (TST-3). See the module's documentation.
pub struct FakeGitHub {
    inner: Arc<Inner>,
}

impl FakeGitHub {
    /// Serves `POST /graphql` on `127.0.0.1:0` on the test clock, and adds itself to it as a probe.
    /// Must be called inside a tokio runtime.
    pub fn start(clock: Arc<TestClock>) -> FakeGitHub {
        let activity = clock.activity();
        let shared: SharedClock = clock.clone();
        let gh = FakeGitHub::serve(shared, Some(activity), false);
        clock.add_probe(gh.inner.clone());
        gh
    }

    /// The same server on the system clock, for real processes (TST-9): no probe, no excuse, and
    /// the per-daemon rules held per `x-circitor-test-pid` (requests without it are one process).
    pub fn start_real(clock: SharedClock) -> FakeGitHub {
        FakeGitHub::serve(clock, None, true)
    }

    fn serve(clock: SharedClock, activity: Option<Activity>, real: bool) -> FakeGitHub {
        let any = SocketAddr::from(([127, 0, 0, 1], 0));
        // A test can't go on without its fake GitHub.
        let listener = bind(any).expect("the fake GitHub binds 127.0.0.1:0");
        let addr = listener
            .local_addr()
            .expect("a bound listener has an address");
        let start = clock.wall();
        let inner = Arc::new(Inner {
            clock,
            activity,
            real,
            addr,
            state: Mutex::new(State {
                world: World::new(MARKER_LOGIN, start),
                skew_ms: 0,
                date_header: true,
                faults: Vec::new(),
                pr_faults: Vec::new(),
                holds: VecDeque::new(),
                tokens: None,
                lag: 0,
                rates: VecDeque::new(),
                log: Vec::new(),
                broken: Vec::new(),
                rules: Rules::default(),
            }),
            unsettled: Arc::new(AtomicUsize::new(0)),
            held: watch::channel(0).0,
            cleared: watch::channel(0).0,
            conns: Mutex::new(Vec::new()),
            redirect_hits: AtomicU32::new(0),
            accept: Mutex::new(None),
        });
        serve::spawn_accept(&inner, listener);
        FakeGitHub { inner }
    }

    /// The full `http://127.0.0.1:<port>/graphql` (TST-1).
    pub fn url(&self) -> String {
        format!("http://{}/graphql", self.inner.addr)
    }

    /// The address it listens on.
    pub fn addr(&self) -> SocketAddr {
        self.inner.addr
    }

    /// A real client pointed at it.
    pub fn client(&self) -> Client {
        Client::for_tests(&self.url())
    }

    /// Changes the world now, or schedules changes; GitHub's clock is brought up to date first.
    pub fn world<R>(&self, f: impl FnOnce(&mut World) -> R) -> R {
        let mut st = self.inner.lock();
        self.inner.sync_world(&mut st);
        f(&mut st.world)
    }

    /// GitHub's clock runs `ahead` of the clock's wall time (TST-3). It still never goes back.
    pub fn set_skew(&self, ahead: Duration) {
        let ms = i64::try_from(ahead.as_millis()).unwrap_or(i64::MAX);
        self.inner.lock().skew_ms = ms;
    }

    /// GitHub's clock runs `behind` the clock's wall time.
    pub fn set_skew_behind(&self, behind: Duration) {
        let ms = i64::try_from(behind.as_millis()).unwrap_or(i64::MAX);
        self.inner.lock().skew_ms = -ms;
    }

    /// Answers carry no `Date` header from now on (MOD-6).
    pub fn without_date_header(&self) {
        self.inner.lock().date_header = false;
    }

    /// Answers carry a `Date` header again.
    pub fn with_date_header(&self) {
        self.inner.lock().date_header = true;
    }

    /// `viewer.login` (MOD-11).
    pub fn set_login(&self, login: &str) {
        self.world(|w| w.set_login(login));
    }

    /// `remaining` and the reset time, apart from what circitor spends (other tools on the same
    /// token): each answer's cost then comes off it.
    pub fn set_rate(&self, remaining: u32, reset_at: Time) {
        self.world(|w| w.set_rate(remaining, reset_at));
    }

    /// Answers report the rate as it was `requests` answered requests ago, in the headers and the
    /// `rateLimit` block (docs/02 §2: counters lag). 0 ends the lag.
    pub fn set_rate_lag(&self, requests: usize) {
        let mut st = self.inner.lock();
        st.lag = requests;
        let now = st.world.rate();
        st.rates = VecDeque::from([now]);
    }

    /// Plays `fault` on every request for `times`.
    pub fn fault(&self, fault: Fault, times: Times) {
        self.fault_aimed(fault, Aim::All, times);
    }

    /// Plays `fault` on the requests `aim` names, for `times`. Faults set first are played first.
    pub fn fault_aimed(&self, fault: Fault, aim: Aim, times: Times) {
        self.inner.lock().faults.push(Entry::new(fault, aim, times));
    }

    /// The PR's node answers `null` with an error of `type_` (`NOT_FOUND`, `FORBIDDEN`), by node
    /// and by name, on the requests `aim` names, for `times` (`World::node_error` for each; don't
    /// also set that PR's node error on the world).
    pub fn pr_error(&self, pr: PrId, type_: &str, aim: Aim, times: Times) {
        let what = PrFault::Node {
            pr,
            type_: type_.to_owned(),
        };
        self.inner
            .lock()
            .pr_faults
            .push(Entry::new(what, aim, times));
    }

    /// The PR's field `field` answers an error of `type_` inside its node, on the requests `aim`
    /// names, for `times` (`World::field_error` for each).
    pub fn field_error(&self, pr: PrId, field: &str, type_: &str, aim: Aim, times: Times) {
        let what = PrFault::Field {
            pr,
            field: field.to_owned(),
            type_: type_.to_owned(),
        };
        self.inner
            .lock()
            .pr_faults
            .push(Entry::new(what, aim, times));
    }

    /// Clears every fault, the world's under an aim too. A hang ends: its connection is closed.
    pub fn clear_faults(&self) {
        {
            let mut st = self.inner.lock();
            st.faults.clear();
            st.pr_faults.clear();
        }
        self.inner.cleared.send_modify(|g| *g += 1);
    }

    /// A refused connection, until `listen`: the listener is closed, and every open connection
    /// with it, so a pooled one can't be reused. Returns once they're closed.
    pub async fn refuse(&self) {
        let task = self
            .inner
            .accept
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(task) = task {
            task.abort();
            // The listener is closed once the aborted task is gone.
            let _ = task.await;
        }
        self.inner.kill_all();
        // Returns once every connection is closed, so the next request can't reuse one. Each
        // closes at its task's next poll; the yields let those tasks, and the clients', run.
        for _ in 0..10_000 {
            if self.inner.open_conns() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    /// Listens again on the same port, so the URL stays valid.
    pub async fn listen(&self) {
        if self
            .inner
            .accept
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            return;
        }
        // A test can't go on if its port can't be had back.
        let listener = bind(self.inner.addr).expect("the fake GitHub binds its port again");
        serve::spawn_accept(&self.inner, listener);
    }

    /// The hits on a redirect's target so far (OPS-16: circitor never follows one).
    pub fn redirect_hits(&self) -> u32 {
        self.inner.redirect_hits.load(Ordering::SeqCst)
    }

    /// The next request not yet held waits until the handle is released, then is answered from
    /// the world as it is then. Excused from the work in flight while held (TST-4).
    pub fn hold_next(&self) -> Hold {
        let (tx, rx) = oneshot::channel();
        let (arrived_tx, arrived_rx) = watch::channel(false);
        self.inner.lock().holds.push_back(HoldSlot {
            release: rx,
            arrived: arrived_tx,
        });
        Hold {
            release: Some(tx),
            arrived: arrived_rx,
        }
    }

    /// Requests held by script now: holds and hangs.
    pub fn held(&self) -> usize {
        *self.inner.held.borrow()
    }

    /// Waits, in real time, until at least `n` requests are held by script.
    pub async fn until_held(&self, n: usize) {
        let mut rx = self.inner.held.subscribe();
        let _ = rx.wait_for(|held| *held >= n).await;
    }

    /// Only these tokens are accepted: any other `Authorization` gets the 401.
    pub fn accept_only(&self, tokens: &[&str]) {
        self.inner.lock().tokens = Some(tokens.iter().map(|t| (*t).to_owned()).collect());
    }

    /// Any token is accepted again (the default).
    pub fn accept_any(&self) {
        self.inner.lock().tokens = None;
    }

    /// Every request so far, faulted ones too (TST-6).
    pub fn requests(&self) -> Vec<Request> {
        self.inner.lock().log.clone()
    }

    /// The sum of the requests' costs.
    pub fn points(&self) -> u32 {
        self.inner.lock().log.iter().map(|r| r.cost).sum()
    }

    /// Starts the per-daemon checks afresh (a start, a restart, a crash).
    pub fn new_daemon(&self) {
        self.inner.lock().rules.reset();
    }

    /// The rules broken so far, each with the offending request.
    pub fn broken(&self) -> Vec<String> {
        self.inner.lock().broken.clone()
    }

    /// The rules broken so far, cleared: for a test that breaks one on purpose.
    pub fn take_broken(&self) -> Vec<String> {
        std::mem::take(&mut self.inner.lock().broken)
    }

    /// Panics if a rule was broken, naming each with its request.
    pub fn check(&self) {
        let broken = self.broken();
        if !broken.is_empty() {
            panic!(
                "the fake GitHub saw {} broken rule(s):\n{}",
                broken.len(),
                broken.join("\n")
            );
        }
    }
}

// TST-4: the fake GitHub is a probe for the test clock.
impl SettleProbe for FakeGitHub {
    fn settled(&self) -> bool {
        self.inner.settled()
    }
}

impl Drop for FakeGitHub {
    fn drop(&mut self) {
        if let Some(task) = self
            .inner
            .accept
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            task.abort();
        }
        self.inner.kill_all();
        // TST-3: a broken rule fails the test at drop, but never on top of another panic.
        if !std::thread::panicking() {
            self.check();
        }
    }
}

/// A listener on `addr` with `SO_REUSEADDR`, so the port can be had back after a refusal.
fn bind(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(1024)
}
