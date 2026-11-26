//! The poll (POL-4 to POL-8, POL-10, POL-12), its whole-request failures (POL-19, POL-24, POL-25) and a
//! node's error as far as `store.attempt_failed` (POL-21).

use std::collections::{HashMap, HashSet};

use crate::clock::{Mono, Time};
use crate::github::{Answer, Find, Found, FoundPr, NodeAnswer, NodeQuery, Query, Want};
use crate::model::{ItemsOutcome, ItemsRead, NodeId, PrRead, ReadMeta, ReadState, Source};
use crate::poller::{FirstPoll, PollOutcome, PollReport, cadence};
use crate::state::{Applied, Class, MAX_PLACES, Target};
use crate::status::WarningCode;

use super::gate::Fail;
use super::lock;
use super::pr_errors::{NodeOutcome, NodeResult, PollDone};
use super::run::Run;

/// POL-8: check pages are fetched while a read holds fewer than this.
const MAX_CHECKS: usize = 300;
/// POL-7: PRs a request.
const BATCH: usize = 40;

/// A read in hand, waiting for its check pages or its items (POL-8, POL-12).
struct Held {
    id: NodeId,
    read: PrRead,
    items: Option<ItemsRead>,
    meta: ReadMeta,
    first: bool,
}

#[derive(Default)]
struct PollState {
    /// The poll's own send time: its first request's (P10).
    sent: Option<(Mono, Time)>,
    login: Option<String>,
    cost: u32,
    answered: bool,
    /// The last answer said `remaining: 0` (POL-25): the poll stops once it's applied.
    zero: bool,
    /// The open list, after the owners filter (POL-5).
    listed: HashSet<NodeId>,
    found_failed: bool,
    finding_complete: bool,
    left_out: bool,
    /// Found this poll, a place reserved, the first refresh not yet sent.
    found_new: Vec<NodeId>,
    /// The PRs a request of this poll was sent for.
    sent_ids: HashSet<NodeId>,
    checks: Vec<Held>,
    items: Vec<Held>,
    /// The PRs whose refresh this poll applied and read open (POL-5).
    applied_open: Vec<NodeId>,
    results: Vec<(NodeId, NodeOutcome)>,
    stop: Option<PollOutcome>,
}

fn outcome_of(f: &Fail) -> PollOutcome {
    match f {
        Fail::Auth => PollOutcome::Auth,
        Fail::RateLimited => PollOutcome::RateLimited,
        Fail::Unavailable => PollOutcome::GithubUnavailable,
    }
}

/// POL-5: compared without case; empty `owners` takes every owner.
fn owner_ok(owners: &[String], owner: &str) -> bool {
    owners.is_empty() || owners.iter().any(|o| o.eq_ignore_ascii_case(owner))
}

impl Run {
    /// One poll, whole or abandoned. `forced`: API-16's (POL-15's window runs from it).
    pub(super) async fn poll(&mut self, forced: bool) -> PollReport {
        self.in_poll = true;
        lock(&self.shared.flight).polling = true;
        let mut p = PollState::default();
        self.steps(&mut p).await;
        // A `poll_now` that reached the channel meanwhile joins this poll.
        self.between_requests();
        let joined = {
            let mut flight = lock(&self.shared.flight);
            flight.polling = false;
            std::mem::take(&mut flight.joined)
        };
        self.in_poll = false;
        let report = self.finish(p, forced);
        for tx in joined {
            let _ = tx.send(PollReport {
                outcome: PollOutcome::Shared,
                cost: 0,
            });
        }
        report
    }

