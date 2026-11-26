//! Cadence: tiers, periods, windows (POL-10, POL-13 to POL-15, MOD-16; T09). Pure.

use std::collections::HashSet;
use std::time::Duration;

use crate::clock::Mono;
use crate::config::Config;
use crate::model::{NodeId, PrState, Record, Source, Status};
use crate::poller::{Policy, Tier};
use crate::state::{Class, Target, Timing};

/// POL-14: a request names a PR for this long (checks pending or unknown).
const NAMED_HOT_FOR: Duration = Duration::from_secs(120);
/// POL-11: a request names a PR for this long (the follow-up).
const NAMED_FOLLOW_UP_FOR: Duration = Duration::from_secs(60);
/// MOD-16: a record is stale when fetched more than this many periods ago.
const STALE_AFTER_PERIODS: u32 = 2;
/// POL-7: PRs in one request.
const BATCH: usize = 40;

/// POL-13.
pub fn tier(last_request: Option<Mono>, now: Mono, cfg: &Config) -> Tier {
    // POL-13: active while a request came less than `idle_after` ago.
    match last_request {
        Some(at) if now.since(at) < cfg.idle_after => Tier::Active,
        _ => Tier::Idle,
    }
}

/// POL-13 with NTF-5: while a subscription on an open PR is held the tier is active, whatever the last client
/// request; once none is, it follows the last client request. So: `now` while `open` is above 0, else
/// `last_request`. The engine passes the result to `tier` as its `last_request`.
pub fn last_ask(last_request: Option<Mono>, open: u32, now: Mono) -> Option<Mono> {
    // NTF-5: an open subscription counts as a request just now; an end doesn't.
    if open > 0 { Some(now) } else { last_request }
}

/// POL-14. It reads `timing.waiters` and `timing.named`, never `timing.subscriptions` (NTF-5).
pub fn hot(status: Option<Status>, timing: &Timing, now: Mono) -> bool {
    // POL-14: a waiter, or named lately while the checks are pending or unknown.
    if timing.waiters > 0 {
        return true;
    }
    let pending = matches!(status, Some(Status::ChecksPending | Status::Unknown));
    let named = timing.named.is_some_and(|at| now.since(at) < NAMED_HOT_FOR);
    pending && named
}

fn tier_interval(tier: Tier, cfg: &Config) -> Duration {
    match tier {
        Tier::Active => cfg.interval,
        Tier::Idle => cfg.idle_interval,
    }
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
    // POL-10: the first row that applies.
    if hot {
        cfg.hot_interval
    } else if class == Class::Open && subscribed {
        tier_interval(tier, cfg)
    } else if class == Class::Ended || source == Source::Asked {
        cfg.idle_interval
    } else {
        tier_interval(tier, cfg)
    }
}

/// MOD-16. A `gone` record is never stale.
pub fn stale(record: &Record, timing: &Timing, tier: Tier, cfg: &Config, now: Mono) -> bool {
    // MOD-16: never for a gone record.
    if record.state == PrState::Gone {
        return false;
    }
    if timing.attempt_failed || timing.slept {
        return true;
    }
    let Some(fetched) = timing.fetched else {
        return false;
    };
    let class = match record.state {
        PrState::Open => Class::Open,
        _ => Class::Ended,
    };
    let is_hot = hot(Some(record.status), timing, now);
    let p = period(
        class,
        record.source,
        is_hot,
        timing.subscriptions > 0,
        tier,
        cfg,
    );
    // MOD-16: strictly more than twice the period.
    now.since(fetched) > p * STALE_AFTER_PERIODS
}

/// `None`: no polls; never polled: `Mono::ZERO`, due at once.
pub fn poll_due(last_poll_sent: Option<Mono>, policy: &Policy) -> Option<Mono> {
    // POL-13: a poll is due `poll_every` after the last one was sent.
    let every = policy.poll_every?;
    Some(last_poll_sent.map_or(Mono::ZERO, |at| at.plus(every)))
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
    targets
        .iter()
        .filter(|t| {
            // POL-10: a first refresh is the engine's own request.
            if t.class == Class::First {
                return false;
            }
            if t.class == Class::Loaded {
                return true;
            }
            let Some(last) = t.timing.last_sent else {
                return true;
            };
            let is_hot = hot(t.status, &t.timing, now);
            // POL-10: a listed PR is due as an open, tracked one.
            let (class, source) = if listed.is_some_and(|l| l.contains(&t.id)) {
                (Class::Open, Source::Authored)
            } else {
                (t.class, t.source)
            };
            let p = period(class, source, is_hot, t.timing.subscriptions > 0, tier, cfg);
            // POL-13: at least its period ago.
            now.since(last) >= p
        })
        .map(|t| t.id.clone())
        .collect()
}

pub struct HotDue {
    pub now: Vec<NodeId>,
    pub next: Option<Mono>,
}

/// POL-14.
pub fn hot_due(targets: &[Target], policy: &Policy, now: Mono) -> HotDue {
    let mut due = HotDue {
        now: Vec::new(),
        next: None,
    };
    // POL-14, POL-23: no hot refreshes.
    let Some(every) = policy.hot_every else {
        return due;
    };
    for t in targets {
        if t.class == Class::First || !hot(t.status, &t.timing, now) {
            continue;
        }
        match t.timing.last_sent {
            None => due.now.push(t.id.clone()),
            Some(last) => {
                let at = last.plus(every);
                if at <= now {
                    due.now.push(t.id.clone());
                } else if due.next.is_none_or(|n| at < n) {
                    due.next = Some(at);
                }
            }
        }
    }
    due
}

/// More than `window` ago, or never.
fn outside_window(last: Option<Mono>, window: Duration, now: Mono) -> bool {
    last.is_none_or(|at| now.since(at) > window)
}

/// POL-15's window.
pub fn may_force_pr(timing: &Timing, policy: &Policy, now: Mono) -> bool {
    outside_window(timing.last_sent, policy.pr_window, now)
}

/// POL-15: "only when the last forced poll was sent more than 10 s ago". An ordinary poll doesn't count.
pub fn may_force_poll(last_forced_poll: Option<Mono>, policy: &Policy, now: Mono) -> bool {
    outside_window(last_forced_poll, policy.poll_window, now)
}

/// POL-11: whether a refresh that read `UNKNOWN` for this open PR gets one follow-up.
pub fn wants_follow_up(
    target: &Target,
    first_since_start: bool,
    policy: &Policy,
    now: Mono,
) -> bool {
    if !policy.follow_up || target.class != Class::Open || !target.mergeable_unknown {
        return false;
    }
    let named = target
        .timing
        .named
        .is_some_and(|at| now.since(at) < NAMED_FOLLOW_UP_FOR);
    target.timing.waiters > 0 || named || first_since_start
}

/// POL-7: 40 a request.
pub fn batches<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    items.chunks(BATCH).map(<[T]>::to_vec).collect()
}

#[cfg(test)]
mod tests;
