//! The registry and the loop that evaluates it (T46).
//!
//! One loop evaluates; every notice goes out on its own task, apart from the loop and from the engine
//! (NTF-6, POL-2). A subscription is confirming, open or telling, and nothing else: no notice ever waits
//! to be tried again (NTF-6, NTF-10).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinHandle};

use crate::api::condition::{Condition, Progress, evaluate};
use crate::api::{Ask, Outcome};
use crate::cli::render::stale_mark;
use crate::clock::{Mono, Time, timeout};
use crate::log::Logger;
use crate::model::{NodeId, PrName, Status};
use crate::poller::{PollerStatus, ProblemCode, cadence};
use crate::state::store::Subscribed;
use crate::state::{Cursor, MAX_SUBSCRIPTIONS, View};
use crate::status::WarningCode;

use super::text::{self, Told};
use super::{
    CATCH_UP, End, Looked, NewSubscription, NotifyDeps, NotifyHandle, PER_SOCKET, POST_LIMIT,
    PROBLEM_AFTER, Posted, ProblemWhy, RegisterError, Registered, STOP_LIMIT, SessionToken,
    SocketId, TICK,
};

#[cfg(test)]
mod tests;

/// An end found, with the notice it posts (`None`: it posts nothing).
struct Ending {
    end: End,
    text: Option<String>,
}

enum State {
    /// From its entry until its confirmation's post has ended (NTF-2). An end found meanwhile waits here.
    Confirming {
        confirmed: watch::Receiver<Option<bool>>,
        ended: Option<Ending>,
    },
    /// Evaluated at every wake.
    Open,
    /// Ended; its one notice is being posted (NTF-6).
    Telling { task: Option<JoinHandle<bool>> },
}

/// An `expired` held for a catch-up (NTF-4): since when, and the PR's `timing.fetched` then.
struct Held {
    at: Mono,
    fetched: Option<Mono>,
}

// NTF-1: a subscription holds its PR by node ID and remembers the name it was registered under.
/// One subscription (NTF-1), in memory only (NTF-9): no `Serialize`, no `Debug`.
struct Sub {
    seq: u64,
    id: NodeId,
    /// The name it was registered under (NTF-3's cancel matches it).
    pr: PrName,
    cond: Condition,
    progress: Progress,
    cursor: Cursor,
    life: Duration,
    life_given: bool,
    socket: PathBuf,
    socket_id: SocketId,
    // NTF-9: the session's token, held only while the subscription lasts.
    token: Option<SessionToken>,
    /// The expiry's base: the monotonic time and the sleep noticed at registration (clock.md).
    base_mono: Mono,
    base_sleep: Duration,
    held: Option<Held>,
    stat_logged: bool,
    state: State,
    /// STA-1: the PR stays tracked while this is held; its drop is the subscription's end.
    _guard: Subscribed,
}

impl Sub {
    /// Confirming without an end yet, or open: evaluated, ticked.
    fn live(&self) -> bool {
        matches!(
            self.state,
            State::Open | State::Confirming { ended: None, .. }
        )
    }

    fn telling(&self) -> bool {
        matches!(self.state, State::Telling { .. })
    }

    /// What a notice says of it, named as its record is in `view` (MOD-1, NTF-1).
    fn told(&self, view: &View) -> Told {
        let pr = view
            .get(&self.id)
            .map(|p| p.record.pr.clone())
            .unwrap_or_else(|| self.pr.clone());
        Told {
            pr,
            cond: self.cond.ask.clone(),
            life: self.life,
            life_given: self.life_given,
            cursor: self.cursor.clone(),
        }
    }
}

#[derive(Default)]
struct Registry {
    /// In the order registered; a replacement takes the place of the one it replaced... at the end.
    subs: Vec<Sub>,
    next_seq: u64,
    /// `stop` or `abort` has begun.
    stopping: bool,
}

/// `kind` as a set of words (NTF-2).
fn kind_set(kind: &Option<String>) -> Option<BTreeSet<&str>> {
    kind.as_deref().map(|k| k.split(',').collect())
}

