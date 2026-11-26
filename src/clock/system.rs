use std::sync::Arc;
use std::time::Instant;

use futures::FutureExt;
use futures::future::BoxFuture;

use super::{Clock, Mono, Time};

/// The real clock: tokio timers inside.
// TST-2: this file and the test clock are the only places that read the system's time.
pub struct SystemClock {
    /// The `Instant` it was made at.
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Arc<SystemClock> {
        Arc::new(SystemClock {
            start: Instant::now(),
        })
    }
}

fn mono_since(start: Instant) -> Mono {
    // `Instant` never goes back, so neither does this.
    Mono(start.elapsed())
}

impl Clock for SystemClock {
    fn wall(&self) -> Time {
        Time::from_unix_millis(chrono::Utc::now().timestamp_millis())
    }

    fn mono(&self) -> Mono {
        mono_since(self.start)
    }

    fn sleep_until(&self, at: Mono) -> BoxFuture<'static, ()> {
        let start = self.start;
        async move {
            let left = at.since(mono_since(start));
            if !left.is_zero() {
                tokio::time::sleep(left).await;
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn system_mono_never_goes_back() {
        let clock = SystemClock::new();
        let mut last = clock.mono();
        for _ in 0..1000 {
            let now = clock.mono();
            assert!(now >= last);
            last = now;
        }
    }

    #[test]
    fn system_wall_is_now_ish() {
        let clock = SystemClock::new();
        // After 2026-01-01 and before 2100-01-01: a wall time, not a monotonic one.
        let ms = clock.wall().unix_millis();
        assert!(ms > 1_767_225_600_000 && ms < 4_102_444_800_000);
    }

    #[test]
    fn system_a_time_already_reached_is_ready_at_once() {
        let clock = SystemClock::new();
        assert!(clock.sleep_until(Mono::ZERO).now_or_never().is_some());
        let now = clock.mono();
        assert!(clock.sleep_until(now).now_or_never().is_some());
        // A time far ahead isn't ready: polled outside a runtime it would panic in tokio, so only
        // check it inside one.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            let far = clock.mono().plus(Duration::from_secs(3600));
            assert!(clock.sleep_until(far).now_or_never().is_none());
        });
    }
}
