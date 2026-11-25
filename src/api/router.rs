#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! The router, its layer and fallbacks, and the routes' shared test state (T43).

#[cfg(any(test, feature = "test-endpoint"))]
use std::sync::Arc;

#[cfg(any(test, feature = "test-endpoint"))]
use crate::clock::TestClock;
#[cfg(any(test, feature = "test-endpoint"))]
use crate::poller::fake::FakePoller;

use super::AppState;

/// `server::routes()` merged with `wait::routes()` and `notify::routes()`, the `unknown_route` fallback, and the layer below.
pub fn router(state: AppState) -> axum::Router {
    unimplemented!("T43")
}

/// Put on every request by `router`'s layer: the request's share of the work in flight (TST-4).
#[derive(Clone)]
pub struct InFlight(());

impl InFlight {
    /// The request now waits on the clock or on the poller: not work in flight.
    pub fn park(&self) {
        unimplemented!("T43")
    }

    /// It woke: work again, until dropped.
    pub fn resume(&self) {
        unimplemented!("T43")
    }
}

/// An `AppState` over a real, empty `Store`, `FakePoller`, `NoFlush` and `NoNotify`; its `activity` is
/// the test clock's, so `settle` sees a handler's work.
#[cfg(any(test, feature = "test-endpoint"))]
pub fn test_state(clock: Arc<TestClock>) -> (AppState, Arc<FakePoller>) {
    unimplemented!("T43")
}
