//! Start, stop, wiring (T28): STA-11's order at start, serving the socket, OPS-6's two modes, OPS-7's
//! stop with NTF-10's hook, OPS-8's idle exit and STA-11's minute's check.
//!
//! `start` brings a daemon up and returns a `Running`. Every stop goes through the one `StopHandle`
//! (signals, `POST /v1/shutdown`, the idle exit, the minute's check, `Running::stop`); one task, the
//! life, waits for it and runs OPS-7's sequence once.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

use crate::config::DirError;
use crate::daemon::{StopCause, StopHandle};
use crate::notify::Notifier;
use crate::poller::Engine;
use crate::state::Saver;

mod entry;
mod life;
mod serve;
mod start;
#[cfg(test)]
mod tests;

pub use entry::main;
pub use start::start;

use serve::Conns;

/// Why it didn't start. `Display` is the one line for stderr; `code()` the exit status (0 only for
/// a spawned daemon whose lock's holder answers, STA-11).
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// OPS-3: a directory that can't be made private.
    #[error("{0}")]
    Dir(#[from] DirError),
    /// STA-11: the lock file can't be opened or created; never retried.
    #[error("{}: the lock file can't be opened: {source}", path.display())]
    LockOpen {
        path: PathBuf,
        source: std::io::Error,
    },
    /// STA-11: a spawned daemon whose lock's holder answers on its socket. Status 0.
    #[error("a daemon already answers on {}", .0.display())]
    AlreadyRunning(PathBuf),
    /// STA-11: the lock is held by a daemon that doesn't answer on its socket.
    #[error("the lock is held by a daemon that doesn't answer on {}", .0.display())]
    HolderNotAnswering(PathBuf),
    /// STA-11: the lock is held and its file holds no complete line, so no socket.
    #[error("{}: the lock is held, and the file names no socket", .0.display())]
    NoHolder(PathBuf),
    /// OPS-6: a foreground holder is left alone.
    #[error("a foreground daemon already runs on {}; it's left alone", .0.display())]
    ForegroundHolder(PathBuf),
    /// OPS-6: no lock after 15 s of taking over.
    #[error("the daemon on {} didn't give up the lock within 15 s", .0.display())]
    TakeoverTimedOut(PathBuf),
    /// STA-11: the holder line can't be written.
    #[error("{}: the lock file can't be written: {source}", path.display())]
    WriteHolder {
        path: PathBuf,
        source: std::io::Error,
    },
    /// OPS-4: something that isn't a socket is at the socket path.
    #[error("{}: is not a socket; remove it, or set CIRCITOR_SOCKET to another path", .0.display())]
    NotASocket(PathBuf),
    /// OPS-4: a socket is there that doesn't answer and doesn't refuse.
    #[error("{}: a socket is there that doesn't answer; remove it, or set CIRCITOR_SOCKET to another path", .0.display())]
    SocketNotAnswering(PathBuf),
    /// OPS-4: the socket path can't be looked at, or a stale socket can't be removed.
    #[error("{}: the socket path can't be checked: {source}", path.display())]
    SocketCheck {
        path: PathBuf,
        source: std::io::Error,
    },
    /// OPS-4: the bind failed.
    #[error("{}: the socket can't be bound: {source}", path.display())]
    Bind {
        path: PathBuf,
        source: std::io::Error,
    },
    /// OPS-7: the handlers for SIGTERM, SIGINT and SIGHUP can't be installed.
    #[error("the signal handlers can't be installed: {0}")]
    Signals(std::io::Error),
}

impl StartError {
    pub fn code(&self) -> i32 {
        match self {
            // STA-11: a spawned daemon exits 0 when the holder answers.
            StartError::AlreadyRunning(_) => 0,
            _ => 1,
        }
    }
}

/// The daemon's tasks besides the connections; each is taken out once, by the stop or by `crash`.
#[derive(Default)]
struct Tasks {
    life: Option<JoinHandle<()>>,
    accept: Option<JoinHandle<()>>,
    idle: Option<JoinHandle<()>>,
    minute: Option<JoinHandle<()>>,
    signals: Option<JoinHandle<()>>,
}

/// A std lock, taken and released inside one function; a poisoned one still holds its value.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Asks for a stop from outside the daemon's own loops (a signal, `Running::stop`): a second request
/// during the stop ends it at once (OPS-7).
fn ask(stop: &StopHandle, second: &Notify, cause: StopCause) {
    stop.request(cause);
    if stop.times() >= 2 {
        second.notify_one();
    }
}

pub struct Running {
    socket: PathBuf,
    stop: StopHandle,
    /// Woken by a second request during the stop (OPS-7).
    second: Arc<Notify>,
    conns: Arc<Conns>,
    notifier: Arc<Notifier>,
    engine: Arc<Engine>,
    saver: Arc<Saver>,
    status: watch::Receiver<Option<i32>>,
    tasks: Arc<Mutex<Tasks>>,
}

impl Running {
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn stop(&self, cause: StopCause) {
        ask(&self.stop, &self.second, cause);
    }

    /// How many client connections are open now (the harness waits on it after a client goes away).
    pub fn open_connections(&self) -> usize {
        self.conns.open()
    }

    /// The notifier, for T28's own tests: they need a subscription, and T28 is built before the notify route (T47).
    #[cfg(any(test, feature = "test-endpoint"))]
    pub fn notifier(&self) -> Arc<Notifier> {
        self.notifier.clone()
    }

    /// `Some(status)` once it has stopped by itself or been stopped; never waits.
    pub fn exit_status(&self) -> Option<i32> {
        *self.status.borrow()
    }

    /// Resolves with the exit status when the daemon has stopped (OPS-7, OPS-8): 0 for every stop.
    pub async fn exited(&mut self) -> i32 {
        loop {
            if let Some(status) = *self.status.borrow_and_update() {
                return status;
            }
            if self.status.changed().await.is_err() {
                // OPS-7: the life ended without a status: it panicked, which is a crash.
                return self.status.borrow().unwrap_or(1);
            }
        }
    }

    /// TST-5's crash: everything is dropped at once, without OPS-7's stop. The socket file and
    /// `state.json` stay as they are; the lock is released, as a dead process's is.
    pub async fn crash(self) {
        // NTF-10: a crash tells nobody; TST-5: and writes nothing.
        self.notifier.abort();
        self.saver.abort();
        let handles = {
            let mut t = lock(&self.tasks);
            [
                t.life.take(),
                t.accept.take(),
                t.idle.take(),
                t.minute.take(),
                t.signals.take(),
            ]
        };
        // Awaited, so the lock and the listener (its file left in place) are dropped when this returns.
        for handle in handles.into_iter().flatten() {
            handle.abort();
            let _ = handle.await;
        }
        self.conns.abort_all();
        self.engine.stop().await;
    }
}
