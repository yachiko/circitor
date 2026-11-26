#![cfg(feature = "test-endpoint")]
//! T28: an in-process daemon from `start`, on the test clock, serving its real socket. GitHub is
//! `ScriptedGitHub` answering from a small world; the token `FakeTokenSource`; notices go to a
//! `RecordingPoster`. Requests go through `transport`. Every PR comes in through a poll's finding.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;

use circitor::api::condition::{Condition, Progress};
use circitor::api::{Ask, Request, Until};
use circitor::clock::{Clock, SharedClock, TestClock, Time};
use circitor::config::{Config, Paths};
use circitor::daemon::instance::is_held;
use circitor::daemon::{Deps, Mode, Running, StartError, StopCause, start};
use circitor::github::fake::{ScriptedGitHub, answers};
use circitor::github::{Answer, Failure, Found, FoundPr, NodeAnswer, Query, Want};
use circitor::log::{BufferSink, Level, Logger};
use circitor::model::{NodeId, PrName, PrRead, ReadState};
use circitor::notify::fake::RecordingPoster;
use circitor::notify::{DEFAULT_LIFE, Looked, NewSubscription, NotifyHandle};
use circitor::state::file::{FaultFs, Step};
use circitor::state::{Cursor, Loaded, RealFs, StateFs, load};
use circitor::status::Warnings;
use circitor::testkit::{ShortDir, TEST_VERSION};
use circitor::token::fake::FakeTokenSource;
use circitor::transport::{RealProbe, connect};

const PR1: &str = "o/r#1";
const PR2: &str = "o/r#2";
const SOCK_A: &str = "/tmp/ct-session-a.sock";
const SOCK_B: &str = "/tmp/ct-session-b.sock";

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

fn id(n: &str) -> NodeId {
    answers::pr_read(n).id
}

fn name(n: &str) -> PrName {
    PrName::parse(n).unwrap()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- the world GitHub answers from ----

#[derive(Default)]
struct World {
    prs: BTreeMap<NodeId, PrRead>,
    /// Nodes that answer an error in place of their PR.
    errors: HashSet<NodeId>,
}

impl World {
    fn of(names: &[&str]) -> World {
        let mut w = World::default();
        for (i, n) in names.iter().enumerate() {
            let mut r = answers::pr_read(n);
            r.updated_at = Some(t0().plus(s(i as u64)));
            w.prs.insert(r.id.clone(), r);
        }
        w
    }

    fn set_state(&mut self, n: &str, state: ReadState) {
        self.prs.get_mut(&id(n)).unwrap().state = state;
    }

    fn answer(&self, q: &Query) -> Result<Answer, Failure> {
        let found = q.find.as_ref().map(|_| Found {
            login: "octo".to_string(),
            prs: self
                .prs
                .values()
                .filter(|r| r.state == ReadState::Open)
                .map(|r| FoundPr {
                    id: r.id.clone(),
                    pr: r.pr.clone(),
                    updated_at: r.updated_at,
                })
                .collect(),
            next: None,
        });
        let nodes = q
            .nodes
            .iter()
            .map(|n| {
                if self.errors.contains(&n.id) {
                    return NodeAnswer::Error {
                        type_: "INTERNAL".to_string(),
                    };
                }
                match &n.want {
                    Want::Record | Want::RecordAndItems => match self.prs.get(&n.id) {
                        Some(r) => NodeAnswer::Record {
                            read: Box::new(r.clone()),
                            items: (n.want == Want::RecordAndItems).then(Default::default),
                        },
                        None => NodeAnswer::NotFound,
                    },
                    Want::Items => NodeAnswer::Items(Default::default()),
                    Want::ChecksPage { .. } => {
                        NodeAnswer::ChecksPage(answers::pr_read("o/r#9").checks)
                    }
                }
            })
            .collect();
        Ok(Answer {
            rate: Some(answers::rate(1, 4_999)),
            date: None,
            found,
            nodes,
            lookup: None,
        })
    }
}

// ---- the rig: one clock and one directory, any number of daemons ----

struct Rig {
    clock: Arc<TestClock>,
    dir: ShortDir,
}

struct Opts {
    mode: Mode,
    exit_after: Option<Duration>,
    stop_delay: Option<Duration>,
    fs: Option<Arc<dyn StateFs>>,
    token_delay: Option<Duration>,
    world: World,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            mode: Mode::Spawned,
            exit_after: None,
            stop_delay: None,
            fs: None,
            token_delay: None,
            world: World::of(&[PR1, PR2]),
        }
    }
}

