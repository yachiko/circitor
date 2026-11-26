//! The demand side (T25): forced refreshes and their sharing (POL-15), reading through (POL-16), and
//! `track`'s poller side (API-15). Commands reach the loop over the channel; the loop does the work
//! and answers through the channels it holds, so a caller that goes away leaves nothing half-done.

use std::collections::{HashSet, VecDeque};

use tokio::sync::oneshot;

use crate::clock::Mono;
use crate::github::{LookupAnswer, Query};
use crate::model::{ItemsOutcome, NodeId, PrName, Source};
use crate::poller::auth::AuthProblem;
use crate::poller::{PollOutcome, Refreshed, Refusal, RefusalCode, cadence};
use crate::state::{Class, MAX_PLACES};

use super::gate::Fail;
use super::poll::{Held, PollState, node_queries, wants_page};
use super::run::Run;
use super::{Joiner, LookupFlight, LookupTx, lock};

/// A demand waiting for the loop (POL-15: it goes next).
enum Queued {
    Refresh {
        id: NodeId,
        txs: Vec<oneshot::Sender<Refreshed>>,
    },
    Lookup {
        name: PrName,
        manual: bool,
        txs: Vec<LookupTx>,
    },
}

/// The loop's demand state, and what a refresh outside a poll needs to know about itself.
#[derive(Default)]
pub(super) struct Demand {
    queue: VecDeque<Queued>,
    /// D11: the PRs a demand refreshed since the poll in flight began.
    sent_in_poll: HashSet<NodeId>,
    /// The request being sent is a hot, forced or follow-up refresh, or a read-through: not a poll (MOD-11).
    pub(super) not_poll: bool,
    /// POL-19: a read-through sent during a backoff; its failure doesn't move it.
    pub(super) keep_backoff: bool,
    /// POL-11: the request being sent is a follow-up; its reads never ask again.
    pub(super) follow_up_run: bool,
    /// POL-11: follow-ups due, each one request: when, and for which PRs.
    pub(super) follow_ups: Vec<(Mono, Vec<NodeId>)>,
    /// POL-11: the PRs refreshed since the engine started.
    pub(super) refreshed: HashSet<NodeId>,
}

impl Demand {
    pub(super) fn poll_started(&mut self) {
        self.sent_in_poll.clear();
    }

    pub(super) fn sent_in_poll(&self, id: &NodeId) -> bool {
        self.sent_in_poll.contains(id)
    }

    pub(super) fn has_queued(&self) -> bool {
        !self.queue.is_empty()
    }
}

/// API-3's `limit`.
fn limit_refusal() -> Refusal {
    Refusal {
        code: RefusalCode::Limit,
        message: format!(
            "circitor already tracks {MAX_PLACES} PRs, its limit: untrack one to make room."
        ),
        until: None,
    }
}

impl Run {
    // ---- taking commands in ----

    /// POL-15: a forced refresh joins the request in flight that holds its PR, or the one queued.
    pub(super) fn take_refresh(&mut self, id: NodeId, tx: oneshot::Sender<Refreshed>) {
        {
            let mut flight = lock(&self.shared.flight);
            if let Some(joined) = flight.prs.get_mut(&id) {
                joined.push(Joiner::Refresh(tx));
                return;
            }
        }
        for q in &mut self.dm.queue {
            if let Queued::Refresh { id: queued, txs } = q
                && *queued == id
            {
                txs.push(tx);
                return;
            }
        }
        self.dm
            .queue
            .push_back(Queued::Refresh { id, txs: vec![tx] });
    }

    /// POL-16: a read-through joins the one in hand for that name, a `First` PR's refresh in flight,
    /// or the one queued.
    pub(super) fn take_lookup(&mut self, name: PrName, manual: bool, tx: LookupTx) {
        let tracked = self.d.store.lookup(&name);
        {
            let mut flight = lock(&self.shared.flight);
            let flight = &mut *flight;
            if let Some(f) = flight.lookups.get_mut(&name) {
                f.manual |= manual;
                f.txs.push(tx);
                return;
            }
            if let Some(joined) = tracked.and_then(|t| flight.prs.get_mut(&t.id)) {
                joined.push(Joiner::Lookup(tx));
                return;
            }
        }
        for q in &mut self.dm.queue {
            if let Queued::Lookup {
                name: queued,
                manual: m,
                txs,
            } = q
                && *queued == name
            {
                *m |= manual;
                txs.push(tx);
                return;
            }
        }
        self.dm.queue.push_back(Queued::Lookup {
            name,
            manual,
            txs: vec![tx],
        });
    }

