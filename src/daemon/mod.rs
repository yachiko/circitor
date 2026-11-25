//! The daemon (`plan/contracts/daemon.md`). T02's and final: T18 builds `instance`, T28 `run`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::clock::{Activity, SharedClock};
use crate::config::{Config, Paths};
use crate::github::GitHub;
use crate::log::{Level, LogSink};
use crate::notify::Poster;
use crate::state::StateFs;
use crate::token::TokenSource;
use crate::transport::SocketProbe;

pub mod instance;
pub mod run;

pub use run::{Running, StartError, start};

/// OPS-6; API-5's `mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Spawned,
    Foreground,
}

/// Why the daemon stops (OPS-9 logs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopCause {
    Signal(i32),
    Request,
    Idle,
    NotOwnFiles,
    Test,
}

struct StopInner {
    cause: tokio::sync::watch::Sender<Option<StopCause>>,
    times: AtomicU32,
}

/// The one way anything asks the daemon to stop (OPS-7, API-17).
#[derive(Clone)]
pub struct StopHandle(Arc<StopInner>);

impl StopHandle {
    pub fn new() -> StopHandle {
        let (cause, _) = tokio::sync::watch::channel(None);
        StopHandle(Arc::new(StopInner {
            cause,
            times: AtomicU32::new(0),
        }))
    }

    /// The first cause wins; a second call is the "second signal" of OPS-7.
    pub fn request(&self, cause: StopCause) {
        self.0.times.fetch_add(1, Ordering::SeqCst);
        self.0.cause.send_if_modified(|held| {
            if held.is_none() {
                *held = Some(cause);
                true
            } else {
                false
            }
        });
    }

    pub fn requested(&self) -> Option<StopCause> {
        *self.0.cause.borrow()
    }

    /// Resolves with the first cause, once one is requested.
    pub async fn wait(&self) -> StopCause {
        let mut rx = self.0.cause.subscribe();
        loop {
            if let Some(cause) = *rx.borrow_and_update() {
                return cause;
            }
            // The sender lives as long as `self`: `changed` can't fail while this runs.
            if rx.changed().await.is_err() {
                return futures::future::pending().await;
            }
        }
    }

    /// How often it was asked (OPS-7: a second SIGTERM or SIGINT).
    pub fn times(&self) -> u32 {
        self.0.times.load(Ordering::SeqCst)
    }
}

impl Default for StopHandle {
    fn default() -> StopHandle {
        StopHandle::new()
    }
}

/// Everything from outside (TST-5).
pub struct Deps {
    pub clock: SharedClock,
    pub activity: Activity,
    pub github: Arc<dyn GitHub>,
    pub token_source: Arc<dyn TokenSource>,
    pub fs: Arc<dyn StateFs>,
    pub probe: Arc<dyn SocketProbe>,
    /// `notify.md`: `RealPoster` for real and in the harness, `RecordingPoster` in T28's tests.
    pub poster: Arc<dyn Poster>,
    pub paths: Paths,
    pub config: Config,
    pub mode: Mode,
    pub version: String,
    pub log: Arc<dyn LogSink>,
    pub level: Level,
    pub seed: u64,
    /// Install handlers for SIGTERM, SIGINT, SIGHUP; `false` in-process.
    pub signals: bool,
    /// `CIRCITOR_TEST_STOP_DELAY` (TST-1).
    pub stop_delay: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

    #[tokio::test]
    async fn stop_handle_first_cause_wins_and_counts() {
        let stop = StopHandle::new();
        assert_eq!(stop.requested(), None);
        assert_eq!(stop.times(), 0);
        assert!(stop.wait().now_or_never().is_none());
        let waiting = stop.clone();
        let waiter = tokio::spawn(async move { waiting.wait().await });
        stop.request(StopCause::Signal(15));
        stop.request(StopCause::Request);
        assert_eq!(stop.requested(), Some(StopCause::Signal(15)));
        assert_eq!(stop.times(), 2);
        assert_eq!(waiter.await.unwrap(), StopCause::Signal(15));
        assert_eq!(stop.wait().await, StopCause::Signal(15));
        assert_eq!(StopHandle::default().requested(), None);
    }

    #[test]
    fn mode_words() {
        assert_eq!(
            serde_json::to_string(&Mode::Spawned).unwrap(),
            "\"spawned\""
        );
        assert_eq!(
            serde_json::to_string(&Mode::Foreground).unwrap(),
            "\"foreground\""
        );
    }
}