    async fn steps(&mut self, p: &mut PollState) {
        // POL-4: the first request is the finding's first page and the first 40 due PRs.
        let targets = self.d.store.targets();
        let due = cadence::in_poll(&targets, None, self.tier, &self.d.config, self.now());
        let first: Vec<NodeId> = due.into_iter().take(BATCH).collect();
        let query = Query {
            find: Some(Find::default()),
            nodes: node_queries(&first, &targets),
            lookup: None,
        };
        let Some((answer, meta)) = self.step(p, &query, "poll").await else {
            self.abandon(p, &query.nodes);
            return;
        };
        let mut next = self.take_found(p, answer.found);
        self.take_nodes(p, &query.nodes, answer.nodes, &meta);
        if self.stopped(p) {
            return;
        }

        // Further finding pages (POL-7), until every place is taken (STA-3).
        while let Some(after) = next.take() {
            if self.d.store.places_free() == 0 {
                next = Some(after);
                break;
            }
            let query = Query {
                find: Some(Find { after: Some(after) }),
                ..Query::default()
            };
            let Some((answer, _)) = self.step(p, &query, "find").await else {
                self.abandon(p, &[]);
                return;
            };
            next = self.take_found(p, answer.found);
            if self.stopped(p) {
                return;
            }
        }
        p.finding_complete = !p.found_failed && next.is_none();

        // Further batches, then the PRs the list shows that weren't refreshed yet (POL-10).
        for listed in [false, true] {
            loop {
                let targets = self.d.store.targets();
                let set = listed.then_some(&p.listed);
                let due: Vec<NodeId> =
                    cadence::in_poll(&targets, set, self.tier, &self.d.config, self.now())
                        .into_iter()
                        .filter(|id| !p.sent_ids.contains(id))
                        .take(BATCH)
                        .collect();
                if due.is_empty() {
                    break;
                }
                let query = Query {
                    nodes: node_queries(&due, &targets),
                    ..Query::default()
                };
                let Some((answer, meta)) = self.step(p, &query, "refresh").await else {
                    self.abandon(p, &query.nodes);
                    return;
                };
                self.take_nodes(p, &query.nodes, answer.nodes, &meta);
                if self.stopped(p) {
                    return;
                }
            }
        }

        // The first refresh of newly found PRs, with POL-12's fields (POL-4).
        while !p.found_new.is_empty() {
            let n = p.found_new.len().min(BATCH);
            let batch: Vec<NodeId> = p.found_new.drain(..n).collect();
            let query = Query {
                nodes: batch
                    .into_iter()
                    .map(|id| NodeQuery {
                        id,
                        want: Want::RecordAndItems,
                    })
                    .collect(),
                ..Query::default()
            };
            let Some((answer, meta)) = self.step(p, &query, "first").await else {
                self.abandon(p, &query.nodes);
                return;
            };
            self.take_nodes(p, &query.nodes, answer.nodes, &meta);
            if self.stopped(p) {
                return;
            }
        }

        // Check pages (POL-8): a PR's pages are one read.
        loop {
            let need: Vec<usize> = (0..p.checks.len())
                .filter(|&i| wants_page(&p.checks[i].read))
                .take(BATCH)
                .collect();
            if need.is_empty() {
                break;
            }
            let query = Query {
                nodes: need
                    .iter()
                    .map(|&i| NodeQuery {
                        id: p.checks[i].id.clone(),
                        want: Want::ChecksPage {
                            after: p.checks[i]
                                .read
                                .checks
                                .next_page
                                .clone()
                                .unwrap_or_default(),
                        },
                    })
                    .collect(),
                ..Query::default()
            };
            let Some((answer, meta)) = self.step(p, &query, "checks").await else {
                self.abandon(p, &[]);
                return;
            };
            let mut failed = Vec::new();
            let mut answers = answer.nodes.into_iter();
            for &i in &need {
                match answers.next() {
                    Some(NodeAnswer::ChecksPage(page)) => p.checks[i].read.checks.append_page(page),
                    Some(NodeAnswer::NotFound) => failed.push((i, Some(NodeOutcome::NotFound))),
                    Some(NodeAnswer::Error { type_ }) => {
                        failed.push((i, Some(NodeOutcome::Error(type_))));
                    }
                    _ => failed.push((i, None)),
                }
            }
            // POL-8: a failed page applies nothing; the record keeps its values and `error`.
            for (i, outcome) in failed.into_iter().rev() {
                let held = p.checks.remove(i);
                self.d.store.attempt_failed(&held.id, None, self.wall());
                self.d.store.release(&held.id);
                if let Some(outcome) = outcome {
                    self.report_node(&held.id, &outcome, &meta);
                    p.results.push((held.id, outcome));
                }
            }
            if self.stopped(p) {
                return;
            }
        }
        let mut done = std::mem::take(&mut p.checks);
        by_name(&mut done);
        for held in done {
            self.finish_read(p, held);
        }

        // New items (POL-12): straight after, in batches of 40; each PR's part on its own.
        while !p.items.is_empty() {
            let n = p.items.len().min(BATCH);
            let batch: Vec<Held> = p.items.drain(..n).collect();
            let query = Query {
                nodes: batch
                    .iter()
                    .map(|h| NodeQuery {
                        id: h.id.clone(),
                        want: Want::Items,
                    })
                    .collect(),
                ..Query::default()
            };
            let Some((answer, meta)) = self.step(p, &query, "items").await else {
                // POL-12: the refreshes are applied with the old counts (in `abandon`).
                let rest = std::mem::take(&mut p.items);
                p.items = batch;
                p.items.extend(rest);
                self.abandon(p, &[]);
                return;
            };
            let mut parts: Vec<(Held, Option<NodeAnswer>)> = batch
                .into_iter()
                .zip(
                    answer
                        .nodes
                        .into_iter()
                        .map(Some)
                        .chain(std::iter::repeat_with(|| None)),
                )
                .collect();
            // MOD-15: in the order of `pr`.
            parts.sort_by(|a, b| a.0.read.pr.cmp(&b.0.read.pr));
            for (held, part) in parts {
                match part {
                    Some(NodeAnswer::Items(items)) => {
                        self.apply_held(p, &held, ItemsOutcome::Read(&items));
                    }
                    other => {
                        // POL-12: the part failed; the refresh is applied with the old counts.
                        self.apply_held(p, &held, ItemsOutcome::Failed);
                        let outcome = match other {
                            Some(NodeAnswer::NotFound) => Some(NodeOutcome::NotFound),
                            Some(NodeAnswer::Error { type_ }) => Some(NodeOutcome::Error(type_)),
                            _ => None,
                        };
                        if let Some(outcome) = outcome {
                            self.report_node(&held.id, &outcome, &meta);
                            p.results.push((held.id, outcome));
                        }
                    }
                }
            }
            if self.stopped(p) {
                return;
            }
        }
    }

