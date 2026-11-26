# Seam: the CLI

**Purpose.** The CLI as a function the harness can call (CLI-13, TST-5), and the parts it's built from, which three tasks build at the same time and a fourth joins: arguments and naming (T11), rendering (T10), the connection to the daemon (T24), the commands (T27). The `notify` command (NTF-11) is a fifth part, T48's, which T27 hands over to.

**Producer:** T02 declares `CliEnv`, `Output`, `Spawner`, `Git`, `Interrupt`, the exit codes and `run`'s signature. **Consumers:** T29 (the harness runs commands), T36 (the binary), T46 (the notifier's texts call `render::outcome_line`, `flags` and `duration`), and the five CLI tasks among themselves.

## Declarations (`src/cli/mod.rs`)

```rust
/// CLI-5.
pub mod exit {
    pub const OK: i32 = 0;             pub const ERROR: i32 = 1;        pub const USAGE: i32 = 2;
    pub const TIMEOUT: i32 = 3;        pub const ENDED: i32 = 4;        pub const NOT_FOUND: i32 = 5;
    pub const NO_DAEMON: i32 = 6;      pub const AUTH: i32 = 7;         pub const CHECKS_FAILED: i32 = 8;
    pub const UNAVAILABLE: i32 = 9;    pub const LIMIT: i32 = 10;       pub const NO_SESSION: i32 = 11;   // `notify` (NTF-11)
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

/// How a daemon is started (CLI-7). The real one execs `circitor daemon --spawned`, detached, with `env`
/// less `GH_TOKEN`, `GITHUB_TOKEN` (OPS-5), `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN` (NTF-9).
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
    Wait { pr: Option<String>, cond: Ask, timeout: Option<u32>, json: bool },   // `Ask`: `api.md`; `--fail-fast` and `--kind` are in it (D8)
    Notify { pr: Option<String>, what: NotifyWhat, json: bool },                // NTF-11
    Changes { pr: Option<String>, since: Option<String>, json: bool },
    Track { pr: String, json: bool }, Untrack { pr: String, json: bool },
    Refresh { json: bool }, Status { json: bool }, Stop,
    Daemon { spawned: bool, check_config: bool },
    Guide, Version, Help(String),
}
#[derive(Clone, Debug, PartialEq)]
pub enum NotifyWhat { Register { cond: Ask, life: Option<u32> /* `--for`, 60 to 86 400 */ }, Cancel }
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
pub const GUIDE: &str;                                               // CLI-11, under 120 lines

// ---- src/cli/render/ (T10): pure, text only ----
#[derive(Clone, Debug)] pub struct RenderCtx { pub now: Time, pub tz: chrono::FixedOffset, pub cli_version: String }
#[derive(Clone, Debug, PartialEq)]
pub struct WaitAsk { pub pr: PrName, pub cond: Ask,
                     pub timeout: u32,            // the deadline in force: `--timeout`, or `DEFAULT_WAIT_TIMEOUT`
                     pub timeout_given: bool,     // `again:` repeats `--timeout` only if it was given (CLI-6)
                     pub json: bool }             // and `--json` likewise
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum NoAnswer { NoTimeLeft, DaemonStopped, NotAnswering }
pub fn list(e: &Envelope<ListBody>, with_since: bool, ctx: &RenderCtx) -> String;
pub fn show(e: &Envelope<RecordBody>, ctx: &RenderCtx) -> String;
/// `pr`: the PR asked about, for `more events: circitor changes <pr> --since …` (CLI-3, D9); `None` over every PR.
pub fn changes(e: &Envelope<ChangesBody>, pr: Option<&PrName>, ctx: &RenderCtx) -> String;
pub fn wait(e: &Envelope<WaitBody>, ask: &WaitAsk, ctx: &RenderCtx) -> String;
/// CLI-6's outcome line alone, with no newline and no stale mark: `checks passed (5/5) on 1a2b3c4`,
/// `checks failing (1 failed, 3 of 5 done) on 1a2b3c4` (satisfied while `checks.state` is still `pending`:
/// `--fail-fast`), `merged`, `changed`, `ended: closed`, `ended: head moved to 9f8e7d6, expected 1a2b3c4`,
/// with `(truncated: unread checks may still run)` where CLI-6 adds it. **`wait` prints it, `already` prints
/// it, and `notify::text::outcome` puts it in a notice (NTF-8): one function, so the three can't drift.**
/// `sha` is the `--sha` given. `Outcome::Timeout` has no outcome line: an empty string.
pub fn outcome_line(outcome: Outcome, reason: Option<Reason>, record: Option<&Record>, until: Until, sha: Option<&str>) -> String;
/// A condition as the flags that ask for it: `--until checks-done`, then `--sha`, `--rerun-since`,
/// `--fail-fast` and `--kind`, each only if it was given and as it was. Never `--since`: `again:` sets it
/// to the cursor (CLI-6), `Ask again:` repeats the one given (NTF-8).
pub fn flags(cond: &Ask) -> String;
/// CLI-3's durations: the largest unit and the next, of `d`, `h`, `m`, `s`.
pub fn duration(d: Duration) -> String;
/// NTF-11's "already known", in NTF-11's order: what `wait` prints for the answer `e.body.as_wait()` (a
/// reset and its records, the events), `already: ` and the outcome line, `replaced an earlier notify` when
/// `replaced`, `more events:`, `again:`, `problem:`, `cursor:`. Never a timeout. **With `reset: true` the
/// `again:` line is `again: circitor notify <pr> ` and `flags(cond)`, ` --for <life>` if given, and
/// ` --since <the new cursor>`** (NTF-11).
pub fn already(e: &Envelope<NotifyBody>, pr: &PrName, cond: &Ask, life: Option<u32>, ctx: &RenderCtx) -> String;
/// NTF-11's registered output, whole, around its first line (`registered: ` and `notify::text::watching`,
/// which T48 builds and passes in): a reset and its records and the events first, as `wait` prints them;
/// then `first`; the record's one line (CLI-3's `list` line, stale mark included); `a confirmation was
/// posted to this session; if none arrives, notices are held or refused here: run circitor notify <pr>
/// --cancel, then use circitor wait`; `replaced an earlier notify` when `replaced`; `more events:`;
/// `problem:`; `cursor:`. NTF-11's order, which follows CLI-3's (`specs/05-cli.md`, "After the events").
pub fn registered(e: &Envelope<NotifyBody>, first: &str, pr: &PrName, ctx: &RenderCtx) -> String;
/// CLI-3's stale mark alone, with its leading space: ` (stale, 4m old)`, ` (stale, 4m old, error FORBIDDEN)`,
/// or "" when `record.stale` is false. The record line uses it, and `notify::text::expired` (NTF-8).
pub fn stale_mark(record: &Record, now: Time) -> String;
/// CLI-6's three endings without an answer; `cursor` is the `--since` given or the first `circitor-cursor`.
pub fn wait_no_answer(kind: NoAnswer, ask: &WaitAsk, cursor: Option<&str>, socket: &Path) -> String;
/// `track`, `untrack`. With `record: null` (API-15) it names `pr`.
pub fn tracked(e: &Envelope<RecordBody>, pr: &PrName, ctx: &RenderCtx) -> String;
pub fn refresh(e: &Envelope<RefreshBody>, ctx: &RenderCtx) -> String;
pub fn status(e: &Envelope<StatusBody>, ctx: &RenderCtx) -> String;
pub fn stop(was_running: bool) -> String;
/// An API-3 error as CLI-5's one stderr line; `older_daemon` adds CLI-9's hint.
pub fn api_error(e: &ApiError, older_daemon: bool, ctx: &RenderCtx) -> String;
/// CLI-5's code for a wait's answer: 0, 3, 4 or 8. 8 is `satisfied` for `checks-done` with `checks.state`
/// `failing`, or still `pending` with `checks.failed` above 0 (`--fail-fast`, D8). `notify` never calls it (NTF-11).
pub fn wait_code(body: &WaitBody, until: Until) -> i32;
pub fn error_code(code: ErrorCode, older_daemon: bool) -> i32;      // `no_session` is 11 (CLI-5)
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
/// CLI-2 before any request: `Full` as it is, `Number` with `origin`'s repository. `Ok(None)` is `Bare`,
/// to be found with `find_bare`; `Err` is the whole `Output` (exit 2 and its line).
pub async fn name_offline(env: &CliEnv, arg: Option<&str>) -> Result<Option<PrName>, Output>;
/// CLI-2's flow for no argument, against a daemon: the list, the login, the poll, the list again, the
/// choice. `note` gets the line for an ended match. `Err` is the whole `Output`.
pub async fn find_bare(env: &CliEnv, link: &mut Link, note: &mut String) -> Result<PrName, Output>;

