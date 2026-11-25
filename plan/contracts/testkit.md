# Seam: the test kit

**Purpose.** The fake GitHub (TST-3) and the harness (TST-5): what every scenario task builds on. Compiled only in a test build or under `cfg(test)` (TST-1).

**Producer:** T02 declares the module and `MARKER_TOKEN`. T14 builds `testkit::github` (the world and the GraphQL engine), T20 `testkit::server` (the HTTP server around it), T29 `testkit::harness`. **Consumers:** T26, T29, T36 (the fake GitHub), T30 to T35 (the harness).

## Declarations (`src/testkit/mod.rs`, T02)

```rust
/// The token every test uses (TST-11). Nothing but the `Authorization` header may ever hold it.
pub const MARKER_TOKEN: &str = "gho_CIRCITOR_TEST_MARKER_TOKEN_0000";
/// A second one, for "another token than the one refused" (POL-20) and TST-9's `GH_TOKEN` case. Swept like the first.
pub const MARKER_TOKEN_2: &str = "gho_CIRCITOR_TEST_MARKER_TOKEN_0002";
/// The version the harness gives the daemon and the CLI unless a test says otherwise, so golden files don't change with every commit (TST-7).
pub const TEST_VERSION: &str = "1.2.3+test";
/// The fake GitHub puts these in every title, every error `message` and the 401's body (TST-11).
pub const MARKER_TITLE: &str = "MARKER-TITLE";
pub const MARKER_GITHUB_TEXT: &str = "MARKER-GITHUB-TEXT";
/// A short directory under `/tmp`, removed when dropped (socket paths stay under 103 bytes, OPS-4).
pub struct ShortDir(/* … */);
impl ShortDir { pub fn new() -> ShortDir; pub fn path(&self) -> &Path; }
/// Compares `actual` with the golden file at `path` (relative to the package root); with
/// `CIRCITOR_UPDATE_GOLDEN=1` it writes the file instead (TST-7). On a difference it panics with both texts.
pub mod golden {
    pub fn check(path: &str, actual: &str);
    /// The same, but it never writes: for files written by hand from the spec (T41), which no run may overwrite.
    pub fn compare(path: &str, actual: &str);
}
```

## The fake GitHub: what T14 gives T20 (`src/testkit/github/`)

```rust
/// GitHub's public schema, parsed once per process from `testdata/github/schema.graphql`.
pub fn schema() -> &'static Schema;
/// The scripted world (TST-3): PRs with their fields, checks, comments and reviews, and changes
/// scheduled at times on GitHub's own clock. T14 designs its building methods; they must let a test
/// play every row of docs/02 §4 and every scenario of TST-8 without touching JSON.
pub struct World { /* … */ }
impl World {
    pub fn new(login: &str, start: Time) -> World;
    /// The `rateLimit` block's `remaining` and `resetAt`; `execute` answers them, T20 sets and spends them.
    pub fn set_rate(&mut self, remaining: u32, reset_at: Time);
    pub fn now(&self) -> Time;                                   // GitHub's clock: only ever forward
    pub fn advance_to(&mut self, t: Time);                       // applies the scheduled changes due by `t`, in order
}
pub struct Executed {
    pub body: serde_json::Value,                                 // `{"data": …}` with `errors` when there are any
    pub cost: u32,                                               // GitHub's formula (docs/02 §2)
    pub node_ids: Vec<String>,                                   // the IDs `nodes(ids:)` asked for
}
#[derive(Debug)] pub enum Rejected { Unparsable(String), NotInSchema(String), NoResolver(String) /* in the schema, not in the fake */, Mutation }
/// Parses `request` (the POST body: `query`, `variables`), checks it against the schema, answers only
/// the fields asked from the world as it is now, and costs it. Paging is by cursor over the world as it
/// is at this call.
pub fn execute(world: &World, request: &[u8]) -> Result<Executed, Rejected>;
/// What a request asks, for T20's request log, which never parses GraphQL itself.
pub struct Asked {
    pub finding: Option<Option<String>>,                         // `Some`: it holds a finding; inside, that page's cursor
    pub nodes: Vec<(String, AskedOf)>,                           // each node ID with what was asked of it
    pub lookup: Option<(String, String, u64)>,                   // owner, repository, number
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum AskedOf { Record, RecordAndItems, Items, ChecksPage }
pub fn describe(request: &[u8]) -> Result<Asked, Rejected>;
```

**Faults that change an answer are the world's** (T14): `mergeable` unknown for N reads, the old head served for a while, a check read out of order, `NOT_FOUND` for one node, an error inside a PR's node. T20 exposes them and adds the transport's.

## The fake GitHub: what T20 gives everyone (`src/testkit/server/`)

