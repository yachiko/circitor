#![allow(unused_variables, dead_code)] // stub: T09 removes this
//! Cadence: tiers, periods, windows (POL-10, POL-13 to POL-15, MOD-16; T09). Pure.

use std::collections::HashSet;
use std::time::Duration;

use crate::clock::Mono;
use crate::config::Config;
use crate::model::{NodeId, Record, Source, Status};
use crate::poller::{Policy, Tier};
use crate::state::{Class, Target, Timing};

/// POL-13.
pub fn tier(last_request: Option<Mono>, now: Mono, cfg: &Config) -> Tier {
    unimplemented!("T09")
}

/// POL-13 with NTF-5: while a subscription on an open PR is held the tier is active, whatever the last client
/// request; once none is, it follows the last client request. So: `now` while `open` is above 0, else
/// `last_request`. The engine passes the result to `tier` as its `last_request`.
pub fn last_ask(last_request: Option<Mono>, open: u32, now: Mono) -> Option<Mono> {
    unimplemented!("T09")
}

/// POL-14. It reads `timing.waiters` and `timing.named`, never `timing.subscriptions` (NTF-5).
pub fn hot(status: Option<Status>, timing: &Timing, now: Mono) -> bool {
    unimplemented!("T09")
}

/// POL-10's table, with the configured values (MOD-16 needs those, not slowed ones). `subscribed`:
/// `timing.subscriptions` is above 0. An open PR with a subscription has its tier's period whatever its
/// `source`; an ended one `idle_interval` (NTF-5).
pub fn period(
    class: Class,
    source: Source,
    hot: bool,
    subscribed: bool,
    tier: Tier,
    cfg: &Config,
) -> Duration {
    unimplemented!("T09")
}

/// MOD-16. A `gone` record is never stale. **`false` until T09** (`plan/contracts/api.md`).
pub fn stale(record: &Record, timing: &Timing, tier: Tier, cfg: &Config, now: Mono) -> bool {
    false
}

/// `None`: no polls; never polled: `Mono::ZERO`, due at once.
pub fn poll_due(last_poll_sent: Option<Mono>, policy: &Policy) -> Option<Mono> {
    unimplemented!("T09")
}

/// Which tracked PRs a poll built at `now` refreshes (POL-4, POL-10, POL-13). `listed` is `None` for the
/// poll's first request, and the open list once it's read: a listed PR is due as an open one (POL-10).
pub fn in_poll(
    targets: &[Target],
    listed: Option<&HashSet<NodeId>>,
    tier: Tier,
    cfg: &Config,
    now: Mono,
) -> Vec<NodeId> {
    unimplemented!("T09")
}

pub struct HotDue {
    pub now: Vec<NodeId>,
    pub next: Option<Mono>,
}

/// POL-14.
pub fn hot_due(targets: &[Target], policy: &Policy, now: Mono) -> HotDue {
    unimplemented!("T09")
}

/// POL-15's window.
pub fn may_force_pr(timing: &Timing, policy: &Policy, now: Mono) -> bool {
    unimplemented!("T09")
}

/// POL-15: "only when the last forced poll was sent more than 10 s ago". An ordinary poll doesn't count.
pub fn may_force_poll(last_forced_poll: Option<Mono>, policy: &Policy, now: Mono) -> bool {
    unimplemented!("T09")
}

/// POL-11: whether a refresh that read `UNKNOWN` for this open PR gets one follow-up.
pub fn wants_follow_up(
    target: &Target,
    first_since_start: bool,
    policy: &Policy,
    now: Mono,
) -> bool {
    unimplemented!("T09")
}

/// POL-7: 40 a request.
pub fn batches<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    unimplemented!("T09")
}
