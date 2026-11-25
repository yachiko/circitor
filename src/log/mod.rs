#![allow(unused_variables, dead_code)] // stub: T04 removes this
//! Logging (`plan/contracts/log.md`). T02 declares; T04 builds.

use std::sync::{Arc, Mutex};

use crate::clock::{SharedClock, Time};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

pub trait LogSink: Send + Sync + 'static {
    /// One complete line, without its newline. An error means it wasn't written.
    fn write_line(&self, line: &str) -> std::io::Result<()>;
}

/// A foreground daemon, and a spawned one (its stderr is the log file, CLI-7).
pub struct StderrSink;

impl LogSink for StderrSink {
    fn write_line(&self, line: &str) -> std::io::Result<()> {
        unimplemented!("T04")
    }
}

/// The test sink.
#[derive(Clone, Default)]
pub struct BufferSink(Arc<Mutex<String>>);

impl BufferSink {
    pub fn contents(&self) -> String {
        unimplemented!("T04")
    }
}

impl LogSink for BufferSink {
    fn write_line(&self, line: &str) -> std::io::Result<()> {
        unimplemented!("T04")
    }
}

#[derive(Clone)]
pub struct Logger {
    clock: SharedClock,
    sink: Arc<dyn LogSink>,
    level: Level,
}

impl Logger {
    pub fn new(clock: SharedClock, sink: Arc<dyn LogSink>, level: Level) -> Logger {
        unimplemented!("T04")
    }

    /// `CIRCITOR_LOG` (OPS-9): unset is `Info`; an unknown value is `Info` and the name to warn about.
    pub fn level_from_env(value: Option<&str>) -> (Level, Option<String>) {
        unimplemented!("T04")
    }

    pub fn error(&self, msg: impl std::fmt::Display) {
        unimplemented!("T04")
    }

    pub fn warn(&self, msg: impl std::fmt::Display) {
        unimplemented!("T04")
    }

    pub fn info(&self, msg: impl std::fmt::Display) {
        unimplemented!("T04")
    }

    pub fn debug(&self, msg: impl std::fmt::Display) {
        unimplemented!("T04")
    }

    pub fn enabled(&self, level: Level) -> bool {
        unimplemented!("T04")
    }

    /// Since when lines can't be written; `None` while they can (OPS-9: logging never stops the daemon).
    pub fn failing_since(&self) -> Option<Time> {
        unimplemented!("T04")
    }
}