struct Daemon {
    running: Running,
    sink: BufferSink,
    poster: Arc<RecordingPoster>,
    world: Arc<Mutex<World>>,
    socket: PathBuf,
}

impl Rig {
    fn new() -> Rig {
        Rig {
            clock: TestClock::new(t0()),
            dir: ShortDir::new(),
        }
    }

    fn paths(&self) -> Paths {
        let state_dir = self.dir.path().join("state");
        Paths {
            config_file: self.dir.path().join("config.toml"),
            state_file: state_dir.join("state.json"),
            lock_file: state_dir.join("circitor.lock"),
            log_file: state_dir.join("circitor.log"),
            socket: state_dir.join("circitor.sock"),
            socket_dir: None,
            state_dir,
        }
    }

    /// What `start` is handed, and the handles the test keeps.
    fn deps(&self, opts: Opts) -> (Deps, BufferSink, Arc<RecordingPoster>, Arc<Mutex<World>>) {
        let shared: SharedClock = self.clock.clone();
        let sink = BufferSink::default();
        let world = Arc::new(Mutex::new(opts.world));
        let w = world.clone();
        let github = ScriptedGitHub::new(move |q| lock(&w).answer(q));
        let token = FakeTokenSource::new(shared.clone());
        if let Some(d) = opts.token_delay {
            token.set_delay(d);
        }
        let poster = RecordingPoster::new(shared.clone());
        let config = Config {
            exit_after: opts.exit_after,
            ..Config::default()
        };
        let deps = Deps {
            clock: shared.clone(),
            activity: self.clock.activity(),
            github,
            token_source: token,
            fs: opts.fs.unwrap_or_else(|| Arc::new(RealFs)),
            probe: Arc::new(RealProbe { clock: shared }),
            poster: poster.clone(),
            paths: self.paths(),
            config,
            mode: opts.mode,
            version: TEST_VERSION.to_string(),
            log: Arc::new(sink.clone()),
            level: Level::Info,
            seed: 7,
            signals: false,
            stop_delay: opts.stop_delay,
        };
        (deps, sink, poster, world)
    }

    async fn try_start(&self, opts: Opts) -> Result<Daemon, (StartError, BufferSink)> {
        self.launch(opts, true).await
    }

    /// `first_poll`: settle and give the first poll its second. A start run in a task while the
    /// test moves the clock passes `false`.
    async fn launch(
        &self,
        opts: Opts,
        first_poll: bool,
    ) -> Result<Daemon, (StartError, BufferSink)> {
        let (deps, sink, poster, world) = self.deps(opts);
        let socket = deps.paths.socket.clone();
        match start(deps).await {
            Ok(running) => {
                if first_poll {
                    // POL-2: the first poll's finding, then 1 s later its refreshes (API-18).
                    self.clock.settle().await;
                    self.clock.advance(s(1)).await;
                }
                Ok(Daemon {
                    running,
                    sink,
                    poster,
                    world,
                    socket,
                })
            }
            Err(e) => Err((e, sink)),
        }
    }

    async fn start(&self, opts: Opts) -> Daemon {
        match self.try_start(opts).await {
            Ok(d) => d,
            Err((e, sink)) => panic!("start: {e}\n{}", sink.contents()),
        }
    }

    async fn advance(&self, d: Duration) {
        self.clock.advance(d).await;
    }

    /// The state file, loaded as the next daemon would.
    fn saved(&self) -> Option<Vec<String>> {
        let shared: SharedClock = self.clock.clone();
        let logger = Logger::new(shared.clone(), Arc::new(BufferSink::default()), Level::Info);
        let warnings = Warnings::new(shared, logger.clone());
        let path = self.paths().state_file;
        if !path.exists() {
            return None;
        }
        match load(&RealFs, &path, &logger, &warnings) {
            Loaded::Content(c) => Some(c.prs.iter().map(|e| e.pr.to_string()).collect()),
            _ => panic!("state.json doesn't load"),
        }
    }

