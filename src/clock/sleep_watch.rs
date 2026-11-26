use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::TimeDelta;

use super::{Mono, SharedClock, Time};

/// POL-22: a gap between the wall clock's move and the monotonic clock's move over this is a sleep.
pub const SLEEP_GAP: Duration = Duration::from_secs(30);

struct Baseline {
    wall: Time,
    mono: Mono,
    total: Duration,
}

/// POL-22's check. One per daemon; the engine calls `check` at every tick, the API at every client request.
pub struct SleepWatch {
    clock: SharedClock,
    // Held for the whole of a check, clock reads included, so concurrent calls see one sleep once.
    // Never held across an `.await`: `check` is synchronous.
    state: Mutex<Baseline>,
    total_tx: tokio::sync::watch::Sender<Duration>,
}

impl SleepWatch {
    pub fn new(clock: SharedClock) -> Arc<SleepWatch> {
        let state = Baseline {
            wall: clock.wall(),
            mono: clock.mono(),
            total: Duration::ZERO,
        };
        let (total_tx, _) = tokio::sync::watch::channel(Duration::ZERO);
        Arc::new(SleepWatch {
            clock,
            state: Mutex::new(state),
            total_tx,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Baseline> {
        // A panic elsewhere while holding it leaves a baseline that is still a valid one.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// How far wall and monotonic time moved since the last call. Wall ahead by more than 30 s is a
    /// sleep: returns its length (the gap) and adds it to `total`. A wall clock set back is never a sleep.
    pub fn check(&self) -> Option<Duration> {
        let mut state = self.lock();
        let wall = self.clock.wall();
        let mono = self.clock.mono();
        // POL-22: the gap is how far the wall clock moved less how far the monotonic clock moved.
        let wall_move = wall.inner() - state.wall.inner();
        let mono_move = TimeDelta::from_std(mono.since(state.mono)).unwrap_or(TimeDelta::MAX);
        let gap = wall_move.checked_sub(&mono_move).unwrap_or(TimeDelta::MIN);
        // Every check moves the baseline, a clock set back included.
        state.wall = wall;
        state.mono = mono;
        let limit = TimeDelta::from_std(SLEEP_GAP).unwrap_or(TimeDelta::MAX);
        // POL-22: over 30 s is a sleep; exactly 30 s isn't, and a negative gap never is.
        if gap <= limit {
            return None;
        }
        let gap = gap.to_std().ok()?;
        state.total = state.total.saturating_add(gap);
        self.total_tx.send_replace(state.total);
        Some(gap)
    }

    /// All the sleep noticed since the daemon started.
    pub fn total(&self) -> Duration {
        self.lock().total
    }

    /// Changes whenever `total` does. Waiters re-check their deadline on it (API-12).
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Duration> {
        self.total_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    const H: Duration = Duration::from_secs(3600);

    fn setup() -> (Arc<TestClock>, Arc<SleepWatch>) {
        let clock = TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap());
        let watch = SleepWatch::new(clock.clone());
        (clock, watch)
    }

    // POL-22: a gap over 30 s is a sleep, exactly 30 s isn't; both of TST-4's sleeps.
    #[tokio::test]
    async fn pol22_a_gap_over_30_s_is_a_sleep() {
        let (clock, watch) = setup();
        assert_eq!(watch.check(), None);
        clock.sleep_wall_only(Duration::from_secs(31)).await;
        assert_eq!(watch.check(), Some(Duration::from_secs(31)));
        clock.sleep_wall_only(Duration::from_secs(30)).await;
        assert_eq!(watch.check(), None);
        clock.sleep_wall_only(2 * H).await;
        assert_eq!(watch.check(), Some(2 * H));
        clock.sleep_both(2 * H).await;
        assert_eq!(watch.check(), None);
        // Time moving normally is never a sleep.
        clock.advance(H).await;
        assert_eq!(watch.check(), None);
    }

    // POL-22: a wall clock set back is never a sleep, and the check after it starts from there.
    #[tokio::test]
    async fn pol22_a_clock_set_back_is_never_a_sleep() {
        let (clock, watch) = setup();
        clock.set_wall_back(H).await;
        assert_eq!(watch.check(), None);
        assert_eq!(watch.total(), Duration::ZERO);
        clock.sleep_wall_only(Duration::from_secs(31)).await;
        assert_eq!(watch.check(), Some(Duration::from_secs(31)));
    }

    // POL-22: total adds every sleep up, subscribers hear each, and one jump is reported once.
    #[tokio::test]
    async fn pol22_total_adds_up_and_subscribers_hear() {
        let (clock, watch) = setup();
        let mut rx = watch.subscribe();
        assert!(!rx.has_changed().unwrap());
        clock.sleep_wall_only(2 * H).await;
        assert_eq!(watch.check(), Some(2 * H));
        assert_eq!(watch.check(), None);
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), 2 * H);
        clock.sleep_wall_only(Duration::from_secs(40)).await;
        assert_eq!(watch.check(), Some(Duration::from_secs(40)));
        assert_eq!(watch.total(), 2 * H + Duration::from_secs(40));
        assert_eq!(*rx.borrow_and_update(), 2 * H + Duration::from_secs(40));
        assert_eq!(watch.check(), None);
        assert!(!rx.has_changed().unwrap());
    }

    // POL-22: concurrent checks see one sleep once.
    #[test]
    fn pol22_concurrent_checks_see_one_sleep_once() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let (clock, watch) = setup();
        rt.block_on(clock.sleep_wall_only(H));
        let seen: usize = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8).map(|_| s.spawn(|| watch.check())).collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(Option::is_some)
                .count()
        });
        assert_eq!(seen, 1);
        assert_eq!(watch.total(), H);
    }
}
