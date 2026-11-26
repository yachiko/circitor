//! The daemon's life once it serves: OPS-7's stop (with NTF-10's hook), OPS-8's idle exit, STA-11's
//! minute's check and the signals.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::{Notify, watch};

use super::serve::Conns;
use super::{Tasks, ask, lock};
use crate::clock::{Activity, Busy, SharedClock, sleep, timeout};
use crate::daemon::instance::{Bound, LockGuard};
use crate::daemon::{Mode, StopCause, StopHandle};
use crate::log::Logger;
use crate::notify::Notifier;
use crate::poller::{Engine, PollerHandle};
use crate::state::{Saver, Store};

/// OPS-7: the whole stop, from the request to the lock's release.
pub(super) const STOP_DEADLINE: Duration = Duration::from_secs(5);
/// STA-11: how often the socket and the lock file are checked.
pub(super) const CHECK_EVERY: Duration = Duration::from_secs(60);
/// OPS-7: how often a stop under way looks for a second request from a caller that has no `Notify`
/// (T22's `shutdown` route asks the `StopHandle` directly).
pub(super) const SECOND_LOOK: Duration = Duration::from_millis(100);

/// What the stop works on. The lock and the socket are taken out as the stop lets go of them.
pub(super) struct Life {
    pub clock: SharedClock,
    pub activity: Activity,
    pub logger: Logger,
    pub stop: StopHandle,
    pub second: Arc<Notify>,
    pub tasks: Arc<Mutex<Tasks>>,
    pub bound: Option<Arc<Bound>>,
    pub lock: Option<Arc<LockGuard>>,
    pub engine: Arc<Engine>,
    pub notifier: Arc<Notifier>,
    pub saver: Arc<Saver>,
    pub conns: Arc<Conns>,
    pub stop_delay: Option<Duration>,
    pub status: watch::Sender<Option<i32>>,
}

/// OPS-9's words for a cause.
fn cause_words(cause: StopCause) -> String {
    match cause {
        StopCause::Signal(n) => format!("cause=signal signal={n}"),
        // OPS-9: a CLI replacing the daemon can't be told from `stop`: both are a shutdown request.
        StopCause::Request => "cause=request".to_string(),
        StopCause::Idle => "cause=idle".to_string(),
        StopCause::NotOwnFiles => "cause=files".to_string(),
        StopCause::Test => "cause=test".to_string(),
    }
}

/// OPS-7: resolves once the stop has been asked for twice. Parked between looks (no `Busy` held across
/// the sleep); it ends with the sequence, which drops it.
async fn second_request(clock: &SharedClock, stop: &StopHandle) {
    loop {
        sleep(clock, SECOND_LOOK).await;
        if stop.times() >= 2 {
            return;
        }
    }
}

impl Life {
    /// The life's task: parked until a stop is asked for, then OPS-7's sequence, once.
    pub(super) async fn run(mut self) {
        let cause = self.stop.wait().await;
        // TST-4: work from waking.
        let busy = self.activity.begin();
        // OPS-9: the stop and its cause.
        self.logger
            .info(format_args!("stop {}", cause_words(cause)));
        let second = self.second.clone();
        let stop = self.stop.clone();
        let clock = self.clock.clone();
        // OPS-7: 5 s for the whole stop; a second request during it does the same at once.
        let cut = tokio::select! {
            done = timeout(&clock, STOP_DEADLINE, self.sequence(cause, busy)) => done.is_err(),
            () = second.notified() => true,
            // OPS-7: a second request of any kind, e.g. a `POST /v1/shutdown`.
            () = second_request(&clock, &stop) => true,
        };
        let busy = self.activity.begin();
        if cut {
            if self.stop.times() >= 2 {
                self.logger.warn("stop deadline cause=second_request");
            } else {
                self.logger.warn("stop deadline seconds=5");
            }
            self.drop_everything().await;
        }
        let signals = lock(&self.tasks).signals.take();
        if let Some(signals) = signals {
            signals.abort();
        }
        drop(busy);
        // OPS-7: every stop exits 0.
        let _ = self.status.send(Some(0));
    }

    /// The loops that serve or ask for a stop end here: accepting, the idle exit, the minute's check.
    /// Awaited, so the listener and the lock are held by `Life` alone afterwards.
    async fn end_loops(&mut self) {
        let handles = {
            let mut t = lock(&self.tasks);
            [t.accept.take(), t.idle.take(), t.minute.take()]
        };
        for handle in handles.into_iter().flatten() {
            handle.abort();
            let _ = handle.await;
        }
    }

    /// Stop accepting and remove the socket if it's still its own (OPS-7, STA-11).
    async fn stop_accepting(&mut self) {
        self.end_loops().await;
        if let Some(bound) = self.bound.take() {
            bound.remove_if_own();
        }
    }