    fn state_dir_names(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.paths().state_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

/// Waits, in real time, for real I/O the clock doesn't count: a bounded number of yields.
async fn until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..20_000 {
        if f() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("never: {what}");
}

async fn call(socket: &Path, req: &Request) -> (u16, Value) {
    let head = connect(socket).await.unwrap().send(req).await.unwrap();
    let status = head.status();
    let body = head.body().await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn status(socket: &Path) -> Value {
    let (code, v) = call(socket, &Request::Status).await;
    assert_eq!(code, 200, "{v}");
    v
}

async fn list(socket: &Path) -> Value {
    let (code, v) = call(
        socket,
        &Request::List {
            all: false,
            since: None,
            checks: false,
        },
    )
    .await;
    assert_eq!(code, 200, "{v}");
    v
}

fn wait_req(pr: &str, until: Until, timeout: u32) -> Request {
    Request::Wait {
        pr: name(pr),
        cond: Ask::new(until),
        timeout,
        limit: None,
    }
}

/// A held wait: its head has arrived, with its cursor; its body is read in a task of its own.
struct HeldWait {
    cursor: Option<String>,
    body: tokio::task::JoinHandle<Result<Vec<u8>, circitor::transport::Dropped>>,
}

async fn hold_wait(socket: &Path, pr: &str, until: Until, timeout: u32) -> HeldWait {
    let head = connect(socket)
        .await
        .unwrap()
        .send(&wait_req(pr, until, timeout))
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    let cursor = head.cursor().map(str::to_string);
    HeldWait {
        cursor,
        body: tokio::spawn(head.body()),
    }
}

fn wait_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

impl Daemon {
    fn log(&self) -> String {
        self.sink.contents()
    }

    fn world(&self) -> MutexGuard<'_, World> {
        lock(&self.world)
    }

    /// A subscription on `pr` for `socket`, as the notify route would hand it over (T47 isn't a
    /// dependency): checked, its socket looked at, its cursor the daemon's at registering. The route's
    /// one evaluation is left to the notifier's first wake: the PR is open, so it finds nothing either.
    async fn subscribe(&self, rig: &Rig, pr: &str, socket: &str) {
        let notifier = self.running.notifier();
        let cond = Condition::check(Ask::new(Until::Merged)).unwrap();
        let cursor: Cursor = list(&self.socket).await["cursor"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let socket = PathBuf::from(socket);
        let Looked::Socket(socket_id) = notifier.look(&socket).await else {
            panic!("the fake's socket");
        };
        let new = NewSubscription {
            id: id(pr),
            pr: name(pr),
            cond,
            progress: Progress::new(rig.clock.mono()),
            cursor,
            life: DEFAULT_LIFE,
            life_given: false,
            socket,
            socket_id,
            token: None,
        };
        notifier.register(new).await.unwrap();
        rig.clock.settle().await;
    }
}

// ---- serving ----

// T28: two PRs found by the first poll; `list` and `show` answer over the socket.
#[tokio::test]
async fn m2_list_and_show() {
    let rig = Rig::new();
    let d = rig.start(Opts::default()).await;
    let v = list(&d.socket).await;
    let names: Vec<&str> = v["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["pr"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{v}");
    assert!(names.contains(&PR1) && names.contains(&PR2), "{v}");
    assert_eq!(v["version"], TEST_VERSION);

    let (code, v) = call(
        &d.socket,
        &Request::Show {
            pr: name(PR1),
            fresh: false,
        },
    )
    .await;
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["record"]["pr"], PR1);
}

// API-9, via the daemon: a `merged` wait whose PR merges, seen by a later poll.
#[tokio::test]
async fn m2_wait_satisfied() {
    let rig = Rig::new();
    let d = rig.start(Opts::default()).await;
    let w = hold_wait(&d.socket, PR1, Until::Merged, 600).await;
    assert!(w.cursor.is_some());
    rig.clock.settle().await;
    assert!(!w.body.is_finished());
    d.world().set_state(PR1, ReadState::Merged);
    for _ in 0..12 {
        if w.body.is_finished() {
            break;
        }
        rig.advance(s(10)).await;
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }
    let v = wait_json(&w.body.await.unwrap().unwrap());
    assert_eq!(v["outcome"], "satisfied", "{v}");
    assert_eq!(v["record"]["state"], "merged", "{v}");
}

// API-9: one whose PR closes ends.
#[tokio::test]
async fn m2_wait_ended() {
    let rig = Rig::new();
    let d = rig.start(Opts::default()).await;
    let w = hold_wait(&d.socket, PR1, Until::Merged, 600).await;
    assert!(w.cursor.is_some());
    d.world().set_state(PR1, ReadState::Closed);
    for _ in 0..12 {
        if w.body.is_finished() {
            break;
        }
        rig.advance(s(10)).await;
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
    }
    let v = wait_json(&w.body.await.unwrap().unwrap());
    assert_eq!(v["outcome"], "ended", "{v}");
    assert_eq!(v["reason"], "closed", "{v}");
}

// API-12: one that ends at 100 s of the clock.
#[tokio::test]
async fn m2_wait_timeout() {
    let rig = Rig::new();
    let d = rig.start(Opts::default()).await;
    let w = hold_wait(&d.socket, PR1, Until::Merged, 100).await;
    assert!(w.cursor.is_some());
    rig.advance(s(99)).await;
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(!w.body.is_finished());
    rig.advance(s(1)).await;
    let v = wait_json(&w.body.await.unwrap().unwrap());
    assert_eq!(v["outcome"], "timeout", "{v}");
}

// API-14: a client that drops a held wait leaves no waiter.
#[tokio::test]
async fn api14_client_drop() {
    let rig = Rig::new();
    let d = rig.start(Opts::default()).await;
    let w = hold_wait(&d.socket, PR1, Until::Merged, 600).await;
    rig.clock.settle().await;
    assert_eq!(status(&d.socket).await["waiters"], 1);
    w.body.abort();
    let _ = w.body.await;
    until("the connection closed", || {
        d.running.open_connections() == 0
    })
    .await;
    rig.clock.settle().await;
    assert_eq!(status(&d.socket).await["waiters"], 0);
}

// API-14, OPS-7: at a stop, a held wait's connection is closed without an answer.
#[tokio::test]
async fn api14_ops7_stop_closes_waiters() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    let w = hold_wait(&d.socket, PR1, Until::Merged, 600).await;
    rig.clock.settle().await;
    d.running.stop(StopCause::Test);
    assert!(w.body.await.unwrap().is_err());
    assert_eq!(d.running.exited().await, 0);
}

// API-17: `POST /v1/shutdown` answers in full, then the daemon stops as a signal would.
#[tokio::test]
async fn api17_shutdown_answers_then_stops() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    let (code, v) = call(&d.socket, &Request::Shutdown).await;
    assert_eq!(code, 200);
    assert_eq!(v["version"], TEST_VERSION, "{v}");
    assert_eq!(d.running.exited().await, 0);
    assert!(d.log().contains("info stop cause=request"), "{}", d.log());
}

