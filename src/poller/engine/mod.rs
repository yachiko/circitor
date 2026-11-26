//! The engine: the loop, the gate, the poll (T21), its demand side (T25), a PR's errors (T42).
//!
//! One task, `run::Run`, owns the gate, `Quota`, `Backoff` and `Auth`. The handle (`Engine`) talks to
//! it over a channel and reads what it last published; it never waits for the loop (POL-2, OPS-5).

use std::collections::HashMap;
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
use crate::state::{Class, Store};
use crate::status::Warnings;
use crate::token::TokenSource;

mod demand;
mod gate;
mod hot;
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
    /// The hot set may have changed: the loop looks again when it wakes (POL-14).
    Nudge,
    /// API-16.
    PollNow(oneshot::Sender<PollReport>),
    /// POL-15: one PR's forced refresh.
    Refresh(NodeId, oneshot::Sender<Refreshed>),
    /// POL-16: a read-through; `manual` for `track` (API-15).
    Lookup {
        name: PrName,
        manual: bool,
        tx: LookupTx,
    },
}

/// Where a read-through's answer goes.
type LookupTx = oneshot::Sender<Result<NodeId, Refusal>>;

/// A caller waiting for a PR's refresh in flight (POL-15: sharing is by PR, not by caller).
enum Joiner {
    /// `refresh_pr`.
    Refresh(oneshot::Sender<Refreshed>),
    /// `read_through` or `track` of a PR whose first refresh is in flight.
    Lookup(LookupTx),
}

/// A read-through in hand: the callers sharing it, and whether one of them is a `track`.
struct LookupFlight {
    manual: bool,
    txs: Vec<LookupTx>,
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
    /// POL-15: the PRs whose refresh has been sent and isn't applied or failed yet, with who joined it.
    prs: HashMap<NodeId, Vec<Joiner>>,
    /// POL-16: the read-throughs the loop is working on, by name.
    lookups: HashMap<PrName, LookupFlight>,
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
        // A `poll_now` that joined a poll in flight answers now; so does every demand that joined a request.
        let mut flight = lock(&self.shared.flight);
        flight.joined.clear();
        flight.prs.clear();
        flight.lookups.clear();
    }
}

/// What a `poll_now` answers when the engine is gone before it could.
fn stopped_report() -> PollReport {
    PollReport {
        outcome: PollOutcome::GithubUnavailable,
        cost: 0,
    }
}

/// What a read-through or a `track` answers when the engine is gone before it could.
fn stopped_refusal() -> Refusal {
    Refusal {
        code: RefusalCode::GithubUnavailable,
        message: "circitor stopped before it could read this PR from GitHub.".to_string(),
        until: None,
    }
}

/// `Open` and `Ended` have a record; `Loaded` and `First` don't (API-18's `pending`).
fn has_record(class: Class) -> bool {
    matches!(class, Class::Open | Class::Ended)
}

impl Engine {
    /// POL-16: hands a read-through to the loop, or joins one that holds this name or this PR.
    async fn lookup(&self, name: &PrName, manual: bool) -> Result<NodeId, Refusal> {
        let s = &self.shared;
        let tracked = s.store.lookup(name);
        let (tx, rx) = oneshot::channel();
        {
            let mut flight = lock(&s.flight);
            let flight = &mut *flight;
            if let Some(f) = flight.lookups.get_mut(name) {
                // POL-16: two calls for one name share one request.
                f.manual |= manual;
                f.txs.push(tx);
            } else if let Some(joined) = tracked.and_then(|t| flight.prs.get_mut(&t.id)) {
                // A `First` PR: its first refresh is in flight; this joins it.
                joined.push(Joiner::Lookup(tx));
            } else {
                let _ = s.cmds.send(Cmd::Lookup {
                    name: name.clone(),
                    manual,
                    tx,
                });
            }
        }
        rx.await.unwrap_or_else(|_| Err(stopped_refusal()))
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

    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal> {
        // POL-16: a PR with a record is answered from memory; nothing is sent.
        if let Some(t) = self.shared.store.lookup(name)
            && has_record(t.class)
        {
            return Ok(t.id);
        }
        self.lookup(name, false).await
    }

    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal> {
        let store = &self.shared.store;
        if let Some(t) = store.lookup(name) {
            // API-15: `manual` first, also at the limit; a loaded PR's saved source is set before
            // its read-through, so it stays `manual` when that fails.
            store.set_manual(&t.id);
            if has_record(t.class) {
                return Ok(t.id);
            }
        }
        let read = self.lookup(name, true).await;
        if let Ok(id) = &read {
            // API-15: a `track` that joined a read-through is `manual` too.
            store.set_manual(id);
        }
        read
    }

    async fn refresh_pr(&self, id: &NodeId) -> Refreshed {
        let (tx, rx) = oneshot::channel();
        {
            let mut flight = lock(&self.shared.flight);
            match flight.prs.get_mut(id) {
                // POL-15: a request in flight that holds the PR is joined, before any window.
                Some(joined) => joined.push(Joiner::Refresh(tx)),
                None => {
                    let _ = self.shared.cmds.send(Cmd::Refresh(id.clone(), tx));
                }
            }
        }
        rx.await.unwrap_or(Refreshed::Failed)
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
        // POL-14: the loop works out the hot set again when it wakes.
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
