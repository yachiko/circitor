#![allow(unused_variables, dead_code)] // stub: T44 removes this
//! A condition, checked and evaluated (API-10, API-11, NTF-4; T44). Pure: no clock, no store, no I/O.

use std::time::Duration;

use crate::clock::Mono;
use crate::model::NodeId;
use crate::state::{Cursor, View};

use super::types::{ApiError, Ask, Outcome, Reason};

/// MOD-15's kinds a `kind` may name (API-10): all but `tracked` and `untracked`, in MOD-15's order.
pub const KIND_WORDS: [&str; 12] = [
    "status",
    "head",
    "checks",
    "merged",
    "closed",
    "reopened",
    "gone",
    "draft",
    "mergeable",
    "review",
    "activity",
    "edited",
];

/// A condition, checked (API-10). `ask` is kept as given, for the texts.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub ask: Ask,
    pub since: Option<Cursor>,
    pub rerun_since: Option<Cursor>,
    /// Of `KIND_WORDS`, in the order given.
    pub kinds: Option<Vec<&'static str>>,
}

impl Condition {
    /// API-10's checks, once: `sha` is 7 to 40 hex characters; `sha`, `rerun_since` and `fail_fast` go
    /// with `checks-done` only and `kind` with `changed` only; every word of `kind` is one of
    /// `KIND_WORDS`, and there's at least one; the two cursors are in STA-9's form. `Err` is
    /// `bad_request` naming the parameter (API-1).
    pub fn check(ask: Ask) -> Result<Condition, ApiError> {
        unimplemented!("T44")
    }
}

/// What one wait or one subscription remembers between evaluations.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// The request's arrival: `head_moved`'s 60 s run from it (API-11, NTF-4).
    pub began: Mono,
    /// `changed` without `since`: the record's `rev` at the first evaluation that saw one.
    pub baseline: Option<u64>,
    /// `sha` matched `head_sha` once.
    pub sha_matched: bool,
    /// The view's `rev` at the last evaluation: events up to it have been looked at.
    pub looked: Option<u64>,
    /// The point was too old at the first evaluation (STA-9): `changed` holds.
    pub held_at_once: bool,
    /// An event of a named kind was recorded after the point.
    pub kind_seen: bool,
    /// A `checks` event was recorded after `rerun_since`.
    pub rerun_seen: bool,
    /// `rerun_since` was too old at the first evaluation: that part is dropped.
    pub rerun_dropped: bool,
}

impl Progress {
    /// Nothing seen yet.
    pub fn new(began: Mono) -> Progress {
        Progress {
            began,
            baseline: None,
            sha_matched: false,
            looked: None,
            held_at_once: false,
            kind_seen: false,
            rerun_seen: false,
            rerun_dropped: false,
        }
    }
}

/// API-11.
pub const HEAD_MOVED_AFTER: Duration = Duration::from_secs(60);

/// API-10 and API-11 on one view, for the PR `id`. `None`: neither the condition nor an end holds.
/// The condition is checked before the ends. Never `Outcome::Timeout`: a deadline is its caller's.
pub fn evaluate(
    cond: &Condition,
    view: &View,
    id: &NodeId,
    progress: &mut Progress,
) -> Option<(Outcome, Option<Reason>)> {
    unimplemented!("T44")
}
