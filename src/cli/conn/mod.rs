#![allow(unused_variables, dead_code)] // stub: T24 removes this
//! Connecting, spawning, replacing (CLI-7 to CLI-10; T24).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::api::Request;
use crate::cli::{CliEnv, Spawned, Spawner};
use crate::config::{Env, Paths};
use crate::transport::{Dropped, Head};

/// Exit 6 and its lines.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnError {
    pub code: i32,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The socket that answered, the daemon's version and mode.
pub struct Link {
    socket: PathBuf,
}

/// The real `Spawner` (T24).
pub struct RealSpawner;

#[async_trait::async_trait]
impl Spawner for RealSpawner {
    async fn spawn(&self, log: std::fs::File, env: &Env) -> std::io::Result<Box<dyn Spawned>> {
        unimplemented!("T24")
    }
}

/// CLI-7 and CLI-9: connect; with no daemon, start one; replace an older one. `budget` is what's left of a
/// wait's deadline, `None` for other commands. Warnings (a second socket, a newer daemon) go to `warn`.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn ensure(
    env: &CliEnv,
    paths: &Paths,
    no_spawn: bool,
    budget: Option<Duration>,
    warn: &mut String,
) -> Result<Link, ConnError> {
    unimplemented!("T24")
}

/// CLI-8: connect only. `Ok(None)`: not running.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn existing(
    env: &CliEnv,
    paths: &Paths,
    warn: &mut String,
) -> Result<Option<Link>, ConnError> {
    unimplemented!("T24")
}

/// CLI-10, for a wait that had its cursor: wait up to 8 s (within `budget`) for a daemon to answer; start none.
pub async fn reconnect(env: &CliEnv, paths: &Paths, budget: Duration) -> Result<Link, ConnError> {
    unimplemented!("T24")
}

impl Link {
    /// The one that answered (CLI-7): for CLI-6's lines.
    pub fn socket(&self) -> &Path {
        unimplemented!("T24")
    }

    /// CLI-9: an older daemon that was kept.
    pub fn older_unreplaced(&self) -> bool {
        unimplemented!("T24")
    }

    /// Any request but a wait: CLI-7's limit, and CLI-10's one retry as a new command.
    pub async fn request(&mut self, env: &CliEnv, req: &Request) -> Result<Reply, ConnError> {
        unimplemented!("T24")
    }

    /// A wait: sends and returns the head. No limit, no retry: T27 owns CLI-6's deadline and CLI-10's rule.
    pub async fn open(&self, req: &Request) -> Result<Head, Dropped> {
        unimplemented!("T24")
    }
}

/// CLI-8's `stop`, whole: ask, wait for the lock, `SIGTERM`, `SIGKILL`. `Ok(false)`: nothing was running.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn stop(env: &CliEnv, paths: &Paths, warn: &mut String) -> Result<bool, ConnError> {
    unimplemented!("T24")
}
