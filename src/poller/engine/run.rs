//! The loop: one task that waits on the command channel, the sleep watch and one timer, and owns the
//! gate, `Quota`, `Backoff` and `Auth` (POL-2, POL-13, POL-22, POL-26).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::{Busy, Mono, Time};
use crate::model::NodeId;
use crate::poller::auth::Auth;
use crate::poller::limits::{Backoff, Quota};
use crate::poller::{
    FirstPoll, Policy, PollOutcome, PollReport, ProblemCode, Refusal, RefusalCode, Tier, cadence,
};
use crate::status::WarningCode;

use super::demand::Demand;
use super::pr_errors::PrErrors;
use super::{Cmd, EngineDeps, Shared, lock};

/// POL-26: the tick.
const TICK: Duration = Duration::from_secs(10);

pub(super) struct Run {
    pub(super) d: EngineDeps,
    pub(super) shared: Arc<Shared>,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    sleep_rx: watch::Receiver<Duration>,
    /// TST-4: held from each wake until the loop parks again.
    busy: Option<Busy>,
    pub(super) auth: Auth,
    pub(super) quota: Quota,
    pub(super) backoff: Backoff,
    // ---- the gate (POL-2, MOD-2) ----
    pub(super) seq: u64,
    pub(super) last_end: Option<Mono>,
    // ---- polls ----
    /// The send time of the last poll's first request (POL-13).
    pub(super) last_poll_sent: Option<Mono>,
    /// The send time of the last forced poll (POL-15); an ordinary poll doesn't move it.
    pub(super) last_forced: Option<Mono>,
    /// A poll is wanted at once (a try in `auth`, a wake, a success after a backoff).
    pub(super) poll_asap: bool,
    /// One try with a new token is open in `auth` (POL-20).
    pub(super) try_open: bool,
    pub(super) in_poll: bool,
    pub(super) login: Option<String>,
    pub(super) last_poll_at: Option<Time>,
    pub(super) last_poll: Option<(Time, PollOutcome)>,
    /// `poll_now` calls waiting for the loop.
    poll_nows: Vec<oneshot::Sender<PollReport>>,
    client_woke: bool,
    // ---- problems ----
    /// What the backoff's failure was, for API-3's message: `status 503`, `a timeout`.
    pub(super) unavailable: Option<String>,
    /// A `problem begin code=rate_limited` line was written and its end wasn't.
    pub(super) rate_logged: bool,
    /// The problem's code and when it last changed (`PollerStatus::problem_since`).
    pub(super) problem: Option<(ProblemCode, Mono)>,
    // ---- the tier, the tick, sleep ----
    pub(super) tier: Tier,
    next_tick: Mono,
    sleep_seen: Duration,
    pub(super) pr_errors: PrErrors,
    /// T25: the demand queue, the follow-ups, and what a refresh outside a poll needs.
    pub(super) dm: Demand,
}

impl Run {
    pub(super) fn new(
        d: EngineDeps,
        shared: Arc<Shared>,
        cmds: mpsc::UnboundedReceiver<Cmd>,
    ) -> Run {
        let auth = Auth::new(d.token_source.clone(), d.clock.clone(), d.logger.clone());
        let quota = Quota::new(&d.config.quota);
        let sleep_rx = d.sleep.subscribe();
        let sleep_seen = d.sleep.total();
        let next_tick = d.clock.mono().plus(TICK);
        Run {
            d,
            shared,
            cmds,
            sleep_rx,
            busy: None,
            auth,
            quota,
            backoff: Backoff::new(),
            seq: 0,
            last_end: None,
            last_poll_sent: None,
            last_forced: None,
            poll_asap: false,
            try_open: false,
            in_poll: false,
            login: None,
            last_poll_at: None,
            last_poll: None,
            poll_nows: Vec::new(),
            client_woke: false,
            unavailable: None,
            rate_logged: false,
            problem: None,
            tier: Tier::Idle,
            next_tick,
            sleep_seen,
            pr_errors: PrErrors::default(),
            dm: Demand::default(),
        }
    }

