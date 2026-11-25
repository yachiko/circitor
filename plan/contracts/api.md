# Seam: the API's types and the server's shared parts

**Purpose.** The request and response JSON of every route, as Rust types the server and the CLI share (API-1 to API-19), and the three things both halves of the server (the routes, T22; the waiters, T23) must do the same way: strict parameters, the envelope, the error body.

**Producer:** T02 writes `src/api/types.rs` (the types and their serde, `Until`'s words) and `src/api/mod.rs`; **T43 builds the shared parts**: `request.rs` (`Request`'s three methods), `params.rs`, `respond.rs` and `router.rs`; T41 writes the golden files. T22 builds `api::server`, T23 `api::wait`. **Consumers:** T10, T24, T27 (the CLI decodes and renders these), T17 (sends `Request`), T28 (serves `router`).

## Declarations (`src/api/types.rs`)

```rust
/// API-3. `http()`: 400, 404, 404, 503, 503, 503, 503, 409.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ErrorCode { BadRequest, NotFound, UnknownRoute, Auth, Starting, GithubUnavailable, RateLimited, Limit }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiError { pub code: ErrorCode, pub message: String, pub until: Option<Time> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody { pub version: String, pub error: ApiError }

/// API-4's `daemon`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DaemonBlock { pub tier: Tier, pub last_poll_at: Option<Time>, pub problem: Option<ProblemCode>,
                         pub message: Option<String>, pub until: Option<Time> }
/// Every successful body: `version`, `cursor`, `daemon`, then the route's keys (API-2, API-4, API-19).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> { pub version: String, pub cursor: String, pub daemon: DaemonBlock, #[serde(flatten)] pub body: T }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListBody { pub records: Vec<Record>, pub untracked: Vec<PrName>, pub pending: Vec<PrName>, pub reset: bool }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordBody { pub record: Option<Record> }             // show, track: always `Some`; untrack: may be `null`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangesBody { pub events: Vec<Event>, pub records: Vec<Record>, pub pending: Vec<PrName>,
                         pub more_events: bool, pub reset: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Outcome { Satisfied, Ended, Timeout }                   // API-11
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Reason { Merged, Closed, Gone, HeadMoved }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitBody { pub outcome: Outcome, pub reason: Option<Reason>, pub record: Option<Record>,
                      pub events: Vec<Event>, pub records: Vec<Record>, pub more_events: bool, pub reset: bool }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefreshBody { pub outcome: PollOutcome, pub cost: u32 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShutdownBody {}

/// API-5, in its order. `cursor` and `daemon` come from the envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusBody {
    pub mode: Mode,                                              // `spawned`, `foreground`
    pub started_at: Time,
    pub last_poll: Option<LastPoll>,
    pub cost_last_hour: u32,
    pub github: GithubInfo,
    pub tracked: TrackedCounts,
    pub waiters: u32,
    pub owners: Vec<String>,
    pub token_source: String,                                    // `gh`, `GH_TOKEN`, `GITHUB_TOKEN`
    pub gh_path: String,                                         // plan decision P10 (OPS-5: "status shows the path that was run")
    pub paths: PathsInfo,
    pub warnings: Vec<Warning>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct LastPoll { pub at: Time, pub outcome: PollOutcome }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GithubInfo { pub remaining: Option<u32>, pub reset_at: Option<Time>, pub login: Option<String> }
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackedCounts { pub authored: u32, pub manual: u32, pub asked: u32 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathsInfo { pub config: String, pub state: String, pub socket: String, pub log: String }   // `state`: state.json
/// What any CLI reads of any daemon's status (API-2): only these two keys, everything else ignored.
#[derive(Clone, Debug, PartialEq, Deserialize)] pub struct StatusLite { pub version: String, pub mode: String }

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Until { Merged, ChecksDone, Changed }   // `merged`, `checks-done`, `changed`
impl Until { pub fn as_str(self) -> &'static str; pub fn parse(s: &str) -> Option<Until>; }
/// Every request the CLI sends. The one place paths and parameter names are spelt.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    List { all: bool, since: Option<String>, checks: bool },
    Show { pr: PrName, fresh: bool },
    Changes { pr: Option<PrName>, since: Option<String>, limit: Option<u32> },
    Wait { pr: PrName, until: Until, sha: Option<String>, rerun_since: Option<String>,
           since: Option<String>, timeout: u32, limit: Option<u32> },
    Track { pr: PrName }, Untrack { pr: PrName },
    Refresh, Status, Shutdown,
}
impl Request {
    pub fn method(&self) -> http::Method;
    /// `/v1/prs?state=all&since=…`, `/v1/prs/{owner}/{repo}/{number}/wait?until=…&timeout=…` and so on,
    /// exactly as 04-api writes them; a flag is sent only when set, as `1`.
    pub fn path_and_query(&self) -> String;
    pub fn body(&self) -> Option<String>;                        // `track`: `{"pr":"owner/repo#12"}`
}
pub const CURSOR_HEADER: &str = "circitor-cursor";               // API-9; header names compare without case
```

