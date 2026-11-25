#![allow(unused_variables, dead_code)] // stub: T46 removes this
//! `RecordingPoster` (T46): a `Poster` that records every post.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::clock::{Activity, SharedClock, Time};

use super::{Looked, Posted, Poster, SessionToken};

/// One post asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub socket: PathBuf,
    pub token: Option<String>,
    pub text: String,
    pub at: Time,
}

/// `look` answers `Looked::Socket` with one fixed `SocketId` per path, until `gone(path)` (`Missing`),
/// `swapped(path)` (another `SocketId`) or `stat_fails(path)` (`Failed`).
pub struct RecordingPoster {
    clock: SharedClock,
}

impl RecordingPoster {
    pub fn new(clock: SharedClock) -> Arc<RecordingPoster> {
        unimplemented!("T46")
    }

    /// Every post asked for, in order, a stalled one included from the moment it's asked.
    pub fn posts(&self) -> Vec<Post> {
        unimplemented!("T46")
    }

    /// The answer to every post from now on.
    pub fn answer(&self, posted: Posted) {
        unimplemented!("T46")
    }

    /// The answer to the next post.
    pub fn answer_next(&self, posted: Posted) {
        unimplemented!("T46")
    }

    /// Every post waits until the `Release` is dropped, excused from the work in flight meanwhile.
    pub fn stall(&self, activity: &Activity) -> Release {
        unimplemented!("T46")
    }

    pub fn gone(&self, path: &Path) {
        unimplemented!("T46")
    }

    pub fn swapped(&self, path: &Path) {
        unimplemented!("T46")
    }

    pub fn stat_fails(&self, path: &Path) {
        unimplemented!("T46")
    }
}

#[async_trait::async_trait]
impl Poster for RecordingPoster {
    async fn look(&self, socket: &Path) -> Looked {
        unimplemented!("T46")
    }

    async fn post(&self, socket: &Path, token: Option<&SessionToken>, text: &str) -> Posted {
        unimplemented!("T46")
    }
}

/// Dropping it lets the stalled posts go on.
pub struct Release {
    inner: Option<tokio::sync::watch::Sender<bool>>,
}