    // ---- the PRs in flight ----

    /// Called by the gate just before it sends: these PRs are in flight from now on. A forced
    /// refresh that reached the channel or the queue meanwhile joins this request (POL-15).
    pub(super) fn flight_begin(&mut self, ids: &[NodeId]) {
        self.drain();
        if self.in_poll && self.dm.not_poll {
            // D11: a forced poll in flight doesn't send these again.
            self.dm.sent_in_poll.extend(ids.iter().cloned());
        }
        let ids: HashSet<&NodeId> = ids.iter().collect();
        let mut absorbed = Vec::new();
        let mut i = 0;
        while i < self.dm.queue.len() {
            match &self.dm.queue[i] {
                Queued::Refresh { id, .. } if ids.contains(id) => {
                    if let Some(Queued::Refresh { id, txs }) = self.dm.queue.remove(i) {
                        absorbed.push((id, txs));
                    }
                }
                _ => i += 1,
            }
        }
        let mut flight = lock(&self.shared.flight);
        for id in ids {
            flight.prs.entry(id.clone()).or_default();
        }
        for (id, txs) in absorbed {
            flight
                .prs
                .entry(id)
                .or_default()
                .extend(txs.into_iter().map(Joiner::Refresh));
        }
    }

    /// This PR's refresh was applied or failed: whoever joined it gets the result.
    pub(super) fn settle(&mut self, id: &NodeId, result: Refreshed) {
        let joined = lock(&self.shared.flight).prs.remove(id);
        for j in joined.into_iter().flatten() {
            answer(j, id, result, self);
        }
    }

    /// Whoever joined a PR that's neither applied nor failed when its run ends: `Failed`.
    pub(super) fn settle_rest(&mut self) {
        let rest: Vec<(NodeId, Vec<Joiner>)> = lock(&self.shared.flight).prs.drain().collect();
        for (id, joined) in rest {
            for j in joined {
                answer(j, &id, Refreshed::Failed, self);
            }
        }
    }

    fn settle_ids(&mut self, ids: &[NodeId]) {
        for id in ids {
            self.settle(id, Refreshed::Failed);
        }
    }

    // ---- serving ----

    /// POL-15: every demand queued, one after the other, before anything else is sent.
    pub(super) async fn serve_demand(&mut self) {
        loop {
            self.drain();
            let Some(next) = self.dm.queue.pop_front() else {
                return;
            };
            match next {
                Queued::Refresh { id, txs } => self.serve_refresh(id, txs).await,
                Queued::Lookup { name, manual, txs } => self.serve_lookup(name, manual, txs).await,
            }
        }
    }

    /// POL-15: one PR by ID now, unless its window holds, the policy sends none, a backoff runs or
    /// there's no token. API-13: a wait's arrival refresh is this one, sent ahead of a poll in flight;
    /// capped it's still served, in the 30 s window.
    async fn serve_refresh(&mut self, id: NodeId, txs: Vec<oneshot::Sender<Refreshed>>) {
        let target = self.d.store.targets().into_iter().find(|t| t.id == id);
        let not_sent = |txs: Vec<oneshot::Sender<Refreshed>>| {
            for tx in txs {
                let _ = tx.send(Refreshed::NotSent);
            }
        };
        let Some(target) = target else {
            not_sent(txs);
            return;
        };
        let policy = self.policy();
        // POL-15: inside the window, what's in memory; POL-23, POL-19: none sent.
        if !cadence::may_force_pr(&target.timing, &policy, self.now())
            || !policy.demand
            || self.backoff.active()
        {
            not_sent(txs);
            return;
        }
        // POL-20: in `auth`, a demand reads the token again (at most once in 5 s).
        if self.token_for_demand().await.is_err() {
            not_sent(txs);
            return;
        }
        {
            let mut flight = lock(&self.shared.flight);
            flight
                .prs
                .entry(id.clone())
                .or_default()
                .extend(txs.into_iter().map(Joiner::Refresh));
        }
        self.run_refresh(vec![id], "forced", false).await;
    }

