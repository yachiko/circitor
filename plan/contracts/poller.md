# Seam: the poller

**Purpose.** Two seams in one file. **Outside:** `PollerHandle`, what the API, the waiters and the daemon ask of the poller (POL-13 to POL-16, API-13, API-16, API-18, API-4, API-5). **Inside:** the three parts the engine is built from, each a task of its own: limits (POL-17 to POL-19, POL-23, POL-25), cadence (POL-10, POL-13 to POL-15, MOD-16) and the token's life (POL-3, POL-20).

**Producer:** T02 declares `PollerHandle`, its types, `Tier`, `QuotaState` and `Policy`; T43 builds `FakePoller`. T08 builds `poller::limits`, T09 `poller::cadence`, T13 `poller::auth`, T21 the engine and its poll, T25 the engine's demand side. **Consumers:** T22, T23 (through `PollerHandle`), T28 (wiring), T26.

## Outside: `src/poller/mod.rs`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Tier { Active, Idle }                                   // POL-13; hot isn't a tier

/// `daemon.problem` (API-4), in its order of precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ProblemCode { Auth, RateLimited, GithubUnavailable, QuotaLow }
#[derive(Clone, Debug, PartialEq)]
pub struct DaemonProblem { pub code: ProblemCode, pub message: String, pub until: Option<Time> }