    /// One request of the poll, through the gate. `None`: it failed as a whole (`p.stop` says why).
    async fn step(
        &mut self,
        p: &mut PollState,
        query: &Query,
        kind: &'static str,
    ) -> Option<(Answer, ReadMeta)> {
        if p.sent.is_some() {
            // POL-15: demand goes next, between a poll's requests (T25).
            self.between_requests();
        }
        let mark_at = p.sent.map(|(m, _)| m);
        let sent = self.request(query, kind, mark_at).await;
        p.cost += sent.cost;
        if let Some(meta) = &sent.meta {
            if p.sent.is_none() {
                p.sent = Some((meta.sent_mono, meta.sent_wall));
            }
            p.sent_ids.extend(query.nodes.iter().map(|n| n.id.clone()));
        }
        match (sent.result, sent.meta) {
            (Ok(answer), Some(meta)) => {
                p.answered = true;
                // POL-25: an answer with `remaining: 0` is applied, then the poll stops.
                p.zero = answer.rate.as_ref().is_some_and(|r| r.remaining == 0);
                Some((answer, meta))
            }
            (Err(f), _) => {
                p.stop = Some(outcome_of(&f));
                None
            }
            (Ok(_), None) => None,
        }
    }

    /// After an answer was taken in: stop when it said `remaining: 0`.
    fn stopped(&mut self, p: &mut PollState) -> bool {
        if p.zero {
            p.stop = Some(PollOutcome::RateLimited);
            self.abandon(p, &[]);
            true
        } else {
            false
        }
    }

    /// POL-4, POL-5, POL-7, STA-3: one page of the finding. Returns the next page's cursor.
    fn take_found(&mut self, p: &mut PollState, found: Option<Found>) -> Option<String> {
        let Some(found) = found else {
            // An error on `viewer`: the finding failed; the nodes still count.
            p.found_failed = true;
            return None;
        };
        if p.login.is_none() {
            // OPS-5: the login at the first poll and when it changes.
            if self.login.as_deref() != Some(found.login.as_str()) {
                self.d
                    .logger
                    .info(format_args!("login login={}", found.login));
            }
            self.login = Some(found.login.clone());
            p.login = Some(found.login.clone());
        }
        let mut new: Vec<FoundPr> = Vec::new();
        for f in found.prs {
            // POL-5: filtered after finding.
            if !owner_ok(&self.d.config.owners, &f.pr.owner) {
                continue;
            }
            p.listed.insert(f.id.clone());
            if self.d.store.is_tracked(&f.id) {
                // MOD-1: a found PR whose ID is tracked is that PR; STA-1 keeps the stronger source.
                let _ = self.d.store.reserve(&f.id, &f.pr, Source::Authored);
            } else {
                new.push(f);
            }
        }
        // STA-3: newest `updated_at` first, while places are free.
        new.sort_by_key(|f| std::cmp::Reverse(f.updated_at));
        for f in new {
            if self.d.store.places_free() > 0
                && self.d.store.reserve(&f.id, &f.pr, Source::Authored).is_ok()
            {
                p.found_new.push(f.id);
            } else {
                p.left_out = true;
            }
        }
        found.next
    }

