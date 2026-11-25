# Seam: the GitHub client and the token source

**Purpose.** Everything circitor asks GitHub goes through one trait with one method, so "one request" (POL-2), "never a mutation" (POL-1) and "talks to nothing else" (OPS-16) each have one place; and the token comes from one trait (OPS-5).

**Producer:** T02 declares both traits and their types; T43 builds the two simple fakes. T07 builds the real client (`github::client`), T12 the real token source (`token::gh`). **Consumers:** T21 and T25 (the engine sends), T13 (reads the token), T26, T28, T29.

## Declarations (`src/github/`)

```rust
/// One HTTP request to GitHub's GraphQL endpoint. A poll's request holds `find` and `nodes` together (POL-4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// Finding (POL-4): one page of `viewer.pullRequests(states: OPEN, first: 100, orderBy: UPDATED_AT desc)`
    /// with `id`, `number`, `updatedAt` and the repository's owner and name, and `viewer { login }`.
    pub find: Option<Find>,
    /// By node ID, through `nodes(ids:)` and shared fragments (POL-4, POL-7). At most 40.
    pub nodes: Vec<NodeQuery>,
    /// Reading through (POL-16): one PR by owner, repository and number, as `Want::RecordAndItems`.
    pub lookup: Option<PrName>,
}
#[derive(Clone, Debug, Default, PartialEq)] pub struct Find { pub after: Option<String> }
#[derive(Clone, Debug, PartialEq)] pub struct NodeQuery { pub id: NodeId, pub want: Want }
#[derive(Clone, Debug, PartialEq)]
pub enum Want {
    Record,                         // POL-6: the full record, checks `contexts(first: 100)`
    RecordAndItems,                 // a first refresh (POL-4): the record plus POL-12's fields
    Items,                          // POL-12: the last 20 comments and the last 20 reviews, nothing else
    ChecksPage { after: String },   // POL-8: the next page of the last commit's checks, nothing else
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub rate: Option<Rate>,         // the `rateLimit` block; `None` if GitHub left it out
    pub date: Option<Time>,         // the `Date` header (MOD-6)
    pub found: Option<Found>,       // `Some` iff the query had `find` and that part came back
    pub nodes: Vec<NodeAnswer>,     // one per `Query::nodes`, same order
    pub lookup: Option<LookupAnswer>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Rate { pub cost: u32, pub remaining: u32, pub reset_at: Option<Time> }
#[derive(Clone, Debug, PartialEq)]
pub struct Found { pub login: String, pub prs: Vec<FoundPr>, pub next: Option<String> }
#[derive(Clone, Debug, PartialEq)]
pub struct FoundPr { pub id: NodeId, pub pr: PrName, pub updated_at: Option<Time> }
#[derive(Clone, Debug, PartialEq)]
pub enum NodeAnswer {
    Record { read: Box<PrRead>, items: Option<ItemsRead> },   // `items` iff `RecordAndItems`
    Items(ItemsRead),
    ChecksPage(ChecksRead),
    /// The node is `null` with a `NOT_FOUND` error on its path (POL-21).
    NotFound,
    /// Any other error whose path points at or inside the node, or a node of another type (POL-21, POL-24).
    /// `type_` is GitHub's `type`, as written, or a word of the client's own (`UNLISTED_STATE`, `NOT_A_PULL_REQUEST`).
    Error { type_: String },
}
#[derive(Clone, Debug, PartialEq)]
pub enum LookupAnswer {
    Found { read: Box<PrRead>, items: ItemsRead },
    NotFound,                       // `NOT_FOUND` for the repository or the PR (POL-16)
    Error { type_: String },
}

/// A request that failed as a whole. Classified in this order (POL-25 before POL-19 and POL-24).
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    Unauthorized,                                 // HTTP 401 (POL-20)
    Exhausted { reset_at: Option<Time> },         // POL-25: no `data`, and `x-ratelimit-remaining: 0`
    Secondary { retry_after: Option<Duration> },  // POL-25: a 403, or a 200 with an error, naming a secondary rate limit
    Unavailable { kind: Unavailable, status: Option<u16> },   // POL-19, POL-24
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable { Network, Http, NoData, BadBody }       // a timeout is the caller's (the gate, POL-2)

#[async_trait::async_trait]
pub trait GitHub: Send + Sync + 'static {
    /// Sends one request and decodes its answer. No retry, no sleep, no timeout of its own.
    /// Dropping the future abandons the request.
    async fn send(&self, token: &Token, query: &Query) -> Result<Answer, Failure>;
}
```

## Declarations (`src/token/`)

```rust
/// The GitHub token. No `Display`, no `Serialize`; `Debug` prints `Token(…)`.
#[derive(Clone)] pub struct Token(/* String */);
impl Token {
    pub fn new(value: String) -> Token;
    pub fn expose(&self) -> &str;               // for the `Authorization` header only
    pub fn same_as(&self, other: &Token) -> bool;
}

/// "The token source in use" (OPS-5). `as_str`: `gh`, `GH_TOKEN`, `GITHUB_TOKEN` (API-5's `token_source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum TokenSourceKind { Gh, GhTokenVar, GithubTokenVar }