/// Why something that needs GitHub wasn't done. Each is one API-3 error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalCode { NotFound, Auth, GithubUnavailable, RateLimited, Limit }
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal { pub code: RefusalCode, pub message: String, pub until: Option<Time> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refreshed {
    Applied,      // sent, answered and applied
    Failed,       // sent, and it failed
    NotSent,      // POL-15's window, POL-19's backoff, `auth`, or a quota state that sends none (API-9: failed at once)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum PollOutcome { Polled, Shared, Auth, GithubUnavailable, RateLimited }      // API-16's `outcome`
#[derive(Clone, Debug, PartialEq)] pub struct PollReport { pub outcome: PollOutcome, pub cost: u32 }

#[derive(Clone, Debug, PartialEq)]
pub enum FirstPoll {
    Running,
    /// API-18: its finding and every first refresh have been applied or have failed.
    Finished,
    /// Finished, but none of its requests was answered (POL-19): the routes answer this error until one is.
    Unanswered(Refusal),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollerStatus {
    pub tier: Tier,
    pub last_poll_at: Option<Time>,                              // the last poll applied (API-4)
    pub last_poll: Option<(Time, PollOutcome)>,                  // API-5's `last_poll`
    pub problem: Option<DaemonProblem>,
    pub cost_last_hour: u32,
    pub remaining: Option<u32>, pub reset_at: Option<Time>, pub login: Option<String>,   // API-5's `github`
    pub token_source: TokenSourceKind, pub gh_path: String,
}

#[async_trait::async_trait]
pub trait PollerHandle: Send + Sync + 'static {
    /// A client request arrived (POL-13). Every route but `status` and `shutdown` calls it first. Moves the tier to
    /// active, starts a poll if one is due, and runs POL-22's check. Never waits.
    fn client_request(&self);
    /// Monotonic time of the last `client_request`, or of the start (OPS-8).
    fn idle_since(&self) -> Mono;
    /// POL-16: fetch an untracked PR by name, track it as `asked`, apply its first refresh. `Ok` holds its ID.
    /// Also for a loaded PR with no record (STA-8), which keeps its saved source.
    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal>;
    /// API-15 `track`: source `manual`, and a read-through when there's no record. A loaded PR's saved
    /// source is set first, so it's kept when the read-through fails. `Ok` holds its ID.
    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal>;
    /// POL-15: refresh one tracked PR by ID now (`fresh`, a wait's arrival), sharing one in flight.
    async fn refresh_pr(&self, id: &NodeId) -> Refreshed;
    /// API-16: a whole poll now. Answers when it's done; never waits out a backoff or a reset.
    async fn poll_now(&self) -> PollReport;
    /// The hot set may have changed (a waiter came or went): look again.
    fn nudge(&self);
    fn first_poll(&self) -> tokio::sync::watch::Receiver<FirstPoll>;
    fn status(&self) -> PollerStatus;
}
```

- **Every async method may be dropped half-way** (the client went away). The request it started goes on and is applied; nothing is left half-done.
- **None of them outlives the engine's own limits:** one waits at most for the request in flight (POL-15), its own request's 20 s (POL-2), and its check and item pages. The API's and the CLI's limits are on top (API-12, CLI-7).
- **`read_through` and `track` at STA-3's limit** return `Limit` before sending anything. In the `auth` state, `Auth`; exhausted or under a secondary limit, `RateLimited` with `until`; unreachable or another error `type`, `GithubUnavailable`, its message naming the type and the token source (POL-16, API-3). **The message never holds GitHub's text.**
- **`client_request` in the `auth` state** (POL-20, TST-8 `first-run`): when the last token read is 5 s old, it sets a `first_poll` of `Unanswered` back to `Running` before it returns, and the loop reads the token again. No new token: `Unanswered` again at once. A new one: a poll, then `Finished`. So a `list` straight after `gh auth login` waits for that poll, and one with no login still answers `auth` as soon as `gh` has.
- **`refresh_pr` joins a request in flight that holds the PR**, a poll's included, before it looks at POL-15's window. **`poll_now` is served while capped**, in the 30 s window: `Policy::poll_every` is `None` there, `demand` is true.
- **`status()` is a cheap copy** of what the engine last published. It never waits, also during the first token read (OPS-5).

## Inside: limits (`src/poller/limits/`, T08)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaState { Normal, Slowed, Reserve, Capped, Exhausted, Secondary }     // POL-23
/// POL-23's row, worked out for a tier.
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    pub state: QuotaState,
    pub poll_every: Option<Duration>,        // `None`: no polls
    pub hot_every: Option<Duration>,         // `None`: no hot refreshes
    pub demand: bool,                        // forced refreshes, read-throughs and `track` are sent
    pub pr_window: Duration,                 // POL-15: 5 s, or 30 s
    pub poll_window: Duration,               // POL-15: 10 s, or 30 s
    pub follow_up: bool,                     // POL-11's request is sent
}
// `QuotaState` and `Policy` are declared in `src/poller/mod.rs` by T02, so T09 can use them.

pub struct Quota { /* … */ }
impl Quota {
    pub fn new(cfg: &QuotaConfig) -> Quota;
    pub fn spent(&mut self, cost: u32, at: Time);                                 // POL-17
    /// Every answer's `rateLimit` block (POL-18). `remaining == 0` is exhausted from then on (POL-25).
    pub fn observed(&mut self, remaining: u32, reset_at: Option<Time>, at: Time);
    pub fn exhausted(&mut self, reset_at: Option<Time>, at: Time);                // `Failure::Exhausted`
    pub fn secondary(&mut self, retry_after: Option<Duration>, at: Mono, at_wall: Time, idle_interval: Duration);
    pub fn answered(&mut self);              // any request that succeeds: the run of secondary limits ends
    pub fn state(&self, wall: Time, mono: Mono) -> QuotaState;
    pub fn policy(&self, wall: Time, mono: Mono, tier: Tier, cfg: &Config) -> Policy;
    pub fn cost_last_hour(&self, wall: Time) -> u32;
    pub fn capped(&self, wall: Time) -> bool;        // `quota_capped`'s own condition
    pub fn in_reserve(&self) -> bool;                // `quota_reserve`'s own condition
    pub fn remaining(&self) -> Option<u32>;
    pub fn reset_at(&self) -> Option<Time>;
    /// When sending resumes, while exhausted or under a secondary limit (API-4's `until`).
    pub fn resume_at(&self, wall: Time, mono: Mono) -> Option<Time>;
}
pub struct Backoff { /* … */ }               // POL-19
impl Backoff {
    pub fn new() -> Backoff;
    pub fn failed(&mut self, at: Mono, at_wall: Time);   // 5 s, doubling to 5 min
    pub fn succeeded(&mut self);
    pub fn woke(&mut self, at: Mono, at_wall: Time);     // POL-22: starts again at 5 s, if it was running
    pub fn active(&self) -> bool;
    pub fn next_try(&self) -> Option<Mono>;
    pub fn next_try_wall(&self) -> Option<Time>;         // API-4's `until`
    pub fn take_read_through(&mut self) -> bool;         // POL-16: one per step
}
```

Pure: no clock inside, no I/O. The caller passes the time. The sliding hour, the reset time and a wall clock set back follow POL-26.

## Inside: cadence (`src/poller/cadence/`, T09)

```rust
pub fn tier(last_request: Option<Mono>, now: Mono, cfg: &Config) -> Tier;                    // POL-13
pub fn hot(status: Option<Status>, timing: &Timing, now: Mono) -> bool;                      // POL-14
/// POL-10's table, with the configured values (MOD-16 needs those, not slowed ones).
pub fn period(class: Class, source: Source, hot: bool, tier: Tier, cfg: &Config) -> Duration;
/// MOD-16. A `gone` record is never stale.
pub fn stale(record: &Record, timing: &Timing, tier: Tier, cfg: &Config, now: Mono) -> bool;
pub fn poll_due(last_poll_sent: Option<Mono>, policy: &Policy) -> Option<Mono>;              // `None`: no polls; never polled: `Mono::ZERO`, due at once
/// Which tracked PRs a poll built at `now` refreshes (POL-4, POL-10, POL-13). `listed` is `None` for the
/// poll's first request, and the open list once it's read: a listed PR is due as an open one (POL-10).
/// A quota state changes when a poll is sent, never what a sent poll carries, so there's no `Policy` here.
pub fn in_poll(targets: &[Target], listed: Option<&HashSet<NodeId>>, tier: Tier, cfg: &Config, now: Mono) -> Vec<NodeId>;
pub struct HotDue { pub now: Vec<NodeId>, pub next: Option<Mono> }
pub fn hot_due(targets: &[Target], policy: &Policy, now: Mono) -> HotDue;                    // POL-14
pub fn may_force_pr(timing: &Timing, policy: &Policy, now: Mono) -> bool;                    // POL-15's window
pub fn may_force_poll(last_poll_sent: Option<Mono>, policy: &Policy, now: Mono) -> bool;
/// POL-11: whether a refresh that read `UNKNOWN` for this open PR gets one follow-up.
pub fn wants_follow_up(target: &Target, first_since_start: bool, policy: &Policy, now: Mono) -> bool;
pub fn batches<T: Clone>(items: &[T]) -> Vec<Vec<T>>;                                        // POL-7: 40 a request
```

Pure. `Target`, `Timing` and `Class` are `state::store`'s (see `state.md`).

## Inside: the token's life (`src/poller/auth/`, T13)

```rust
#[derive(Clone, Debug, PartialEq)] pub struct AuthProblem { pub message: String }   // API-3's `auth` message
pub enum AfterRefusal { Retry(Token), InAuth(AuthProblem) }

pub struct Auth { /* the token in hand, the token refused, the read times */ }
impl Auth {
    pub fn new(source: Arc<dyn TokenSource>, clock: SharedClock, logger: Logger) -> Auth;   // no `Activity`: the real source counts `gh` itself (clock.md)
    /// The token to send with. Runs the first read if none has ended: everything that needs GitHub
    /// waits for it (OPS-5). In `auth`, with `demand`, reads again if the last read is 5 s old (POL-20).
    pub async fn token(&mut self, demand: bool) -> Result<Token, AuthProblem>;
    /// A request sent with `refused` got a 401 (POL-20). First time: reads again, `Retry`. When the
    /// repeat (`was_retry`) or a try in `auth` gets one: `InAuth`.
    pub async fn unauthorized(&mut self, refused: &Token, was_retry: bool) -> AfterRefusal;
    /// A request sent with `used` succeeded: `auth` ends.
    pub fn accepted(&mut self, used: &Token);
    /// When the next periodic read is due: 10 min after the last, 60 s in `auth` (POL-3, POL-20).
    pub fn due(&self) -> Mono;
    /// The periodic read. `true` when there's now a token worth one try (another than the one refused,
    /// or the same after 10 minutes).
    pub async fn periodic(&mut self) -> bool;
    pub fn problem(&self) -> Option<AuthProblem>;                // `Some` while in `auth`, the one try included, until `accepted`
    /// When the last read ended; `None` before the first has. The engine publishes it for `client_request` (POL-20's 5 s).
    pub fn last_read_ended(&self) -> Option<Mono>;
    pub fn first_read_done(&self) -> bool;
}
```

The engine owns the one `Auth` and calls it from its loop. A read takes up to 10 s of the clock (OPS-5); `status()` doesn't wait for it, because the engine publishes its status before it awaits.

## Inside: the engine (`src/poller/engine/`, T21 then T25)

```rust
pub struct EngineDeps {
    pub clock: SharedClock, pub github: Arc<dyn GitHub>, pub token_source: Arc<dyn TokenSource>,
    pub store: Arc<Store>, pub config: Arc<Config>, pub logger: Logger, pub warnings: Warnings,
    pub activity: Activity, pub sleep: Arc<SleepWatch>,
}
pub struct Engine { /* … */ }
impl Engine {
    /// Starts the loop: the first token read, then the first poll (STA-11's order is T28's: this is called after the socket is bound).
    pub fn spawn(deps: EngineDeps) -> Arc<Engine>;
    /// OPS-7: stops polling, drops a request in flight, kills a running `gh`. Returns when the loop has ended.
    pub async fn stop(&self);
}
// impl PollerHandle for Engine
```

- **A poll's requests all count as sent when the poll was** (plan decision P10). `store.mark_sent` gets the send time of the poll's first request for every PR the poll refreshes, in whichever of its requests. Otherwise a PR refreshed in a poll's second request falls due a second after the next poll's first request is built, and every poll takes two requests for good (POL-13, TST-6). `ReadMeta` keeps each request's own send times: settling and "a newer read wins" go by those.
- **The tier at start is idle:** `cadence::tier` gets `None` until the first `client_request` (POL-13). `idle_since()` answering the start time is for OPS-8 only.
- **`poll_now` while a poll is in flight joins it:** it answers when that poll is done, with `Shared` and cost 0 (API-16).
- **One loop, one request at a time (POL-2).** The loop owns the gate (the 1 s gap, the 20 s timeout, the request numbers and send times of MOD-2), `Quota`, `Backoff` and `Auth`. Commands from `PollerHandle` reach it over a channel; a forced refresh, a read-through and a `track` go next, between a poll's requests (POL-15).
- **T21 builds** the loop, the gate, the poll (POL-4 to POL-8, POL-10, POL-12), whole-request errors (POL-19, POL-24, POL-25), a node's error as far as `store.attempt_failed` (POL-21's "keeps its last values and is stale"), the tick and the sleep reaction (POL-22, POL-26), `client_request`, `idle_since`, `poll_now`, `nudge`, `first_poll` and `status`. Its `read_through`, `track` and `refresh_pr` answer `GithubUnavailable` and `NotSent`, marked `// T25`.
- **T25 builds** hot refreshes (POL-14), forced refreshes and their sharing (POL-15), reading through (POL-16), `track`, and POL-11's follow-up.
- **T42 builds** what a PR's errors add up to: the `NOT_FOUND` run that makes a PR `gone` under MOD-11's conditions (POL-21), `pr_failing` after 5 minutes of any error, and the one log line while a PR's error lasts (POL-24).

## The fake (T43)

**`poller::fake::FakePoller`**: implements `PollerHandle` for route and waiter tests. `set_status(PollerStatus)`, `set_first_poll(FirstPoll)`, `on_refresh(impl FnMut(&NodeId) -> Refreshed)`, `on_read_through(impl FnMut(&PrName) -> Result<NodeId, Refusal>)` (the test's closure fills the real store through `reserve` and `apply`), `on_track(…)`, `on_poll(impl FnMut() -> PollReport)`, and counters for each call. Its defaults: tier active, no problem, first poll `Finished`, `refresh_pr` `Refreshed::NotSent`, `read_through` and `track` a `GithubUnavailable` refusal, `poll_now` `Polled` at cost 0.

## Don't assume

- That `refresh_pr` returning `Applied` means the record changed: evaluate on the store's view.
- That `poll_now`'s `Polled` means every PR was refreshed: a PR's own error stays with that PR (POL-24).
- That the tier in `status()` is slowed-aware: quota states change what's sent, never the tier.