// NTF-2: "the same request", compared as the daemon reads it: the socket, the PR by node ID, `until`,
// `sha` lower-cased as given, `fail_fast`, and `kind` as a set.
fn same_request(sub: &Sub, id: &NodeId, socket: &Path, ask: &Ask) -> bool {
    let mine = &sub.cond.ask;
    sub.socket == socket
        && sub.id == *id
        && mine.until == ask.until
        && mine.sha.as_ref().map(|s| s.to_ascii_lowercase())
            == ask.sha.as_ref().map(|s| s.to_ascii_lowercase())
        && mine.fail_fast == ask.fail_fast
        && kind_set(&mine.kind) == kind_set(&ask.kind)
}

impl Registry {
    /// The one "the same request" names, in any state but telling (NTF-3: a telling one has ended).
    fn position_same(&self, id: &NodeId, socket: &Path, ask: &Ask) -> Option<usize> {
        self.subs
            .iter()
            .position(|s| !s.telling() && same_request(s, id, socket, ask))
    }
}

struct Shared {
    deps: NotifyDeps,
    reg: Mutex<Registry>,
    wake: tokio::sync::Notify,
}

// OPS-9: names, ends and reasons only: never a notice's text, a token or a socket's path.
fn log_end(logger: &Logger, pr: &PrName, end: End) {
    match end {
        End::Problem(why) => logger.info(format!(
            "subscription ended pr={pr} end={} why={}",
            end.as_str(),
            why.as_str()
        )),
        _ => logger.info(format!("subscription ended pr={pr} end={}", end.as_str())),
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        // Nothing panics while holding it; the registry stays consistent if something did.
        self.reg.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Any subscription confirming or open: the tick runs only then.
    fn any_live(&self) -> bool {
        self.lock().subs.iter().any(Sub::live)
    }

    /// A telling one's post has ended: it leaves, and drops its guard (STA-1's end).
    fn leave(&self, seq: u64) {
        let gone = {
            let mut reg = self.lock();
            reg.subs
                .iter()
                .position(|s| s.seq == seq && s.telling())
                .map(|i| reg.subs.remove(i))
        };
        drop(gone);
    }

    /// NTF-6: the path checked as at a tick, then one post inside `POST_LIMIT`. `true` when the line was
    /// written in full. Nothing is tried again. With `pr`, a notice not posted is logged.
    async fn deliver(
        &self,
        socket: &Path,
        socket_id: SocketId,
        token: Option<&SessionToken>,
        text: &str,
        pr: Option<&PrName>,
    ) -> bool {
        let d = &self.deps;
        // NTF-6: before connecting, a socket that isn't the one registered means the session is gone.
        let gone = match d.poster.look(socket).await {
            Looked::Missing => true,
            Looked::Socket(id) | Looked::Other(id, _) => id != socket_id,
            // NTF-4: a `stat` that fails otherwise doesn't stop the post.
            Looked::Failed(_) => false,
        };
        let reason = if gone {
            "session_gone"
        } else {
            match timeout(&d.clock, POST_LIMIT, d.poster.post(socket, token, text)).await {
                Ok(Posted::Done) => return true,
                Ok(Posted::SessionGone(_)) => "session_gone",
                Ok(Posted::Failed(_)) => "failed",
                Err(_) => "timeout",
            }
        };
        // OPS-9: never the text, the token or the socket's path.
        if let Some(pr) = pr {
            d.logger
                .warn(format!("notice not posted pr={pr} reason={reason}"));
        }
        false
    }
}

/// Starts the one post of an ended subscription, on its own task (NTF-6, POL-2): it leaves when the
/// post has ended, however it ended.
fn start_telling(sh: &Arc<Shared>, sub: &mut Sub, text: String) {
    // TST-4: a notice being posted is work in flight, from its `look` to its end.
    let busy = sh.deps.activity.begin();
    let task_sh = sh.clone();
    let socket = sub.socket.clone();
    let socket_id = sub.socket_id;
    let token = sub.token.clone();
    let pr = sub.pr.clone();
    let seq = sub.seq;
    let task = tokio::spawn(async move {
        let _busy = busy;
        let written = task_sh
            .deliver(&socket, socket_id, token.as_ref(), &text, Some(&pr))
            .await;
        drop(token);
        task_sh.leave(seq);
        written
    });
    sub.state = State::Telling { task: Some(task) };
}

/// What one wake saw, the same for every subscription (MOD-13: one view).
struct Wake<'a> {
    view: &'a View,
    tick: bool,
    /// `sleep.total()` moved since the previous wake.
    slept: bool,
    total_sleep: Duration,
    mono: Mono,
    now: Time,
    status: &'a PollerStatus,
    looks: &'a HashMap<u64, Looked>,
}

