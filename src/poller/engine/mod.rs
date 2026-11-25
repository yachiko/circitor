#![allow(unused_variables, dead_code)] // stub: T21 removes this
//! The engine: the loop, the gate, the poll (T21), its demand side (T25), a PR's errors (T42).

use std::sync::Arc;

use crate::clock::{Activity, Mono, SharedClock, SleepWatch};
use crate::config::Config;
use crate::github::GitHub;
use crate::log::Logger;
use crate::model::{NodeId, PrName};
use crate::poller::{FirstPoll, PollReport, PollerHandle, PollerStatus, Refreshed, Refusal};
use crate::state::Store;
use crate::status::Warnings;
use crate::token::TokenSource;

pub struct EngineDeps {
    pub clock: SharedClock,
    pub github: Arc<dyn GitHub>,
    pub token_source: Arc<dyn TokenSource>,
    pub store: Arc<Store>,
    pub config: Arc<Config>,
    pub logger: Logger,
    pub warnings: Warnings,
    pub activity: Activity,
    pub sleep: Arc<SleepWatch>,
}

pub struct Engine {
    deps: EngineDeps,
}

impl Engine {
    /// Starts the loop: the first token read, then the first poll (STA-11's order is T28's: this is called after the socket is bound).
    pub fn spawn(deps: EngineDeps) -> Arc<Engine> {
        unimplemented!("T21")
    }

    /// OPS-7: stops polling, drops a request in flight, kills a running `gh`. Returns when the loop has ended.
    pub async fn stop(&self) {
        unimplemented!("T21")
    }
}

#[async_trait::async_trait]
impl PollerHandle for Engine {
    fn client_request(&self) {
        unimplemented!("T21")
    }

    fn idle_since(&self) -> Mono {
        unimplemented!("T21")
    }

    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal> {
        unimplemented!("T21")
    }

    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal> {
        unimplemented!("T21")
    }

    async fn refresh_pr(&self, id: &NodeId) -> Refreshed {
        unimplemented!("T21")
    }

    async fn poll_now(&self) -> PollReport {
        unimplemented!("T21")
    }

    fn nudge(&self) {
        unimplemented!("T21")
    }

    fn first_poll(&self) -> tokio::sync::watch::Receiver<FirstPoll> {
        unimplemented!("T21")
    }

    fn status(&self) -> PollerStatus {
        unimplemented!("T21")
    }
}