    /// The token for a demand (POL-20): in `auth`, read again; a new one opens the one try.
    async fn token_for_demand(&mut self) -> Result<(), AuthProblem> {
        self.park();
        let token = self.auth.token(true).await;
        self.wake();
        self.after_auth_read();
        token.map(|_| {
            if self.auth.problem().is_some() {
                self.try_open = true;
            }
        })
    }

    /// A refresh by ID outside a poll: hot, forced or a follow-up. 40 a request, then check pages and
    /// items straight after (POL-7, POL-8, POL-12); every request marks its PRs with its own send time.
    pub(super) async fn run_refresh(
        &mut self,
        ids: Vec<NodeId>,
        kind: &'static str,
        follow_up: bool,
    ) {
        let saved = (self.dm.not_poll, self.dm.follow_up_run);
        self.dm.not_poll = true;
        self.dm.follow_up_run = follow_up;
        let mut p = PollState::refresh();
        let targets = self.d.store.targets();
        let mut whole = true;
        for batch in cadence::batches(&ids) {
            let query = Query {
                nodes: node_queries(&batch, &targets),
                ..Query::default()
            };
            let Some((answer, meta)) = self.step(&mut p, &query, kind).await else {
                // POL-19: the request failed as a whole; its PRs are stale.
                self.abandon(&mut p, &query.nodes);
                whole = false;
                break;
            };
            // POL-21: a failing node is the PR's own; the rest are applied.
            self.take_nodes(&mut p, &query.nodes, answer.nodes, &meta);
            if self.stopped(&mut p) {
                whole = false;
                break;
            }
        }
        if whole {
            self.pages_and_items(&mut p).await;
        }
        self.settle_ids(&ids);
        (self.dm.not_poll, self.dm.follow_up_run) = saved;
    }

    /// POL-16: one read-through, shared by every caller for that name.
    async fn serve_lookup(&mut self, name: PrName, manual: bool, txs: Vec<LookupTx>) {
        if let Some(t) = self.d.store.lookup(&name)
            && t.class == Class::First
        {
            let mut flight = lock(&self.shared.flight);
            if let Some(joined) = flight.prs.get_mut(&t.id) {
                // Its first refresh is in flight: joined, not sent twice.
                joined.extend(txs.into_iter().map(Joiner::Lookup));
                return;
            }
        }
        lock(&self.shared.flight)
            .lookups
            .insert(name.clone(), LookupFlight { manual, txs });
        let result = self.read_through_now(&name).await;
        let flight = lock(&self.shared.flight).lookups.remove(&name);
        for tx in flight.map(|f| f.txs).into_iter().flatten() {
            let _ = tx.send(result.clone());
        }
    }

    /// Whether a `track` shares the read-through for this name (API-15: it reserves as `manual`).
    fn lookup_manual(&self, name: &PrName) -> bool {
        lock(&self.shared.flight)
            .lookups
            .get(name)
            .is_some_and(|f| f.manual)
    }

    /// POL-16, STA-3, API-3.
    async fn read_through_now(&mut self, name: &PrName) -> Result<NodeId, Refusal> {
        match self.d.store.lookup(name) {
            Some(t) if matches!(t.class, Class::Open | Class::Ended) => {
                // POL-16: a record now: answered from memory.
                if self.lookup_manual(name) {
                    self.d.store.set_manual(&t.id);
                }
                return Ok(t.id);
            }
            // STA-8: a loaded PR is read through, keeping its saved source; so is a found one whose
            // first refresh isn't in flight.
            Some(_) => {}
            None => {
                // STA-3: at the limit nothing is sent.
                if self.d.store.places_free() == 0 {
                    return Err(limit_refusal());
                }
            }
        }
        // POL-20, API-3: in `auth`, after reading the token again.
        if let Err(problem) = self.token_for_demand().await {
            return Err(Refusal {
                code: RefusalCode::Auth,
                message: problem.message,
                until: None,
            });
        }
        let policy = self.policy();
        if !policy.demand {
            // POL-23: exhausted or under a secondary limit.
            return Err(self.refusal(PollOutcome::RateLimited));
        }
        let in_backoff = self.backoff.active();
        // POL-16, POL-19: tried during a backoff, at most one per step.
        if in_backoff && !self.backoff.take_read_through() {
            return Err(self.refusal(PollOutcome::GithubUnavailable));
        }
        let query = Query {
            lookup: Some(name.clone()),
            ..Query::default()
        };
        let saved = self.dm.not_poll;
        self.dm.not_poll = true;
        self.dm.keep_backoff = in_backoff;
        let sent = self.request(&query, "lookup", None).await;
        self.dm.keep_backoff = false;
        let result = self.take_lookup_answer(name, sent).await;
        self.dm.not_poll = saved;
        result
    }

