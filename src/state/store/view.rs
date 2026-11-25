//! One consistent view of the store (MOD-13), and its pure methods. Built by T02.

use std::sync::Arc;

use crate::model::{Event, NodeId, PrName, Record};
use crate::state::Cursor;

use super::Timing;

#[derive(Clone, Debug)]
pub struct PrView {
    pub id: NodeId,
    pub record: Arc<Record>,
    pub timing: Timing,
}

/// One consistent view (MOD-13): every answer is built from one of these.
#[derive(Clone, Debug)]
pub struct View {
    pub epoch: String,
    /// The counter: the highest revision taken.
    pub rev: u64,
    /// Every PR with a record, in no promised order.
    pub prs: Vec<PrView>,
    /// The history held, oldest first (STA-10).
    pub events: Arc<[Event]>,
    /// `Loaded` and `First`, sorted.
    pub pending: Vec<PrName>,
    /// Counts by source: authored, manual, asked (API-5).
    pub tracked: [u32; 3],
    pub waiters: u32,
    /// Open ones, over every PR (API-5, NTF-9).
    pub subscriptions: u32,
}

impl View {
    pub fn cursor(&self) -> Cursor {
        Cursor {
            epoch: self.epoch.clone(),
            rev: self.rev,
        }
    }

    /// STA-9: another epoch, a `rev` above the counter, or an event after it no longer held.
    pub fn too_old(&self, c: &Cursor) -> bool {
        // STA-9: revisions are compared only within one epoch.
        if c.epoch != self.epoch || c.rev > self.rev {
            return true;
        }
        if c.rev == self.rev {
            return false;
        }
        // Every revision has its event (MOD-13): the one after the cursor must still be held.
        match self.events.first() {
            Some(first) => c.rev.saturating_add(1) < first.rev,
            None => true,
        }
    }

    /// The record whose current name is `name` (without case, MOD-1).
    pub fn find(&self, name: &PrName) -> Option<&PrView> {
        self.prs.iter().find(|p| p.record.pr == *name)
    }

    pub fn get(&self, id: &NodeId) -> Option<&PrView> {
        self.prs.iter().find(|p| p.id == *id)
    }

    /// Events with `rev` above `after`, oldest first; for one PR when `id` is given.
    pub fn events_after(&self, after: u64, id: Option<&NodeId>) -> Vec<&Event> {
        let start = self.events.partition_point(|e| e.rev <= after);
        self.events[start..]
            .iter()
            .filter(|e| id.is_none_or(|id| e.id == *id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Time;
    use crate::model::{EventKind, Source};

    fn event(rev: u64, id: &str) -> Event {
        Event {
            rev,
            pr: PrName::parse("o/r#1").unwrap(),
            at: Time::from_unix_millis(0),
            kinds: vec![EventKind::Tracked {
                source: Source::Authored,
            }],
            id: NodeId(id.into()),
        }
    }

    fn view(rev: u64, events: Vec<Event>) -> View {
        View {
            epoch: "k3x9a2bq".into(),
            rev,
            prs: Vec::new(),
            events: events.into(),
            pending: Vec::new(),
            tracked: [0; 3],
            waiters: 0,
            subscriptions: 0,
        }
    }

    fn at(rev: u64) -> Cursor {
        Cursor {
            epoch: "k3x9a2bq".into(),
            rev,
        }
    }

    // STA-9: a cursor is too old from another epoch, above the counter, or past the history held.
    #[test]
    fn sta9_too_old() {
        let full = view(3, vec![event(1, "a"), event(2, "b"), event(3, "a")]);
        assert_eq!(full.cursor(), at(3));
        for rev in 0..=3 {
            assert!(!full.too_old(&at(rev)), "{rev}");
        }
        // Another epoch.
        assert!(full.too_old(&Cursor {
            epoch: "zzzzzzzz".into(),
            rev: 1
        }));
        // A `rev` above the counter.
        assert!(full.too_old(&at(4)));
        // History that no longer starts at revision 1: a cursor below its first event.
        let cut = view(5, vec![event(3, "a"), event(4, "a"), event(5, "b")]);
        assert!(cut.too_old(&at(0)));
        assert!(cut.too_old(&at(1)));
        assert!(!cut.too_old(&at(2)));
        assert!(!cut.too_old(&at(5)));
        // No event held at all, and a counter past the cursor.
        assert!(view(2, Vec::new()).too_old(&at(1)));
        assert!(!view(0, Vec::new()).too_old(&at(0)));
    }

    #[test]
    fn events_after_and_lookups() {
        let v = view(3, vec![event(1, "a"), event(2, "b"), event(3, "a")]);
        let revs = |es: Vec<&Event>| es.iter().map(|e| e.rev).collect::<Vec<_>>();
        assert_eq!(revs(v.events_after(0, None)), [1, 2, 3]);
        assert_eq!(revs(v.events_after(1, None)), [2, 3]);
        assert_eq!(revs(v.events_after(0, Some(&NodeId("a".into())))), [1, 3]);
        assert_eq!(revs(v.events_after(3, None)), Vec::<u64>::new());
        assert!(v.get(&NodeId("a".into())).is_none());
        assert!(v.find(&PrName::parse("o/r#1").unwrap()).is_none());
    }
}