`FakeGitHub::start(clock: Arc<TestClock>) -> FakeGitHub` serves `POST /graphql` on `127.0.0.1:0` and adds itself to the test clock as a probe. **`FakeGitHub::start_real(clock: SharedClock)`** is the same server on the system clock, for T36's real processes: its world's clock and its POL-2 rule then run on real time, and its per-daemon rules are held per connection (it can't tell processes apart otherwise; plan decision P10). It must offer, with names of T20's choosing documented in the module:

- `url()`: the full `http://127.0.0.1:<port>/graphql` (TST-1); `client()`: a `github::client::Client` pointed at it.
- `world(|w| …)`: change the world now, and schedule changes; its clock follows the test clock's wall time forward and never back, with an optional fixed skew (TST-3).
- Faults, each for a number of requests or until cleared: every item of TST-3's "it can be told to misbehave" list. **A fault can be aimed:** at requests with or without a finding, at one node, at a by-name lookup (an error `type` on `repository.pullRequest`). A redirect counts the hits on its target (OPS-16).
- **Holding an answer:** `hold_next()` makes the next request wait until the returned handle is released, and then answers it (a poll in flight for `forced`, `crash`, `starting`). While held it's excused (`clock.md`).
- `set_rate(remaining, reset_at)`, apart from what circitor spends, and a lag; `set_login`; `without_date_header`.
- `requests()`: every request (TST-6), each with: the clock's monotonic and wall time when it arrived; its cost; whether it held a finding, and that page's cursor; the node IDs asked and what was asked of each (the record, the items, a check page); a by-name lookup's owner, repository and number; and the `Authorization` value received. `points()`: the costs' sum.
- **The rules that fail the test:** a mutation, a request while another is in flight, a request under 1 s after the last ended on the clock's monotonic time (POL-1, POL-2), a query the schema refuses. `new_daemon()` starts the per-daemon checks afresh; the harness calls it at every start, restart and crash. A broken rule is recorded and panics at `check()` and at drop, with the offending request.
- It implements `clock::SettleProbe`: settled when every connection it accepted is answered, closed, or held by script. While it holds an answer by script it also holds `clock.activity().excuse()`, so the daemon's request in flight isn't counted (`clock.md`).

## The harness (`src/testkit/harness/`, T29)

`Harness::start(opts)` gives a running in-process daemon on the test clock (TST-5's defaults: `spawned`, OPS-2's defaults but `exit_after` 0, a buffer for the log, the clock at 2026-11-24T22:00:00Z, a fixed seed, `TEST_VERSION` for the daemon and the CLI). **`opts` sets everything that must hold before the daemon's first request:** a closure that scripts the world; config values (`owners`, `keep_done`, `exit_after`, `quota.*`, the intervals); faults and a held answer in force from the first request; the token source's first answer and delay; the fake GitHub's clock skew; the daemon's version and `mode`; and **`no_daemon`**, for a harness whose daemon isn't running yet (the fake spawner then starts it, and counts its spawns). It must offer:

- `clock()`, `github()` (the `FakeGitHub`), `token()` (the `FakeTokenSource`), `fs()` (the `FaultFs`), `log()`, `state_file()`, `paths()`.
- `cli(&["wait", "o/r#1", "--until", "merged"]) -> Output`, awaited; `cli_bg(…)` for a command the test advances time under (every `wait`, every command that spawns, a `list` before the first poll has finished); variants that set the working directory, the environment, the CLI's version and the spawner. A background command counts for settling: the harness adds a `SettleProbe` that's unsettled while one has been woken and not yet polled. Its handle says whether it's done without awaiting, gives the `Output` when awaited, and has **`cancel()`**: the client closes its connection, and `cancel` returns once the daemon has seen the close (`Running::open_connections`), so a test can look at `waiters` right after.
- `api_raw(method, path_and_query, body) -> (status, body)`: a request the CLI can't send (an unknown parameter, `timeout=3601`; TST-7).
- `stop()`, `crash()`, `restart()` and `restart_with(config)` on the same directories; a daemon with another version or `mode`. **The daemon's own exit is visible:** `exited() -> Option<i32>` (an idle exit, a stop for files not its own), and a `restart()` after it. **Each start gets another seed** (the fixed seed plus the start's number): the epoch comes from the seed alone, and a restart must give a new one (STA-9).
- `git_repo(origin, branch) -> PathBuf`: a temporary git repository (TST-5).
- **`finish()`**, which every test ends with: TST-11's sweep for `MARKER_TOKEN` of the log, the state file, all CLI output seen (`--json` output is the API's body), every `api_raw` answer, and a `status`, a `list` and a `changes` fetched then; and of the log and the state file for the two GitHub markers; then the fake GitHub's `check()`.

## What the world must be able to play (T14)

The scenario tasks use these; T14's building methods must cover each.

- A push with the old head served for N seconds; a force-push back to an earlier SHA with its old checks.
- Check runs with a `databaseId`, any status and any conclusion string, also one the schema doesn't have (`FOO`); commit statuses, posted again; one out-of-order read; a re-run of the same check run.
- 100, 101, 187, 300, 301 and 400 checks on one commit, with the unread tail's state and the rollup chosen.
- `updatedAt` set against GitHub's clock.
- `UNKNOWN` mergeability for N reads, for one PR and for all; a base change; `CONFLICTING` with `DIRTY`.
- Merge, close, reopen.
- Comments and reviews with `databaseId`, author, time, state and inline count; a thread reply as a review of its own.
- A repository renamed or transferred, node IDs kept.
- A PR or a repository hidden and returned: off the open list, `NOT_FOUND` by node and by name, then back with the same node ID, open or merged.
- An error `type` on one node until cleared.
- Bulk worlds of 41, 150, 200 and 205 PRs with distinct `updatedAt`.
- PRs by other authors and of other owners, a head branch name, two PRs on one branch.
- A change that repeats at every poll (26 PRs flipping `draft`).

## Semantics

- **The fake GitHub shares no type with `src/github/` or `src/model/`.** It builds JSON from the schema and docs/02. A test that needs a `PrRead` gets it by sending a query through the real client.
- **By default the world behaves as docs/02 §4 records** (TST-3): the rollup `null` with no checks and `FAILURE` while others run, `updatedAt` moved by a push, a comment, a draft change, a close or a merge and never by a check, a merged PR's `mergeable` `UNKNOWN`.
- **Settling.** `TestClock::advance` returns only when the daemon's `Activity` is zero and the fake GitHub is settled. Tests therefore never sleep or poll for a state; they advance and look.
- **Everything runs on one current-thread runtime** (`#[tokio::test]`).

## Don't assume

- That the fake is GitHub. It plays what docs/02 records and what POL-25 assumes; a behaviour docs/02 doesn't list isn't in it until a measurement adds it.
- That `cli()` output is checked for you: the sweep looks for markers only.