    async fn take_lookup_answer(
        &mut self,
        name: &PrName,
        sent: super::gate::Sent,
    ) -> Result<NodeId, Refusal> {
        let answer = match (sent.result, sent.meta) {
            (Ok(answer), Some(meta)) => (answer, meta),
            (Err(Fail::Auth), _) => return Err(self.refusal(PollOutcome::Auth)),
            (Err(Fail::RateLimited), _) => return Err(self.refusal(PollOutcome::RateLimited)),
            _ => return Err(self.refusal(PollOutcome::GithubUnavailable)),
        };
        let (answer, meta) = answer;
        let kind = self.d.token_source.kind().as_str();
        let (read, items) = match answer.lookup {
            Some(LookupAnswer::Found { read, items }) => (*read, items),
            Some(LookupAnswer::NotFound) => {
                return Err(Refusal {
                    code: RefusalCode::NotFound,
                    message: format!(
                        "GitHub answered NOT_FOUND for {name}: there is no such PR, or the token from {kind} can't see its repository."
                    ),
                    until: None,
                });
            }
            Some(LookupAnswer::Error { type_ }) => {
                return Err(Refusal {
                    code: RefusalCode::GithubUnavailable,
                    message: format!(
                        "GitHub answered {name} with an error of type {type_}, with the token from {kind}."
                    ),
                    until: None,
                });
            }
            None => {
                return Err(Refusal {
                    code: RefusalCode::GithubUnavailable,
                    message: format!(
                        "GitHub's answer for {name} held no PR, with the token from {kind}."
                    ),
                    until: None,
                });
            }
        };
        // POL-8: further check pages first; they're one read with the first.
        let mut held = Held {
            id: read.id.clone(),
            read,
            items: Some(items),
            meta,
            first: true,
        };
        if wants_page(&held.read) {
            let mut p = PollState::refresh();
            p.checks.push(held);
            let whole = self.fetch_pages(&mut p).await;
            match p.checks.pop() {
                Some(h) if whole => held = h,
                _ => {
                    return Err(match p.stop {
                        Some(PollOutcome::Auth) => self.refusal(PollOutcome::Auth),
                        Some(PollOutcome::RateLimited) => self.refusal(PollOutcome::RateLimited),
                        _ => self.refusal(PollOutcome::GithubUnavailable),
                    });
                }
            }
        }
        // STA-3: checked again; a poll may have taken the last place meanwhile. MOD-1: an ID already
        // tracked, under another name too, is that PR and takes no place; STA-1 keeps the stronger source.
        let source = if self.lookup_manual(name) {
            Source::Manual
        } else {
            Source::Asked
        };
        let id = held.id.clone();
        if self.d.store.reserve(&id, &held.read.pr, source).is_err() {
            return Err(limit_refusal());
        }
        // Every refresh moves `last_sent`: a read-through with its own send time.
        self.d
            .store
            .mark_sent(std::slice::from_ref(&id), held.meta.sent_mono);
        if self.in_poll {
            self.dm.sent_in_poll.insert(id.clone());
        }
        let items = held.items.take().unwrap_or_default();
        self.apply(
            &id,
            &held.read,
            ItemsOutcome::Read(&items),
            &held.meta,
            None,
        );
        Ok(id)
    }
}

/// One joiner's answer.
fn answer(j: Joiner, id: &NodeId, result: Refreshed, run: &Run) {
    match j {
        Joiner::Refresh(tx) => {
            let _ = tx.send(result);
        }
        Joiner::Lookup(tx) => {
            let r = if result == Refreshed::Applied {
                Ok(id.clone())
            } else {
                Err(run.refusal(PollOutcome::GithubUnavailable))
            };
            let _ = tx.send(r);
        }
    }
}