/// NTF-4's ends, in order; the first that applies. `None`: it stays.
fn decide(d: &NotifyDeps, sub: &mut Sub, w: &Wake<'_>) -> Option<Ending> {
    let pv = w.view.get(&sub.id);

    // NTF-4 step 1: the function the waiters call.
    if let Some((outcome, reason)) = evaluate(&sub.cond, w.view, &sub.id, &mut sub.progress) {
        let end = match (outcome, reason) {
            (Outcome::Ended, Some(r)) => End::Ended(r),
            // `evaluate` gives a reason with every `Ended`, and never `Timeout`.
            _ => End::Satisfied,
        };
        let told = sub.told(w.view);
        // NTF-8, STA-9: a cursor too old as the notice is built names `show`.
        let history = !w.view.too_old(&sub.cursor);
        let record = pv.map(|p| &*p.record);
        return Some(Ending {
            end,
            text: Some(text::outcome(&told, (outcome, reason), record, history)),
        });
    }

    if w.tick {
        // NTF-4 step 2: the socket, a `stat`, never a connection.
        match w.looks.get(&sub.seq) {
            Some(Looked::Missing) => {
                return Some(Ending {
                    end: End::SessionGone,
                    text: None,
                });
            }
            Some(Looked::Socket(id) | Looked::Other(id, _)) if *id != sub.socket_id => {
                return Some(Ending {
                    end: End::SessionGone,
                    text: None,
                });
            }
            // NTF-4: a `stat` that fails otherwise changes nothing at that tick, and is logged once.
            Some(Looked::Failed(_)) if !sub.stat_logged => {
                sub.stat_logged = true;
                d.logger.warn(format!("socket not checked pr={}", sub.pr));
            }
            _ => {}
        }

        // NTF-4 step 3: `problem`, what only the user can end.
        let why = if d
            .warnings
            .since(WarningCode::PrFailing, &sub.id.0)
            .is_some()
        {
            Some(ProblemWhy::PrFailing)
        } else {
            // NTF-4, POL-26: 10 minutes from when `auth` began, not from when this loop saw it.
            let auth = w
                .status
                .problem
                .as_ref()
                .is_some_and(|p| p.code == ProblemCode::Auth);
            let long = w
                .status
                .problem_since
                .is_some_and(|since| w.mono.since(since) >= PROBLEM_AFTER);
            (auth && long).then_some(ProblemWhy::Auth)
        };
        if let Some(why) = why {
            let told = sub.told(w.view);
            return Some(Ending {
                end: End::Problem(why),
                text: Some(text::problem(&told, why)),
            });
        }
    }

    // NTF-4 step 4: `for`, on the monotonic clock plus the sleep noticed, never the wall clock.
    let expire = match &sub.held {
        // NTF-4: expiry never beats a catch-up: the PR's own next applied refresh, or `CATCH_UP`.
        Some(h) => {
            let fetched = pv.and_then(|p| p.timing.fetched);
            fetched > h.fetched || w.mono.since(h.at) >= CATCH_UP
        }
        None if w.tick || w.slept => {
            let ran = w.mono.since(sub.base_mono) + w.total_sleep.saturating_sub(sub.base_sleep);
            if ran < sub.life {
                false
            } else {
                let stale = pv.is_some_and(|p| {
                    cadence::stale(&p.record, &p.timing, w.status.tier, &d.config, w.mono)
                });
                if w.slept || stale {
                    sub.held = Some(Held {
                        at: w.mono,
                        fetched: pv.and_then(|p| p.timing.fetched),
                    });
                    false
                } else {
                    true
                }
            }
        }
        None => false,
    };
    if !expire {
        return None;
    }
    let told = sub.told(w.view);
    let (status, mark) = match pv {
        Some(p) => {
            // NTF-8: the record's `status`, with CLI-3's stale mark when it's stale (as `present` sets it).
            let mut record = (*p.record).clone();
            record.stale = cadence::stale(&record, &p.timing, w.status.tier, &d.config, w.mono);
            (record.status, stale_mark(&record, w.now))
        }
        // A subscription's PR always has a record (NTF-2); this is only a fallback.
        None => (Status::Unknown, String::new()),
    };
    Some(Ending {
        end: End::Expired,
        text: Some(text::expired(&told, status, &mark)),
    })
}

