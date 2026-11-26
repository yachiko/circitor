//! Quota and backoff (POL-17 to POL-19, POL-23, POL-25; T08). Pure: the caller passes the time.

use std::time::Duration;

use crate::clock::{Mono, Time};
use crate::config::{Config, QuotaConfig};
use crate::poller::{Policy, QuotaState, Tier};

/// POL-17: the sliding hour.
const HOUR: Duration = Duration::from_secs(3_600);
/// POL-23: slowed above this share of `max_per_hour`, in percent.
const SLOWED_PERCENT: u64 = 80;
/// POL-15 and POL-23: the windows when polls are on.
const PR_WINDOW: Duration = Duration::from_secs(5);
const POLL_WINDOW: Duration = Duration::from_secs(10);
/// POL-23: the windows when the quota is short.
const WIDE_WINDOW: Duration = Duration::from_secs(30);
/// POL-25: the first secondary pause without a `Retry-After`.
const SECONDARY_FIRST: Duration = Duration::from_secs(60);
/// POL-19: the backoff's first step and its cap.
const BACKOFF_FIRST: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(300);

/// A secondary limit in force (POL-25): the monotonic end, and the wall time to show.
#[derive(Clone, Copy, Debug)]
struct SecondaryHold {
    until: Mono,
    until_wall: Time,
}

pub struct Quota {
    cfg: QuotaConfig,
    /// Costs and the wall times they were spent at, in the order spent (not in time order after a
    /// clock set back).
    spends: Vec<(Time, u32)>,
    remaining: Option<u32>,
    reset_at: Option<Time>,
    /// POL-25: exhausted until this wall time.
    exhausted_until: Option<Time>,
    secondary: Option<SecondaryHold>,
    /// Secondary answers in a row.
    secondary_run: u32,
}

impl Quota {
    pub fn new(cfg: &QuotaConfig) -> Quota {
        Quota {
            cfg: cfg.clone(),
            spends: Vec::new(),
            remaining: None,
            reset_at: None,
            exhausted_until: None,
            secondary: None,
            secondary_run: 0,
        }
    }

    /// POL-17.
    pub fn spent(&mut self, cost: u32, at: Time) {
        // POL-26: drop only what is truly an hour old by `at`; after a step back the list isn't in
        // time order.
        self.spends.retain(|(t, _)| at.since(*t) < HOUR);
        self.spends.push((at, cost));
    }

    /// Every answer's `rateLimit` block (POL-18). `remaining == 0` is exhausted from then on (POL-25).
    pub fn observed(&mut self, remaining: u32, reset_at: Option<Time>, at: Time) {
        self.remaining = Some(remaining);
        self.reset_at = reset_at;
        self.exhausted_until = if remaining == 0 {
            Some(reset_at.unwrap_or_else(|| at.plus(HOUR)))
        } else {
            None
        };
    }

    /// `Failure::Exhausted`.
    pub fn exhausted(&mut self, reset_at: Option<Time>, at: Time) {
        self.observed(0, reset_at, at);
    }

    pub fn secondary(
        &mut self,
        retry_after: Option<Duration>,
        at: Mono,
        at_wall: Time,
        idle_interval: Duration,
    ) {
        let factor = 1u32
            .checked_shl(self.secondary_run.min(16))
            .unwrap_or(u32::MAX);
        let step = SECONDARY_FIRST.saturating_mul(factor).min(idle_interval);
        self.secondary_run = self.secondary_run.saturating_add(1);
        let hold = retry_after.unwrap_or(step);
        self.secondary = Some(SecondaryHold {
            until: at.plus(hold),
            until_wall: at_wall.plus(hold),
        });
    }

    /// Any request that succeeds: the run of secondary limits ends.
    pub fn answered(&mut self) {
        self.secondary_run = 0;
    }

    fn is_secondary(&self, mono: Mono) -> bool {
        self.secondary.is_some_and(|s| mono < s.until)
    }

    fn is_exhausted(&self, wall: Time) -> bool {
        self.exhausted_until.is_some_and(|u| wall < u)
    }

