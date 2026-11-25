//! The API's types, the server's shared parts, and a condition's one evaluation
//! (`plan/contracts/api.md`). T02's and final: T43 builds `request`, `params`, `respond` and
//! `router`; T44 `condition`; T22 `server`; T23 `wait`; T47 `notify`.

use std::sync::Arc;

use crate::clock::{Activity, SharedClock, SleepWatch, Time};
use crate::config::Config;
use crate::daemon::{Mode, StopHandle};
use crate::log::Logger;
use crate::notify::NotifyHandle;
use crate::poller::PollerHandle;
use crate::state::{Flush, Store};
use crate::status::Warnings;

pub mod condition;
pub mod notify;
mod params;
mod request;
mod respond;
mod router;
pub mod server;
pub mod types;
pub mod wait;

pub use params::{Params, bad_request, pr_from_path};
pub use respond::{Window, daemon_block, envelope, error, ok, ok_at, present, refusal, window};
#[cfg(any(test, feature = "test-endpoint"))]
pub use router::test_state;
pub use router::{InFlight, router};
pub use types::*;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub poller: Arc<dyn PollerHandle>,
    pub flush: Arc<dyn Flush>,
    /// `notify.md`; `NoNotify` in `test_state`.
    pub notify: Arc<dyn NotifyHandle>,
    pub clock: SharedClock,
    pub sleep: Arc<SleepWatch>,
    pub activity: Activity,
    pub logger: Logger,
    pub warnings: Warnings,
    pub config: Arc<Config>,
    pub info: Arc<DaemonInfo>,
    pub stop: StopHandle,
}

#[derive(Clone, Debug)]
pub struct DaemonInfo {
    pub version: String,
    pub mode: Mode,
    pub started_at: Time,
    pub paths: PathsInfo,
}
