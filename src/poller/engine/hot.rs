//! Hot refreshes (POL-14) and the `mergeable` follow-up (POL-11), outside polls (T25).

use std::time::Duration;

use crate::clock::Mono;
use crate::model::{Mergeable, NodeId, PrRead, ReadState};
use crate::poller::cadence;

use super::run::Run;

/// POL-11 (a proposal): the follow-up is sent this long after the answer that read `UNKNOWN`.
const FOLLOW_UP_AFTER: Duration = Duration::from_secs(3);

impl Run {
    /// POL-14: the hot PRs that are due, by ID, without finding. Held by a backoff and by `auth`
    /// (POL-19, POL-20); the policy says how often (POL-23).
    pub(super) async fn serve_hot(&mut self) {
        if self.auth_blocked() || self.backoff.active() {
            return;
        }
        let policy = self.policy();
        let due = cadence::hot_due(&self.d.store.targets(), &policy, self.now());
        if due.now.is_empty() {
            return;
        }
        self.run_refresh(due.now, "hot", false).await;
    }

    /// When the next hot refresh or follow-up is due.
    pub(super) fn hot_wake(&self) -> Option<Mono> {
        let follow = self.dm.follow_ups.iter().map(|(at, _)| *at).min();
        if self.auth_blocked() || self.backoff.active() {
            return follow;
        }
        let now = self.now();
        let due = cadence::hot_due(&self.d.store.targets(), &self.policy(), now);
        let hot = if due.now.is_empty() {
            due.next
        } else {
            Some(now)
        };
        match (follow, hot) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// POL-11: after a read is applied. An open PR read `UNKNOWN` is asked once more, 3 s after
    /// that answer, when `cadence::wants_follow_up` says so; a follow-up's own reads never ask again.
    pub(super) fn after_apply(&mut self, id: &NodeId, read: &PrRead) {
        // POL-11: "refreshed for the first time since the daemon started".
        let first = self.dm.refreshed.insert(id.clone());
        if self.dm.follow_up_run
            || read.mergeable != Mergeable::Unknown
            || read.state != ReadState::Open
        {
            return;
        }
        let Some(target) = self.d.store.targets().into_iter().find(|t| t.id == *id) else {
            return;
        };
        let now = self.now();
        // `Target::mergeable_unknown` is the read's value, whatever MOD-8 kept in the record.
        if !cadence::wants_follow_up(&target, first, &self.policy(), now) {
            return;
        }
        if self.dm.follow_ups.iter().any(|(_, ids)| ids.contains(id)) {
            return;
        }
        let at = now.plus(FOLLOW_UP_AFTER);
        // POL-11: the PRs of one answer share one request.
        match self.dm.follow_ups.iter_mut().find(|(due, _)| *due == at) {
            Some((_, ids)) => ids.push(id.clone()),
            None => self.dm.follow_ups.push((at, vec![id.clone()])),
        }
    }

    /// POL-11: the follow-ups due, each one request, as `Record`. Not sent in a backoff, in `auth`,
    /// or when the policy forbids it (POL-19, POL-23): then it's dropped.
    pub(super) async fn serve_follow_ups(&mut self) {
        loop {
            let now = self.now();
            let Some(i) = self.dm.follow_ups.iter().position(|(at, _)| *at <= now) else {
                return;
            };
            let (_, ids) = self.dm.follow_ups.remove(i);
            if self.backoff.active() || self.auth_blocked() || !self.policy().follow_up {
                continue;
            }
            let ids: Vec<NodeId> = ids
                .into_iter()
                .filter(|id| self.d.store.is_tracked(id))
                .collect();
            if ids.is_empty() {
                continue;
            }
            self.run_refresh(ids, "follow_up", true).await;
        }
    }
}
