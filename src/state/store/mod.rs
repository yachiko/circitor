#![allow(unused_variables, dead_code)] // stub: T15 removes this
//! The state store (T15). `View`'s methods (`view.rs`) are T02's.

use std::sync::{Arc, Mutex};

use crate::clock::{Mono, SharedClock, Time};
use crate::config::Config;
use crate::log::Logger;
use crate::model::{ItemsOutcome, NodeId, PrName, PrRead, ReadMeta, Record, Source, Status};
use crate::state::{FileContent, FileSource, Limit};

mod view;

pub use view::{PrView, View};

pub struct StoreDeps {
    pub clock: SharedClock,
    pub logger: Logger,
    pub config: Arc<Config>,
    pub seed: u64,
}

/// Why a tracked PR is where it is. `Loaded` and `First` have no record yet: they're API-18's `pending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Loaded,
    First,
    Open,
    Ended,
}

/// What the store knows about a PR's refreshes, for cadence and staleness.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timing {
    /// Its last refresh of any kind, when sent (POL-13).
    pub last_sent: Option<Mono>,
    /// The send time of the read last applied.
    pub fetched: Option<Mono>,
    /// MOD-16: its last attempt failed.
    pub attempt_failed: bool,
    /// MOD-16: a sleep was noticed since it was fetched.
    pub slept: bool,
    pub waiters: u32,
    /// When a request other than `notify`'s last named it (POL-14, POL-11).
    pub named: Option<Mono>,
    /// Open ones on this PR: POL-10's period reads it, `hot` never does (NTF-5).
    pub subscriptions: u32,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub id: NodeId,
    pub pr: PrName,
    pub source: Source,
    pub class: Class,
    pub status: Option<Status>,
    /// The read last applied said `UNKNOWN` (POL-11), whatever MOD-8 kept in the record.
    pub mergeable_unknown: bool,
    pub updated_at: Option<Time>,
    pub timing: Timing,
}

#[derive(Clone, Debug)]
pub struct Tracked {
    pub id: NodeId,
    pub class: Class,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    Changed { rev: u64 },
    Unchanged,
    Discarded,
}

#[derive(Clone, Debug)]
pub struct Untracked {
    pub record: Option<Record>,
    pub file_changed: bool,
}

/// A std::sync::Mutex inside; never held across an await.
pub struct Store {
    inner: Mutex<()>,
}

impl Store {
    /// A new epoch (STA-9). `loaded` entries are tracked without a record (STA-8); those past
    /// `keep_done` are dropped without an event; an `authored` one outside `owners` becomes `asked` (POL-5).
    pub fn new(deps: StoreDeps, loaded: FileContent) -> Arc<Store> {
        unimplemented!("T15")
    }

    // ---- the tracked set ----

    pub fn lookup(&self, name: &PrName) -> Option<Tracked> {
        unimplemented!("T15")
    }

    pub fn is_tracked(&self, id: &NodeId) -> bool {
        unimplemented!("T15")
    }

    /// 200 minus every place taken (STA-3).
    pub fn places_free(&self) -> usize {
        unimplemented!("T15")
    }

    /// Takes a place for a PR found (`Authored`) or read through (`Asked`, or `Manual` for `track`).
    /// For a tracked ID: no new place, the stronger source is kept (STA-1), `Ok`.
    pub fn reserve(&self, id: &NodeId, name: &PrName, source: Source) -> Result<(), Limit> {
        unimplemented!("T15")
    }

    /// Gives back the place of a `First` PR whose first refresh failed (STA-3). Nothing for any other.
    pub fn release(&self, id: &NodeId) {
        unimplemented!("T15")
    }

    /// POL-5: these PRs become `asked`. The engine passes the `authored` PRs whose refresh **this poll
    /// applied and showed open**, and which this poll's finding, after the owners filter and read to its
    /// last page, didn't list. The store changes only those that are `authored` and open; never one the
    /// poll didn't refresh (a PR merged since the last poll is unlisted and still reads open until then).
    pub fn demote_unlisted(&self, unlisted_open: &[NodeId]) {
        unimplemented!("T15")
    }

    /// API-15 `track`: source `manual`. `true` if the file's content changed. On a `Loaded` entry it
    /// changes the saved source: the one change a saved entry takes before its first refresh (STA-8).
    pub fn set_manual(&self, id: &NodeId) -> bool {
        unimplemented!("T15")
    }

    /// API-15 `untrack`, whole: `authored`, `asked` (with a waiter or a subscription), dropped with an `untracked` event, or nothing.
    pub fn untrack(&self, name: &PrName) -> Untracked {
        unimplemented!("T15")
    }

    // ---- refreshes (the engine) ----

    pub fn targets(&self) -> Vec<Target> {
        unimplemented!("T15")
    }

    /// `Timing::last_sent`. For a poll's requests `at` is the poll's own send time (`poller.md`).
    pub fn mark_sent(&self, ids: &[NodeId], at: Mono) {
        unimplemented!("T15")
    }

