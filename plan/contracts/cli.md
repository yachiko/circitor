# Seam: the CLI

**Purpose.** The CLI as a function the harness can call (CLI-13, TST-5), and the four parts it's built from, which three tasks build at the same time and a fourth joins: arguments and naming (T11), rendering (T10), the connection to the daemon (T24), the commands (T27).

**Producer:** T02 declares `CliEnv`, `Output`, `Spawner`, `Git`, `Interrupt`, the exit codes and `run`'s signature. **Consumers:** T29 (the harness runs commands), T36 (the binary), and the four CLI tasks among themselves.

## Declarations (`src/cli/mod.rs`)

```rust
/// CLI-5.
pub mod exit {
    pub const OK: i32 = 0;             pub const ERROR: i32 = 1;        pub const USAGE: i32 = 2;
    pub const TIMEOUT: i32 = 3;        pub const ENDED: i32 = 4;        pub const NOT_FOUND: i32 = 5;
    pub const NO_DAEMON: i32 = 6;      pub const AUTH: i32 = 7;         pub const CHECKS_FAILED: i32 = 8;
    pub const UNAVAILABLE: i32 = 9;    pub const LIMIT: i32 = 10;
    pub const SIGINT: i32 = 130;       pub const SIGTERM: i32 = 143;
}

/// API-12's default, which the CLI always sends (CLI-6).
pub const DEFAULT_WAIT_TIMEOUT: u32 = 100;

/// Everything a command prints, and its exit code. Nothing is printed before the command ends (CLI-6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output { pub stdout: String, pub stderr: String, pub code: i32 }

/// A command's inputs, each through one interface (CLI-13).
#[derive(Clone)]
pub struct CliEnv {
    pub clock: SharedClock,                    // durations by `mono()`, ages and local times by `wall()`
    pub tz: chrono::FixedOffset,               // for "until 14:05 local" (CLI-3); UTC in tests (TST-5)
    pub cwd: PathBuf,
    pub env: Env,
    pub platform: Platform,
    pub version: String,                       // the CLI's own (CLI-9)
    pub spawner: Arc<dyn Spawner>,
    pub git: Arc<dyn Git>,
    pub interrupt: Interrupt,
    pub request_limit: Duration,               // CLI-7's 90 s; `CIRCITOR_TEST_CLI_LIMIT` in a test build (TST-1)
}
impl CliEnv { pub fn real() -> CliEnv; }       // T27

/// Runs one command to its end. `args` excludes the program's name. Never `circitor daemon` without
/// `--check-config`: `main` hands that to `daemon::run::main`.
pub async fn run(args: &[String], env: &CliEnv) -> Output;
/// The binary's entry: real `CliEnv`, a runtime, `run`, prints, returns the exit code.
pub fn main() -> i32;

/// How a daemon is started (CLI-7). The real one execs `circitor daemon --spawned`, detached.
#[async_trait::async_trait]
pub trait Spawner: Send + Sync + 'static {
    /// `log` is `circitor.log`, already rotated and opened for appending by the caller.
    async fn spawn(&self, log: std::fs::File, env: &Env) -> std::io::Result<Box<dyn Spawned>>;
}
pub trait Spawned: Send {
    /// `Some(status)` once the process has exited; `None` while it runs.
    fn exited(&mut self) -> Option<i32>;
}

#[derive(Clone, Debug, PartialEq, Eq)] pub enum Branch { Named(String), Detached, NoRepo }
#[async_trait::async_trait]
pub trait Git: Send + Sync + 'static {
    async fn origin_url(&self, cwd: &Path) -> Option<String>;      // `None`: no repository, or no `origin`
    /// The name `git` pushes the current branch to on `origin`: its upstream's branch, or its own name (CLI-2).
    async fn push_branch(&self, cwd: &Path) -> Branch;
}

/// `SIGINT` and `SIGTERM` (CLI-6). Resolves with 130 or 143.
#[derive(Clone)] pub struct Interrupt(/* a watch receiver */);
impl Interrupt {
    pub fn never() -> Interrupt;
    pub fn pair() -> (InterruptTrigger, Interrupt);                // tests, and `main`'s signal handlers; `InterruptTrigger::fire(&self, code: i32)`
    pub async fn wait(&mut self) -> i32;
}
```

## What T27 expects of the three parts

Signatures are fixed here so T10, T11 and T24 can be built apart. A change goes through the orchestrator.

