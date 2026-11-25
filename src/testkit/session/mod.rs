#![allow(unused_variables, dead_code)] // stub: T45 removes this
//! The fake session socket (TST-16; T45): a Unix stream socket, mode 0600, in a directory of mode
//! 0700, that takes lines and never answers. It shares no code with `src/notify/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::clock::{Mono, TestClock, Time};

/// One line as it arrived.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Which connection, from 0.
    pub conn: usize,
    pub at: Time,
    pub mono: Mono,
    pub text: String,
}

/// One connection read as a notice (NTF-6): its auth line's token if it sent one, and the message's `content`.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub at: Time,
    pub mono: Mono,
    pub token: Option<String>,
    pub text: String,
}

pub struct FakeSession {
    path: PathBuf,
}

impl FakeSession {
    /// Binds `<dir>/<name>.sock` and adds itself to the clock as a `SettleProbe`. `dir` is a short one (`ShortDir`).
    pub fn bind(dir: &Path, name: &str, clock: Arc<TestClock>) -> Arc<FakeSession> {
        unimplemented!("T45")
    }

    pub fn path(&self) -> &Path {
        unimplemented!("T45")
    }

    /// `MARKER_SESSION_TOKEN`.
    pub fn token(&self) -> &'static str {
        unimplemented!("T45")
    }

    /// Every line posted so far, in the order read, each with the clock's time when it was read. Reads what's pending first.
    pub fn lines(&self) -> Vec<Line> {
        unimplemented!("T45")
    }

    /// The same, one per connection. Panics, naming the line, on anything that isn't NTF-6's shape: an
    /// optional `{"type":"auth","token":…}` line, then exactly one `{"type":"user","message":{"role":"user","content":…}}` line.
    pub fn notices(&self) -> Vec<Notice> {
        unimplemented!("T45")
    }

    /// The listener is closed and its file left, so a connect is refused and a `stat` still sees the same socket.
    pub fn refuse(&self) {
        unimplemented!("T45")
    }

    /// The file is removed too.
    pub fn vanish(&self) {
        unimplemented!("T45")
    }

    /// Connections are accepted and nothing is ever read from them.
    pub fn accept_and_never_read(&self) {
        unimplemented!("T45")
    }

    /// It listens without accepting until its backlog is full, so a connect fails at once.
    pub fn queue_full(&self) {
        unimplemented!("T45")
    }

    /// Back to recording, on the same path. After `vanish` the socket is a new one: another inode.
    pub fn restore(&self) {
        unimplemented!("T45")
    }
}