    /// The nodes of one answer, in the query's order.
    fn take_nodes(
        &mut self,
        p: &mut PollState,
        nodes: &[NodeQuery],
        answers: Vec<NodeAnswer>,
        meta: &ReadMeta,
    ) {
        let mut answers = answers.into_iter();
        let mut reads = Vec::new();
        for nq in nodes {
            match answers.next() {
                Some(NodeAnswer::Record { read, items })
                    if matches!(nq.want, Want::Record | Want::RecordAndItems) =>
                {
                    reads.push(Held {
                        id: nq.id.clone(),
                        read: *read,
                        items,
                        meta: meta.clone(),
                        first: nq.want == Want::RecordAndItems,
                    });
                }
                Some(NodeAnswer::NotFound) => {
                    self.node_failed(p, &nq.id, NodeOutcome::NotFound, meta);
                }
                Some(NodeAnswer::Error { type_ }) => {
                    self.node_failed(p, &nq.id, NodeOutcome::Error(type_), meta);
                }
                _ => {
                    // Missing, or of another kind: not applied, and no type to name.
                    self.d.store.attempt_failed(&nq.id, None, self.wall());
                    self.d.store.release(&nq.id);
                }
            }
        }
        // MOD-15: the PRs of one response are applied in the order of `pr`.
        by_name(&mut reads);
        for held in reads {
            self.hold_or_finish(p, held);
        }
    }

    fn hold_or_finish(&mut self, p: &mut PollState, held: Held) {
        if wants_page(&held.read) {
            p.checks.push(held);
        } else {
            self.finish_read(p, held);
        }
    }

    /// POL-12: a raised count holds the refresh for its items; else it's applied now.
    fn finish_read(&mut self, p: &mut PollState, held: Held) {
        if !held.first && self.d.store.counts_rose(&held.id, &held.read) {
            p.items.push(held);
            return;
        }
        match &held.items {
            Some(items) => self.apply_held(p, &held, ItemsOutcome::Read(items)),
            None => self.apply_held(p, &held, ItemsOutcome::NotAsked),
        }
    }

    fn apply_held(&mut self, p: &mut PollState, held: &Held, items: ItemsOutcome<'_>) {
        let applied = self.apply(&held.id, &held.read, items, &held.meta, p.login.as_deref());
        if applied != Applied::Discarded && held.read.state == ReadState::Open {
            p.applied_open.push(held.id.clone());
        }
        p.results.push((held.id.clone(), NodeOutcome::Applied));
    }

    /// The one place a read is applied (MOD-15: a response's PRs in the order of `pr`). T25 hooks the follow-up here.
    pub(super) fn apply(
        &mut self,
        id: &NodeId,
        read: &PrRead,
        items: ItemsOutcome<'_>,
        meta: &ReadMeta,
        login: Option<&str>,
    ) -> Applied {
        let applied = self.d.store.apply(id, read, items, meta, login);
        self.report_node(id, &NodeOutcome::Applied, meta);
        applied
    }

    /// POL-21, POL-24: the other nodes are applied; this one keeps its values and is stale. Nothing more (T42).
    fn node_failed(
        &mut self,
        p: &mut PollState,
        id: &NodeId,
        outcome: NodeOutcome,
        meta: &ReadMeta,
    ) {
        let type_ = match &outcome {
            NodeOutcome::NotFound => "NOT_FOUND",
            NodeOutcome::Error(t) => t.as_str(),
            NodeOutcome::Applied => "",
        };
        self.d.store.attempt_failed(id, Some(type_), self.wall());
        // STA-3: a first refresh that fails gives the place back.
        self.d.store.release(id);
        self.report_node(id, &outcome, meta);
        p.results.push((id.clone(), outcome));
    }

    fn report_node(&mut self, id: &NodeId, outcome: &NodeOutcome, meta: &ReadMeta) {
        let result = NodeResult {
            id,
            in_poll: self.in_poll,
            outcome,
            sent_wall: meta.sent_wall,
            sent_mono: meta.sent_mono,
        };
        self.pr_errors.node(&self.d, &result);
    }