```rust
// ---- src/cli/args/ (T11) ----
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    List { all: bool, since: Option<String>, json: bool },
    Show { pr: Option<String>, fresh: bool, json: bool },
    Wait { pr: Option<String>, until: Until, sha: Option<String>, rerun_since: Option<String>,
           since: Option<String>, timeout: Option<u32>, json: bool },
    Changes { pr: Option<String>, since: Option<String>, json: bool },
    Track { pr: String, json: bool }, Untrack { pr: String, json: bool },
    Refresh { json: bool }, Status { json: bool }, Stop,
    Daemon { spawned: bool, check_config: bool },
    Guide, Version, Help(String),
}
#[derive(Clone, Debug, PartialEq)] pub struct Parsed { pub command: Command, pub no_spawn: bool }
/// `Err` is a usage error: its text for stderr, exit code 2 (CLI-5).
pub fn parse(args: &[String], env: &Env) -> Result<Parsed, String>;

// ---- src/cli/naming/ (T11) ----
#[derive(Clone, Debug, PartialEq)] pub enum PrArg { Full(PrName), Number(u64), Bare }
pub fn pr_arg(arg: Option<&str>) -> Result<PrArg, String>;          // `owner/repo#12`, a URL, `12`, nothing
pub fn repo_of_origin(url: &str) -> Option<(String, String)>;        // github.com only: https, ssh, `git@`
#[derive(Clone, Debug, PartialEq)]
pub enum BranchMatch { Open(PrName), Ended(PrName), None, Several(Vec<PrName>) }
/// CLI-2's choice among the records of a `list`: head branch, repository, author, open before ended.
pub fn match_branch(records: &[Record], owner: &str, repo: &str, branch: &str, login: Option<&str>, ended_too: bool) -> BranchMatch;

// ---- src/cli/guide/ (T11) ----
pub const GUIDE: &str;                                               // CLI-11, under 60 lines

// ---- src/cli/render/ (T10): pure, text only ----
#[derive(Clone, Debug)] pub struct RenderCtx { pub now: Time, pub tz: chrono::FixedOffset, pub cli_version: String }
#[derive(Clone, Debug, PartialEq)]
pub struct WaitAsk { pub pr: PrName, pub until: Until, pub sha: Option<String>, pub rerun_since: Option<String>,
                     pub since: Option<String>,
                     pub timeout: u32,            // the deadline in force: `--timeout`, or `DEFAULT_WAIT_TIMEOUT`
                     pub timeout_given: bool,     // `again:` repeats `--timeout` only if it was given (CLI-6)
                     pub json: bool }             // and `--json` likewise
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum NoAnswer { NoTimeLeft, DaemonStopped, NotAnswering }
pub fn list(e: &Envelope<ListBody>, with_since: bool, ctx: &RenderCtx) -> String;
pub fn show(e: &Envelope<RecordBody>, ctx: &RenderCtx) -> String;
pub fn changes(e: &Envelope<ChangesBody>, ctx: &RenderCtx) -> String;
pub fn wait(e: &Envelope<WaitBody>, ask: &WaitAsk, ctx: &RenderCtx) -> String;
/// CLI-6's three endings without an answer; `cursor` is the `--since` given or the first `circitor-cursor`.
pub fn wait_no_answer(kind: NoAnswer, ask: &WaitAsk, cursor: Option<&str>, socket: &Path) -> String;
/// `track`, `untrack`. With `record: null` (API-15) it names `pr`.
pub fn tracked(e: &Envelope<RecordBody>, pr: &PrName, ctx: &RenderCtx) -> String;
pub fn refresh(e: &Envelope<RefreshBody>, ctx: &RenderCtx) -> String;
pub fn status(e: &Envelope<StatusBody>, ctx: &RenderCtx) -> String;
pub fn stop(was_running: bool) -> String;
/// An API-3 error as CLI-5's one stderr line; `older_daemon` adds CLI-9's hint.
pub fn api_error(e: &ApiError, older_daemon: bool, ctx: &RenderCtx) -> String;
/// CLI-5's code for a wait's answer: 0, 3, 4 or 8.
pub fn wait_code(body: &WaitBody, until: Until) -> i32;
pub fn error_code(code: ErrorCode, older_daemon: bool) -> i32;
/// `refresh`'s code: 0 for `polled` and `shared`, else the code of the API-3 error of that name (API-16).
pub fn refresh_code(outcome: PollOutcome) -> i32;
/// CLI-6's three endings: 3, 6, 6.
pub fn no_answer_code(kind: NoAnswer) -> i32;

