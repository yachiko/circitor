# Seam: the API's types and the server's shared parts

**Purpose.** The request and response JSON of every route, as Rust types the server and the CLI share (API-1 to API-19, NTF-2, NTF-3), the three things every part of the server (the routes, T22; the waiters, T23; the notify routes, T47) must do the same way: strict parameters, the envelope, the error body; and **the one evaluation of a condition** that waiters and subscriptions share (API-10, API-11, NTF-4).

**Producer:** T02 writes `src/api/types.rs` (the types and their serde, `Until`'s words) and `src/api/mod.rs`; **T43 builds the shared parts**: `request.rs` (`Request`'s three methods), `params.rs`, `respond.rs` and `router.rs`; T41 writes the golden files. T22 builds `api::server`, T23 `api::wait`, **T44 `api::condition`**, T47 `api::notify`. **Consumers:** T10, T24, T27, T48 (the CLI decodes and renders these), T17 (sends `Request`), T28 (serves `router`), T46 (the notifier calls `condition::evaluate`).

## Declarations (`src/api/types.rs`)

```rust
/// API-3, in its table's order. `http()`: 400, 404, 404, 503, 503, 503, 503, 409, 409. `Limit` is STA-3's
/// places, API-14's waiters and NTF-2's subscriptions; `NoSession` is NTF-2's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ErrorCode { BadRequest, NotFound, UnknownRoute, Auth, Starting, GithubUnavailable, RateLimited, Limit, NoSession }
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
/// `POST …/notify` (NTF-2, API-19's row, in its order). Kept: `registered: true`, `expires_at`, `outcome`
/// and `reason` `null`, the record, the events since `since` (none without it). Already known:
/// `registered: false`, `expires_at: null`, and the rest as the `wait` it stands for. `replaced` either way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotifyBody { pub registered: bool, pub replaced: bool, pub expires_at: Option<Time>,
                        pub outcome: Option<Outcome>, pub reason: Option<Reason>, pub record: Option<Record>,
                        pub events: Vec<Event>, pub records: Vec<Record>, pub more_events: bool, pub reset: bool }
impl NotifyBody { /// The `wait` answer it stands for; `None` when `registered`. Built by T02.
                  pub fn as_wait(&self) -> Option<WaitBody>; }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CancelBody { pub cancelled: u32 }                     // `DELETE …/notify` (NTF-3)
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
    pub subscriptions: u32,                                      // NTF-9: open ones; `View::subscriptions`
    pub owners: Vec<String>,
    pub token_source: String,                                    // `gh`, `GH_TOKEN`, `GITHUB_TOKEN`
    pub gh_path: String,                                         // API-5: the `gh` that was run last, or as configured before any run
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
/// A condition in the client's own words, unchecked (API-10, D8): the CLI's flags, a `wait`'s query and a
/// `notify`'s body all carry these six, and `again:` and `Ask again:` print them back (CLI-6, NTF-8).
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub until: Until,
    pub sha: Option<String>, pub rerun_since: Option<String>,
    pub fail_fast: bool,                                         // `--fail-fast`, `fail_fast` (D8)
    pub kind: Option<String>,                                    // `--kind`, `kind`: the comma-separated list as given (D8)
    pub since: Option<String>,
}
impl Ask { pub fn new(until: Until) -> Ask; }                    // everything else unset
/// Every request the CLI sends. The one place paths, parameter names and body keys are spelt.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    List { all: bool, since: Option<String>, checks: bool },
    Show { pr: PrName, fresh: bool },
    Changes { pr: Option<PrName>, since: Option<String>, limit: Option<u32> },
    Wait { pr: PrName, cond: Ask, timeout: u32, limit: Option<u32> },
    /// NTF-2. `life` is `for`, sent only when `--for` was given; `token` is sent as a string or `null`.
    Notify { pr: PrName, cond: Ask, life: Option<u32>, socket: String, token: Option<String>, limit: Option<u32> },
    NotifyCancel { pr: PrName, socket: String },                 // NTF-3
    Track { pr: PrName }, Untrack { pr: PrName },
    Refresh, Status, Shutdown,
}
impl Request {
    pub fn method(&self) -> http::Method;
    /// `/v1/prs?state=all&since=…`, `/v1/prs/{owner}/{repo}/{number}/wait?until=…&timeout=…` and so on,
    /// exactly as 04-api writes them; a flag is sent only when set, as `1`. A `wait`'s query holds `until`,
    /// then `sha`, `rerun_since`, `fail_fast`, `kind` and `since` as set, then `timeout` and `limit`.
    /// `NotifyCancel` is `DELETE /v1/prs/{owner}/{repo}/{number}/notify?socket=…`.
    pub fn path_and_query(&self) -> String;
    /// `track`: `{"pr":"owner/repo#12"}`. `notify`: one JSON object with `until`, then `sha`, `rerun_since`,
    /// `fail_fast`, `kind`, `since`, `for` and `limit` only as set, then `socket` and `token`. In a body
    /// `fail_fast` is a JSON boolean and `kind` one comma-separated string (NTF-2).
    pub fn body(&self) -> Option<String>;
}
pub const CURSOR_HEADER: &str = "circitor-cursor";               // API-9; header names compare without case
```

