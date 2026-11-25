#![allow(unused_variables, dead_code)] // stub: T28 removes this
//! Start, stop, wiring (T28).

use std::path::{Path, PathBuf};
#[cfg(any(test, feature = "test-endpoint"))]
use std::sync::Arc;

use crate::daemon::{Deps, StopCause};
#[cfg(any(test, feature = "test-endpoint"))]
use crate::notify::Notifier;

/// Why it didn't start. `Display` is the one line for stderr; `code()` the exit status (0 only for
/// a spawned daemon whose lock's holder answers, STA-11). T28 fills it.
#[derive(Debug, thiserror::Error)]
pub enum StartError {}

impl StartError {
    pub fn code(&self) -> i32 {
        match *self {}
    }
}

pub struct Running {
    socket: PathBuf,
}

impl Running {
    pub fn socket(&self) -> &Path {
        unimplemented!("T28")
    }

    pub fn stop(&self, cause: StopCause) {
        unimplemented!("T28")
    }

    /// How many client connections are open now (the harness waits on it after a client goes away).
    pub fn open_connections(&self) -> usize {
        unimplemented!("T28")
    }

    /// The notifier, for T28's own tests: they need a subscription, and T28 is built before the notify route (T47).
    #[cfg(any(test, feature = "test-endpoint"))]
    pub fn notifier(&self) -> Arc<Notifier> {
        unimplemented!("T28")
    }

    /// `Some(status)` once it has stopped by itself or been stopped; never waits.
    pub fn exit_status(&self) -> Option<i32> {
        unimplemented!("T28")
    }

    /// Resolves with the exit status when the daemon has stopped (OPS-7, OPS-8): 0 for every stop.
    pub async fn exited(&mut self) -> i32 {
        unimplemented!("T28")
    }

    /// TST-5's crash: everything is dropped at once, without OPS-7's stop. The socket file and
    /// `state.json` stay as they are; the lock is released, as a dead process's is.
    pub async fn crash(self) {
        unimplemented!("T28")
    }
}

/// STA-11's order: the directories, the lock, the lock file's line, a stale socket, `state.json`, the
/// socket, and only then the engine (which reads the token). The notifier is spawned after the engine (it reads
/// `PollerHandle::status`) and before the accept loop; it starts empty (NTF-9). Returns once the socket is bound and serving.
pub async fn start(deps: Deps) -> Result<Running, StartError> {
    unimplemented!("T28")
}

/// `circitor daemon [--spawned]`: real `Deps` (config, paths, `SystemClock`, `github::client`, `token::gh`,
/// stderr), a runtime, `start`, then waits for the end. Returns the exit status.
pub fn main(spawned: bool) -> i32 {
    unimplemented!("T28")
}