// OPS-7, STA-11, STA-12: after a stop the socket is gone, the file is written, the lock is free, and
// the state directory holds nothing else.
#[tokio::test]
async fn ops7_stop_leaves_no_socket_and_a_state_file() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    assert!(d.socket.exists());
    d.running.stop(StopCause::Test);
    assert_eq!(d.running.exited().await, 0);
    assert_eq!(d.running.exit_status(), Some(0));
    assert!(!d.socket.exists());
    let mut saved = rig.saved().unwrap();
    saved.sort();
    assert_eq!(saved, [PR1, PR2]);
    assert!(!is_held(&rig.paths().lock_file).unwrap());
    assert_eq!(rig.state_dir_names(), ["circitor.lock", "state.json"]);
}

// STA-8, API-18: a new start names the saved PR in `pending` until its first refresh.
#[tokio::test]
async fn m2_restart_loads_it() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    d.running.stop(StopCause::Test);
    assert_eq!(d.running.exited().await, 0);

    // The second daemon's GitHub can't read PR1 yet.
    let mut world = World::of(&[PR1, PR2]);
    world.errors.insert(id(PR1));
    let d = rig
        .start(Opts {
            world,
            ..Opts::default()
        })
        .await;
    let v = list(&d.socket).await;
    assert_eq!(v["pending"], serde_json::json!([PR1]), "{v}");
    d.world().errors.clear();
    rig.advance(s(31)).await;
    let v = list(&d.socket).await;
    assert_eq!(v["pending"], serde_json::json!([]), "{v}");
    assert_eq!(v["records"].as_array().unwrap().len(), 2, "{v}");
}

