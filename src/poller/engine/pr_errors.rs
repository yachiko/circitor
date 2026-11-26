//! What a PR's errors add up to (POL-21, POL-24): the run that makes it `gone`, `pr_failing` and the
//! `pr error` log line. T42 fills these three calls; T21 only calls them.

use crate::clock::{Mono, Time};
use crate::model::NodeId;

use super::EngineDeps;

/// How one node of a request answered.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum NodeOutcome {
    Applied,
    NotFound,
    Error(String),
}

/// One node's result.
#[allow(dead_code)] // T42 reads these
pub(super) struct NodeResult<'a> {
    pub id: &'a NodeId,
    /// The request was one of a poll's (MOD-11: `gone` only from a poll).
    pub in_poll: bool,
    pub outcome: &'a NodeOutcome,
    pub sent_wall: Time,
    pub sent_mono: Mono,
}

/// A poll that ended, whole or abandoned.
#[allow(dead_code)] // T42 reads these
pub(super) struct PollDone<'a> {
    /// Its `viewer.login`; `None` when the finding failed.
    pub login: Option<&'a str>,
    /// The tracked PRs it refreshed, and how each answered.
    pub refreshed: &'a [(NodeId, NodeOutcome)],
}

/// T42's state.
#[derive(Default)]
pub(super) struct PrErrors {}

impl PrErrors {
    /// One per node result.
    pub(super) fn node(&mut self, _deps: &EngineDeps, _result: &NodeResult<'_>) {
        // T42
    }

    /// One per finished poll.
    pub(super) fn poll_done(&mut self, _deps: &EngineDeps, _done: &PollDone<'_>) {
        // T42
    }

    /// One per tick.
    pub(super) fn tick(&mut self, _deps: &EngineDeps) {
        // T42
    }
}