## Declarations (`src/api/mod.rs`, `router.rs`, `params.rs`, `respond.rs`)

```rust
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>, pub poller: Arc<dyn PollerHandle>, pub flush: Arc<dyn Flush>,
    pub clock: SharedClock, pub sleep: Arc<SleepWatch>, pub activity: Activity,
    pub logger: Logger, pub warnings: Warnings, pub config: Arc<Config>,
    pub info: Arc<DaemonInfo>, pub stop: StopHandle,
}
#[derive(Clone, Debug)]
pub struct DaemonInfo { pub version: String, pub mode: Mode, pub started_at: Time, pub paths: PathsInfo }

/// `server::routes()` merged with `wait::routes()`, the `unknown_route` fallback, and the layer below.
pub fn router(state: AppState) -> axum::Router;

/// Put on every request by `router`'s layer: the request's share of the work in flight (TST-4).
#[derive(Clone)] pub struct InFlight(/* … */);
impl InFlight {
    pub fn park(&self);        // the request now waits on the clock or on the poller: not work in flight
    pub fn resume(&self);      // it woke: work again, until dropped
}                              // both may be called any number of times; a second `park` in a row does nothing

/// Strict query parameters (API-1).
pub struct Params { /* … */ }
impl Params {
    pub fn parse(query: Option<&str>) -> Result<Params, ApiError>;        // a name given twice is `bad_request`
    pub fn take(&mut self, name: &str) -> Option<String>;
    pub fn flag(&mut self, name: &str) -> Result<bool, ApiError>;         // `1` or `0`; absent is `false`
    pub fn number(&mut self, name: &str, min: u32, max: u32) -> Result<Option<u32>, ApiError>;   // never clamped
    pub fn cursor(&mut self, name: &str) -> Result<Option<Cursor>, ApiError>;                    // STA-9's form
    pub fn finish(self) -> Result<(), ApiError>;                          // anything left: `bad_request` naming it
}
pub fn bad_request(message: impl Into<String>) -> ApiError;
/// `{owner}/{repo}/{number}` from a path; a number that isn't one is `bad_request`.
pub fn pr_from_path(owner: &str, repo: &str, number: &str) -> Result<PrName, ApiError>;

/// The daemon block from the poller's status (API-4).
pub fn daemon_block(status: &PollerStatus) -> DaemonBlock;
/// A 200 with the envelope: `version`, the cursor of `view`, `daemon`, and `body`'s keys.
pub fn ok<T: Serialize>(state: &AppState, view: &View, body: T) -> axum::response::Response;
/// API-8's rule, once, for `changes` and for `wait`: the events after `since`, for every PR or for one,
/// cut at `limit`; `reset` for a too-old cursor, which gets no events unless it's from another epoch and
/// the history still holds every event of this one; without `since`, no events. `cursor` is the view's,
/// or the last returned event's when `limit` cut the list.
pub struct Window { pub events: Vec<Event>, pub more_events: bool, pub reset: bool, pub cursor: Cursor }
pub fn window(view: &View, since: Option<&Cursor>, pr: Option<&NodeId>, limit: Option<u32>) -> Window;
/// The same with another cursor: API-8's `limit` answers with the cursor of the last event returned.
pub fn ok_at<T: Serialize>(state: &AppState, cursor: &Cursor, body: T) -> axum::response::Response;
/// The envelope's JSON alone, on one line: what `ok` and `ok_at` send. A wait's late body is this (API-9).
pub fn envelope<T: Serialize>(state: &AppState, cursor: &Cursor, body: T) -> Vec<u8>;
/// API-3's body with the code's HTTP status.
pub fn error(state: &AppState, e: ApiError) -> axum::response::Response;
/// A `Refusal` from the poller as its API-3 error.
pub fn refusal(r: Refusal) -> ApiError;
/// A record as an answer shows it: `stale` computed (MOD-16), `checks.items` kept or left out.
pub fn present(state: &AppState, pr: &PrView, tier: Tier, with_items: bool) -> Record;
```