    pub fn state(&self, wall: Time, mono: Mono) -> QuotaState {
        if self.is_secondary(mono) {
            return QuotaState::Secondary;
        }
        if self.is_exhausted(wall) {
            return QuotaState::Exhausted;
        }
        if self.capped(wall) {
            return QuotaState::Capped;
        }
        if self.in_reserve() {
            return QuotaState::Reserve;
        }
        let count = u64::from(self.cost_last_hour(wall));
        if count * 100 > u64::from(self.cfg.max_per_hour) * SLOWED_PERCENT {
            return QuotaState::Slowed;
        }
        QuotaState::Normal
    }

    pub fn policy(&self, wall: Time, mono: Mono, tier: Tier, cfg: &Config) -> Policy {
        let state = self.state(wall, mono);
        let base = match tier {
            Tier::Active => cfg.interval,
            Tier::Idle => cfg.idle_interval,
        };
        let (poll_every, hot_every, demand, pr_window, poll_window, follow_up) = match state {
            QuotaState::Normal => (
                Some(base),
                Some(cfg.hot_interval),
                true,
                PR_WINDOW,
                POLL_WINDOW,
                true,
            ),
            QuotaState::Slowed => (
                Some(base * 2),
                Some(cfg.hot_interval * 2),
                true,
                PR_WINDOW,
                POLL_WINDOW,
                true,
            ),
            QuotaState::Reserve => (
                Some(cfg.idle_interval),
                Some(cfg.idle_interval),
                true,
                WIDE_WINDOW,
                WIDE_WINDOW,
                false,
            ),
            QuotaState::Capped => (None, None, true, WIDE_WINDOW, WIDE_WINDOW, false),
            QuotaState::Exhausted | QuotaState::Secondary => {
                (None, None, false, WIDE_WINDOW, WIDE_WINDOW, false)
            }
        };
        Policy {
            state,
            poll_every,
            hot_every,
            demand,
            pr_window,
            poll_window,
            follow_up,
        }
    }

    pub fn cost_last_hour(&self, wall: Time) -> u32 {
        self.spends
            .iter()
            .filter(|(t, _)| wall.since(*t) < HOUR)
            .fold(0u32, |sum, (_, c)| sum.saturating_add(*c))
    }

    /// `quota_capped`'s own condition.
    pub fn capped(&self, wall: Time) -> bool {
        self.cost_last_hour(wall) >= self.cfg.max_per_hour
    }

    /// `quota_reserve`'s own condition.
    pub fn in_reserve(&self) -> bool {
        self.remaining.is_some_and(|r| r < self.cfg.reserve)
    }

    pub fn remaining(&self) -> Option<u32> {
        self.remaining
    }

    pub fn reset_at(&self) -> Option<Time> {
        self.reset_at
    }

    /// When sending resumes, while exhausted or under a secondary limit (API-4's `until`).
    pub fn resume_at(&self, wall: Time, mono: Mono) -> Option<Time> {
        let exhausted = self.exhausted_until.filter(|_| self.is_exhausted(wall));
        let secondary = self
            .secondary
            .filter(|_| self.is_secondary(mono))
            .map(|s| s.until_wall);
        match (exhausted, secondary) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }
}

/// POL-19.
pub struct Backoff {
    /// The last step; `None` when no backoff runs.
    step: Option<Duration>,
    next: Option<Mono>,
    next_wall: Option<Time>,
    read_through_taken: bool,
}

impl Backoff {
    pub fn new() -> Backoff {
        Backoff {
            step: None,
            next: None,
            next_wall: None,
            read_through_taken: false,
        }
    }

    /// 5 s, doubling to 5 min.
    pub fn failed(&mut self, at: Mono, at_wall: Time) {
        let step = match self.step {
            None => BACKOFF_FIRST,
            Some(s) => (s * 2).min(BACKOFF_MAX),
        };
        self.set(step, at, at_wall);
    }

    pub fn succeeded(&mut self) {
        *self = Backoff::new();
    }

    /// POL-22: starts again at 5 s, if it was running.
    pub fn woke(&mut self, at: Mono, at_wall: Time) {
        if self.step.is_some() {
            self.set(BACKOFF_FIRST, at, at_wall);
        }
    }

