# Seam: logging and warnings

**Purpose.** One logger handle every component writes through, so OPS-9's "never logged" rules hold in one place and each in-process daemon logs to its own buffer (TST-5); and the registry behind `status`'s `warnings` (API-5).

**Producer:** T02 declares; T04 builds. **Consumers:** every daemon-side task.

## Declarations (`src/log/`: `Level`, `LogSink`, the sinks, `Logger`; `src/status/`: `WarningCode`, `Warning`, `Warnings`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level { Error, Warn, Info, Debug }

pub trait LogSink: Send + Sync + 'static {
    /// One complete line, without its newline. An error means it wasn't written.
    fn write_line(&self, line: &str) -> std::io::Result<()>;
}
pub struct StderrSink;                       // a foreground daemon, and a spawned one (its stderr is the log file, CLI-7)
#[derive(Clone, Default)] pub struct BufferSink(/* Arc<Mutex<String>> */);
impl BufferSink { pub fn contents(&self) -> String; }

#[derive(Clone)] pub struct Logger { /* … */ }
impl Logger {
    pub fn new(clock: SharedClock, sink: Arc<dyn LogSink>, level: Level) -> Logger;
    /// `CIRCITOR_LOG` (OPS-9): unset is `Info`; an unknown value is `Info` and the name to warn about.
    pub fn level_from_env(value: Option<&str>) -> (Level, Option<String>);
    pub fn error(&self, msg: impl std::fmt::Display);
    pub fn warn(&self, msg: impl std::fmt::Display);
    pub fn info(&self, msg: impl std::fmt::Display);
    pub fn debug(&self, msg: impl std::fmt::Display);
    pub fn enabled(&self, level: Level) -> bool;
    /// Since when lines can't be written; `None` while they can (OPS-9: logging never stops the daemon).
    pub fn failing_since(&self) -> Option<Time>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    QuotaCapped, QuotaReserve, PrLimit, PrFailing,
    StateFileSetAside, StateFileNewer, StateFileUnwritable,
    LogUnwritable,                           // OPS-9, API-5: a log line couldn't be written
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warning { pub code: WarningCode, pub message: String, pub since: Time }

#[derive(Clone)] pub struct Warnings { /* … */ }
impl Warnings {
    pub fn new(clock: SharedClock, logger: Logger) -> Warnings;
    /// Raises `code` for `key`, or keeps it: `since` stays that of the first raise, `message` is replaced.
    /// `key` tells apart several warnings of one code (`pr_failing`: the PR's node ID); "" otherwise.
    pub fn raise(&self, code: WarningCode, key: &str, message: String);
    pub fn clear(&self, code: WarningCode, key: &str);      // nothing when it isn't raised
    pub fn list(&self) -> Vec<Warning>;                     // by `since`, then code, then key
    /// Since when `code` has been raised for `key`; `None` when it isn't (the notifier's `pr_failing`, NTF-4).
    pub fn since(&self, code: WarningCode, key: &str) -> Option<Time>;
}
```

## Semantics

- **A line is `<time> <level> <message>`** (OPS-9): the time RFC 3339 UTC from the clock's wall time, the level in lower case.
- **`Logger` never fails and never blocks for long.** A line the sink refuses is dropped; `failing_since` is set until a line is written again. The status route (T22) adds a `log_unwritable` warning while it's set.
- **`Warnings::raise` and `clear` log** one `warn` line when a warning is first raised and one `info` line when it's cleared (OPS-9), never on a repeat.
- **A warning's `key` is never logged or shown.** For `pr_failing` it's a node ID (MOD-1); the message names the PR.
- **Messages are built by circitor's own code.** A warning's message and a log line never carry a title, GitHub's text, `gh`'s output, the token, **a session's token or a notice's text** (OPS-9, OPS-5, NTF-9). TST-11's marker sweep is what proves it.

## Who logs what (OPS-9's list)

**A message starts with its event word, then `key=value` pairs**, so tests and people find lines by the word: `stop cause=idle`, `sleep seconds=7200`, `problem begin code=github_unavailable`. The words are fixed here; the pairs are each task's, within OPS-9's "never logged".

| Event (its word) | Level | Task |
|---|---|---|
| Start, with the version and paths (`start`); stop, with its cause (`stop`; causes `signal`, `request`, `idle`, `files`); the 5 s deadline hit (`stop deadline`); a failed final write (`state file write failed`) | info, warn | T28 |
| An unknown `CIRCITOR_LOG` value (`log level unknown`) | warn | T28 |
| A sleep noticed, with its length (`sleep`); the tier changing (`tier`) | info | T21 |
| A problem beginning and ending (backoff, rate limit, `auth`), never each retry (`problem begin`, `problem end`, with `code=`) | warn, info | T21 (T13 for `auth`) |
| A failed periodic token read, once (POL-3) (`token read failed`); the login at the first poll and when it changes (OPS-5) (`login`) | warn, info | T13, T21 |
| A PR's error, once while it lasts (POL-24): the PR, the status or the error `type` (`pr error`) | warn | T42 |
| A PR tracked, untracked or gone (`tracked`, `untracked`, `gone`) | info | T15 |
| The state file set aside, left alone (newer), an entry skipped (`state file set aside`, `state file newer`, `state file entry skipped`). A write failing and working again is the `state_file_unwritable` warning's two lines, below | warn | T16 |
| Every warning raised and cleared (`warning raised`, `warning cleared`, with `code=`) | warn, info | T04 (`Warnings`) |
| A subscription registered, and ended with its end's name (`subscription registered`, `subscription ended`, with `pr=`, `until=` and `end=`, and `why=` for `problem`); a notice that couldn't be posted (`notice not posted`, with `reason=`); a socket whose `stat` failed otherwise, once per subscription (`socket not checked`, with `pr=`; NTF-4); never a notice's text, a token or a socket's path (NTF-6, NTF-9, NTF-10) | info, warn | T46 |
| `debug`: one line per request to GitHub: its kind, cost and duration (`request`) | debug | T21 |

## The fake

`BufferSink` is the test sink. A sink that fails on demand, for OPS-9's test, is T04's own.

## Don't assume

- A global logger. There is none: two daemons in one test process each have their own `Logger`.
- That `Display` of a foreign error is safe to log. An `io::Error`'s text is; an error built from a response body isn't. Log the `type` or the status.
