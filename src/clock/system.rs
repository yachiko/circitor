#![allow(unused_variables, dead_code)] // stub: T04 removes this

use std::sync::Arc;
use std::time::Instant;

use futures::future::BoxFuture;

use super::{Clock, Mono, Time};

/// The real clock: tokio timers inside.
pub struct SystemClock {
    /// The `Instant` it was made at.
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Arc<SystemClock> {
        unimplemented!("T04")
    }
}

impl Clock for SystemClock {
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
