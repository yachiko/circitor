#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! `FakePoller` (T43): a `PollerHandle` for route and waiter tests.
//!
//! Its defaults: tier active, no problem, first poll `Finished`, `refresh_pr` `Refreshed::NotSent`,
//! `read_through` and `track` a `GithubUnavailable` refusal, `poll_now` `Polled` at cost 0.

use std::sync::Arc;

use crate::clock::Mono;
use crate::model::{NodeId, PrName};
use crate::poller::{FirstPoll, PollReport, PollerHandle, PollerStatus, Refreshed, Refusal};

pub struct FakePoller {
    status: std::sync::Mutex<Option<PollerStatus>>,
}

impl FakePoller {
    pub fn new() -> Arc<FakePoller> {
        unimplemented!("T43")
    }

    pub fn set_status(&self, status: PollerStatus) {
        unimplemented!("T43")
    }

    pub fn set_first_poll(&self, first: FirstPoll) {
        unimplemented!("T43")
    }

    pub fn on_refresh(&self, f: impl FnMut(&NodeId) -> Refreshed + Send + 'static) {
        unimplemented!("T43")
    }

    /// The test's closure fills the real store through `reserve` and `apply`.
    pub fn on_read_through(
        &self,
        f: impl FnMut(&PrName) -> Result<NodeId, Refusal> + Send + 'static,
    ) {
        unimplemented!("T43")
    }

    pub fn on_track(&self, f: impl FnMut(&PrName) -> Result<NodeId, Refusal> + Send + 'static) {
        unimplemented!("T43")
    }

    pub fn on_poll(&self, f: impl FnMut() -> PollReport + Send + 'static) {
        unimplemented!("T43")
    }

    // ---- counters, one per call ----

    pub fn client_requests(&self) -> usize {
        unimplemented!("T43")
    }

    pub fn read_throughs(&self) -> usize {
        unimplemented!("T43")
    }

    pub fn tracks(&self) -> usize {
        unimplemented!("T43")
    }

    pub fn refreshes(&self) -> usize {
        unimplemented!("T43")
    }

    pub fn polls(&self) -> usize {
        unimplemented!("T43")
    }

    pub fn nudges(&self) -> usize {
        unimplemented!("T43")
    }
}

#[async_trait::async_trait]
impl PollerHandle for FakePoller {
    fn client_request(&self) {
        unimplemented!("T43")
    }

    fn idle_since(&self) -> Mono {
        unimplemented!("T43")
    }

    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal> {
        unimplemented!("T43")
    }

    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal> {
        unimplemented!("T43")
    }

    async fn refresh_pr(&self, id: &NodeId) -> Refreshed {
        unimplemented!("T43")
    }

    async fn poll_now(&self) -> PollReport {
        unimplemented!("T43")
    }

    fn nudge(&self) {
        unimplemented!("T43")
    }

    fn first_poll(&self) -> tokio::sync::watch::Receiver<FirstPoll> {
        unimplemented!("T43")
    }

    fn status(&self) -> PollerStatus {
        unimplemented!("T43")
    }
}