// OPS-7: the stop has 5 s; past them it's logged and the status is still 0.
#[tokio::test]
async fn ops7_deadline() {
    let rig = Rig::new();
    let mut d = rig
        .start(Opts {
            stop_delay: Some(s(10)),
            ..Opts::default()
        })
        .await;
    d.running.stop(StopCause::Test);
    rig.advance(ms(4_900)).await;
    assert_eq!(d.running.exit_status(), None);
    rig.advance(ms(100)).await;
    until("the stop's end", || d.running.exit_status().is_some()).await;
    assert_eq!(d.running.exited().await, 0);
    assert!(d.log().contains("warn stop deadline"), "{}", d.log());
    assert!(!is_held(&rig.paths().lock_file).unwrap());
}

// OPS-7: a second request during the stop ends it at once.
#[tokio::test]
async fn ops7_second_request() {
    let rig = Rig::new();
    let mut d = rig
        .start(Opts {
            stop_delay: Some(s(10)),
            ..Opts::default()
        })
        .await;
    d.running.stop(StopCause::Test);
    rig.advance(s(1)).await;
    assert_eq!(d.running.exit_status(), None);
    let before = rig.clock.mono();
    d.running.stop(StopCause::Test);
    assert_eq!(d.running.exited().await, 0);
    assert_eq!(rig.clock.mono(), before);
    assert!(d.log().contains("warn stop deadline"), "{}", d.log());
}

// TST-10, OPS-7: a final write that fails is logged; the stop still exits 0.
#[tokio::test]
async fn tst10_failed_final_write() {
    let rig = Rig::new();
    let fs = Arc::new(FaultFs::new());
    fs.fail_at(Step::WriteTmp);
    let mut d = rig
        .start(Opts {
            fs: Some(fs.clone()),
            ..Opts::default()
        })
        .await;
    d.running.stop(StopCause::Test);
    assert_eq!(d.running.exited().await, 0);
    assert!(
        d.log().contains("warn state file write failed"),
        "{}",
        d.log()
    );
    assert!(fs.file(&rig.paths().state_file).is_none());
}

// OPS-8: a spawned daemon exits after `exit_after` without a request; a held wait keeps it, and it
// exits `exit_after` after the wait ended; `status` doesn't count; a foreground one never exits idle.
#[tokio::test]
async fn ops8_idle_exit() {
    let opts = || Opts {
        exit_after: Some(s(300)),
        ..Opts::default()
    };
    // No request for 300 s.
    let rig = Rig::new();
    let mut d = rig.start(opts()).await;
    // `start` took the first second.
    rig.advance(s(298)).await;
    assert_eq!(d.running.exit_status(), None);
    rig.advance(s(1)).await;
    until("the idle exit", || d.running.exit_status().is_some()).await;
    assert_eq!(d.running.exited().await, 0);
    assert!(d.log().contains("info stop cause=idle"), "{}", d.log());
    assert!(rig.saved().is_some());

    // A held wait prevents it; `status` calls don't count.
    let rig = Rig::new();
    let mut d = rig.start(opts()).await;
    rig.advance(s(9)).await;
    let w = hold_wait(&d.socket, PR1, Until::Merged, 3_600).await;
    for _ in 0..39 {
        rig.advance(s(10)).await;
        status(&d.socket).await;
    }
    // 400 s: the wait goes.
    assert_eq!(d.running.exit_status(), None);
    w.body.abort();
    let _ = w.body.await;
    until("the connection closed", || {
        d.running.open_connections() == 0
    })
    .await;
    rig.clock.settle().await;
    assert_eq!(status(&d.socket).await["waiters"], 0);
    for _ in 0..29 {
        rig.advance(s(10)).await;
        status(&d.socket).await;
    }
    rig.advance(s(9)).await;
    assert_eq!(d.running.exit_status(), None, "{}", d.log());
    rig.advance(s(1)).await;
    until("the idle exit", || d.running.exit_status().is_some()).await;
    assert_eq!(d.running.exited().await, 0);

    // A foreground daemon doesn't exit.
    let rig = Rig::new();
    let d = rig
        .start(Opts {
            mode: Mode::Foreground,
            ..opts()
        })
        .await;
    rig.advance(s(1_000)).await;
    assert_eq!(d.running.exit_status(), None);
}

