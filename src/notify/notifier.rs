#![allow(unused_variables, dead_code)] // stub: T46 removes this
//! The registry and the loop that evaluates it (T46).

use std::path::Path;
use std::sync::Arc;

use crate::api::Ask;
use crate::model::{NodeId, PrName};

use super::{Looked, NewSubscription, NotifyDeps, NotifyHandle, RegisterError, Registered};

/// The registry, the loop's task, the notices on their way.
pub struct Notifier {
    deps: NotifyDeps,
}

impl Notifier {
    /// Starts the loop. It starts with no subscription: none is ever saved or loaded (NTF-9).
    pub fn spawn(deps: NotifyDeps) -> Arc<Notifier> {
        unimplemented!("T46")
    }

    /// OPS-7's hook (NTF-10): takes every subscription out at once, posts to all of them together, and
    /// returns when every post has ended or `STOP_LIMIT` has passed, whichever is first. `asked` is false
    /// for a stop the daemon chose itself (`StopCause::NotOwnFiles`, STA-11): its notices' command then has
    /// no `--no-spawn`. Nothing is tried again. Those not written by then are dropped and logged in one
    /// line, also when this future is dropped.
    pub async fn stop(&self, asked: bool) {
        unimplemented!("T46")
    }

    /// `Running::crash` (TST-5): ends the loop and drops everything. Posts nothing, logs nothing (NTF-10).
    pub fn abort(&self) {
        unimplemented!("T46")
    }
}

#[async_trait::async_trait]
impl NotifyHandle for Notifier {
    async fn look(&self, socket: &Path) -> Looked {
        unimplemented!("T46")
    }

    fn drop_same(&self, id: &NodeId, socket: &Path, ask: &Ask) -> bool {
        unimplemented!("T46")
    }

    async fn register(&self, new: NewSubscription) -> Result<Registered, RegisterError> {
        unimplemented!("T46")
    }

    fn cancel(&self, name: &PrName, id: Option<&NodeId>, socket: &Path) -> u32 {
        unimplemented!("T46")
    }

    fn count(&self) -> u32 {
        unimplemented!("T46")
    }
}
