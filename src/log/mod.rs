//! Logging (`plan/contracts/log.md`). T02 declared; T04 built.
//!
//! OPS-9: one line per event, `<time> <level> <message>`. There is no global logger: each daemon has
//! its own. The logger takes only text a caller built: no method takes a record, a response or a
//! token, so what's never logged stays out by construction as far as a logger can keep it out.

use std::fmt::Display;
use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::{SharedClock, Time};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    /// The word a line carries (OPS-9).
    fn word(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
        }
    }
}

pub trait LogSink: Send + Sync + 'static {
    /// One complete line, without its newline. An error means it wasn't written.
    fn write_line(&self, line: &str) -> std::io::Result<()>;
}

/// A foreground daemon, and a spawned one (its stderr is the log file, CLI-7).
pub struct StderrSink;

impl LogSink for StderrSink {
    fn write_line(&self, line: &str) -> std::io::Result<()> {
        // One write per line, so lines from two threads don't interleave.
        let mut buf = String::with_capacity(line.len() + 1);
        buf.push_str(line);
        buf.push('\n');
        let mut err = std::io::stderr().lock();
        err.write_all(buf.as_bytes())?;
        err.flush()
    }
}

/// The test sink.
#[derive(Clone, Default)]
pub struct BufferSink(Arc<Mutex<String>>);

impl BufferSink {
    fn lock(&self) -> MutexGuard<'_, String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn contents(&self) -> String {
        self.lock().clone()
    }
}

impl LogSink for BufferSink {
    fn write_line(&self, line: &str) -> std::io::Result<()> {
        let mut buf = self.lock();
        buf.push_str(line);
        buf.push('\n');
        Ok(())
    }
}

#[derive(Clone)]
pub struct Logger {
    clock: SharedClock,
    sink: Arc<dyn LogSink>,
    level: Level,
    /// The wall time of the first line refused since the last one written. Shared by clones.
    failing: Arc<Mutex<Option<Time>>>,
}

impl Logger {
    pub fn new(clock: SharedClock, sink: Arc<dyn LogSink>, level: Level) -> Logger {
        Logger {
            clock,
            sink,
            level,
            failing: Arc::default(),
        }
    }

    /// `CIRCITOR_LOG` (OPS-9): unset is `Info`; an unknown value is `Info` and the name to warn about.
    // OPS-9: unset or empty is `info`; only the four words in lower case name a level.
    pub fn level_from_env(value: Option<&str>) -> (Level, Option<String>) {
        match value {
            None | Some("") => (Level::Info, None),
            Some("error") => (Level::Error, None),
            Some("warn") => (Level::Warn, None),
            Some("info") => (Level::Info, None),
            Some("debug") => (Level::Debug, None),
            Some(other) => (Level::Info, Some(other.to_string())),
        }
    }

    pub fn error(&self, msg: impl Display) {
        self.log(Level::Error, msg);
    }

    pub fn warn(&self, msg: impl Display) {
        self.log(Level::Warn, msg);
    }

    pub fn info(&self, msg: impl Display) {
        self.log(Level::Info, msg);
    }

    pub fn debug(&self, msg: impl Display) {
        self.log(Level::Debug, msg);
    }

    pub fn enabled(&self, level: Level) -> bool {
        level <= self.level
    }

    /// Since when lines can't be written; `None` while they can (OPS-9: logging never stops the daemon).
    pub fn failing_since(&self) -> Option<Time> {
        *self.failing_lock()
    }