#[derive(Clone, Debug)]
pub enum TokenRead {
    Token(Token),
    NoLogin,                                    // non-zero exit, stderr holds `no oauth token found`
    Failed(GhFailure),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhFailure { CantRun, Exit(i32), Killed, Timeout, NoOutput }   // never `gh`'s output

#[async_trait::async_trait]
pub trait TokenSource: Send + Sync + 'static {
    /// One read of `gh auth token --hostname github.com` (OPS-5), within 10 s of the clock.
    /// Concurrent calls share one run.
    async fn read(&self) -> TokenRead;
    fn kind(&self) -> TokenSourceKind;          // from the daemon's own environment, fixed at start
    /// The `gh` that was run last, or `gh_path` as configured before any run (API-5, API-3's `auth` message).
    fn gh_path(&self) -> String;
}
```

## The real ones' constructors (`src/github/client/`, T07; `src/token/gh/`, T12)

```rust
pub struct Client { /* a reqwest client and the endpoint */ }
impl Client {
    /// A release build: `https://api.github.com/graphql`, and `env` isn't read. A test build (TST-1):
    /// `CIRCITOR_TEST_GITHUB` from `env`, loopback only; `Err` names the variable or the host.
    pub fn from_env(env: &Env, version: &str) -> Result<Client, EndpointError>;
    #[cfg(any(test, feature = "test-endpoint"))]
    pub fn for_tests(url: &str) -> Client;                       // the fake GitHub's `client()`
}
// impl GitHub for Client
/// The POST body `send` would send for `query`: `{"query": …, "variables": …}`. T37 records with these.
pub fn request_body(query: &Query) -> serde_json::Value;

pub struct GhTokenSource { /* … */ }
impl GhTokenSource {
    /// `gh_path` as configured (OPS-2); `env` is the daemon's own: it gives `PATH`, decides `kind()`, and is the child's environment.
    pub fn new(gh_path: &str, env: Env, clock: SharedClock, activity: Activity) -> Arc<GhTokenSource>;
}
// impl TokenSource for GhTokenSource
```

## Semantics

- **`Client::for_tests` panics on any host but the loopback** (TST-1).
- **A test build's client sends `x-circitor-test-pid: <its process ID>` with every request** (TST-1), behind `cfg(feature = "test-endpoint")`: the fake GitHub tells real processes apart by it (TST-3, T20). A release build sends no such header.
- **A fixture file** (`testdata/github/responses/`, `testdata/github/recorded/`) is one JSON object: `{"request": {"find": …, "nodes": […], "lookup": …}, "status": 200, "headers": {…}, "body": …}`, the request part saying which `Query` the response answers, so the decode test can decode it as that query's answer. `headers` holds only `date`, `retry-after` and the `x-ratelimit-*` ones.
- **Who calls `send`:** only the engine's gate (T21), one request at a time, with POL-2's 1 s gap and 20 s timeout around it. The client itself is stateless between calls.
- **Partial errors (POL-21).** A response with `data` and `errors` is an `Ok(Answer)`. A node an error's path points at or inside is `NotFound` or `Error`; the other nodes are returned. An error on `viewer` makes `found: None`. An error on the `rateLimit` block alone makes `rate: None`.
- **`Rate.reset_at`** is `rateLimit.resetAt`, or else the `x-ratelimit-reset` header. An `Ok` answer with `remaining == 0` is applied; the engine then treats the daemon as exhausted (POL-25).
- **`Failure::Unavailable`** covers a refused or dropped connection and TLS or DNS failure (`Network`), any status but 200, 401 and the rate-limit cases (`Http`, with the status; a redirect is one, never followed), a 200 without `data` and without a rate-limit error (`NoData`), and a body that isn't JSON or is cut short (`BadBody`).
- **Mapping and lowering are the client's** (MOD-2, MOD-3): `PrRead` already holds `yes`, `in_progress`, `skipped`. An unlisted enum value passes through lowered; an unlisted PR `state` makes that node `Error { type_: "UNLISTED_STATE" }`; a node that can't be decoded (a field missing, `null` with no error) `UNDECODABLE`; an error with no `type` `UNTYPED` (MOD-2). None of them fails the whole answer.
- **`TokenRead::Token`** means exit 0 and one non-empty line. Exit 0 with nothing on stdout is `Failed(NoOutput)`.
- **The token crosses this seam only inside `Token`.** `send` puts it in the `Authorization` header and nowhere else.

## The fakes (T43)

- **`github::fake::ScriptedGitHub`**: `new(handler: impl FnMut(&Query) -> Result<Answer, Failure> + Send + 'static)`; `calls() -> Vec<Query>`; `hold(activity)` makes the next `send` wait until the returned `Release` is dropped (for in-flight tests), and excuses that request from the work in flight meanwhile (`clock.md`). Good for unit tests of the engine and the daemon. It doesn't check queries against a schema, compute costs, or enforce POL-2: the fake GitHub server (T14, T20) does.
- **`github::fake::answers`**: helpers that build a plain `PrRead`, `Answer` and `Rate` for tests (`pr_read("o/r#1")`, `answer_with(nodes)`).
- **`token::fake::FakeTokenSource`**: `new(clock)`; `set(TokenRead)`; `set_delay(Duration)` (slow, on the clock); `reads() -> usize`. Its default read is `Token(MARKER_TOKEN)`; `kind()` is `Gh` and `gh_path()` is `gh` unless set.

## Don't assume

- That `Answer.nodes` has fewer entries than the query: it's always one per `NodeQuery`.
- That the fake GitHub server shares these types. It builds JSON itself from docs/02 and the schema, so a wrong field name in T07 fails a test (T26) instead of agreeing with itself.
- Any GitHub behaviour not in `docs/02-github-api.md`. The exhausted and secondary-limit answers are `assumed` there (§6 item 6); POL-25 says what to recognise.
