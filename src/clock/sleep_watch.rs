#![allow(unused_variables, dead_code)] // stub: T04 removes this

use std::sync::Arc;
use std::time::Duration;

use super::SharedClock;

/// POL-22's check. One per daemon; the engine calls `check` at every tick, the API at every client request.
pub struct SleepWatch {
    clock: SharedClock,
}

impl SleepWatch {
    pub fn new(clock: SharedClock) -> Arc<SleepWatch> {
        unimplemented!("T04")
    }

    /// How far wall and monotonic time moved since the last call. Wall ahead by more than 30 s is a
    /// sleep: returns its length (the gap) and adds it to `total`. A wall clock set back is never a sleep.
    pub fn check(&self) -> Option<Duration> {
        unimplemented!("T04")
    }

    /// All the sleep noticed since the daemon started.
    pub fn total(&self) -> Duration {
        unimplemented!("T04")
    }

    /// Changes whenever `total` does. Waiters re-check their deadline on it (API-12).
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Duration> {
        unimplemented!("T04")
    }
}
