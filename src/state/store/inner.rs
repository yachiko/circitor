//! What the store holds behind its lock, and the rules that read it: which PR is due (STA-1,
//! STA-2), what the file holds (STA-6), and how a revision and its event are taken (MOD-13).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use crate::clock::{Mono, Time};
use crate::model::{Carry, Event, EventKind, NodeId, PrName, PrState, Record, Source};
use crate::state::{Ended, FileContent, FileEntry, MAX_EVENTS};

use super::{Class, Timing};

/// STA-1: an `asked` PR stays 60 minutes after the last request that named it ended; a
/// subscription's end keeps an ended PR as long (NTF-4). By the wall clock (POL-26).
pub(super) const ASKED_HOUR: Duration = Duration::from_secs(3_600);

/// One tracked PR.
pub(super) struct Entry {
    /// Tells this entry apart from a later one under the same ID, so a guard never counts on it.
    pub generation: u64,
    pub id: NodeId,
    /// The record's current name; a `Loaded` entry's saved one; a `First` one's reserved one (MOD-1).
    pub name: PrName,
    pub source: Source,
    pub class: Class,
    pub record: Option<Arc<Record>>,
    pub carry: Carry,
    pub timing: Timing,
    /// The login it was last read with (MOD-11, STA-6).
    pub login: Option<String>,
    /// The read last applied said `UNKNOWN` (POL-11).
    pub mergeable_unknown: bool,
    /// STA-2: when the retention clock started (wall); `None` while open.
    pub retention: Option<Time>,
    /// STA-1: when the asked hour last started (wall).
    pub asked_at: Option<Time>,
    /// NTF-4: when its last subscription ended (wall): an ended PR stays 60 minutes past it.
    pub sub_ended_at: Option<Time>,
    /// Open naming requests (STA-1).
    pub named_open: u32,
    /// A `Loaded` entry's saved entry, as loaded but for `set_manual` and `mark_gone` (STA-8).
    pub saved: Option<FileEntry>,
}

impl Entry {
    pub fn set_source(&mut self, source: Source) {
        self.source = source;
        // MOD-14: a source change takes no revision; the record just carries it.
        if let Some(r) = &self.record
            && r.source != source
        {
            let mut next = (**r).clone();
            next.source = source;
            self.record = Some(Arc::new(next));
        }
        if let Some(saved) = &mut self.saved
            && source != Source::Asked
        {
            saved.source = source;
        }
    }

    /// STA-6: what the file holds for this PR, if anything.
    fn file_entry(&self) -> Option<FileEntry> {
        // STA-6: never an `asked` PR; POL-4: a found PR only once its first refresh is applied.
        if self.source == Source::Asked || self.class == Class::First {
            return None;
        }
        if self.class == Class::Loaded {
            // STA-8: a loaded PR keeps its saved entry until its first refresh is applied.
            return self.saved.clone();
        }
        let record = self.record.as_ref()?;
        let ended = match record.state {
            PrState::Open => None,
            state => Some(Ended {
                state,
                seen_at: self.retention.unwrap_or(record.fetched_at),
            }),
        };
        Some(FileEntry {
            pr: self.name.clone(),
            id: self.id.clone(),
            source: self.source,
            login: self.login.clone(),
            ended,
            // STA-5: the seen times and IDs, as in memory.
            seen: self.carry.seen.clone(),
        })
    }

    /// STA-1, STA-2: whether a drop is due at `now`. Never with a waiter or a subscription.
    pub fn due(&self, now: Time, keep_done: Duration) -> bool {
        // STA-1: no PR is dropped while it has a waiter or a subscription.
        if self.timing.waiters > 0 || self.timing.subscriptions > 0 {
            return false;
        }
        // NTF-4: a subscription's end keeps an ended PR 60 minutes, past `keep_done` too.
        let held = self.sub_ended_at.is_some_and(|t| now.since(t) < ASKED_HOUR);
        // STA-2: `keep_done` from the retention clock, by wall time (`since` saturates, POL-26).
        let ended = !held && self.retention.is_some_and(|r| now.since(r) >= keep_done);
        // STA-1: 60 minutes after the last request that named it, none open.
        let asked = self.source == Source::Asked
            && self.named_open == 0
            && self.asked_at.is_some_and(|a| now.since(a) >= ASKED_HOUR);
        ended || asked
    }
}

/// What one locked step did besides its result: log lines to write, and channels to move,
/// once the lock is let go.
#[derive(Default)]
pub(super) struct Fx {
    pub lines: Vec<String>,
    pub changed: bool,
    pub events: bool,
}

pub(super) struct Inner {
    pub epoch: String,
    /// MOD-13: the one counter, the highest revision taken.
    pub rev: u64,
    pub prs: HashMap<NodeId, Entry>,
    pub next_generation: u64,
    /// STA-10: the last `MAX_EVENTS` events, oldest first.
    pub history: VecDeque<Event>,
    /// `history`, shared with every view; rebuilt when it changes.
    pub events: Arc<[Event]>,
    pub waiters: u32,
    pub subscriptions: u32,
    pub last_waiter_ended: Option<Mono>,
    pub last_subscription_ended: Option<Mono>,
    /// The file's content as last handed out by `file_changed` (STA-7).
    pub file: FileContent,
}

pub(super) fn word(source: Source) -> &'static str {
    match source {
        Source::Asked => "asked",
        Source::Authored => "authored",
        Source::Manual => "manual",
    }
}

impl Inner {
    pub fn generation(&mut self) -> u64 {
        self.next_generation += 1;
        self.next_generation
    }

    /// The tracked PR named `name`, without case (MOD-1).
    pub fn find(&self, name: &PrName) -> Option<NodeId> {
        self.prs
            .values()
            .find(|e| e.name == *name)
            .map(|e| e.id.clone())
    }

    /// MOD-13: the next revision, its event and the history, in one step (the caller holds the lock
    /// and stores the record's `rev` before letting it go).
    pub fn event(
        &mut self,
        fx: &mut Fx,
        id: NodeId,
        pr: PrName,
        kinds: Vec<EventKind>,
        at: Time,
    ) -> u64 {
        self.rev += 1;
        self.history.push_back(Event {
            rev: self.rev,
            pr,
            at,
            kinds,
            id,
        });
        // STA-10: the last 500 events, across all PRs.
        while self.history.len() > MAX_EVENTS {
            self.history.pop_front();
        }
        fx.events = true;
        fx.changed = true;
        self.rev
    }

    /// Drops a PR, with its `untracked` event (STA-2, MOD-14) and its log line (OPS-9). A `First` PR
    /// never appeared, so it goes without an event. Returns its last record with the event's `rev`.
    pub fn drop_entry(&mut self, fx: &mut Fx, id: &NodeId, at: Time) -> Option<Record> {
        let e = self.prs.remove(id)?;
        fx.changed = true;
        fx.lines
            .push(format!("untracked pr={} source={}", e.name, word(e.source)));
        if e.class == Class::First {
            return None;
        }
        let rev = self.event(
            fx,
            e.id.clone(),
            e.name.clone(),
            vec![EventKind::Untracked { source: e.source }],
            at,
        );
        e.record.map(|r| {
            let mut r = (*r).clone();
            r.rev = rev;
            r
        })
    }

    /// STA-6: every tracked PR but `asked` and `First` ones, in the order of `pr`.
    pub fn file_content(&self) -> FileContent {
        let mut prs: Vec<FileEntry> = self.prs.values().filter_map(Entry::file_entry).collect();
        prs.sort_by(|a, b| a.pr.cmp(&b.pr).then_with(|| a.id.cmp(&b.id)));
        FileContent { prs }
    }
}
