//! The clock, work in flight, sleep detection (`plan/contracts/clock.md`).
//!
//! T02 builds `Time`, `Mono`, `sleep`, `timeout` and `Activity`; T04 builds `SystemClock`,
//! `TestClock` and `SleepWatch`, and takes the module over.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use futures::future::{BoxFuture, Either};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod sleep_watch;
mod system;
#[cfg(any(test, feature = "test-endpoint"))]
mod test_clock;

pub use sleep_watch::SleepWatch;
pub use system::SystemClock;
#[cfg(any(test, feature = "test-endpoint"))]
pub use test_clock::{SettleProbe, TestClock};

/// Wall time, UTC (POL-26). It can jump either way, and it counts a suspend.
/// Serialises as RFC 3339 in whole seconds with `Z`: `2026-11-24T22:09:23Z` (specs/README).
/// Deserialises any RFC 3339 time. Keeps millisecond precision in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(chrono::DateTime<chrono::Utc>);

/// A time that isn't RFC 3339.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an RFC 3339 time")]
pub struct TimeError;

impl Time {
    /// Out of chrono's range, it's the nearest end of that range.
    pub fn from_unix_millis(ms: i64) -> Time {
        match DateTime::from_timestamp_millis(ms) {
            Some(t) => Time(t),
            None if ms < 0 => Time(DateTime::<Utc>::MIN_UTC),
            None => Time(DateTime::<Utc>::MAX_UTC),
        }
    }

    pub fn unix_millis(self) -> i64 {
        self.0.timestamp_millis()
    }

    /// Any RFC 3339 time, any offset; kept to the millisecond.
    pub fn parse(rfc3339: &str) -> Result<Time, TimeError> {
        let t = DateTime::parse_from_rfc3339(rfc3339).map_err(|_| TimeError)?;
        Ok(Time::from_unix_millis(
            t.with_timezone(&Utc).timestamp_millis(),
        ))
    }

    /// Whole seconds, `Z`.
    pub fn rfc3339(self) -> String {
        self.0.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    pub fn plus(self, d: Duration) -> Time {
        TimeDelta::from_std(d)
            .ok()
            .and_then(|d| self.0.checked_add_signed(d))
            .map_or(Time(DateTime::<Utc>::MAX_UTC), Time)
    }

    pub fn minus(self, d: Duration) -> Time {
        TimeDelta::from_std(d)
            .ok()
            .and_then(|d| self.0.checked_sub_signed(d))
            .map_or(Time(DateTime::<Utc>::MIN_UTC), Time)
    }

    /// `self - earlier`, zero when `earlier` is later (POL-26: a wall time ahead of now counts as now).
    pub fn since(self, earlier: Time) -> Duration {
        (self.0 - earlier.0).to_std().unwrap_or(Duration::ZERO)
    }

    pub fn inner(self) -> chrono::DateTime<chrono::Utc> {
        self.0
    }
}

impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.rfc3339())
    }
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.rfc3339())
    }
}

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Time, D::Error> {
        let s = String::deserialize(d)?;
        Time::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Monotonic time since the clock was made. Never goes back. Whether it counts a suspend is the
/// platform's choice (docs/04 §8 item 6): nothing that must survive a sleep is timed on it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mono(Duration);

impl Mono {
    pub const ZERO: Mono = Mono(Duration::ZERO);

    pub fn plus(self, d: Duration) -> Mono {
        Mono(self.0.saturating_add(d))
    }

    /// Saturating: zero when `earlier` is later.
    pub fn since(self, earlier: Mono) -> Duration {
        self.0.saturating_sub(earlier.0)
    }

    pub fn as_duration(self) -> Duration {
        self.0
    }
}

pub trait Clock: Send + Sync + 'static {
    fn wall(&self) -> Time;
    fn mono(&self) -> Mono;
    /// Completes once `mono() >= at`. Dropping it cancels the timer.
    fn sleep_until(&self, at: Mono) -> BoxFuture<'static, ()>;
}
pub type SharedClock = Arc<dyn Clock>;

/// `clock.sleep_until(clock.mono().plus(d))`.
pub fn sleep(clock: &SharedClock, d: Duration) -> BoxFuture<'static, ()> {
    clock.sleep_until(clock.mono().plus(d))
}

#[derive(Debug, thiserror::Error)]
#[error("timed out")]
pub struct Elapsed;

/// Runs `fut` for at most `d` of the clock's monotonic time. Every timeout in circitor is this one (TST-4).
/// When both are ready in one poll, the value wins.
pub async fn timeout<F: Future>(
    clock: &SharedClock,
    d: Duration,
    fut: F,
) -> Result<F::Output, Elapsed> {
    let timer = sleep(clock, d);
    let fut = std::pin::pin!(fut);
    match futures::future::select(fut, timer).await {
        Either::Left((value, _)) => Ok(value),
        Either::Right(((), _)) => Err(Elapsed),
    }
}

