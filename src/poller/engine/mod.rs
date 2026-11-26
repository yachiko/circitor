//! The engine: the loop, the gate, the poll (T21), its demand side (T25), a PR's errors (T42).
//!
//! One task, `run::Run`, owns the gate, `Quota`, `Backoff` and `Auth`. The handle (`Engine`) talks to
//! it over a channel and reads what it last published; it never waits for the loop (POL-2, OPS-5).

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::clock::{Activity, Mono, SharedClock, SleepWatch};
use crate::config::Config;
use crate::github::GitHub;
use crate::log::Logger;
use crate::model::{NodeId, PrName};
use crate::poller::{
    FirstPoll, PollOutcome, PollReport, PollerHandle, PollerStatus, Refreshed, Refusal,
    RefusalCode, Tier, cadence,
};
use crate::state::Store;
use crate::status::Warnings;
use crate::token::TokenSource;

mod gate;
mod poll;
mod pr_errors;
mod publish;
mod run;
#[cfg(test)]
mod tests;

/// POL-20 (a proposal): in `auth`, a client request reads the token again when the last read ended this long ago.
const AUTH_DEMAND_AFTER: Duration = Duration::from_secs(5);

pub struct EngineDeps {
    pub clock: SharedClock,
    pub github: Arc<dyn GitHub>,
    pub token_source: Arc<dyn TokenSource>,
    pub store: Arc<Store>,
    pub config: Arc<Config>,
    pub logger: Logger,
    pub warnings: Warnings,
    pub activity: Activity,
    pub sleep: Arc<SleepWatch>,
}

/// What reaches the loop from the handle.
enum Cmd {
    /// A client request arrived (POL-13, POL-20, POL-22).
    ClientRequest,
    /// The hot set may have changed (T25).
    Nudge,
    /// API-16.
    PollNow(oneshot::Sender<PollReport>),
}

/// What `client_request` reads and sets, behind a short lock.
#[derive(Default)]
struct Hand {
    last_request: Option<Mono>,
    /// Published by the loop: the daemon is in `auth` (POL-20).
    in_auth: bool,
    /// Published by the loop after every token read (`Auth::last_read_ended`).
    last_read_ended: Option<Mono>,
}

/// A poll in flight, and the `poll_now` calls that joined it (API-16).
#[derive(Default)]
struct Flight {
    polling: bool,
    joined: Vec<oneshot::Sender<PollReport>>,
}

/// What the handle and the loop share. Every lock is taken and released inside one function.
struct Shared {
    clock: SharedClock,
    store: Arc<Store>,
    config: Arc<Config>,
    sleep: Arc<SleepWatch>,
    started: Mono,
    cmds: mpsc::UnboundedSender<Cmd>,
    status: Mutex<PollerStatus>,
    first_poll: watch::Sender<FirstPoll>,
    hand: Mutex<Hand>,
    flight: Mutex<Flight>,
}

/// A std lock; a poisoned one still holds a consistent value (nothing panics while holding these).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// POL-13 with NTF-5: the tier now, from the last client request and the open subscriptions.
    fn tier_now(&self) -> Tier {
        let now = self.clock.mono();
        let last = lock(&self.hand).last_request;
        // NTF-5: only a subscription on an open PR keeps the active tier.
        let open = self.store.open_subscriptions();
        cadence::tier(cadence::last_ask(last, open, now), now, &self.config)
    }

    fn last_request(&self) -> Option<Mono> {
        lock(&self.hand).last_request
    }
}