    /// OPS-7's order, NTF-10's hook between the engine and the connections.
    async fn sequence(&mut self, cause: StopCause, busy: Busy) {
        self.stop_accepting().await;
        // TST-1: `CIRCITOR_TEST_STOP_DELAY` holds the stop; parked meanwhile.
        drop(busy);
        if let Some(delay) = self.stop_delay {
            sleep(&self.clock, delay).await;
        }
        let busy = self.activity.begin();
        self.engine.stop().await;
        // NTF-10: after the engine, so no refresh ends a subscription meanwhile; before the connections,
        // so each session is told while the clients are still connected. It keeps its own 1 s, and its
        // posts count as work in flight themselves.
        drop(busy);
        self.notifier.stop(cause != StopCause::NotOwnFiles).await;
        let _busy = self.activity.begin();
        // OPS-7, API-14: every connection closed without an answer.
        self.conns.abort_all();
        // OPS-7, STA-7: the final write, unless STA-8 left the file alone (the saver knows).
        if let Err(e) = self.saver.shutdown().await {
            // OPS-9, TST-10: a failed final write is logged; the stop still exits 0.
            self.logger
                .warn(format_args!("state file write failed type={:?}", e.kind()));
        }
        // STA-11: the lock goes last.
        self.lock.take();
    }

    /// OPS-7: past the deadline, or on a second request: everything is dropped at once.
    async fn drop_everything(&mut self) {
        self.conns.abort_all();
        self.notifier.abort();
        self.saver.abort();
        self.engine.stop().await;
        self.stop_accepting().await;
        self.lock.take();
    }
}

/// OPS-8, NTF-5: a spawned daemon with `exit_after` asks for `Idle` once it has had no client request,
/// no waiter and no subscription for `exit_after` of the monotonic clock. It asks once, then ends.
pub(super) async fn idle_loop(
    clock: SharedClock,
    activity: Activity,
    store: Arc<Store>,
    poller: Arc<Engine>,
    stop: StopHandle,
    exit_after: Duration,
) {
    loop {
        let due = {
            // TST-4: work from waking.
            let _busy = activity.begin();
            let view = store.view();
            if view.waiters == 0 && view.subscriptions == 0 {
                // OPS-8: the latest of the last client request (`status` isn't one, API-5), the last
                // waiter's end and the last subscription's end (NTF-5).
                let mut latest = poller.idle_since();
                if let Some(at) = store.last_waiter_ended() {
                    latest = latest.max(at);
                }
                if let Some(at) = store.last_subscription_ended() {
                    latest = latest.max(at);
                }
                let due = latest.plus(exit_after);
                if clock.mono() >= due {
                    stop.request(StopCause::Idle);
                    return;
                }
                due
            } else {
                // OPS-8: a waiter or a subscription is held: look again `exit_after` later.
                clock.mono().plus(exit_after)
            }
        };
        clock.sleep_until(due).await;
    }
}

/// STA-11: every minute, the socket and the lock file must still be the ones it made. Asks once.
pub(super) async fn minute_loop(
    clock: SharedClock,
    activity: Activity,
    lock_file: Arc<LockGuard>,
    bound: Arc<Bound>,
    stop: StopHandle,
) {
    let mut next = clock.mono().plus(CHECK_EVERY);
    loop {
        clock.sleep_until(next).await;
        let _busy = activity.begin();
        // STA-11: the same device and inode, the path's `stat` against the descriptor's.
        if !lock_file.still_own() || !bound.still_own() {
            stop.request(StopCause::NotOwnFiles);
            return;
        }
        next = next.plus(CHECK_EVERY);
    }
}

/// OPS-7's three signals, installed before anything else is done.
pub(super) struct Signals {
    term: Signal,
    int: Signal,
    hup: Signal,
}

impl Signals {
    pub(super) fn install() -> std::io::Result<Signals> {
        Ok(Signals {
            term: signal(SignalKind::terminate())?,
            int: signal(SignalKind::interrupt())?,
            hup: signal(SignalKind::hangup())?,
        })
    }

    /// Each signal asks for `Signal(n)`; a later `SIGTERM` or `SIGINT` asks again, which ends a stop
    /// under way at once (OPS-7). A `SIGHUP` during a stop changes nothing.
    pub(super) async fn run(mut self, stop: StopHandle, second: Arc<Notify>, activity: Activity) {
        use nix::sys::signal::Signal as Sig;
        loop {
            let n = tokio::select! {
                got = self.term.recv() => got.map(|()| Sig::SIGTERM),
                got = self.int.recv() => got.map(|()| Sig::SIGINT),
                got = self.hup.recv() => got.map(|()| Sig::SIGHUP),
            };
            let Some(n) = n else {
                // The runtime's signal driver is gone: nothing more will come.
                return;
            };
            let _busy = activity.begin();
            if n == Sig::SIGHUP && stop.requested().is_some() {
                continue;
            }
            ask(&stop, &second, StopCause::Signal(n as i32));
        }
    }
}

/// Whether the idle exit applies (OPS-6, OPS-8): spawned, with `exit_after` set.
pub(super) fn idle_after(mode: Mode, exit_after: Option<Duration>) -> Option<Duration> {
    match mode {
        Mode::Spawned => exit_after,
        Mode::Foreground => None,
    }
}
