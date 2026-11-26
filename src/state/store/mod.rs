//! The state store (T15). `View`'s methods (`view.rs`) are T02's.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::{Mono, SharedClock, Time};
use crate::config::Config;
use crate::log::Logger;
use crate::model::{
    Before, Carry, ItemsOutcome, Mergeable, Merged, NodeId, PrName, PrRead, PrState, ReadMeta,
    Record, RecordError, Source, Status, apply_gone, apply_read,
};
use crate::state::{
    Ended, FileContent, FileSource, Limit, MAX_PLACES, MAX_SUBSCRIPTIONS, MAX_WAITERS,
};

mod guards;
mod inner;
#[cfg(test)]
mod tests;
mod view;

pub use guards::{Named, Subscribed, Waiting};
use inner::{Entry, Fx, Inner, word};

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

/// The shared state the store and its guards hold.
struct Shared {
    /// Taken and let go inside one function; never held across an `.await`.
    inner: Mutex<Inner>,
    clock: SharedClock,
    logger: Logger,
    config: Arc<Config>,
    changed: tokio::sync::watch::Sender<u64>,
    file_changed: tokio::sync::watch::Sender<u64>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Nothing panics while holding it; the state stays consistent if something did.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One locked step. Afterwards, outside the lock: the log lines are written, `changed` moved if
    /// the step says so, and `file_changed` moved if the file's content differs from the last.
    /// Returns the step's result and whether the file's content changed.
    fn with<R>(&self, f: impl FnOnce(&mut Inner, &mut Fx) -> R) -> (R, bool) {
        let mut fx = Fx::default();
        let (r, file_moved) = {
            let mut inner = self.lock();
            let r = f(&mut inner, &mut fx);
            if fx.events {
                // STA-10: one shared slice, rebuilt on change, under the same lock (MOD-13).
                inner.events = inner.history.iter().cloned().collect::<Vec<_>>().into();
            }
            let content = inner.file_content();
            let file_moved = content != inner.file;
            if file_moved {
                inner.file = content;
            }
            (r, file_moved)
        };
        // OPS-9: a PR tracked, untracked or gone; names and sources only, never a title.
        for line in fx.lines {
            self.logger.info(line);
        }
        if fx.changed {
            self.changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        if file_moved {
            self.file_changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        (r, file_moved)
    }

    fn keep_done(&self) -> std::time::Duration {
        self.config.keep_done
    }

    /// POL-5: the owner is in `owners`, without case; every owner when `owners` is empty.
    fn owner_ok(&self, owner: &str) -> bool {
        self.config.owners.is_empty()
            || self
                .config
                .owners
                .iter()
                .any(|o| o.eq_ignore_ascii_case(owner))
    }
}

/// STA-9: 8 of `[a-z0-9]`, from the seed.
fn make_epoch(seed: u64) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = fastrand::Rng::with_seed(seed);
    (0..8)
        .map(|_| char::from(ALPHABET[rng.usize(..ALPHABET.len())]))
        .collect()
}

/// A std::sync::Mutex inside; never held across an await.
pub struct Store {
    shared: Arc<Shared>,
}

impl Store {
    /// A new epoch (STA-9). `loaded` entries are tracked without a record (STA-8); those past
    /// `keep_done` are dropped without an event; an `authored` one outside `owners` becomes `asked` (POL-5).
    pub fn new(deps: StoreDeps, loaded: FileContent) -> Arc<Store> {
        let now = deps.clock.wall();
        let (changed, _) = tokio::sync::watch::channel(0u64);
        let (file_changed, _) = tokio::sync::watch::channel(0u64);
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                // STA-9: a new epoch at each start; the counter starts at 0.
                epoch: make_epoch(deps.seed),
                rev: 0,
                prs: HashMap::new(),
                next_generation: 0,
                history: VecDeque::new(),
                events: Vec::new().into(),
                waiters: 0,
                subscriptions: 0,
                last_waiter_ended: None,
                last_subscription_ended: None,
                file: FileContent::default(),
            }),
            clock: deps.clock,
            logger: deps.logger,
            config: deps.config,
            changed,
            file_changed,
        });
        let mut lines = Vec::new();
        {
            let mut inner = shared.lock();
            for saved in loaded.prs {
                if inner.prs.contains_key(&saved.id) || inner.prs.len() >= MAX_PLACES {
                    // T16 skips a node ID given twice; a file of more than 200 can't come from us.
                    continue;
                }
                // STA-8: entries past their `keep_done` are dropped on loading, without an event.
                if let Some(ended) = &saved.ended
                    && now.since(ended.seen_at) >= shared.keep_done()
                {
                    lines.push(format!(
                        "untracked pr={} source={} cause=expired",
                        saved.pr,
                        word(saved.source)
                    ));
                    continue;
                }
                let mut source = saved.source;
                let mut asked_at = None;
                // POL-5: a loaded `authored` entry outside `owners` becomes `asked`, and starts its
                // hour. A saved `asked` can't come from us (STA-6); it's treated the same way.
                if (source == Source::Authored && !shared.owner_ok(&saved.pr.owner))
                    || source == Source::Asked
                {
                    source = Source::Asked;
                    asked_at = Some(now);
                }
                let generation = inner.generation();
                let entry = Entry {
                    generation,
                    id: saved.id.clone(),
                    name: saved.pr.clone(),
                    source,
                    class: Class::Loaded,
                    record: None,
                    // STA-5: the saved seen times and IDs are the first refresh's baseline.
                    carry: Carry {
                        seen: saved.seen.clone(),
                        ..Carry::default()
                    },
                    timing: Timing::default(),
                    login: saved.login.clone(),
                    mergeable_unknown: false,
                    // STA-8: its `keep_done` clock runs from its saved time.
                    retention: saved.ended.as_ref().map(|e| e.seen_at),
                    asked_at,
                    sub_ended_at: None,
                    named_open: 0,
                    saved: Some(saved),
                };
                inner.prs.insert(entry.id.clone(), entry);
            }
            inner.file = inner.file_content();
        }
        for line in lines {
            shared.logger.info(line);
        }
        Arc::new(Store { shared })
    }

    // ---- the tracked set ----

    pub fn lookup(&self, name: &PrName) -> Option<Tracked> {
        let inner = self.shared.lock();
        let id = inner.find(name)?;
        let e = inner.prs.get(&id)?;
        Some(Tracked {
            id: e.id.clone(),
            class: e.class,
            source: e.source,
        })
    }

    pub fn is_tracked(&self, id: &NodeId) -> bool {
        self.shared.lock().prs.contains_key(id)
    }

    /// 200 minus every place taken (STA-3).
    pub fn places_free(&self) -> usize {
        MAX_PLACES.saturating_sub(self.shared.lock().prs.len())
    }

    /// Takes a place for a PR found (`Authored`) or read through (`Asked`, or `Manual` for `track`).
    /// For a tracked ID: no new place, the stronger source is kept (STA-1), `Ok`.
    pub fn reserve(&self, id: &NodeId, name: &PrName, source: Source) -> Result<(), Limit> {
        let now = self.shared.clock.wall();
        self.shared
            .with(|inner, _fx| {
                if let Some(e) = inner.prs.get_mut(id) {
                    // STA-1, U3: a PR tracked for two reasons keeps the stronger; STA-3: no new place.
                    if source > e.source {
                        e.set_source(source);
                    }
                    return Ok(());
                }
                // STA-3: every class holds a place; at the limit nothing more is taken.
                if inner.prs.len() >= MAX_PLACES {
                    return Err(Limit);
                }
                let generation = inner.generation();
                inner.prs.insert(
                    id.clone(),
                    Entry {
                        generation,
                        id: id.clone(),
                        name: name.clone(),
                        source,
                        class: Class::First,
                        record: None,
                        carry: Carry::default(),
                        timing: Timing::default(),
                        login: None,
                        mergeable_unknown: false,
                        retention: None,
                        // STA-1: the hour starts whenever a PR becomes `asked`.
                        asked_at: (source == Source::Asked).then_some(now),
                        sub_ended_at: None,
                        named_open: 0,
                        saved: None,
                    },
                );
                Ok(())
            })
            .0
    }

    /// Gives back the place of a `First` PR whose first refresh failed (STA-3). Nothing for any other.
    pub fn release(&self, id: &NodeId) {
        self.shared.with(|inner, _fx| {
            // STA-3: only a place whose first refresh never applied.
            if inner.prs.get(id).is_some_and(|e| e.class == Class::First) {
                inner.prs.remove(id);
            }
        });
    }

    /// POL-5: these PRs become `asked`. The engine passes the `authored` PRs whose refresh **this poll
    /// applied and showed open**, and which this poll's finding, after the owners filter and read to its
    /// last page, didn't list. The store changes only those that are `authored` and open; never one the
    /// poll didn't refresh (a PR merged since the last poll is unlisted and still reads open until then).
    pub fn demote_unlisted(&self, unlisted_open: &[NodeId]) {
        let now = self.shared.clock.wall();
        self.shared.with(|inner, fx| {
            fx.changed = true;
            for id in unlisted_open {
                // POL-5: checked again here; anything but an open `authored` record is left alone.
                if let Some(e) = inner.prs.get_mut(id)
                    && e.source == Source::Authored
                    && e.record.as_ref().is_some_and(|r| r.state == PrState::Open)
                {
                    e.set_source(Source::Asked);
                    // STA-2: a PR that becomes `asked` without a request starts its 60 minutes then.
                    e.asked_at = Some(now);
                }
            }
        });
    }

    /// API-15 `track`: source `manual`. `true` if the file's content changed. On a `Loaded` entry it
    /// changes the saved source: the one change a saved entry takes before its first refresh (STA-8).
    pub fn set_manual(&self, id: &NodeId) -> bool {
        self.shared
            .with(|inner, fx| {
                if let Some(e) = inner.prs.get_mut(id) {
                    fx.changed = true;
                    // MOD-14: no revision for a source.
                    e.set_source(Source::Manual);
                }
            })
            .1
    }

    /// API-15 `untrack`, whole: `authored`, `asked` (with a waiter or a subscription), dropped with an `untracked` event, or nothing.
    pub fn untrack(&self, name: &PrName) -> Untracked {
        let now = self.shared.clock.wall();
        let shared = &self.shared;
        let (record, file_changed) = shared.with(|inner, fx| {
            // API-15: `untrack` never reads through; a PR that isn't tracked changes nothing.
            let id = inner.find(name)?;
            let e = inner.prs.get_mut(&id)?;
            // API-15: on an `authored` or `asked` PR it changes nothing.
            if e.source != Source::Manual {
                return e.record.as_ref().map(|r| (**r).clone());
            }
            fx.changed = true;
            // API-15: the user's own (`author` is the login it was read with, without case), open,
            // and its owner in `owners` (POL-5). A loaded entry has no record: never.
            let own = e.record.as_ref().is_some_and(|r| {
                r.state == PrState::Open
                    && shared.owner_ok(&r.pr.owner)
                    && match (&r.author, &e.login) {
                        (Some(a), Some(l)) => a.eq_ignore_ascii_case(l),
                        _ => false,
                    }
            });
            if own {
                e.set_source(Source::Authored);
                return e.record.as_ref().map(|r| (**r).clone());
            }
            // API-15, STA-1, NTF-4: with a waiter or a subscription it stays, as `asked`.
            if e.timing.waiters > 0 || e.timing.subscriptions > 0 {
                e.set_source(Source::Asked);
                e.asked_at = Some(now);
                return e.record.as_ref().map(|r| (**r).clone());
            }
            inner.drop_entry(fx, &id, now)
        });
        Untracked {
            record,
            file_changed,
        }
    }

    // ---- refreshes (the engine) ----

    pub fn targets(&self) -> Vec<Target> {
        let inner = self.shared.lock();
        let mut out: Vec<Target> = inner
            .prs
            .values()
            .map(|e| Target {
                id: e.id.clone(),
                pr: e.name.clone(),
                source: e.source,
                class: e.class,
                status: e.record.as_ref().map(|r| r.status),
                mergeable_unknown: e.mergeable_unknown,
                updated_at: e.record.as_ref().and_then(|r| r.updated_at),
                timing: e.timing.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.pr.cmp(&b.pr).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// `Timing::last_sent`. For a poll's requests `at` is the poll's own send time (`poller.md`).
    pub fn mark_sent(&self, ids: &[NodeId], at: Mono) {
        self.shared.with(|inner, _fx| {
            for id in ids {
                if let Some(e) = inner.prs.get_mut(id) {
                    e.timing.last_sent = Some(at);
                }
            }
        });
    }

    /// POL-12; `false` without a record.
    pub fn counts_rose(&self, id: &NodeId, read: &PrRead) -> bool {
        let inner = self.shared.lock();
        inner
            .prs
            .get(id)
            .and_then(|e| e.record.as_ref())
            .is_some_and(|r| crate::model::counts_rose(r, read))
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
        let now = self.shared.clock.wall();
        self.shared
            .with(|inner, fx| {
                // STA-4: a read for a PR that holds no place is discarded, with no event.
                let Some(e) = inner.prs.get(id) else {
                    return Applied::Discarded;
                };
                let saved_state = e
                    .saved
                    .as_ref()
                    .map(|s| s.ended.as_ref().map_or(PrState::Open, |x| x.state));
                let before = match (e.class, &e.record) {
                    (Class::Loaded, _) => Before::Loaded {
                        // STA-8: the first refresh keeps the saved source.
                        source: e.source,
                        saved_open: saved_state == Some(PrState::Open),
                    },
                    (_, Some(r)) => Before::Known(r),
                    (_, None) => Before::New { source: e.source },
                };
                let prev_state = e.record.as_ref().map(|r| r.state).or(saved_state);
                let first = matches!(e.class, Class::First | Class::Loaded);
                // STA-5: the carry holds the seen times and IDs (a loaded entry's saved ones).
                let Merged::Applied {
                    mut record,
                    carry,
                    kinds,
                } = apply_read(before, &e.carry, read, items, meta)
                else {
                    // MOD-2: sent before the read last applied.
                    return Applied::Discarded;
                };
                // STA-2, U7: the clock starts at the send time of the read that first shows it ended,
                // is cleared when it's open, and starts again on a change to another ended state.
                // STA-8: a first refresh that confirms the saved state keeps the saved time.
                let retention = match record.state {
                    PrState::Open => None,
                    s if prev_state == Some(s) => e.retention.or(Some(meta.sent_wall)),
                    _ => Some(meta.sent_wall),
                };
                let source = e.source;
                // MOD-13: a visible change takes the next revision, with its event, in this one step.
                let applied = if kinds.is_empty() {
                    Applied::Unchanged
                } else {
                    let rev = inner.event(fx, id.clone(), record.pr.clone(), kinds, now);
                    record.rev = rev;
                    Applied::Changed { rev }
                };
                if first {
                    // OPS-9: tracked at its first applied refresh.
                    fx.lines
                        .push(format!("tracked pr={} source={}", record.pr, word(source)));
                }
                // The entry was found above, under the same lock.
                let Some(e) = inner.prs.get_mut(id) else {
                    return applied;
                };
                e.class = if record.state == PrState::Open {
                    Class::Open
                } else {
                    Class::Ended
                };
                // MOD-1: the record's current name.
                e.name = record.pr.clone();
                e.retention = retention;
                e.carry = carry;
                e.saved = None;
                // POL-11: the read's value, whatever MOD-8 kept in the record.
                e.mergeable_unknown = read.mergeable == Mergeable::Unknown;
                if let Some(l) = login {
                    e.login = Some(l.to_string());
                }
                e.timing.fetched = Some(meta.sent_mono);
                e.timing.attempt_failed = false;
                e.timing.slept = false;
                e.record = Some(Arc::new(record));
                // STA-4: waiters and subscriptions are evaluated after every applied refresh.
                fx.changed = true;
                applied
            })
            .0
    }

    /// A refresh of this PR failed (POL-19, POL-21, POL-24, POL-8): it's `stale`. With `error_type`
    /// the record's `error` is set (MOD-2); without one it's left as it was.
    pub fn attempt_failed(&self, id: &NodeId, error_type: Option<&str>, at: Time) {
        self.shared.with(|inner, fx| {
            let Some(e) = inner.prs.get_mut(id) else {
                return;
            };
            fx.changed = true;
            e.timing.attempt_failed = true;
            if let (Some(t), Some(r)) = (error_type, &e.record) {
                // MOD-2: `since` is the first failure of the current run; no revision (MOD-14).
                let since = r.error.as_ref().map_or(at, |x| x.since);
                let mut next = (**r).clone();
                next.error = Some(RecordError {
                    type_: t.to_string(),
                    since,
                });
                e.record = Some(Arc::new(next));
            }
        });
    }

    /// MOD-11's "same login".
    pub fn login_of(&self, id: &NodeId) -> Option<String> {
        self.shared.lock().prs.get(id).and_then(|e| e.login.clone())
    }

    /// POL-21: an open PR becomes `gone`, with its event; its retention clock starts at `seen_at` (STA-2).
    /// A `Loaded` entry saved open has no record to change: its saved entry becomes `gone` from
    /// `seen_at`, with no event, and this returns `Unchanged`.
    pub fn mark_gone(&self, id: &NodeId, seen_at: Time) -> Applied {
        let now = self.shared.clock.wall();
        self.shared
            .with(|inner, fx| {
                let Some(e) = inner.prs.get_mut(id) else {
                    return Applied::Discarded;
                };
                if e.class == Class::Loaded {
                    // STA-8: the saved entry takes it; there's no record to make an event of.
                    if let Some(saved) = &mut e.saved
                        && saved.ended.is_none()
                    {
                        saved.ended = Some(Ended {
                            state: PrState::Gone,
                            seen_at,
                        });
                        e.retention = Some(seen_at);
                        fx.changed = true;
                        fx.lines
                            .push(format!("gone pr={} source={}", e.name, word(e.source)));
                    }
                    return Applied::Unchanged;
                }
                let Some(old) = e.record.clone() else {
                    return Applied::Discarded;
                };
                // MOD-11: only an open record becomes `gone`.
                let Some((mut record, kinds)) = apply_gone(&old) else {
                    return Applied::Unchanged;
                };
                fx.lines
                    .push(format!("gone pr={} source={}", e.name, word(e.source)));
                let rev = inner.event(fx, id.clone(), record.pr.clone(), kinds, now);
                record.rev = rev;
                if let Some(e) = inner.prs.get_mut(id) {
                    e.record = Some(Arc::new(record));
                    e.class = Class::Ended;
                    // STA-2: retention from `seen_at`.
                    e.retention = Some(seen_at);
                }
                Applied::Changed { rev }
            })
            .0
    }

    /// POL-22: every record is `stale` until its next refresh.
    pub fn mark_slept(&self) {
        self.shared.with(|inner, _fx| {
            for e in inner.prs.values_mut() {
                e.timing.slept = true;
            }
        });
    }

    /// The tick (POL-26): drops what STA-1's hour and STA-2's `keep_done` say, by wall time, in the
    /// order of `pr`, each with an `untracked` event. Never a PR with a waiter or a subscription (STA-1).
    pub fn sweep(&self) {
        let now = self.shared.clock.wall();
        let keep_done = self.shared.keep_done();
        self.shared.with(|inner, fx| {
            let mut due: Vec<(PrName, NodeId)> = inner
                .prs
                .values()
                .filter(|e| e.due(now, keep_done))
                .map(|e| (e.name.clone(), e.id.clone()))
                .collect();
            // STA-2: PRs dropped at one tick take revisions in the order of `pr`.
            due.sort();
            for (_, id) in due {
                inner.drop_entry(fx, &id, now);
            }
        });
    }

    // ---- readers (the API, the waiters) ----

    pub fn view(&self) -> View {
        let inner = self.shared.lock();
        let mut pending = Vec::new();
        let mut prs = Vec::new();
        let mut tracked = [0u32; 3];
        for e in inner.prs.values() {
            let slot = match e.source {
                Source::Authored => 0,
                Source::Manual => 1,
                Source::Asked => 2,
            };
            tracked[slot] += 1;
            if let Some(r) = &e.record {
                prs.push(PrView {
                    id: e.id.clone(),
                    record: r.clone(),
                    timing: e.timing.clone(),
                });
            }
            if matches!(e.class, Class::Loaded | Class::First) {
                pending.push(e.name.clone());
            }
        }
        pending.sort();
        View {
            epoch: inner.epoch.clone(),
            rev: inner.rev,
            prs,
            events: inner.events.clone(),
            pending,
            tracked,
            waiters: inner.waiters,
            subscriptions: inner.subscriptions,
        }
    }

    /// Changes after every `apply` (changed or not), `attempt_failed`, `mark_gone`, `sweep` that
    /// dropped something, `untrack`, `set_manual` and `demote_unlisted`. Waiters and the notifier evaluate on it (STA-4, NTF-4).
    pub fn changed(&self) -> tokio::sync::watch::Receiver<u64> {
        self.shared.changed.subscribe()
    }

    /// A request names this PR (STA-1): `Timing::named` is now. Dropping the guard is the request's end,
    /// from which an `asked` PR's 60 minutes run.
    pub fn name(&self, id: &NodeId) -> Named {
        self.naming(id, true)
    }

    /// The same for the two notify routes (STA-1, NTF-2, NTF-3): the request counts for STA-1's 60 minutes, and
    /// **`Timing::named` is left as it was**, so POL-14's 2 minutes and POL-11's 60 s don't start.
    pub fn name_quiet(&self, id: &NodeId) -> Named {
        self.naming(id, false)
    }

    fn naming(&self, id: &NodeId, loud: bool) -> Named {
        let mono = self.shared.clock.mono();
        let generation = self
            .shared
            .with(|inner, _fx| {
                let e = inner.prs.get_mut(id)?;
                e.named_open += 1;
                // STA-1: only a request other than `notify`'s sets it (POL-11, POL-14).
                if loud {
                    e.timing.named = Some(mono);
                }
                Some(e.generation)
            })
            .0;
        Named::new(self.shared.clone(), id.clone(), generation)
    }

    /// A waiter on this PR (API-9, API-14: at most 64 in all). Dropping the guard removes it, and
    /// carries out a drop that fell due meanwhile (STA-1).
    pub fn add_waiter(&self, id: &NodeId) -> Result<Waiting, Limit> {
        let generation = self
            .shared
            .with(|inner, _fx| {
                // API-14: at most 64 in all.
                if inner.waiters >= MAX_WAITERS {
                    return Err(Limit);
                }
                inner.waiters += 1;
                Ok(inner.prs.get_mut(id).map(|e| {
                    e.timing.waiters += 1;
                    e.generation
                }))
            })
            .0?;
        Ok(Waiting::new(self.shared.clone(), id.clone(), generation))
    }

    /// When the last waiter ended; `None` if there never was one (OPS-8's "no waiter for `exit_after`").
    pub fn last_waiter_ended(&self) -> Option<Mono> {
        self.shared.lock().last_waiter_ended
    }

    /// A subscription on this PR (NTF-4: it stays tracked; NTF-2: at most 64 in all, apart from the
    /// waiters). Dropping the guard is the subscription's end, and counts as the end of a request that
    /// named the PR (STA-1). It counts in `Timing::subscriptions` and nowhere else in `Timing` (NTF-5).
    pub fn add_subscription(&self, id: &NodeId) -> Result<Subscribed, Limit> {
        let generation = self
            .shared
            .with(|inner, _fx| {
                // NTF-2: at most 64 in all, counted apart from the waiters.
                if inner.subscriptions >= MAX_SUBSCRIPTIONS {
                    return Err(Limit);
                }
                inner.subscriptions += 1;
                Ok(inner.prs.get_mut(id).map(|e| {
                    // NTF-5: this count only; the PR is no hotter for it.
                    e.timing.subscriptions += 1;
                    e.generation
                }))
            })
            .0?;
        Ok(Subscribed::new(self.shared.clone(), id.clone(), generation))
    }

    /// The subscriptions held now: what `View::subscriptions` holds, without building a view (OPS-8's idle exit).
    pub fn subscriptions(&self) -> u32 {
        self.shared.lock().subscriptions
    }

    /// When the last subscription ended; `None` if there never was one (OPS-8).
    pub fn last_subscription_ended(&self) -> Option<Mono> {
        self.shared.lock().last_subscription_ended
    }

    /// The subscriptions held on PRs whose record's `state` is `open`: the engine's tier (POL-13, NTF-5).
    pub fn open_subscriptions(&self) -> u32 {
        let inner = self.shared.lock();
        inner
            .prs
            .values()
            .filter(|e| e.record.as_ref().is_some_and(|r| r.state == PrState::Open))
            .map(|e| e.timing.subscriptions)
            .sum()
    }
}

impl FileSource for Store {
    fn file_content(&self) -> FileContent {
        self.shared.lock().file_content()
    }

    fn file_changed(&self) -> tokio::sync::watch::Receiver<u64> {
        self.shared.file_changed.subscribe()
    }
}