    /// POL-12; `false` without a record.
    pub fn counts_rose(&self, id: &NodeId, read: &PrRead) -> bool {
        unimplemented!("T15")
    }

    /// STA-4. `login` is the poll's `viewer.login`, or `None` outside a poll (kept as it was).
    pub fn apply(
        &self,
        id: &NodeId,
        read: &PrRead,
        items: ItemsOutcome<'_>,
        meta: &ReadMeta,
        login: Option<&str>,
    ) -> Applied {
        unimplemented!("T15")
    }

    /// A refresh of this PR failed (POL-19, POL-21, POL-24, POL-8): it's `stale`. With `error_type`
    /// the record's `error` is set (MOD-2); without one it's left as it was.
    pub fn attempt_failed(&self, id: &NodeId, error_type: Option<&str>, at: Time) {
        unimplemented!("T15")
    }

    /// MOD-11's "same login".
    pub fn login_of(&self, id: &NodeId) -> Option<String> {
        unimplemented!("T15")
    }

    /// POL-21: an open PR becomes `gone`, with its event; its retention clock starts at `seen_at` (STA-2).
    /// A `Loaded` entry saved open has no record to change: its saved entry becomes `gone` from
    /// `seen_at`, with no event, and this returns `Unchanged`.
    pub fn mark_gone(&self, id: &NodeId, seen_at: Time) -> Applied {
        unimplemented!("T15")
    }

    /// POL-22: every record is `stale` until its next refresh.
    pub fn mark_slept(&self) {
        unimplemented!("T15")
    }

    /// The tick (POL-26): drops what STA-1's hour and STA-2's `keep_done` say, by wall time, in the
    /// order of `pr`, each with an `untracked` event. Never a PR with a waiter or a subscription (STA-1).
    pub fn sweep(&self) {
        unimplemented!("T15")
    }

    // ---- readers (the API, the waiters) ----

    pub fn view(&self) -> View {
        unimplemented!("T15")
    }

    /// Changes after every `apply` (changed or not), `attempt_failed`, `mark_gone`, `sweep` that
    /// dropped something, `untrack`, `set_manual` and `demote_unlisted`. Waiters and the notifier evaluate on it (STA-4, NTF-4).
    pub fn changed(&self) -> tokio::sync::watch::Receiver<u64> {
        unimplemented!("T15")
    }

    /// A request names this PR (STA-1): `Timing::named` is now. Dropping the guard is the request's end,
    /// from which an `asked` PR's 60 minutes run.
    pub fn name(&self, id: &NodeId) -> Named {
        unimplemented!("T15")
    }

    /// The same for the two notify routes (STA-1, NTF-2, NTF-3): the request counts for STA-1's 60 minutes, and
    /// **`Timing::named` is left as it was**, so POL-14's 2 minutes and POL-11's 60 s don't start.
    pub fn name_quiet(&self, id: &NodeId) -> Named {
        unimplemented!("T15")
    }

    /// A waiter on this PR (API-9, API-14: at most 64 in all). Dropping the guard removes it, and
    /// carries out a drop that fell due meanwhile (STA-1).
    pub fn add_waiter(&self, id: &NodeId) -> Result<Waiting, Limit> {
        unimplemented!("T15")
    }

    /// When the last waiter ended; `None` if there never was one (OPS-8's "no waiter for `exit_after`").
    pub fn last_waiter_ended(&self) -> Option<Mono> {
        unimplemented!("T15")
    }

    /// A subscription on this PR (NTF-4: it stays tracked; NTF-2: at most 64 in all, apart from the
    /// waiters). Dropping the guard is the subscription's end, and counts as the end of a request that
    /// named the PR (STA-1). It counts in `Timing::subscriptions` and nowhere else in `Timing` (NTF-5).
    pub fn add_subscription(&self, id: &NodeId) -> Result<Subscribed, Limit> {
        unimplemented!("T15")
    }

    /// The subscriptions held now: what `View::subscriptions` holds, without building a view (OPS-8's idle exit).
    pub fn subscriptions(&self) -> u32 {
        unimplemented!("T15")
    }

    /// When the last subscription ended; `None` if there never was one (OPS-8).
    pub fn last_subscription_ended(&self) -> Option<Mono> {
        unimplemented!("T15")
    }

    /// The subscriptions held on PRs whose record's `state` is `open`: the engine's tier (POL-13, NTF-5).
    pub fn open_subscriptions(&self) -> u32 {
        unimplemented!("T15")
    }
}

impl FileSource for Store {
    fn file_content(&self) -> FileContent {
        unimplemented!("T15")
    }

    fn file_changed(&self) -> tokio::sync::watch::Receiver<u64> {
        unimplemented!("T15")
    }
}

/// A request naming a PR; `Drop` does the work (T15).
pub struct Named(());
/// A waiter on a PR; `Drop` does the work (T15).
pub struct Waiting(());
/// A subscription on a PR; `Drop` does the work (T15).
pub struct Subscribed(());