// NTF-5, OPS-8: an open subscription keeps a spawned daemon from its idle exit; once it ends, the
// daemon exits `exit_after` later.
#[tokio::test]
async fn ntf5_a_subscription_prevents_the_idle_exit() {
    let rig = Rig::new();
    let mut d = rig
        .start(Opts {
            exit_after: Some(s(300)),
            ..Opts::default()
        })
        .await;
    d.subscribe(&rig, PR1, SOCK_A).await;
    for _ in 0..20 {
        rig.advance(s(60)).await;
    }
    assert_eq!(d.running.exit_status(), None);
    assert_eq!(status(&d.socket).await["subscriptions"], 1);
    let cancelled = d
        .running
        .notifier()
        .cancel(&name(PR1), Some(&id(PR1)), Path::new(SOCK_A));
    assert_eq!(cancelled, 1);
    rig.advance(s(299)).await;
    assert_eq!(d.running.exit_status(), None);
    rig.advance(s(1)).await;
    until("the idle exit", || d.running.exit_status().is_some()).await;
    assert_eq!(d.running.exited().await, 0);
}

fn stopped(d: &Daemon) -> Vec<String> {
    d.poster
        .posts()
        .into_iter()
        .map(|p| p.text)
        .filter(|t| t.contains("the daemon is stopping"))
        .collect()
}

// NTF-10: a stop tells every session before it closes a connection, within its 1 s.
#[tokio::test]
async fn ntf10_stop_tells_each_session_first() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    d.subscribe(&rig, PR1, SOCK_A).await;
    d.subscribe(&rig, PR2, SOCK_B).await;
    let w = hold_wait(&d.socket, PR1, Until::Merged, 600).await;
    rig.clock.settle().await;
    let release = d.poster.stall(&rig.clock.activity());
    d.running.stop(StopCause::Test);
    rig.advance(ms(500)).await;
    assert_eq!(stopped(&d).len(), 2, "{:?}", d.poster.posts());
    assert_eq!(d.running.open_connections(), 1);
    assert!(!w.body.is_finished());
    rig.advance(ms(500)).await;
    assert!(w.body.await.unwrap().is_err());
    assert_eq!(d.running.exited().await, 0);
    drop(release);
    let saved = rig.saved().unwrap();
    assert_eq!(saved.len(), 2);
    let raw = std::fs::read_to_string(rig.paths().state_file).unwrap();
    assert!(!raw.contains("ct-session"), "{raw}");
    assert!(!d.log().contains("stop deadline"), "{}", d.log());

    // Without the stall: both posted, no clock time.
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    d.subscribe(&rig, PR1, SOCK_A).await;
    d.subscribe(&rig, PR2, SOCK_B).await;
    let before = rig.clock.mono();
    d.running.stop(StopCause::Test);
    assert_eq!(d.running.exited().await, 0);
    assert_eq!(rig.clock.mono(), before);
    assert_eq!(stopped(&d).len(), 2);

    // With no subscription, nothing is posted.
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    d.running.stop(StopCause::Test);
    assert_eq!(d.running.exited().await, 0);
    assert!(d.poster.posts().is_empty());
}

// NTF-10, TST-5: a crash tells nobody, and the next daemon has no subscription.
#[tokio::test]
async fn ntf10_crash_tells_nobody() {
    let rig = Rig::new();
    let d = rig.start(Opts::default()).await;
    d.subscribe(&rig, PR1, SOCK_A).await;
    let poster = d.poster.clone();
    d.running.crash().await;
    let posts = poster.posts();
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert!(posts[0].text.starts_with("circitor: watching"), "{posts:?}");
    let d = rig.start(Opts::default()).await;
    assert_eq!(status(&d.socket).await["subscriptions"], 0);
}