    fn set(&mut self, step: Duration, at: Mono, at_wall: Time) {
        self.step = Some(step);
        self.next = Some(at.plus(step));
        self.next_wall = Some(at_wall.plus(step));
        self.read_through_taken = false;
    }

    pub fn active(&self) -> bool {
        self.step.is_some()
    }

    pub fn next_try(&self) -> Option<Mono> {
        self.next
    }

    /// API-4's `until`.
    pub fn next_try_wall(&self) -> Option<Time> {
        self.next_wall
    }

    /// POL-16: one per step.
    pub fn take_read_through(&mut self) -> bool {
        if self.step.is_none() {
            return true;
        }
        !std::mem::replace(&mut self.read_through_taken, true)
    }
}

impl Default for Backoff {
    fn default() -> Backoff {
        Backoff::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: i64) -> Time {
        Time::from_unix_millis(1_800_000_000_000 + s * 1000)
    }
    fn m(s: u64) -> Mono {
        Mono::ZERO.plus(Duration::from_secs(s))
    }
    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }
    fn quota() -> Quota {
        Quota::new(&QuotaConfig::default())
    }
    fn state(q: &Quota, s: i64) -> QuotaState {
        q.state(t(s), m(s as u64))
    }

    // POL-17:
    #[test]
    fn pol17_sliding_hour() {
        let mut q = quota();
        q.spent(7, t(100));
        assert_eq!(q.cost_last_hour(t(100 + 3_599)), 7);
        assert_eq!(q.cost_last_hour(t(100 + 3_600)), 0);
    }

    // POL-26:
    #[test]
    fn pol26_clock_back_keeps_the_count() {
        let mut q = quota();
        q.spent(5, t(10_000));
        assert_eq!(q.cost_last_hour(t(10_000 - 3_600)), 5);
        q.spent(3, t(10_000 - 3_600));
        q.spent(2, t(10_000 - 3_000));
        assert_eq!(q.cost_last_hour(t(10_000 - 3_000)), 10);
        assert_eq!(q.cost_last_hour(t(9_999)), 10);
    }

    // POL-23:
    #[test]
    fn pol23_thresholds() {
        for (cost, want) in [
            (480, QuotaState::Normal),
            (481, QuotaState::Slowed),
            (599, QuotaState::Slowed),
            (600, QuotaState::Capped),
        ] {
            let mut q = quota();
            q.spent(cost, t(0));
            assert_eq!(state(&q, 1), want, "cost {cost}");
            assert_eq!(q.capped(t(1)), cost >= 600);
        }
        let mut q = quota();
        q.spent(300, t(0));
        q.spent(300, t(1_000));
        assert!(q.capped(t(2_000)));
        assert!(!q.capped(t(3_600)));
    }

    // POL-18:
    #[test]
    fn pol18_reserve() {
        let mut q = quota();
        assert!(!q.in_reserve());
        q.observed(500, Some(t(900)), t(0));
        assert!(!q.in_reserve());
        assert_eq!(state(&q, 1), QuotaState::Normal);
        q.observed(499, Some(t(900)), t(1));
        assert!(q.in_reserve());
        assert_eq!(state(&q, 2), QuotaState::Reserve);
        assert_eq!(q.remaining(), Some(499));
        assert_eq!(q.reset_at(), Some(t(900)));
    }

    // POL-23:
    #[test]
    fn pol23_precedence() {
        let mut q = quota();
        q.spent(600, t(0));
        q.observed(10, Some(t(900)), t(0));
        assert_eq!(state(&q, 1), QuotaState::Capped);

        let mut q = quota();
        q.spent(500, t(0));
        q.observed(10, Some(t(900)), t(0));
        assert_eq!(state(&q, 1), QuotaState::Reserve);

        let mut q = quota();
        q.spent(600, t(0));
        q.exhausted(Some(t(900)), t(0));
        assert_eq!(state(&q, 1), QuotaState::Exhausted);
        assert!(q.capped(t(1)));
        assert!(q.in_reserve());

        q.secondary(Some(secs(30)), m(0), t(0), secs(300));
        assert_eq!(state(&q, 1), QuotaState::Secondary);
        assert!(q.capped(t(1)));
        assert!(q.in_reserve());
    }

    // POL-23:
    #[test]
    fn pol23_policy_table() {
        let cfg = Config::default();
        let pol = |q: &Quota, tier| q.policy(t(1), m(1), tier, &cfg);

        let q = quota();
        let p = pol(&q, Tier::Active);
        assert_eq!(p.state, QuotaState::Normal);
        assert_eq!(p.poll_every, Some(secs(30)));
        assert_eq!(p.hot_every, Some(secs(10)));
        assert!(p.demand && p.follow_up);
        assert_eq!((p.pr_window, p.poll_window), (secs(5), secs(10)));
        let p = pol(&q, Tier::Idle);
        assert_eq!(p.poll_every, Some(secs(300)));
        assert_eq!(p.hot_every, Some(secs(10)));

        let mut q = quota();
        q.spent(481, t(0));
        let p = pol(&q, Tier::Active);
        assert_eq!(p.state, QuotaState::Slowed);
        assert_eq!(p.poll_every, Some(secs(60)));
        assert_eq!(p.hot_every, Some(secs(20)));
        assert!(p.demand && p.follow_up);
        assert_eq!((p.pr_window, p.poll_window), (secs(5), secs(10)));
        let p = pol(&q, Tier::Idle);
        assert_eq!(p.poll_every, Some(secs(600)));
        assert_eq!(p.hot_every, Some(secs(20)));

        let mut q = quota();
        q.observed(10, None, t(0));
        for tier in [Tier::Active, Tier::Idle] {
            let p = pol(&q, tier);
            assert_eq!(p.state, QuotaState::Reserve);
            assert_eq!(p.poll_every, Some(secs(300)));
            assert_eq!(p.hot_every, Some(secs(300)));
            assert!(p.demand && !p.follow_up);
            assert_eq!((p.pr_window, p.poll_window), (secs(30), secs(30)));
        }

        let mut q = quota();
        q.spent(600, t(0));
        for tier in [Tier::Active, Tier::Idle] {
            let p = pol(&q, tier);
            assert_eq!(p.state, QuotaState::Capped);
            assert_eq!((p.poll_every, p.hot_every), (None, None));
            assert!(p.demand && !p.follow_up);
            assert_eq!((p.pr_window, p.poll_window), (secs(30), secs(30)));
        }

        let mut q = quota();
        q.exhausted(Some(t(900)), t(0));
        for tier in [Tier::Active, Tier::Idle] {
            let p = pol(&q, tier);
            assert_eq!(p.state, QuotaState::Exhausted);
            assert_eq!((p.poll_every, p.hot_every), (None, None));
            assert!(!p.demand && !p.follow_up);
            assert_eq!((p.pr_window, p.poll_window), (secs(30), secs(30)));
        }
        let mut q = quota();
        q.secondary(None, m(0), t(0), secs(300));
        for tier in [Tier::Active, Tier::Idle] {
            let p = pol(&q, tier);
            assert_eq!(p.state, QuotaState::Secondary);
            assert_eq!((p.poll_every, p.hot_every), (None, None));
            assert!(!p.demand && !p.follow_up);
        }
    }

    // POL-25:
    #[test]
    fn pol25_exhausted() {
        let mut q = quota();
        q.exhausted(Some(t(1_200)), t(0));
        assert_eq!(q.remaining(), Some(0));
        assert_eq!(state(&q, 1_199), QuotaState::Exhausted);
        assert_eq!(state(&q, 1_200), QuotaState::Reserve);
        assert_eq!(q.resume_at(t(1_199), m(0)), Some(t(1_200)));
        assert_eq!(q.resume_at(t(1_200), m(0)), None);
        // A wall time jumped past the reset.
        assert_eq!(state(&q, 5_000), QuotaState::Reserve);

        let mut q = quota();
        q.exhausted(None, t(100));
        assert_eq!(state(&q, 100 + 3_599), QuotaState::Exhausted);
        assert_eq!(state(&q, 100 + 3_600), QuotaState::Reserve);
        assert_eq!(q.resume_at(t(200), m(0)), Some(t(3_700)));

        let mut q = quota();
        q.observed(0, Some(t(1_200)), t(0));
        assert_eq!(state(&q, 10), QuotaState::Exhausted);
        q.observed(4_000, Some(t(4_000)), t(20));
        assert_eq!(state(&q, 21), QuotaState::Normal);
        assert_eq!(q.resume_at(t(21), m(21)), None);
    }

    // POL-25:
    #[test]
    fn pol25_secondary() {
        let mut q = quota();
        q.secondary(Some(secs(30)), m(100), t(5_000), secs(300));
        // The wall time does what it likes; the monotonic clock decides.
        assert_eq!(q.state(t(0), m(129)), QuotaState::Secondary);
        assert_eq!(q.state(t(99_999), m(130)), QuotaState::Normal);
        assert_eq!(q.resume_at(t(5_001), m(129)), Some(t(5_030)));
        assert_eq!(q.resume_at(t(5_001), m(130)), None);

        let mut q = quota();
        let mut now = 0u64;
        for want in [60u64, 120, 240, 300, 300] {
            q.secondary(None, m(now), t(now as i64), secs(300));
            assert_eq!(q.state(t(0), m(now + want - 1)), QuotaState::Secondary);
            assert_ne!(
                q.state(t(0), m(now + want)),
                QuotaState::Secondary,
                "{want}"
            );
            now += want;
        }
        q.answered();
        q.secondary(None, m(now), t(0), secs(300));
        assert_eq!(q.state(t(0), m(now + 59)), QuotaState::Secondary);
        assert_ne!(q.state(t(0), m(now + 60)), QuotaState::Secondary);
    }

    // POL-25: the later of the two.
    #[test]
    fn pol25_resume_at_takes_the_later() {
        let mut q = quota();
        q.exhausted(Some(t(1_000)), t(0));
        q.secondary(Some(secs(30)), m(0), t(0), secs(300));
        assert_eq!(q.resume_at(t(1), m(1)), Some(t(1_000)));
        q.secondary(Some(secs(2_000)), m(0), t(0), secs(300));
        assert_eq!(q.resume_at(t(1), m(1)), Some(t(2_000)));
    }

    // POL-19:
    #[test]
    fn pol19_backoff_steps() {
        let mut b = Backoff::new();
        assert!(!b.active());
        assert_eq!((b.next_try(), b.next_try_wall()), (None, None));
        for (i, want) in [5u64, 10, 20, 40, 80, 160, 300, 300]
            .into_iter()
            .enumerate()
        {
            let at = i as u64 * 1_000;
            b.failed(m(at), t(at as i64));
            assert!(b.active());
            assert_eq!(b.next_try(), Some(m(at + want)));
            assert_eq!(b.next_try_wall(), Some(t((at + want) as i64)));
        }
        b.succeeded();
        assert!(!b.active());
        assert_eq!((b.next_try(), b.next_try_wall()), (None, None));
        b.failed(m(0), t(0));
        assert_eq!(b.next_try(), Some(m(5)));
    }

    // POL-22:
    #[test]
    fn pol22_woke() {
        let mut b = Backoff::new();
        for _ in 0..5 {
            b.failed(m(0), t(0));
        }
        assert_eq!(b.next_try(), Some(m(80)));
        b.woke(m(1_000), t(1_000));
        assert_eq!(b.next_try(), Some(m(1_005)));
        assert_eq!(b.next_try_wall(), Some(t(1_005)));
        b.failed(m(1_005), t(1_005));
        assert_eq!(b.next_try(), Some(m(1_015)));

        let mut idle = Backoff::new();
        idle.woke(m(5), t(5));
        assert!(!idle.active());
        assert_eq!(idle.next_try(), None);
    }

    // POL-16:
    #[test]
    fn pol16_one_read_through_per_step() {
        let mut b = Backoff::new();
        assert!(b.take_read_through());
        assert!(b.take_read_through());
        b.failed(m(0), t(0));
        assert!(b.take_read_through());
        assert!(!b.take_read_through());
        b.failed(m(5), t(5));
        assert!(b.take_read_through());
        assert!(!b.take_read_through());
        b.woke(m(6), t(6));
        assert!(b.take_read_through());
        assert!(!b.take_read_through());
    }
}
