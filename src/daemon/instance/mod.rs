#![allow(unused_variables, dead_code)] // stub: T18 removes this
//! The lock, the socket, the directories (STA-11, OPS-4; T18).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::api::StatusLite;
use crate::clock::SharedClock;
use crate::transport::SocketProbe;

#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub pid: u32,
    pub socket: PathBuf,
}

pub enum Acquire {
    Held(LockGuard),
    Busy(Option<Holder>),
}

/// STA-11: `flock(LOCK_EX | LOCK_NB)` on `path`, opened without truncating, tried every 100 ms of the
/// clock for up to `patience` (6 s; 15 s for a foreground daemon taking over). `Err`: the file can't be
/// opened or created; that's never retried.
pub async fn acquire(
    path: &Path,
    clock: &SharedClock,
    patience: Duration,
) -> std::io::Result<Acquire> {
    unimplemented!("T18")
}

/// STA-11.
pub const RETRY_EVERY: Duration = Duration::from_millis(100);
pub const PATIENCE: Duration = Duration::from_secs(6);
/// OPS-6.
pub const TAKEOVER_PATIENCE: Duration = Duration::from_secs(15);

/// The open descriptor. `Drop` releases the lock.
pub struct LockGuard {
    file: std::fs::File,
}

impl LockGuard {
    /// Truncates and writes `<pid> <socket path>\n`.
    pub fn write_holder(&self, pid: u32, socket: &Path) -> std::io::Result<()> {
        unimplemented!("T18")
    }

    /// Whether the path still is this descriptor's file (same device and inode).
    pub fn still_own(&self) -> bool {
        unimplemented!("T18")
    }
}

/// The holder a lock file names: one complete line, or `None`.
pub fn read_holder(path: &Path) -> Option<Holder> {
    unimplemented!("T18")
}

/// STA-11's test: take `LOCK_EX | LOCK_NB` on a descriptor of our own and let go at once.
pub fn is_held(path: &Path) -> std::io::Result<bool> {
    unimplemented!("T18")
}

#[derive(Debug, PartialEq)]
pub enum SocketFile {
    Absent,
    RemovedStale,
    Live(StatusLite),
    /// Also `Probe::Denied`.
    NotAnswering,
    NotASocket,
}

/// OPS-4, with the lock held: removes the file only if it's a socket that refuses a connection.
pub async fn clear_socket(path: &Path, probe: &dyn SocketProbe) -> std::io::Result<SocketFile> {
    unimplemented!("T18")
}

pub struct Bound {
    pub listener: tokio::net::UnixListener,
    /// Device and inode.
    id: (u64, u64),
}

/// Binds so that the socket's mode is 0600 (OPS-4): under a umask of **0177** (077 gives 0700 on Linux;
/// macOS isn't checked). Checks the path's length first.
pub fn bind(path: &Path) -> std::io::Result<Bound> {
    unimplemented!("T18")
}

impl Bound {
    pub fn still_own(&self) -> bool {
        unimplemented!("T18")
    }

    /// OPS-7, STA-11: only a socket it made.
    pub fn remove_if_own(&self) {
        unimplemented!("T18")
    }
}