T02 writes `server::routes()` and `wait::routes()` as stubs returning an empty `Router<AppState>`; T22 and T23 fill them. Neither touches `src/api/mod.rs`. `present` calls `poller::cadence::stale`, which T09 builds: until T09 is merged it's T02's stub returning `false`.

**What T43 can and can't test.** It has no working `Store` (T15), so no `AppState`: `ok`, `ok_at`, `envelope` and `error` are built over inner functions that take the version, the daemon block and the cursor, and those are tested; `window` is tested over hand-built `View`s (its fields are public); `daemon_block`, `refusal`, `pr_from_path` and `Params` need no state. `present`, the layer and the fallback are tested through `router` by T22. A fault T22 or T23 finds in these files is reported; the orchestrator fixes it as a follow-up to T43.

## Semantics

- **Anything that isn't a route of 04-api is `unknown_route`** (API-3): an unknown path, and a known path with another method. `router` sets both the fallback and the method-not-allowed fallback, so no route answers a bare 405.
- **`router`'s layer**, on every request: takes `activity.begin()` and puts an `InFlight` in the request's extensions; calls `sleep.check()` (POL-22). It does **not** call `poller.client_request()`: each handler does, first thing, except `status` (API-5) and `shutdown`.
- **A handler parks around every await that can outlast the present instant** (`clock.md`): a held waiter, API-18's wait for the first poll, and every call of `read_through`, `track`, `refresh_pr` and `poll_now`, because the engine may be waiting for POL-2's gap while the handler waits for it. `park()` before the await, `resume()` after. Without it the test clock never settles and `advance` hangs.
- **`shutdown` answers, then stops (API-17).** A handler can't see its answer being written: T22's response body asks for the stop when it's dropped, which is after `hyper` has taken its last byte; T28's stop must not cut bytes already handed to the socket.
- **A body is one line of JSON with no newline at the end.** CLI-4's `--json` prints it and then one newline.
- **Values are percent-encoded on the wire.** `Request::path_and_query` encodes every parameter value and path segment; `Params::parse` decodes. No crate is needed for it.
- **One view per answer (MOD-13).** A handler takes `store.view()` once, after its last change to the store, and builds records, events and the cursor from it.
- **Unknown input is `bad_request` naming it (API-1):** a parameter (through `Params::finish`), a body key (`#[serde(deny_unknown_fields)]` on the server's own body types, with the error mapped to `bad_request`), a value. The shared response types above are **not** `deny_unknown_fields`: a CLI reads a newer daemon's answers and ignores what it doesn't know (CLI-9).
- **Lists and flags are never left out (API-19).** `Option` is `null`, never absent, except `checks.items` (API-6) and an item's review-only keys (see `model.md`).
- **`wait` answers in two steps (API-9):** the status line and headers, with `circitor-cursor`, as soon as the waiter is registered; the body when the wait ends. A refusal before that is an ordinary error response. T23 owns how; T17 reads the head and the body apart.
- **Golden files, `testdata/api/`** (T41 writes them by hand from 04-api and 01-model; its tests keep types and files equal, key order included): `list.json`, `list-reset.json`, `show.json`, `changes.json` (one event of every kind), `wait-satisfied.json`, `wait-timeout-no-record.json`, `track.json`, `untrack-null.json`, `refresh.json`, `status.json`, `status-problem.json` (a problem, an `until`, two warnings), and one `error-<code>.json` per `ErrorCode`. The CLI's render tests (T10) and the routes' tests (T22, T23) both read them.

## The fake

The golden files are the fake server for T10, and the fake client for T22 and T23. `FakePoller` and `NoFlush` stand in behind `AppState`; `api::test_state(clock: Arc<TestClock>) -> (AppState, Arc<FakePoller>)` (T43, test builds only; the state's `activity` is the test clock's, so `settle` sees a handler's work) builds one over a real, empty `Store`. It needs T15 to do anything, so T43 can't test `router` itself: **T22 tests the `unknown_route` fallback and the layer through `router`.**

## Don't assume

- That the CLI and the daemon are the same version (CLI-9). Only `StatusLite` and `POST /v1/shutdown` are promised across versions (API-2).
- That a handler's future runs to its end: the server drops it when the client disconnects (API-14).
