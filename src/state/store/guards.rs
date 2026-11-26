//! The guards a request, a waiter and a subscription hold on a PR. Each `Drop` takes the store's
//! lock briefly and does the work: STA-1's hour starts again, a drop that fell due is carried out.

use std::sync::Arc;

use crate::model::NodeId;

use super::Shared;
use super::inner::{Entry, Inner};

/// The entry the guard counted on, if it's still the same one (not dropped and tracked again).
fn entry<'a>(inner: &'a mut Inner, id: &NodeId, generation: Option<u64>) -> Option<&'a mut Entry> {
    let generation = generation?;
    inner.prs.get_mut(id).filter(|e| e.generation == generation)
}

/// A request naming a PR; `Drop` does the work (T15).
pub struct Named {
    shared: Arc<Shared>,
    id: NodeId,
    generation: Option<u64>,
}

impl Named {
    pub(super) fn new(shared: Arc<Shared>, id: NodeId, generation: Option<u64>) -> Named {
        Named {
            shared,
            id,
            generation,
        }
    }
}

impl Drop for Named {
    fn drop(&mut self) {
        let now = self.shared.clock.wall();
        self.shared.with(|inner, _fx| {
            if let Some(e) = entry(inner, &self.id, self.generation) {
                e.named_open = e.named_open.saturating_sub(1);
                // STA-1: an `asked` PR's 60 minutes run from the end of the last request naming it.
                e.asked_at = Some(now);
            }
        });
    }
}

/// A waiter on a PR; `Drop` does the work (T15).
pub struct Waiting {
    shared: Arc<Shared>,
    id: NodeId,
    generation: Option<u64>,
}

impl Waiting {
    pub(super) fn new(shared: Arc<Shared>, id: NodeId, generation: Option<u64>) -> Waiting {
        Waiting {
            shared,
            id,
            generation,
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let now = self.shared.clock.wall();
        let mono = self.shared.clock.mono();
        let keep_done = self.shared.keep_done();
        self.shared.with(|inner, fx| {
            inner.waiters = inner.waiters.saturating_sub(1);
            // OPS-8: when the last waiter ended.
            inner.last_waiter_ended = Some(mono);
            let due = entry(inner, &self.id, self.generation).is_some_and(|e| {
                e.timing.waiters = e.timing.waiters.saturating_sub(1);
                e.due(now, keep_done)
            });
            // STA-1: a drop that fell due while it had a waiter happens when the last one ends.
            if due {
                inner.drop_entry(fx, &self.id, now);
            }
        });
    }
}

/// A subscription on a PR; `Drop` does the work (T15).
pub struct Subscribed {
    shared: Arc<Shared>,
    id: NodeId,
    generation: Option<u64>,
}

impl Subscribed {
    pub(super) fn new(shared: Arc<Shared>, id: NodeId, generation: Option<u64>) -> Subscribed {
        Subscribed {
            shared,
            id,
            generation,
        }
    }
}

impl Drop for Subscribed {
    fn drop(&mut self) {
        let now = self.shared.clock.wall();
        let mono = self.shared.clock.mono();
        let keep_done = self.shared.keep_done();
        self.shared.with(|inner, fx| {
            inner.subscriptions = inner.subscriptions.saturating_sub(1);
            // OPS-8: when the last subscription ended.
            inner.last_subscription_ended = Some(mono);
            let due = entry(inner, &self.id, self.generation).is_some_and(|e| {
                e.timing.subscriptions = e.timing.subscriptions.saturating_sub(1);
                // STA-1, NTF-4: the end of a request that named the PR: an `asked` PR's hour starts
                // again, and an ended PR stays `ASKED_HOUR` past it, `keep_done` or not.
                e.asked_at = Some(now);
                e.sub_ended_at = Some(now);
                e.due(now, keep_done)
            });
            // STA-1: a drop that fell due meanwhile is carried out once those 60 minutes are over,
            // which `due` says; with them just started, that is at a later `sweep`.
            if due {
                inner.drop_entry(fx, &self.id, now);
            }
        });
    }
}