#[derive(Default)]
struct Counts {
    busy: AtomicUsize,
    excused: AtomicUsize,
}

/// The count of work in flight (TST-4). Cheap; always compiled.
#[derive(Clone, Default)]
pub struct Activity(Arc<Counts>);

impl Activity {
    /// +1 until dropped.
    pub fn begin(&self) -> Busy {
        self.0.busy.fetch_add(1, Ordering::SeqCst);
        Busy(self.0.clone())
    }

    /// The work begun and not yet dropped, less what is excused.
    pub fn in_flight(&self) -> usize {
        let busy = self.0.busy.load(Ordering::SeqCst);
        let excused = self.0.excused.load(Ordering::SeqCst);
        busy.saturating_sub(excused)
    }
}

/// One piece of work in flight. `Drop`: -1.
pub struct Busy(Arc<Counts>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(any(test, feature = "test-endpoint"))]
impl Activity {
    /// One piece of work in flight is held by a test's script (a hung answer): don't count it until
    /// the guard is dropped (TST-4: "requests to GitHub not held by a scripted hang").
    pub fn excuse(&self) -> Excused {
        self.0.excused.fetch_add(1, Ordering::SeqCst);
        Excused(self.0.clone())
    }
}

/// An excuse for one piece of work in flight. `Drop`: it counts again.
#[cfg(any(test, feature = "test-endpoint"))]
pub struct Excused(Arc<Counts>);

#[cfg(any(test, feature = "test-endpoint"))]
impl Drop for Excused {
    fn drop(&mut self) {
        self.0.excused.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

    /// A clock whose timers fire at once, or never.
    struct Fixed {
        fires: bool,
    }
    impl Clock for Fixed {
        fn wall(&self) -> Time {
            Time::from_unix_millis(0)
        }
        fn mono(&self) -> Mono {
            Mono::ZERO
        }
        fn sleep_until(&self, _at: Mono) -> BoxFuture<'static, ()> {
            if self.fires {
                futures::future::ready(()).boxed()
            } else {
                futures::future::pending().boxed()
            }
        }
    }

    #[test]
    fn time_json_is_whole_seconds_utc() {
        let t = Time::parse("2026-11-24T22:09:23.456Z").unwrap();
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            "\"2026-11-24T22:09:23Z\""
        );
        assert_eq!(t.to_string(), "2026-11-24T22:09:23Z");
        assert_eq!(t.unix_millis() % 1000, 456);
        let offset: Time = serde_json::from_str("\"2026-11-24T23:09:23+01:00\"").unwrap();
        assert_eq!(offset.rfc3339(), "2026-11-24T22:09:23Z");
        let fraction: Time = serde_json::from_str("\"2026-11-24T22:09:23.9Z\"").unwrap();
        assert_eq!(fraction.rfc3339(), "2026-11-24T22:09:23Z");
        assert!(serde_json::from_str::<Time>("\"yesterday\"").is_err());
        let later = t.plus(Duration::from_secs(5));
        assert_eq!(later.since(t), Duration::from_secs(5));
        assert_eq!(t.since(later), Duration::ZERO);
        assert_eq!(later.minus(Duration::from_secs(5)), t);
    }

    #[test]
    fn mono_arithmetic_saturates() {
        let a = Mono::ZERO.plus(Duration::from_secs(3));
        assert_eq!(a.since(Mono::ZERO), Duration::from_secs(3));
        assert_eq!(Mono::ZERO.since(a), Duration::ZERO);
        assert_eq!(a.as_duration(), Duration::from_secs(3));
    }

    #[tokio::test]
    async fn clock_timeout_and_sleep() {
        let fires: SharedClock = Arc::new(Fixed { fires: true });
        let never: SharedClock = Arc::new(Fixed { fires: false });
        let r = timeout(
            &fires,
            Duration::from_secs(1),
            futures::future::pending::<u32>(),
        )
        .await;
        assert!(matches!(r, Err(Elapsed)));
        let r = timeout(&never, Duration::from_secs(1), async { 7 }).await;
        assert_eq!(r.unwrap(), 7);
        // Both ready at once: the value wins.
        let r = timeout(&fires, Duration::from_secs(1), async { 8 }).await;
        assert_eq!(r.unwrap(), 8);
        assert!(
            sleep(&fires, Duration::from_secs(1))
                .now_or_never()
                .is_some()
        );
        assert!(
            sleep(&never, Duration::from_secs(1))
                .now_or_never()
                .is_none()
        );
    }

    #[test]
    fn activity_counts_and_excuses() {
        let a = Activity::default();
        let one = a.begin();
        let two = a.begin();
        let excused = a.excuse();
        assert_eq!(a.in_flight(), 1);
        drop(excused);
        assert_eq!(a.in_flight(), 2);
        drop(one);
        assert_eq!(a.in_flight(), 1);
        drop(two);
        assert_eq!(a.in_flight(), 0);
    }
}