/// One wake of the loop (NTF-4): one view, every confirming and open subscription in the order registered.
async fn wake_once(sh: &Arc<Shared>, tick: bool, slept: bool) {
    let d = &sh.deps;
    // NTF-4: at a tick every socket is looked at, a `stat` each; no lock across the await.
    let mut looks = HashMap::new();
    if tick {
        let targets: Vec<(u64, PathBuf)> = sh
            .lock()
            .subs
            .iter()
            .filter(|s| s.live())
            .map(|s| (s.seq, s.socket.clone()))
            .collect();
        for (seq, socket) in targets {
            looks.insert(seq, d.poster.look(&socket).await);
        }
    }
    let view = d.store.view();
    let status = d.poller.status();
    let w = Wake {
        view: &view,
        tick,
        slept,
        total_sleep: d.sleep.total(),
        mono: d.clock.mono(),
        now: d.clock.wall(),
        status: &status,
        looks: &looks,
    };
    let mut gone = Vec::new();
    {
        let mut reg = sh.lock();
        if reg.stopping {
            return;
        }
        let mut i = 0;
        while i < reg.subs.len() {
            let sub = &mut reg.subs[i];
            if !sub.live() {
                i += 1;
                continue;
            }
            let Some(ending) = decide(d, sub, &w) else {
                i += 1;
                continue;
            };
            log_end(&d.logger, &sub.pr, ending.end);
            match (&mut sub.state, ending.text) {
                // NTF-2: an end found while confirming waits for the confirmation.
                (State::Confirming { ended, .. }, text) => {
                    *ended = Some(Ending {
                        end: ending.end,
                        text,
                    });
                    i += 1;
                }
                (_, Some(text)) => {
                    start_telling(sh, sub, text);
                    i += 1;
                }
                // Session gone: it leaves at once, nothing posted.
                (_, None) => gone.push(reg.subs.remove(i)),
            }
        }
    }
    drop(gone);
}

/// The loop (NTF-4): wakes on `store.changed()`, a registration, `sleep.subscribe()`, and every `TICK`
/// while a subscription is held; with none it keeps no timer.
async fn run(sh: Arc<Shared>) {
    let d = &sh.deps;
    let mut changed = d.store.changed();
    let mut slept_rx = d.sleep.subscribe();
    let mut last_total = d.sleep.total();
    let mut next_tick: Option<Mono> = None;
    loop {
        if sh.any_live() {
            if next_tick.is_none() {
                next_tick = Some(d.clock.mono().plus(TICK));
            }
        } else {
            next_tick = None;
        }
        let timer: BoxFuture<'static, ()> = match next_tick {
            Some(at) => d.clock.sleep_until(at),
            None => futures::future::pending().boxed(),
        };
        let tick = tokio::select! {
            biased;
            r = changed.changed() => {
                if r.is_err() {
                    return;
                }
                false
            }
            r = slept_rx.changed() => {
                if r.is_err() {
                    return;
                }
                false
            }
            () = sh.wake.notified() => false,
            () = timer => true,
        };
        // TST-4: work in flight from waking until parking again, taken before the next `.await`.
        let _busy = d.activity.begin();
        // Marked seen before evaluating, so a refresh applied meanwhile wakes the loop again.
        changed.borrow_and_update();
        slept_rx.borrow_and_update();
        let mono = d.clock.mono();
        if tick {
            let next = next_tick.unwrap_or(mono).plus(TICK);
            next_tick = Some(if next <= mono { mono.plus(TICK) } else { next });
        }
        if sh.lock().stopping {
            return;
        }
        let total = d.sleep.total();
        let slept = total != last_total;
        last_total = total;
        wake_once(&sh, tick, slept).await;
    }
}