// NTF-10: a stop the daemon chose itself posts the command without `--no-spawn`; one asked for, with it.
#[tokio::test]
async fn ntf10_a_stop_it_chose() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    d.subscribe(&rig, PR1, SOCK_A).await;
    std::fs::remove_file(&d.socket).unwrap();
    rig.advance(s(60)).await;
    until("the stop", || d.running.exit_status().is_some()).await;
    assert_eq!(d.running.exited().await, 0);
    let notices = stopped(&d);
    assert_eq!(notices.len(), 1);
    assert!(!notices[0].ends_with("--no-spawn"), "{notices:?}");
    assert!(d.log().contains("info stop cause=files"), "{}", d.log());

    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    d.subscribe(&rig, PR1, SOCK_A).await;
    let (code, _) = call(&d.socket, &Request::Shutdown).await;
    assert_eq!(code, 200);
    assert_eq!(d.running.exited().await, 0);
    let notices = stopped(&d);
    assert_eq!(notices.len(), 1);
    assert!(notices[0].ends_with(" --no-spawn"), "{notices:?}");
}

// OPS-5: the token is read after the bind, and `start` never waits for it.
#[tokio::test]
async fn ops5_token_after_bind() {
    let rig = Rig::new();
    let d = rig
        .start(Opts {
            token_delay: Some(s(10)),
            ..Opts::default()
        })
        .await;
    let before = rig.clock.mono();
    let v = status(&d.socket).await;
    assert_eq!(rig.clock.mono(), before);
    assert!(v["daemon"]["problem"].is_null(), "{v}");
}

// STA-11: a second spawned daemon exits 0 once the lock has been busy for 6 s and its holder answers.
#[tokio::test]
async fn sta11_second_spawned() {
    let rig = Arc::new(Rig::new());
    let d = rig.start(Opts::default()).await;
    let r = rig.clone();
    let second = tokio::spawn(async move { r.try_start(Opts::default()).await.err() });
    rig.advance(ms(5_900)).await;
    assert!(!second.is_finished());
    rig.advance(ms(100)).await;
    let (e, _) = second.await.unwrap().expect("refused");
    assert_eq!(e.code(), 0, "{e}");
    assert_eq!(status(&d.socket).await["mode"], "spawned");
}