// ---- src/cli/conn/ (T24) ----
#[derive(Clone, Debug, PartialEq)] pub struct ConnError { pub code: i32, pub stderr: String }   // exit 6 and its lines
#[derive(Clone, Debug, PartialEq)] pub struct Reply { pub status: u16, pub body: Vec<u8> }
pub struct Link { /* the socket that answered, the daemon's version and mode */ }
pub struct RealSpawner;                                              // the real `Spawner` (T24)
/// CLI-7 and CLI-9: connect; with no daemon, start one; replace an older one. `budget` is what's left of a
/// wait's deadline, `None` for other commands. Warnings (a second socket, a newer daemon) go to `warn`.
pub async fn ensure(env: &CliEnv, paths: &Paths, no_spawn: bool, budget: Option<Duration>, warn: &mut String) -> Result<Link, ConnError>;
/// CLI-8: connect only. `Ok(None)`: not running.
pub async fn existing(env: &CliEnv, paths: &Paths, warn: &mut String) -> Result<Option<Link>, ConnError>;
/// CLI-10, for a wait that had its cursor: wait up to 8 s (within `budget`) for a daemon to answer; start none.
pub async fn reconnect(env: &CliEnv, paths: &Paths, budget: Duration) -> Result<Link, ConnError>;
impl Link {
    pub fn socket(&self) -> &Path;                                   // the one that answered (CLI-7): for CLI-6's lines
    pub fn older_unreplaced(&self) -> bool;                          // CLI-9: an older daemon that was kept
    /// Any request but a wait: CLI-7's limit, and CLI-10's one retry as a new command.
    pub async fn request(&mut self, env: &CliEnv, req: &Request) -> Result<Reply, ConnError>;
    /// A wait: sends and returns the head. No limit, no retry: T27 owns CLI-6's deadline and CLI-10's rule.
    pub async fn open(&self, req: &Request) -> Result<Head, Dropped>;
}
/// CLI-8's `stop`, whole: ask, wait for the lock, `SIGTERM`, `SIGKILL`. `Ok(false)`: nothing was running.
pub async fn stop(env: &CliEnv, paths: &Paths, warn: &mut String) -> Result<bool, ConnError>;

// ---- src/cli/commands/ (T27) ----
pub struct RealGit;                                                  // the real `Git`: runs `git` with tokio::process
```

## Semantics

- **`run` never reads the process's environment, clock, directory or terminal.** Everything comes from `CliEnv`. It doesn't print: `main` prints `Output`.
- **`--json` prints the response body unchanged, on one line (CLI-4):** T27 prints the bytes `Reply.body` holds (the daemon already sends one line), never a re-serialised struct.
- **Colour (CLI-12):** nothing is ever coloured, so nothing depends on whether stdout is a terminal.
- **A wait's time running out inside `ensure`** isn't a `ConnError` of its own: T27 looks at its own deadline when `ensure` fails, and under 1 s left ends as CLI-6's `NoTimeLeft` (exit 3), not 6.
- **A wait's limit (CLI-7: its deadline plus 10 s; 60 s for `--timeout 0`) is T27's**, around `Link::open` and `Head::body`. Every other request's limit is inside `Link::request`.
- **`match_branch` with `login: None` matches nothing.** T27 reads the login from a `Status` request (`github.login`) before it calls it.
- **Every exit code is decided in T10's tables** (`wait_code`, `error_code`, `refresh_code`, `no_answer_code`) or is a `ConnError.code`; T27 adds only 1 (unexpected), 2 (usage, naming) and the two signal codes.
- **`Spawner` only starts the process.** Creating the state directory, rotating and opening the log, watching the child and reading the log's tail are T24's, in `conn` (CLI-7).
- **`Interrupt`** ends a `wait` at once with 130 or 143 (CLI-6); T27 selects on it around everything that waits.
- **Text and exit codes are an interface (CLI-3, CLI-5).** T10's golden files (`tests/golden/render/`) fix the text; a changed line or code is a contract change.

## The fakes

- `cli::fake::FakeSpawner` (T43): `exits(code, log_line: Option<&str>)` (a child that first appends that line to the log file it was given, then has exited with the status: OPS-1's message for a bad config), `never_binds()` (runs, binds nothing), `starts(f: impl Fn(std::fs::File, &Env) -> Box<dyn Spawned> + Send + Sync + 'static)` (for the harness, whose closure starts the in-process daemon on a task and returns a `Spawned` that follows it), and `spawns() -> usize`.
- A stub daemon for CLI tests: T24 keeps one as `pub(crate) mod stub` under `cfg(test)` in `src/cli/conn/` (a few lines of server on a `ShortDir` socket that answers the golden files of `testdata/api/`, stalls, or drops), and T27's tests use it.
- `cli::fake::FakeGit` (T43): `new(origin: Option<&str>, branch: Branch)`.
- The harness (T29) builds a `CliEnv` on the test clock and runs `run` against the in-process daemon.

## Don't assume

- That the daemon is the CLI's version, or that it's there at all.
- That a `Reply` with status 200 decodes into the type you expect from an older or newer daemon: decode leniently, and treat a failure as CLI-5's code 1.
