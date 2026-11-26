//! What a PR's errors add up to (POL-21, POL-24): the run that makes it `gone`, `pr_failing` and the
//! `pr error` log line. T21 calls these three; T42 fills them.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::clock::{Mono, Time};
use crate::model::{NodeId, Status};
use crate::status::WarningCode;

use super::EngineDeps;

/// POL-21: how long a PR's `NOT_FOUND`s, or its errors, must have lasted.
const RUN_FOR: Duration = Duration::from_secs(5 * 60);
/// POL-21: and how many `NOT_FOUND`s make a run.
const RUN_COUNT: u32 = 3;

/// How one node of a request answered.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum NodeOutcome {
    Applied,
    NotFound,
    Error(String),
}

/// One node's result.
pub(super) struct NodeResult<'a> {
    pub id: &'a NodeId,
    /// The request was one of a poll's (MOD-11: `gone` only from a poll).
    pub in_poll: bool,
    pub outcome: &'a NodeOutcome,
    pub sent_wall: Time,
    pub sent_mono: Mono,
}

/// A poll that ended, whole or abandoned.
pub(super) struct PollDone<'a> {
    /// Its `viewer.login`; `None` when the finding failed.
    pub login: Option<&'a str>,
    /// The tracked PRs it refreshed, and how each answered.
    pub refreshed: &'a [(NodeId, NodeOutcome)],
}

/// One PR's run of errors, of any kind of refresh (POL-21's `pr_failing`, POL-24).
struct ErrorRun {
    began: Mono,
    latest_type: String,
}

/// One PR's run of `NOT_FOUND`s in polls (POL-21, MOD-11).
struct MissingRun {
    first: Mono,
    count: u32,
}

/// What a PR's errors add up to.
#[derive(Default)]
pub(super) struct PrErrors {
    errors: HashMap<NodeId, ErrorRun>,
    missing: HashMap<NodeId, MissingRun>,
    /// This poll's `NOT_FOUND`s, judged once at its end (MOD-11).
    pending: HashMap<NodeId, (Time, Mono)>,
    /// Made `gone` here: their `NOT_FOUND`s are no run (a `Loaded` entry has no status to tell).
    gone: HashSet<NodeId>,
}

impl PrErrors {
    /// One per node result.
    pub(super) fn node(&mut self, deps: &EngineDeps, result: &NodeResult<'_>) {
        let id = result.id;
        let type_ = match result.outcome {
            NodeOutcome::Applied => {
                // POL-21: an applied refresh ends every run.
                self.forget(deps, id);
                self.gone.remove(id);
                return;
            }
            NodeOutcome::NotFound => "NOT_FOUND",
            NodeOutcome::Error(t) => t.as_str(),
        };
        if self.gone.contains(id) {
            return;
        }
        // A first refresh that failed gave its place back: there's no PR to follow.
        let Some(target) = deps.store.targets().into_iter().find(|t| t.id == *id) else {
            return;
        };
        // POL-21: a gone PR keeps answering `NOT_FOUND` at its slower rate: no run, no warning, no line.
        if target.status == Some(Status::Gone) {
            return;
        }
        // POL-24: once, when the run begins.
        let run = self.errors.entry(id.clone()).or_insert_with(|| {
            deps.logger
                .warn(format_args!("pr error pr={} type={type_}", target.pr));
            ErrorRun {
                began: result.sent_mono,
                latest_type: String::new(),
            }
        });
        run.latest_type = type_.to_string();
        // POL-21: errors of any type have lasted 5 minutes with no applied refresh between.
        let lasted = result.sent_mono.since(run.began);
        if lasted >= RUN_FOR {
            deps.warnings.raise(
                WarningCode::PrFailing,
                id.0.as_str(),
                format!(
                    "{} has failed to refresh for {} minutes: the latest error is {}, with the token from {}.",
                    target.pr,
                    lasted.as_secs() / 60,
                    run.latest_type,
                    deps.token_source.kind().as_str()
                ),
            );
        }
        if result.in_poll {
            if type_ == "NOT_FOUND" {
                self.pending
                    .insert(id.clone(), (result.sent_wall, result.sent_mono));
            } else {
                // POL-21: another type, in a poll, ends the run of `NOT_FOUND`s.
                self.missing.remove(id);
                self.pending.remove(id);
            }
        }
    }

    /// One per finished poll.
    pub(super) fn poll_done(&mut self, deps: &EngineDeps, done: &PollDone<'_>) {
        let pending = std::mem::take(&mut self.pending);
        // MOD-11: a poll whose finding failed has no login, and nothing in it counts.
        let Some(login) = done.login else {
            return;
        };
        // MOD-11: not when two or more tracked PRs were refreshed and every one is missing.
        let mut seen: HashMap<&NodeId, bool> = HashMap::new();
        for (id, outcome) in done.refreshed {
            let missing = *outcome == NodeOutcome::NotFound;
            let e = seen.entry(id).or_insert(true);
            *e = *e && missing;
        }
        if seen.len() >= 2 && seen.values().all(|m| *m) {
            return;
        }
        let mut pending: Vec<_> = pending.into_iter().collect();
        pending.sort_by(|a, b| a.0.cmp(&b.0));
        for (id, (wall, mono)) in pending {
            if seen.get(&id) == Some(&false) {
                continue;
            }
            // MOD-11: the same login, without regard to case.
            if !deps
                .store
                .login_of(&id)
                .is_some_and(|l| l.eq_ignore_ascii_case(login))
            {
                continue;
            }
            let run = self.missing.entry(id.clone()).or_insert(MissingRun {
                first: mono,
                count: 0,
            });
            run.count += 1;
            if run.count >= RUN_COUNT && mono.since(run.first) >= RUN_FOR {
                // STA-2: `seen_at` is the send time of the request that completed the run.
                deps.store.mark_gone(&id, wall);
                self.forget(deps, &id);
                self.gone.insert(id);
            }
        }
    }

    /// One per tick: a PR no longer tracked has no runs.
    pub(super) fn tick(&mut self, deps: &EngineDeps) {
        let tracked: HashSet<NodeId> = deps.store.targets().into_iter().map(|t| t.id).collect();
        let stale: Vec<NodeId> = self
            .errors
            .keys()
            .chain(self.missing.keys())
            .chain(self.gone.iter())
            .filter(|id| !tracked.contains(*id))
            .cloned()
            .collect();
        for id in stale {
            self.forget(deps, &id);
            self.gone.remove(&id);
        }
    }

    fn forget(&mut self, deps: &EngineDeps, id: &NodeId) {
        self.errors.remove(id);
        self.missing.remove(id);
        self.pending.remove(id);
        deps.warnings.clear(WarningCode::PrFailing, id.0.as_str());
    }
}