/// The registry, the loop's task, the notices on their way.
pub struct Notifier {
    shared: Arc<Shared>,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// Takes a registered-but-unconfirmed subscription out if `register`'s future is dropped mid-post: a
/// dropped request is a cancelled one, and nothing is kept unless its confirmation is known written.
struct Unconfirmed<'a> {
    sh: &'a Shared,
    seq: u64,
    armed: bool,
}

impl Drop for Unconfirmed<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let gone = {
            let mut reg = self.sh.lock();
            reg.subs
                .iter()
                .position(|s| s.seq == self.seq && !s.telling())
                .map(|i| reg.subs.remove(i))
        };
        if let Some(sub) = &gone {
            log_end(&self.sh.deps.logger, &sub.pr, End::Cancelled);
        }
        drop(gone);
    }
}

/// NTF-10: whatever wasn't written when `stop` ends, or when its future is dropped, in one line.
struct StopLog {
    logger: Logger,
    total: usize,
    written: Arc<AtomicUsize>,
}

impl Drop for StopLog {
    fn drop(&mut self) {
        let left = self
            .total
            .saturating_sub(self.written.load(Ordering::SeqCst));
        if left > 0 {
            self.logger
                .warn(format!("notice not posted reason=stop count={left}"));
        }
    }
}

/// Aborts the telling posts `stop` awaited and didn't see end.
struct AbortAll(Vec<AbortHandle>);

impl Drop for AbortAll {
    fn drop(&mut self) {
        for h in &self.0 {
            h.abort();
        }
    }
}

impl Notifier {
    /// Starts the loop. It starts with no subscription: none is ever saved or loaded (NTF-9).
    pub fn spawn(deps: NotifyDeps) -> Arc<Notifier> {
        let shared = Arc::new(Shared {
            deps,
            reg: Mutex::new(Registry::default()),
            wake: tokio::sync::Notify::new(),
        });
        let task = tokio::spawn(run(shared.clone()));
        Arc::new(Notifier {
            shared,
            task: Mutex::new(Some(task)),
        })
    }