    pub(super) fn now(&self) -> Mono {
        self.d.clock.mono()
    }

    pub(super) fn wall(&self) -> Time {
        self.d.clock.wall()
    }

    /// TST-4: the first thing after a wake.
    pub(super) fn wake(&mut self) {
        if self.busy.is_none() {
            self.busy = Some(self.d.activity.begin());
        }
    }

    /// Before every wait that isn't a request in flight: the status first (API-4), then the guard.
    pub(super) fn park(&mut self) {
        self.publish();
        self.busy = None;
    }

    /// U2: polling is the only source of data; nothing else feeds the store from GitHub.
    pub(super) async fn run(mut self) {
        loop {
            self.wake();
            self.drain();
            self.notice_sleep();
            if self.now() >= self.next_tick {
                self.tick();
            }
            self.follow_tier();
            if self.auth.first_read_done() && self.now() >= self.auth.due() {
                self.periodic().await;
            }
            if std::mem::take(&mut self.client_woke) {
                self.demand_token().await;
            }
            // POL-15: demand goes next, before a poll or a hot refresh that's due.
            self.serve_demand().await;
            self.serve_poll_nows().await;
            if self.poll_is_due() {
                self.poll(false).await;
            }
            // POL-11, then POL-14.
            self.serve_follow_ups().await;
            self.serve_hot().await;
            if self.dm.has_queued() || !self.poll_nows.is_empty() {
                // Taken in while a request was in flight: served before parking.
                continue;
            }
            let at = self.next_wake();
            self.park();
            let clock = self.d.clock.clone();
            let timer = clock.sleep_until(at);
            tokio::select! {
                biased;
                cmd = self.cmds.recv() => {
                    if let Some(cmd) = cmd {
                        self.take(cmd);
                    }
                }
                changed = self.sleep_rx.changed() => {
                    // The watch lives as long as the daemon; a closed one is never waited on again.
                    if changed.is_err() {
                        let (_tx, rx) = watch::channel(Duration::ZERO);
                        self.sleep_rx = rx;
                    }
                }
                () = timer => {}
            }
        }
    }