// ---- src/cli/notify/ (T48) ----
pub const SOCKET_VAR: &str = "CLAUDE_CODE_MESSAGING_SOCKET";         // docs/07
pub const TOKEN_VAR: &str = "CLAUDE_CODE_MESSAGING_TOKEN";
/// `circitor notify`, whole (NTF-11). `commands::run` calls it for `Command::Notify` and returns what it returns.
pub async fn run(pr: Option<&str>, what: &NotifyWhat, json: bool, no_spawn: bool, env: &CliEnv) -> Output;
```

## Semantics

- **`run` never reads the process's environment, clock, directory or terminal.** Everything comes from `CliEnv`. It doesn't print: `main` prints `Output`.
- **`--json` prints the response body unchanged, on one line (CLI-4):** T27 prints the bytes `Reply.body` holds (the daemon already sends one line), never a re-serialised struct.
- **Colour (CLI-12):** nothing is ever coloured, so nothing depends on whether stdout is a terminal.
- **A wait's time running out inside `ensure`** isn't a `ConnError` of its own: T27 looks at its own deadline when `ensure` fails, and under 1 s left ends as CLI-6's `NoTimeLeft` (exit 3), not 6.
- **A wait's limit (CLI-7: its deadline plus 10 s; 60 s for `--timeout 0`) is T27's**, around `Link::open` and `Head::body`. Every other request's limit is inside `Link::request`.
- **`match_branch` with `login: None` matches nothing.** T27 reads the login from a `Status` request (`github.login`) before it calls it.
- **Every exit code is decided in T10's tables** (`wait_code`, `error_code`, `refresh_code`, `no_answer_code`) or is a `ConnError.code`; T27 adds only 1 (unexpected), 2 (usage, naming) and the two signal codes, and T48 only 11.
- **`notify` (NTF-11; T48).** Without `SOCKET_VAR` in `env.env` it exits 11 **before anything else**: no path is resolved, no daemon is reached or started, no PR is named (plan decision P15). Its line is NTF-11's: `no session here: use circitor wait <pr> ` and `render::flags`, then ` --since <since>` if given: the condition's parameters as given, **never `--timeout`** (`--for` has no counterpart, API-12); `<pr>` is the PR argument as it was typed, if there was one. For `--cancel` it names no `wait`. Otherwise registering names the PR as `wait` does (`name_offline`, `find_bare`), goes through `conn::ensure`, sends `Request::Notify` with `limit` 50 through `Link::request` (safe to repeat, CLI-10), and prints `render::already` for `registered: false`, or `render::registered` with the first line `registered: ` and `notify::text::watching` (`notify.md`: the same words as the confirmation). **With `--no-spawn` and no daemon** it first waits up to 8 s for one through `conn::reconnect`, and only then exits 6 (NTF-10, CLI-7). **An answer of `no_session` exits 11 too**, by `render::error_code`, with the same one line, now with the PR in full. **`--cancel` never starts a daemon and never waits for one** (CLI-8, NTF-3): through `conn::existing`, and with none running it prints `cancelled: 0` and exits 0. **Its exit code is 0 for registered, already known and cancelled**, whatever the checks did; any other API error keeps `render::error_code`. The notices themselves aren't the CLI's: the daemon posts them (`notify.md`).
- **`notify` reads the session from `CliEnv::env` only** (CLI-13): the harness sets the two variables to its fake session socket and a marker token (TST-16, `testkit.md`).
- **`Spawner` only starts the process.** Creating the state directory, rotating and opening the log, watching the child and reading the log's tail are T24's, in `conn` (CLI-7).
- **`CIRCITOR_SPAWNER` (CLI-7, D19; T65).** `conn::start` adds it to a copy of `env.env` before `Spawner::spawn`, so `Spawner`'s signature doesn't change, and `RealSpawner` passes it on as it passes any variable it doesn't withhold. The value is `pid=<n> cmd=<name> cwd=<dir> session=<id>`, built from `std::process::id()`, the command's name (`show`, `wait`, …), `CliEnv::cwd`, and `CLAUDE_CODE_SESSION_ID` from `env.env`. `session=` is left out when that variable isn't set. The command's name reaches `conn` as T65 chooses: a `CliEnv` field, or a parameter of `conn::ensure`. Either way it's a change to this seam, written here when T65 makes it. Never a token or a session's socket (NTF-9).
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