## Declarations (`src/api/condition.rs`, T44): a condition, checked and evaluated

Pure: no clock, no store, no I/O. **One function evaluates for the waiters (T23), for the notify route's one evaluation (T47) and for the notifier's loop (T46).** Nobody else reads API-10 or API-11.

```rust
/// MOD-15's kinds a `kind` may name (API-10): all but `tracked` and `untracked`, in MOD-15's order.
pub const KIND_WORDS: [&str; 12] = ["status", "head", "checks", "merged", "closed", "reopened", "gone",
                                    "draft", "mergeable", "review", "activity", "edited"];
/// A condition, checked (API-10). `ask` is kept as given, for the texts.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub ask: Ask,
    pub since: Option<Cursor>, pub rerun_since: Option<Cursor>,
    pub kinds: Option<Vec<&'static str>>,                        // of `KIND_WORDS`, in the order given
}
impl Condition {
    /// API-10's checks, once: `sha` is 7 to 40 hex characters; `sha`, `rerun_since` and `fail_fast` go
    /// with `checks-done` only and `kind` with `changed` only; every word of `kind` is one of
    /// `KIND_WORDS`, and there's at least one; the two cursors are in STA-9's form. `Err` is
    /// `bad_request` naming the parameter (API-1).
    pub fn check(ask: Ask) -> Result<Condition, ApiError>;
}
/// What one wait or one subscription remembers between evaluations.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub began: Mono,                         // the request's arrival: `head_moved`'s 60 s run from it (API-11, NTF-4)
    pub baseline: Option<u64>,               // `changed` without `since`: the record's `rev` at the first evaluation that saw one
    pub sha_matched: bool,                   // `sha` matched `head_sha` once
    pub looked: Option<u64>,                 // the view's `rev` at the last evaluation: events up to it have been looked at
    pub held_at_once: bool,                  // the point was too old at the first evaluation (STA-9): `changed` holds
    pub kind_seen: bool,                     // an event of a named kind was recorded after the point
    pub rerun_seen: bool,                    // a `checks` event was recorded after `rerun_since`
    pub rerun_dropped: bool,                 // `rerun_since` was too old at the first evaluation: that part is dropped
}
impl Progress { pub fn new(began: Mono) -> Progress; }
pub const HEAD_MOVED_AFTER: Duration = Duration::from_secs(60);  // API-11
/// API-10 and API-11 on one view, for the PR `id`. `None`: neither the condition nor an end holds.
/// The condition is checked before the ends. Never `Outcome::Timeout`: a deadline is its caller's.
pub fn evaluate(cond: &Condition, view: &View, id: &NodeId, progress: &mut Progress) -> Option<(Outcome, Option<Reason>)>;
```