    fn take(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::ClientRequest => self.client_woke = true,
            // POL-14: waking is enough; the hot set is worked out again before parking.
            Cmd::Nudge => {}
            Cmd::Refresh(id, tx) => self.take_refresh(id, tx),
            Cmd::Lookup { name, manual, tx } => self.take_lookup(name, manual, tx),
            Cmd::PollNow(tx) => {
                if self.in_poll {
                    // API-16: it joins the poll in flight.
                    lock(&self.shared.flight).joined.push(tx);
                } else {
                    self.poll_nows.push(tx);
                }
            }
        }
    }

    /// Every command waiting, without waiting.
    pub(super) fn drain(&mut self) {
        while let Ok(cmd) = self.cmds.try_recv() {
            self.take(cmd);
        }
    }

    /// Between a poll's requests (POL-15: demand goes next).
    pub(super) async fn between_requests(&mut self) {
        self.drain();
        self.serve_demand_boxed().await;
    }

    /// A demand's refresh runs the poll's own steps: boxed, as the two call each other.
    fn serve_demand_boxed(&mut self) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(self.serve_demand())
    }

    /// POL-13 with NTF-5; one `tier` line per change.
    pub(super) fn follow_tier(&mut self) {
        let tier = self.shared.tier_now();
        if tier != self.tier {
            self.tier = tier;
            let word = match tier {
                Tier::Active => "active",
                Tier::Idle => "idle",
            };
            self.d.logger.info(format_args!("tier tier={word}"));
        }
    }

    pub(super) fn policy(&self) -> Policy {
        self.quota
            .policy(self.wall(), self.now(), self.tier, &self.d.config)
    }

    /// In `auth` with no try open: nothing can be sent (POL-20).
    pub(super) fn auth_blocked(&self) -> bool {
        self.auth.problem().is_some() && !self.try_open
    }

    /// When the next poll is due; `None` while none may be sent.
    fn poll_due_at(&self) -> Option<Mono> {
        if self.auth_blocked() {
            return None;
        }
        let policy = self.policy();
        // POL-23: nothing at all while `demand` is false.
        if !policy.demand {
            return None;
        }
        // POL-19: the backoff's try is a poll, and it holds every other.
        if self.backoff.active() {
            return self.backoff.next_try();
        }
        if self.poll_asap && policy.poll_every.is_some() {
            return Some(self.now());
        }
        cadence::poll_due(self.last_poll_sent, &policy)
    }

    fn poll_is_due(&self) -> bool {
        self.poll_due_at().is_some_and(|at| at <= self.now())
    }

    /// Timers are absolute (POL-26): the earliest of the next poll, the backoff's try, the token's read and the tick.
    fn next_wake(&self) -> Mono {
        let mut at = self.next_tick;
        if self.auth.first_read_done() {
            at = at.min(self.auth.due());
        }
        if let Some(due) = self.poll_due_at() {
            at = at.min(due);
        }
        // POL-11, POL-14.
        if let Some(due) = self.hot_wake() {
            at = at.min(due);
        }
        at
    }

    /// POL-3, POL-20: the periodic read; a new token in `auth` opens one try, a poll.
    async fn periodic(&mut self) {
        self.park();
        let worth = self.auth.periodic().await;
        self.wake();
        if worth {
            self.try_open = true;
            self.poll_asap = true;
        }
        self.after_auth_read();
    }

    /// POL-20: a client request in `auth` reads the token again, at most once in 5 s (`Auth` keeps that).
    async fn demand_token(&mut self) {
        if !self.auth_blocked() {
            return;
        }
        self.park();
        let token = self.auth.token(true).await;
        self.wake();
        match token {
            Ok(_) => {
                self.try_open = true;
                self.poll_asap = true;
            }
            Err(problem) => {
                // API-18: `client_request` set `Running` for this read; there is still no token.
                let refusal = Refusal {
                    code: RefusalCode::Auth,
                    message: problem.message,
                    until: None,
                };
                self.shared.first_poll.send_if_modified(|f| {
                    if *f == FirstPoll::Running && self.last_poll.is_some() {
                        *f = FirstPoll::Unanswered(refusal);
                        true
                    } else {
                        false
                    }
                });
            }
        }
        self.after_auth_read();
    }

    /// What `client_request` reads of `Auth`, published at once.
    pub(super) fn after_auth_read(&mut self) {
        let mut hand = lock(&self.shared.hand);
        hand.in_auth = self.auth.problem().is_some();
        hand.last_read_ended = self.auth.last_read_ended();
    }

    /// API-16.
    async fn serve_poll_nows(&mut self) {
        if self.poll_nows.is_empty() {
            return;
        }
        let waiting = std::mem::take(&mut self.poll_nows);
        let policy = self.policy();
        let now = self.now();
        let shared = PollReport {
            outcome: PollOutcome::Shared,
            cost: 0,
        };
        let report = if !cadence::may_force_poll(self.last_forced, &policy, now) {
            // POL-15: inside the window, what's in memory.
            shared.clone()
        } else if let Some(outcome) = self.refused_now(&policy) {
            // API-16: it never waits out a backoff or a reset.
            PollReport { outcome, cost: 0 }
        } else {
            self.poll(true).await
        };
        let mut first = Some(report);
        for tx in waiting {
            let _ = tx.send(first.take().unwrap_or_else(|| shared.clone()));
        }
    }

    /// API-16's problems, in API-4's order; `None` when a poll may be sent.
    fn refused_now(&self, policy: &Policy) -> Option<PollOutcome> {
        if self.auth_blocked() {
            Some(PollOutcome::Auth)
        } else if !policy.demand {
            Some(PollOutcome::RateLimited)
        } else if self.backoff.active() {
            Some(PollOutcome::GithubUnavailable)
        } else {
            None
        }
    }

    /// POL-26: the tick. It sends nothing.
    fn tick(&mut self) {
        let now = self.now();
        // A long jump fires one tick.
        while self.next_tick <= now {
            self.next_tick = self.next_tick.plus(TICK);
        }
        self.d.sleep.check();
        self.notice_sleep();
        self.quota_warnings();
        self.d.store.sweep();
        self.pr_errors.tick(&self.d);
    }

    /// POL-23: each warning on its own condition.
    fn quota_warnings(&mut self) {
        let wall = self.wall();
        let w = &self.d.warnings;
        if self.quota.capped(wall) {
            w.raise(
                WarningCode::QuotaCapped,
                "",
                format!(
                    "circitor has spent {} points in the last hour, its whole quota.max_per_hour of {}: polls wait until the count drops.",
                    self.quota.cost_last_hour(wall),
                    self.d.config.quota.max_per_hour
                ),
            );
        } else {
            w.clear(WarningCode::QuotaCapped, "");
        }
        if self.quota.in_reserve() {
            w.raise(
                WarningCode::QuotaReserve,
                "",
                format!(
                    "GitHub reports {} points left, under quota.reserve ({}): circitor polls every {} s.",
                    self.quota.remaining().unwrap_or(0),
                    self.d.config.quota.reserve,
                    self.d.config.idle_interval.as_secs()
                ),
            );
        } else {
            w.clear(WarningCode::QuotaReserve, "");
        }
    }

    /// POL-22: react when the watch's total rose, whoever called `check`.
    fn notice_sleep(&mut self) {
        let total = self.d.sleep.total();
        if total <= self.sleep_seen {
            return;
        }
        let gap = total - self.sleep_seen;
        self.sleep_seen = total;
        self.d
            .logger
            .info(format_args!("sleep seconds={}", gap.as_secs()));
        self.d.store.mark_slept();
        let (now, wall) = (self.now(), self.wall());
        self.backoff.woke(now, wall);
        let client = self
            .shared
            .last_request()
            .is_some_and(|at| now.since(at) < self.d.config.idle_after);
        let view = self.d.store.view();
        if client || view.waiters > 0 || self.d.store.subscriptions() > 0 {
            self.poll_asap = true;
        }
    }

    /// API-18: the refusal a first poll that nothing answered gives.
    pub(super) fn refusal(&self, outcome: PollOutcome) -> Refusal {
        let (wall, mono) = (self.wall(), self.now());
        match outcome {
            PollOutcome::Auth => Refusal {
                code: RefusalCode::Auth,
                message: self
                    .auth
                    .problem()
                    .map(|p| p.message)
                    .unwrap_or_else(|| self.auth_message_fallback()),
                until: None,
            },
            PollOutcome::RateLimited => {
                let until = self.quota.resume_at(wall, mono);
                Refusal {
                    code: RefusalCode::RateLimited,
                    message: super::publish::rate_limited_message(until),
                    until,
                }
            }
            PollOutcome::GithubUnavailable | PollOutcome::Polled | PollOutcome::Shared => {
                let until = self.backoff.next_try_wall();
                Refusal {
                    code: RefusalCode::GithubUnavailable,
                    message: super::publish::unavailable_message(
                        self.unavailable.as_deref(),
                        self.d.token_source.kind(),
                        until,
                    ),
                    until,
                }
            }
        }
    }

    fn auth_message_fallback(&self) -> String {
        format!(
            "There is no usable GitHub token from {}.",
            self.d.token_source.kind().as_str()
        )
    }

    /// The IDs a batch names, for `mark_sent` and the abandon.
    pub(super) fn ids(nodes: &[crate::github::NodeQuery]) -> Vec<NodeId> {
        nodes.iter().map(|n| n.id.clone()).collect()
    }
}