// OPS-6: a foreground daemon takes over from a spawned one, within 15 s.
#[tokio::test]
async fn ops6_takeover() {
    let rig = Arc::new(Rig::new());
    let mut first = rig.start(Opts::default()).await;
    let r = rig.clone();
    let second = tokio::spawn(async move {
        r.launch(
            Opts {
                mode: Mode::Foreground,
                ..Opts::default()
            },
            false,
        )
        .await
        .map_err(|(e, _)| e.to_string())
    });
    let began = rig.clock.mono();
    while !second.is_finished() && rig.clock.mono().since(began) < s(15) {
        rig.advance(ms(100)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }
    assert_eq!(first.running.exited().await, 0);
    let Ok(Ok(d)) = second.await else {
        panic!("no takeover");
    };
    assert!(rig.clock.mono().since(began) <= s(15));
    assert!(
        first.log().contains("stop cause=request"),
        "{}",
        first.log()
    );
    assert_eq!(status(&d.socket).await["mode"], "foreground");
}

// OPS-6: against a foreground holder, a foreground daemon fails, naming the holder's socket.
#[tokio::test]
async fn ops6_foreground_holder() {
    let rig = Arc::new(Rig::new());
    let d = rig
        .start(Opts {
            mode: Mode::Foreground,
            ..Opts::default()
        })
        .await;
    let r = rig.clone();
    let second = tokio::spawn(async move {
        r.try_start(Opts {
            mode: Mode::Foreground,
            ..Opts::default()
        })
        .await
        .err()
    });
    let began = rig.clock.mono();
    while !second.is_finished() && rig.clock.mono().since(began) < s(16) {
        rig.advance(ms(100)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }
    let (e, _) = second.await.unwrap().expect("refused");
    assert_eq!(e.code(), 1);
    assert!(
        e.to_string().contains(&d.socket.display().to_string()),
        "{e}"
    );
    assert_eq!(status(&d.socket).await["mode"], "foreground");
}

// OPS-4, TST-5: after a crash the stale socket is replaced and the file loaded as last written.
#[tokio::test]
async fn ops4_stale_socket() {
    let rig = Rig::new();
    let d = rig.start(Opts::default()).await;
    // The saver's spacing has passed: the found PRs are written.
    rig.advance(s(6)).await;
    let written = std::fs::read(rig.paths().state_file).unwrap();
    let socket = d.socket.clone();
    d.running.crash().await;
    assert!(socket.exists());
    assert_eq!(std::fs::read(rig.paths().state_file).unwrap(), written);
    assert!(!is_held(&rig.paths().lock_file).unwrap());

    let mut world = World::of(&[PR1, PR2]);
    world.errors.insert(id(PR1));
    world.errors.insert(id(PR2));
    let d = rig
        .start(Opts {
            world,
            ..Opts::default()
        })
        .await;
    let v = list(&d.socket).await;
    assert_eq!(v["pending"], serde_json::json!([PR1, PR2]), "{v}");
}

// OPS-4: a regular file at the socket path stops the start, naming it, with the lock free again.
#[tokio::test]
async fn ops4_not_a_socket() {
    let rig = Rig::new();
    let paths = rig.paths();
    std::fs::create_dir_all(&paths.state_dir).unwrap();
    std::fs::write(&paths.socket, b"not a socket").unwrap();
    let Err((e, _)) = rig.try_start(Opts::default()).await else {
        panic!("started");
    };
    assert_eq!(e.code(), 1);
    assert!(
        e.to_string().contains(&paths.socket.display().to_string()),
        "{e}"
    );
    assert!(!is_held(&paths.lock_file).unwrap());
    assert_eq!(std::fs::read(&paths.socket).unwrap(), b"not a socket");
}

// STA-11: a lock file that can't be opened stops the start at once, naming it.
#[tokio::test]
async fn sta11_lock_unopenable() {
    let rig = Rig::new();
    let paths = rig.paths();
    std::fs::create_dir_all(&paths.lock_file).unwrap();
    let before = rig.clock.mono();
    let Err((e, _)) = rig.try_start(Opts::default()).await else {
        panic!("started");
    };
    assert_eq!(rig.clock.mono(), before);
    assert_eq!(e.code(), 1);
    assert!(
        e.to_string()
            .contains(&paths.lock_file.display().to_string()),
        "{e}"
    );
    assert!(!paths.socket.exists());
}

// STA-11: the socket removed, or the lock file replaced: within 60 s the daemon stops, and leaves a
// newer file at the socket's path alone.
#[tokio::test]
async fn sta11_files_not_own() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    std::fs::remove_file(&d.socket).unwrap();
    std::fs::write(&d.socket, b"newer").unwrap();
    rig.advance(s(60)).await;
    until("the stop", || d.running.exit_status().is_some()).await;
    assert_eq!(d.running.exited().await, 0);
    assert_eq!(std::fs::read(&d.socket).unwrap(), b"newer");
    assert!(d.log().contains("info stop cause=files"), "{}", d.log());

    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    let lock_file = rig.paths().lock_file;
    std::fs::remove_file(&lock_file).unwrap();
    std::fs::write(&lock_file, b"").unwrap();
    // `start` took the first second: the check is at 60 s.
    rig.advance(s(58)).await;
    assert_eq!(d.running.exit_status(), None);
    rig.advance(s(1)).await;
    until("the stop", || d.running.exit_status().is_some()).await;
    assert_eq!(d.running.exited().await, 0);
    assert!(!d.socket.exists());
}

// OPS-9: the start line holds the version and the socket's path; the stop line its cause.
#[tokio::test]
async fn ops9_start_and_stop_lines() {
    let rig = Rig::new();
    let mut d = rig.start(Opts::default()).await;
    let log = d.log();
    let start_line = log
        .lines()
        .find(|l| l.contains(" info start "))
        .unwrap_or_else(|| panic!("{log}"));
    assert!(start_line.contains(TEST_VERSION), "{start_line}");
    assert!(
        start_line.contains(&d.socket.display().to_string()),
        "{start_line}"
    );
    assert!(start_line.contains("mode=spawned"), "{start_line}");
    d.running.stop(StopCause::Signal(15));
    assert_eq!(d.running.exited().await, 0);
    assert!(
        d.log().contains("info stop cause=signal signal=15"),
        "{}",
        d.log()
    );
}