- **No record, no answer.** A PR the view has no record for (`Loaded`, STA-8) gives `None`, and `baseline` stays unset: its first evaluation with a record sets it.
- **`changed` without `since`** sets `baseline` at the first evaluation that sees a record and answers `None` then: it waits for the next change (API-10). Callers make that first call after the arrival refresh (API-9, NTF-2).
- **Events are looked at once, and remembered (NTF-4).** The first evaluation that sees a record judges the cursors on that view: a `since` the history no longer covers (`View::too_old`) sets `held_at_once`, with or without `kind`; a too-old `rerun_since` sets `rerun_dropped`, and the answer then says `reset: true` (API-10). It then reads this PR's events after the point (`since`, or `baseline`) through `View::events_after`, and every later evaluation reads only those after `looked`. What it finds stays in `kind_seen` and `rerun_seen` (`EventKind::word`, `model.md`). **So a cursor that goes too old while a wait or a subscription is held neither satisfies it nor drops a part**, and a subscription that lasts a day never searches a history a day can outlast (STA-10). For a wait, which lasts an hour at most, this answers exactly as API-10 writes it.
- **`fail_fast`** (plan decision P15: API-10 doesn't say whether `checks.settled` still has to hold): with `checks.failed` above 0 the first part **and `settled`** count as held; `sha` and `rerun_since` still have to hold. Without a failure it's `checks-done` as ever.
- **The ends**, in API-11's order of telling: a state's reason (`merged`, `closed`, `gone`) before `head_moved`. `merged` ends `checks-done`, and `changed` with `kind` when `merged` isn't named; `closed` and `gone` end `merged`, `checks-done`, and `changed` with `kind` when theirs isn't named. `head_moved` reads `view.get(id).timing.fetched` against `progress.began`: never the clock.

## Declarations (`src/api/mod.rs`, `router.rs`, `params.rs`, `respond.rs`)

```rust
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>, pub poller: Arc<dyn PollerHandle>, pub flush: Arc<dyn Flush>,
    pub notify: Arc<dyn NotifyHandle>,                           // `notify.md`; `NoNotify` in `test_state`
    pub clock: SharedClock, pub sleep: Arc<SleepWatch>, pub activity: Activity,
    pub logger: Logger, pub warnings: Warnings, pub config: Arc<Config>,
    pub info: Arc<DaemonInfo>, pub stop: StopHandle,
}
#[derive(Clone, Debug)]
pub struct DaemonInfo { pub version: String, pub mode: Mode, pub started_at: Time, pub paths: PathsInfo }

/// `server::routes()` merged with `wait::routes()` and `notify::routes()`, the `unknown_route` fallback, and the layer below.
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

T02 writes `server::routes()`, `wait::routes()` and `notify::routes()` as stubs returning an empty `Router<AppState>`; T22, T23 and T47 fill them. None touches `src/api/mod.rs`. `present` calls `poller::cadence::stale`, which T09 builds: until T09 is merged it's T02's stub returning `false`.

**What T43 can and can't test.** It has no working `Store` (T15), so no `AppState`: `ok`, `ok_at`, `envelope` and `error` are built over inner functions that take the version, the daemon block and the cursor, and those are tested; `window` is tested over hand-built `View`s (its fields are public); `daemon_block`, `refusal`, `pr_from_path` and `Params` need no state. `present`, the layer and the fallback are tested through `router` by T22. A fault T22 or T23 finds in these files is reported; the orchestrator fixes it as a follow-up to T43.

## Semantics

- **Anything that isn't a route of 04-api is `unknown_route`** (API-3): an unknown path, and a known path with another method. `router` sets both the fallback and the method-not-allowed fallback, so no route answers a bare 405.
- **`router`'s layer**, on every request: takes `activity.begin()` and puts an `InFlight` in the request's extensions; calls `sleep.check()` (POL-22). It does **not** call `poller.client_request()`: each handler does, first thing, except `status` (API-5) and `shutdown`. The two notify routes do: registering is a client asking.
- **A handler parks around every await that can outlast the present instant** (`clock.md`): a held waiter, API-18's wait for the first poll, and every call of `read_through`, `track`, `refresh_pr` and `poll_now`, because the engine may be waiting for POL-2's gap while the handler waits for it. `park()` before the await, `resume()` after. Without it the test clock never settles and `advance` hangs.
- **`shutdown` answers, then stops (API-17).** A handler can't see its answer being written: T22's response body asks for the stop when it's dropped, which is after `hyper` has taken its last byte; T28's stop must not cut bytes already handed to the socket.
- **A body is one line of JSON with no newline at the end.** CLI-4's `--json` prints it and then one newline.
- **Values are percent-encoded on the wire.** `Request::path_and_query` encodes every parameter value and path segment; `Params::parse` decodes. No crate is needed for it.
- **One view per answer (MOD-13).** A handler takes `store.view()` once, after its last change to the store, and builds records, events and the cursor from it.
- **Unknown input is `bad_request` naming it (API-1):** a parameter (through `Params::finish`), a body key (`#[serde(deny_unknown_fields)]` on the server's own body types, with the error mapped to `bad_request`), a value. The shared response types above are **not** `deny_unknown_fields`: a CLI reads a newer daemon's answers and ignores what it doesn't know (CLI-9).
- **Lists and flags are never left out (API-19).** `Option` is `null`, never absent, except `checks.items` (API-6) and an item's review-only keys (see `model.md`).
- **`checks.items` (API-19).** In every single `record` (`show`, `wait`, `notify`, `track`, `untrack`): `present(…, true)`. In a list of `records` only for `list` with `checks=1`: **a reset's `records`, of `changes`, `wait` or `notify`, leave them out**, `present(…, false)`.
- **The notify routes (NTF-2, NTF-3; T47).** `POST …/notify` takes a JSON body, strict (`deny_unknown_fields` on T47's own type): `until` and `socket` are required, the rest optional. **NTF-2's order:** the parameters; `state.notify.look(socket)`, `no_session` unless it's a socket of the user's, so a bad socket never reaches GitHub and gets `no_session` even when the answer would be known; API-9's arrival without a waiter (a `Loaded` PR with no record is read through, unlike a wait's); one evaluation through `condition::evaluate` on a view `V`; then either it answers as the wait would, `registered: false`, with `replaced: true` when `state.notify.drop_same` ended an earlier one (nothing posted), or, with no outcome, answers `auth` while `poller.status().problem` is `auth`, else hands a `notify::NewSubscription` to `state.notify.register` (the limits, the entry, the confirmation) and answers `registered: true`. **Every answer is built from `V`** (MOD-13), the registered one too: its record, events and cursor, which is the subscription's (NTF-2). **A `since` too old on that view** answers with `reset: true` and is let go: the subscription's cursor is the view's (NTF-2). **It names its PR with `store.name_quiet`, never `store.name`, and never calls `store.add_waiter` or `poller.nudge`:** the notify routes name their PR for STA-1's 60 minutes only (STA-1, cited by POL-11, POL-14 and NTF-2), and a subscription doesn't make its PR hot (NTF-5). `DELETE …/notify?socket=…` is `state.notify.cancel` with the name given and the node ID `store.lookup` finds, if any; it reads nothing through and never answers `starting` (NTF-3, API-18). T47's brief has the order line by line.
- **`wait` answers in two steps (API-9):** the status line and headers, with `circitor-cursor`, as soon as the waiter is registered; the body when the wait ends. A refusal before that is an ordinary error response. T23 owns how; T17 reads the head and the body apart.
- **Golden files, `testdata/api/`** (T41 writes them by hand from 04-api and 01-model; its tests keep types and files equal, key order included): `list.json`, `list-reset.json`, `show.json`, `changes.json` (one event of every kind), `wait-satisfied.json`, `wait-timeout-no-record.json`, `wait-reset.json` (a reset: `records` holds the record without `checks.items`), `notify-registered.json`, `notify-already.json` (a failed `checks-done`, `registered: false`), `notify-cancelled.json`, `track.json`, `untrack-null.json`, `refresh.json`, `status.json`, `status-problem.json` (a problem, an `until`, two warnings), and one `error-<code>.json` per `ErrorCode`, `error-no_session.json` among them. The CLI's render tests (T10) and the routes' tests (T22, T23) both read them.

## The fake

The golden files are the fake server for T10, and the fake client for T22 and T23. `FakePoller`, `NoFlush` and `NoNotify` stand in behind `AppState`; `api::test_state(clock: Arc<TestClock>) -> (AppState, Arc<FakePoller>)` (T43, test builds only; the state's `activity` is the test clock's, so `settle` sees a handler's work) builds one over a real, empty `Store`. **T47's tests put a real `Notifier` over a `RecordingPoster` in `state.notify`** (`AppState` is `Clone`, its fields public): nobody else needs one. It needs T15 to do anything, so T43 can't test `router` itself: **T22 tests the `unknown_route` fallback and the layer through `router`.**

## Don't assume

- That the CLI and the daemon are the same version (CLI-9). Only `StatusLite` and `POST /v1/shutdown` are promised across versions (API-2).
- That a handler's future runs to its end: the server drops it when the client disconnects (API-14).