    /// POL-19, POL-24: the poll is abandoned. The failed request's PRs, and those of the requests not
    /// yet sent, are stale; a first refresh gives its place back; what answered stays applied.
    fn abandon(&mut self, p: &mut PollState, failed: &[NodeQuery]) {
        let wall = self.wall();
        let store = self.d.store.clone();
        for nq in failed {
            if matches!(nq.want, Want::Record | Want::RecordAndItems) {
                store.attempt_failed(&nq.id, None, wall);
                store.release(&nq.id);
            }
        }
        let targets = store.targets();
        for id in cadence::in_poll(
            &targets,
            Some(&p.listed),
            self.tier,
            &self.d.config,
            self.now(),
        ) {
            if !p.sent_ids.contains(&id) {
                store.attempt_failed(&id, None, wall);
            }
        }
        for id in p.found_new.drain(..) {
            store.release(&id);
        }
        for held in std::mem::take(&mut p.checks) {
            store.attempt_failed(&held.id, None, wall);
            store.release(&held.id);
        }
        for held in std::mem::take(&mut p.items) {
            // POL-12: applied with the old counts, so the next refresh asks again; POL-19: stale.
            self.apply_held(p, &held, ItemsOutcome::Failed);
            store.attempt_failed(&held.id, None, wall);
        }
    }

    /// The poll's end: POL-5's demotion, `pr_limit`, T42's call, the times, `first_poll`.
    fn finish(&mut self, p: PollState, forced: bool) -> PollReport {
        let outcome = p.stop.unwrap_or(PollOutcome::Polled);
        // POL-5: once, and only when the finding was read to its last page.
        if p.finding_complete {
            let unlisted: Vec<NodeId> = p
                .applied_open
                .iter()
                .filter(|id| !p.listed.contains(*id))
                .cloned()
                .collect();
            if !unlisted.is_empty() {
                self.d.store.demote_unlisted(&unlisted);
            }
        }
        // POL-7, STA-3: raised when found PRs are left out, cleared by a poll that leaves none out.
        if p.left_out {
            self.d.warnings.raise(
                WarningCode::PrLimit,
                "",
                format!(
                    "More open PRs were found than the {MAX_PLACES} circitor can track: the rest are left out."
                ),
            );
        } else if p.finding_complete {
            self.d.warnings.clear(WarningCode::PrLimit, "");
        }
        let done = PollDone {
            login: p.login.as_deref(),
            refreshed: &p.results,
        };
        self.pr_errors.poll_done(&self.d, &done);

        let (sent_mono, sent_wall) = p.sent.unwrap_or((self.now(), self.wall()));
        self.last_poll_sent = Some(sent_mono);
        if forced {
            self.last_forced = Some(sent_mono);
        }
        self.last_poll = Some((sent_wall, outcome));
        if p.answered {
            self.last_poll_at = Some(sent_wall);
        }
        self.poll_asap = false;
        // API-18, POL-19: finished once a request answered; until then the problem's error.
        let finished = *self.shared.first_poll.borrow() == FirstPoll::Finished;
        if p.answered {
            if !finished {
                self.shared.first_poll.send_replace(FirstPoll::Finished);
            }
        } else if !finished {
            let refusal = self.refusal(outcome);
            self.shared
                .first_poll
                .send_replace(FirstPoll::Unanswered(refusal));
        }
        PollReport {
            outcome,
            cost: p.cost,
        }
    }
}

fn by_name(reads: &mut [Held]) {
    reads.sort_by(|a, b| a.read.pr.cmp(&b.read.pr));
}

/// POL-8: more pages while the read has `next_page` and fewer than 300 items.
fn wants_page(read: &PrRead) -> bool {
    read.checks.next_page.is_some() && read.checks.items.len() < MAX_CHECKS
}

/// A loaded PR has no record: it's asked as `RecordAndItems` (POL-4, STA-8).
fn node_queries(ids: &[NodeId], targets: &[Target]) -> Vec<NodeQuery> {
    let class: HashMap<&NodeId, Class> = targets.iter().map(|t| (&t.id, t.class)).collect();
    ids.iter()
        .map(|id| NodeQuery {
            id: id.clone(),
            want: if class.get(id) == Some(&Class::Loaded) {
                Want::RecordAndItems
            } else {
                Want::Record
            },
        })
        .collect()
}