    fn failing_lock(&self) -> MutexGuard<'_, Option<Time>> {
        self.failing.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn log(&self, level: Level, msg: impl Display) {
        // OPS-9: the level is checked before the message is formatted.
        if !self.enabled(level) {
            return;
        }
        let now = self.clock.wall();
        // OPS-9: one event, one line.
        let text = msg.to_string().replace(['\n', '\r'], " ");
        let line = format!("{} {} {}", now.rfc3339(), level.word(), text);
        // OPS-9: logging never stops the daemon. A refused line is dropped; T22 shows `log_unwritable`
        // while `failing_since` is set.
        match self.sink.write_line(&line) {
            Ok(()) => *self.failing_lock() = None,
            Err(_) => {
                let mut failing = self.failing_lock();
                if failing.is_none() {
                    *failing = Some(now);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, TestClock};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    fn clock() -> Arc<TestClock> {
        TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap())
    }

    fn logger(clock: &Arc<TestClock>, level: Level) -> (Logger, BufferSink) {
        let sink = BufferSink::default();
        (
            Logger::new(clock.clone(), Arc::new(sink.clone()), level),
            sink,
        )
    }

    // OPS-9: `<time> <level> <message>`, RFC 3339 UTC.
    #[tokio::test]
    async fn ops9_line_format() {
        let c = clock();
        let (log, sink) = logger(&c, Level::Info);
        log.info("started");
        assert_eq!(sink.contents(), "2026-11-24T22:00:00Z info started\n");
    }

    // OPS-9: the level word `error`.
    #[tokio::test]
    async fn ops9_line_format_error() {
        let c = clock();
        let (log, sink) = logger(&c, Level::Error);
        log.error("boom");
        assert_eq!(sink.contents(), "2026-11-24T22:00:00Z error boom\n");
    }

    // OPS-9: the level word `warn`.
    #[tokio::test]
    async fn ops9_line_format_warn() {
        let c = clock();
        let (log, sink) = logger(&c, Level::Warn);
        log.warn(format_args!("problem begin code={}", "rate_limited"));
        assert_eq!(
            sink.contents(),
            "2026-11-24T22:00:00Z warn problem begin code=rate_limited\n"
        );
    }

    // OPS-9: the level word `debug`, and the time moving with the clock.
    #[tokio::test]
    async fn ops9_line_format_debug() {
        let c = clock();
        let (log, sink) = logger(&c, Level::Debug);
        c.advance(Duration::from_secs(83)).await;
        log.debug("request kind=search cost=1");
        assert_eq!(
            sink.contents(),
            "2026-11-24T22:01:23Z debug request kind=search cost=1\n"
        );
    }

    // OPS-9: a line is written when its level is at or below the logger's.
    #[tokio::test]
    async fn ops9_levels_filter() {
        let c = clock();
        let (log, sink) = logger(&c, Level::Warn);
        log.error("e");
        log.warn("w");
        log.info("i");
        log.debug("d");
        assert_eq!(
            sink.contents(),
            "2026-11-24T22:00:00Z error e\n2026-11-24T22:00:00Z warn w\n"
        );
        assert!(log.enabled(Level::Warn) && !log.enabled(Level::Info));
    }

    struct Watched<'a>(&'a AtomicBool);
    impl Display for Watched<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.store(true, Ordering::SeqCst);
            f.write_str("formatted")
        }
    }

    // OPS-9: the level is checked before the message is formatted.
    #[tokio::test]
    async fn ops9_level_is_checked_before_formatting() {
        let c = clock();
        let (log, sink) = logger(&c, Level::Info);
        let formatted = AtomicBool::new(false);
        log.debug(Watched(&formatted));
        assert!(!formatted.load(Ordering::SeqCst));
        assert_eq!(sink.contents(), "");
        log.info(Watched(&formatted));
        assert!(formatted.load(Ordering::SeqCst));
    }

    // OPS-9: `CIRCITOR_LOG`: unset is info; an unknown value is info and handed back for a warning.
    #[test]
    fn ops9_level_from_env() {
        assert_eq!(Logger::level_from_env(None), (Level::Info, None));
        assert_eq!(Logger::level_from_env(Some("")), (Level::Info, None));
        assert_eq!(Logger::level_from_env(Some("debug")), (Level::Debug, None));
        assert_eq!(Logger::level_from_env(Some("error")), (Level::Error, None));
        assert_eq!(Logger::level_from_env(Some("warn")), (Level::Warn, None));
        assert_eq!(Logger::level_from_env(Some("info")), (Level::Info, None));
        assert_eq!(
            Logger::level_from_env(Some("loud")),
            (Level::Info, Some("loud".to_string()))
        );
        assert_eq!(
            Logger::level_from_env(Some("DEBUG")),
            (Level::Info, Some("DEBUG".to_string()))
        );
    }

    // OPS-9: one event, one line.
    #[tokio::test]
    async fn ops9_a_message_with_newlines_is_one_line() {
        let c = clock();
        let (log, sink) = logger(&c, Level::Info);
        log.info("one\ntwo\r\nthree\rfour");
        assert_eq!(
            sink.contents(),
            "2026-11-24T22:00:00Z info one two  three four\n"
        );
        assert_eq!(sink.contents().lines().count(), 1);
    }

    struct Flaky {
        fail: AtomicBool,
        inner: BufferSink,
    }
    impl LogSink for Flaky {
        fn write_line(&self, line: &str) -> std::io::Result<()> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("disk full"));
            }
            self.inner.write_line(line)
        }
    }

    // OPS-9: logging never stops the daemon; `failing_since` says since when lines are dropped.
    #[tokio::test]
    async fn ops9_a_failing_sink_never_stops_logging() {
        let c = clock();
        let sink = Arc::new(Flaky {
            fail: AtomicBool::new(true),
            inner: BufferSink::default(),
        });
        let log = Logger::new(c.clone(), sink.clone(), Level::Info);
        let clone = log.clone();
        assert_eq!(log.failing_since(), None);
        log.info("first");
        let first = c.wall();
        assert_eq!(log.failing_since(), Some(first));
        c.advance(Duration::from_secs(5)).await;
        log.warn("second");
        clone.error("third");
        assert_eq!(log.failing_since(), Some(first));
        assert_eq!(clone.failing_since(), Some(first));
        assert_eq!(sink.inner.contents(), "");
        sink.fail.store(false, Ordering::SeqCst);
        c.advance(Duration::from_secs(5)).await;
        log.info("back");
        assert_eq!(sink.inner.contents(), "2026-11-24T22:00:10Z info back\n");
        assert_eq!(log.failing_since(), None);
        assert_eq!(clone.failing_since(), None);
    }
}