    fn take_task(&self) -> Option<JoinHandle<()>> {
        self.task.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// OPS-7's hook (NTF-10): takes every subscription out at once, posts to all of them together, and
    /// returns when every post has ended or `STOP_LIMIT` has passed, whichever is first. `asked` is false
    /// for a stop the daemon chose itself (`StopCause::NotOwnFiles`, STA-11): its notices' command then has
    /// no `--no-spawn`. Nothing is tried again. Those not written by then are dropped and logged in one
    /// line, also when this future is dropped.
    pub async fn stop(&self, asked: bool) {
        let sh = &self.shared;
        let d = &sh.deps;
        // NTF-10: from its first instant the registry is closed.
        let subs = {
            let mut reg = sh.lock();
            if reg.stopping {
                return;
            }
            reg.stopping = true;
            std::mem::take(&mut reg.subs)
        };
        sh.wake.notify_one();
        let view = d.store.view();
        let written = Arc::new(AtomicUsize::new(0));
        let _log = StopLog {
            logger: d.logger.clone(),
            total: subs.len(),
            written: written.clone(),
        };
        let mut tasks = AbortAll(Vec::new());
        let mut posts: Vec<BoxFuture<'_, ()>> = Vec::new();
        for mut sub in subs {
            let state = std::mem::replace(&mut sub.state, State::Open);
            let written = written.clone();
            match state {
                State::Telling { task } => {
                    // NTF-10: a post already under way is awaited in place of `stopped`.
                    let Some(task) = task else { continue };
                    tasks.0.push(task.abort_handle());
                    posts.push(
                        async move {
                            let _sub = sub;
                            if let Ok(true) = task.await {
                                written.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        .boxed(),
                    );
                }
                State::Open => {
                    log_end(&d.logger, &sub.pr, End::Stopped);
                    let text = text::stopped(&sub.told(&view), asked);
                    posts.push(
                        async move {
                            let _busy = sh.deps.activity.begin();
                            let ok = sh
                                .deliver(
                                    &sub.socket,
                                    sub.socket_id,
                                    sub.token.as_ref(),
                                    &text,
                                    None,
                                )
                                .await;
                            if ok {
                                written.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        .boxed(),
                    );
                }
                State::Confirming { mut confirmed, .. } => {
                    log_end(&d.logger, &sub.pr, End::Stopped);
                    let text = text::stopped(&sub.told(&view), asked);
                    posts.push(
                        async move {
                            // NTF-10: to a confirming one once its confirmation was written.
                            let ok = matches!(
                                confirmed.wait_for(|v| v.is_some()).await.map(|v| *v),
                                Ok(Some(true))
                            );
                            if !ok {
                                // Its confirmation wasn't written: nothing was owed to it.
                                written.fetch_add(1, Ordering::SeqCst);
                                return;
                            }
                            let _busy = sh.deps.activity.begin();
                            let ok = sh
                                .deliver(
                                    &sub.socket,
                                    sub.socket_id,
                                    sub.token.as_ref(),
                                    &text,
                                    None,
                                )
                                .await;
                            if ok {
                                written.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        .boxed(),
                    );
                }
            }
        }
        // NTF-10: all together, with `STOP_LIMIT` for all of them.
        let _ = timeout(&d.clock, STOP_LIMIT, futures::future::join_all(posts)).await;
        drop(tasks);
    }

    /// `Running::crash` (TST-5): ends the loop and drops everything. Posts nothing, logs nothing (NTF-10).
    pub fn abort(&self) {
        let subs = {
            let mut reg = self.shared.lock();
            reg.stopping = true;
            std::mem::take(&mut reg.subs)
        };
        for sub in &subs {
            if let State::Telling { task: Some(task) } = &sub.state {
                task.abort();
            }
        }
        if let Some(task) = self.take_task() {
            task.abort();
        }
        drop(subs);
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        if let Some(task) = self.take_task() {
            task.abort();
        }
    }
}

#[async_trait::async_trait]
impl NotifyHandle for Notifier {
    async fn look(&self, socket: &Path) -> Looked {
        let _busy = self.shared.deps.activity.begin();
        self.shared.deps.poster.look(socket).await
    }

    fn drop_same(&self, id: &NodeId, socket: &Path, ask: &Ask) -> bool {
        let gone = {
            let mut reg = self.shared.lock();
            if reg.stopping {
                return false;
            }
            reg.position_same(id, socket, ask)
                .map(|i| reg.subs.remove(i))
        };
        match gone {
            Some(sub) => {
                log_end(&self.shared.deps.logger, &sub.pr, End::Replaced);
                drop(sub);
                true
            }
            None => false,
        }
    }

    async fn register(&self, new: NewSubscription) -> Result<Registered, RegisterError> {
        let sh = &self.shared;
        let d = &sh.deps;
        let NewSubscription {
            id,
            pr,
            cond,
            progress,
            cursor,
            life,
            life_given,
            socket,
            socket_id,
            token,
        } = new;
        let (tx, rx) = watch::channel(None);
        // What the post needs, taken out before the lock is let go (no lock across an `.await`).
        let post_socket = socket.clone();
        let post_token = token.clone();
        let (seq, text, expires_at, replaced) = {
            let mut reg = sh.lock();
            // NTF-10: after `stop` has begun.
            if reg.stopping {
                return Err(RegisterError::Stopping);
            }
            // NTF-2: the one it replaces ends as `Replaced`, posting nothing, and its place is taken.
            let old = reg
                .position_same(&id, &socket, &cond.ask)
                .map(|i| reg.subs.remove(i));
            let replaced = old.is_some();
            if let Some(old) = old {
                log_end(&d.logger, &old.pr, End::Replaced);
                drop(old);
            } else {
                // NTF-2: at most 64 in all and 32 for one socket; a replacement is never refused.
                let all = reg.subs.len();
                let mine = reg.subs.iter().filter(|s| s.socket == socket).count();
                if all >= MAX_SUBSCRIPTIONS as usize || mine >= PER_SOCKET as usize {
                    return Err(RegisterError::Limit);
                }
            }
            let guard = d
                .store
                .add_subscription(&id)
                .map_err(|_| RegisterError::Limit)?;
            let seq = reg.next_seq;
            reg.next_seq += 1;
            let expires_at = d.clock.wall().plus(life);
            let view = d.store.view();
            let sub = Sub {
                seq,
                id,
                pr,
                cond,
                progress,
                cursor,
                life,
                life_given,
                socket,
                socket_id,
                token,
                base_mono: d.clock.mono(),
                base_sleep: d.sleep.total(),
                held: None,
                stat_logged: false,
                state: State::Confirming {
                    confirmed: rx,
                    ended: None,
                },
                _guard: guard,
            };
            let text = text::confirmation(&sub.told(&view));
            d.logger.info(format!(
                "subscription registered pr={} until={}",
                sub.pr,
                sub.cond.ask.until.as_str()
            ));
            reg.subs.push(sub);
            (seq, text, expires_at, replaced)
        };
        // A refresh applied between the route's view and the entry is evaluated now.
        sh.wake.notify_one();

        // NTF-2: the confirmation, posted once, inside `POST_LIMIT`, never tried again.
        let mut unconfirmed = Unconfirmed {
            sh,
            seq,
            armed: true,
        };
        let result = {
            let _busy = d.activity.begin();
            timeout(
                &d.clock,
                POST_LIMIT,
                d.poster.post(&post_socket, post_token.as_ref(), &text),
            )
            .await
        };
        drop(post_token);
        unconfirmed.armed = false;
        let done = matches!(result, Ok(Posted::Done));
        let _ = tx.send(Some(done));

        let mut gone = None;
        let answer = {
            let mut reg = sh.lock();
            let pos = reg.subs.iter().position(|s| s.seq == seq && !s.telling());
            if done {
                if let Some(i) = pos {
                    let sub = &mut reg.subs[i];
                    let ended = match &mut sub.state {
                        State::Confirming { ended, .. } => ended.take(),
                        _ => None,
                    };
                    match ended {
                        None => sub.state = State::Open,
                        // NTF-2: the notice that waited for the confirmation goes now.
                        Some(Ending {
                            text: Some(text), ..
                        }) => start_telling(sh, sub, text),
                        Some(Ending { text: None, .. }) => gone = Some(reg.subs.remove(i)),
                    }
                }
                Ok(Registered {
                    expires_at,
                    replaced,
                })
            } else {
                let (reason, why) = match result {
                    Ok(Posted::SessionGone(e)) => ("session_gone", e),
                    Ok(Posted::Failed(e)) => ("failed", e),
                    Ok(Posted::Done) => ("failed", String::new()),
                    Err(_) => (
                        "timeout",
                        format!("not written within {}s", POST_LIMIT.as_secs()),
                    ),
                };
                if let Some(i) = pos {
                    let sub = reg.subs.remove(i);
                    d.logger
                        .warn(format!("notice not posted pr={} reason={reason}", sub.pr));
                    let had_end = matches!(sub.state, State::Confirming { ended: Some(_), .. });
                    if !had_end {
                        log_end(&d.logger, &sub.pr, End::SessionGone);
                    }
                    gone = Some(sub);
                }
                // NTF-2: nothing is kept, the one it replaced stays gone; API-3's `no_session`.
                Err(RegisterError::NoSession(format!(
                    "the confirmation couldn't be posted to the session's socket: {why}"
                )))
            }
        };
        drop(gone);
        sh.wake.notify_one();
        answer
    }

    fn cancel(&self, name: &PrName, id: Option<&NodeId>, socket: &Path) -> u32 {
        let gone: Vec<Sub> = {
            let mut reg = self.shared.lock();
            if reg.stopping {
                return 0;
            }
            // NTF-3: that socket's, on the PR by node ID or registered under the name; never a telling one.
            let (gone, kept) = std::mem::take(&mut reg.subs).into_iter().partition(|s| {
                !s.telling()
                    && s.socket == socket
                    && (id.is_some_and(|id| *id == s.id) || s.pr == *name)
            });
            reg.subs = kept;
            gone
        };
        for sub in &gone {
            log_end(&self.shared.deps.logger, &sub.pr, End::Cancelled);
        }
        gone.len() as u32
    }

    fn count(&self) -> u32 {
        let reg = self.shared.lock();
        // NTF-10: 0 from the first instant of a stop.
        if reg.stopping {
            0
        } else {
            reg.subs.len() as u32
        }
    }
}