pub struct Engine {
    shared: Arc<Shared>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    /// Starts the loop: the first token read, then the first poll (STA-11's order is T28's: this is called after the socket is bound).
    pub fn spawn(deps: EngineDeps) -> Arc<Engine> {
        let (cmds, rx) = mpsc::unbounded_channel();
        let (first_poll, _) = watch::channel(FirstPoll::Running);
        let status = PollerStatus {
            // POL-13: the engine starts idle.
            tier: Tier::Idle,
            last_poll_at: None,
            last_poll: None,
            problem: None,
            problem_since: None,
            cost_last_hour: 0,
            remaining: None,
            reset_at: None,
            login: None,
            token_source: deps.token_source.kind(),
            gh_path: deps.token_source.gh_path(),
        };
        let shared = Arc::new(Shared {
            clock: deps.clock.clone(),
            store: deps.store.clone(),
            config: deps.config.clone(),
            sleep: deps.sleep.clone(),
            started: deps.clock.mono(),
            cmds,
            status: Mutex::new(status),
            first_poll,
            hand: Mutex::new(Hand::default()),
            flight: Mutex::new(Flight::default()),
        });
        let run = run::Run::new(deps, shared.clone(), rx);
        let task = tokio::spawn(run.run());
        Arc::new(Engine {
            shared,
            task: Mutex::new(Some(task)),
        })
    }

    /// OPS-7: stops polling, drops a request in flight, kills a running `gh`. Returns when the loop has ended.
    pub async fn stop(&self) {
        let task = lock(&self.task).take();
        if let Some(task) = task {
            // Dropping the loop's future drops the request in flight and a running token read.
            task.abort();
            let _ = task.await;
        }
        // A `poll_now` that joined a poll in flight answers now.
        lock(&self.shared.flight).joined.clear();
    }
}

/// What a `poll_now` answers when the engine is gone before it could.
fn stopped_report() -> PollReport {
    PollReport {
        outcome: PollOutcome::GithubUnavailable,
        cost: 0,
    }
}

/// T25 builds reading through, `track` and forced refreshes; until then they refuse.
fn not_built() -> Refusal {
    Refusal {
        code: RefusalCode::GithubUnavailable,
        message: "circitor couldn't read this PR from GitHub.".to_string(),
        until: None,
    }
}

#[async_trait::async_trait]
impl PollerHandle for Engine {
    fn client_request(&self) {
        let s = &self.shared;
        let now = s.clock.mono();
        let reopen = {
            let mut hand = lock(&s.hand);
            // POL-13: the tier follows the last client request.
            hand.last_request = Some(now);
            // POL-20: in `auth`, a request 5 s after the last read makes the loop read again.
            hand.in_auth
                && hand
                    .last_read_ended
                    .is_some_and(|end| now.since(end) >= AUTH_DEMAND_AFTER)
        };
        // POL-22: the check runs at every client request; the loop reacts to the watch.
        s.sleep.check();
        if reopen {
            // POL-20, API-18: the first poll runs again for this read, before this returns.
            s.first_poll.send_if_modified(|f| {
                if matches!(f, FirstPoll::Unanswered(_)) {
                    *f = FirstPoll::Running;
                    true
                } else {
                    false
                }
            });
        }
        let _ = s.cmds.send(Cmd::ClientRequest);
    }

    fn idle_since(&self) -> Mono {
        // OPS-8: the last client request, or the start.
        self.shared.last_request().unwrap_or(self.shared.started)
    }

    async fn read_through(&self, _name: &PrName) -> Result<NodeId, Refusal> {
        // T25
        Err(not_built())
    }

    async fn track(&self, _name: &PrName) -> Result<NodeId, Refusal> {
        // T25
        Err(not_built())
    }

    async fn refresh_pr(&self, _id: &NodeId) -> Refreshed {
        // T25
        Refreshed::NotSent
    }

    async fn poll_now(&self) -> PollReport {
        let (tx, rx) = oneshot::channel();
        {
            let mut flight = lock(&self.shared.flight);
            if flight.polling {
                // API-16: one arriving while a poll is in flight joins it.
                flight.joined.push(tx);
            } else {
                let _ = self.shared.cmds.send(Cmd::PollNow(tx));
            }
        }
        rx.await.unwrap_or_else(|_| stopped_report())
    }

    fn nudge(&self) {
        let _ = self.shared.cmds.send(Cmd::Nudge);
    }

    fn first_poll(&self) -> watch::Receiver<FirstPoll> {
        self.shared.first_poll.subscribe()
    }

    fn status(&self) -> PollerStatus {
        // API-4: a cheap copy of what the loop last published, with the tier as it is now (NTF-5).
        let mut status = lock(&self.shared.status).clone();
        status.tier = self.shared.tier_now();
        status
    }
}
