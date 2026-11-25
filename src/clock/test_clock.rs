#![allow(unused_variables, dead_code)] // stub: T04 removes this

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;

use super::{Activity, Clock, Mono, Time};

pub trait SettleProbe: Send + Sync {
    fn settled(&self) -> bool;
}

/// The test clock (TST-4).
pub struct TestClock {
    activity: Activity,
}

impl TestClock {
    /// Wall time starts at `start` (TST-5: 2026-11-24T22:00:00Z), monotonic time at zero.
    pub fn new(start: Time) -> Arc<TestClock> {
        unimplemented!("T04")
    }

    /// The daemon under test counts its work here; the clock treats it as a probe.
    pub fn activity(&self) -> Activity {
        unimplemented!("T04")
    }

    /// The fake GitHub adds itself.
    pub fn add_probe(&self, probe: Arc<dyn SettleProbe>) {
        unimplemented!("T04")
    }

    /// Settles first. Then moves both clocks by `d`: fires each timer at its own due time, in order (same
    /// instant: in the order they were set), and settles after each before time moves on (TST-4).
    pub async fn advance(&self, d: Duration) {
        unimplemented!("T04")
    }

    /// A suspend the monotonic clock doesn't count: wall time jumps by `d` in one step. Then settles.
    pub async fn sleep_wall_only(&self, d: Duration) {
        unimplemented!("T04")
    }

    /// A suspend the monotonic clock counts: both jump by `d` in one step; every timer that fell due
    /// fires afterwards, in the order of due time. Then settles.
    pub async fn sleep_both(&self, d: Duration) {
        unimplemented!("T04")
    }

    /// Sets wall time back by `d`, alone. Then settles.
    pub async fn set_wall_back(&self, d: Duration) {
        unimplemented!("T04")
    }

    /// Waits, in real time, until settled: `Activity::in_flight() == 0`, no timer fired but not yet
    /// polled, and every probe settled, on three checks in a row with a yield between them.
    pub async fn settle(&self) {
        unimplemented!("T04")
    }
}

impl Clock for TestClock {
    fn wall(&self) -> Time {
        unimplemented!("T04")
    }

    fn mono(&self) -> Mono {
        unimplemented!("T04")
    }

    fn sleep_until(&self, at: Mono) -> BoxFuture<'static, ()> {
        unimplemented!("T04")
    }
}
