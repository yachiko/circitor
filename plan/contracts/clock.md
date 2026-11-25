# Seam: clock, work in flight, sleep detection

**Purpose.** The daemon's and the CLI's only source of time, with wall and monotonic time kept apart (TST-4, POL-26), the count of work in flight the test clock settles on (TST-4), and the one place a laptop's sleep is noticed (POL-22).

**Producer:** T02 declares, and builds `Time`, `Mono`, the `sleep` and `timeout` helpers and `Activity` (with `excuse`: its own fakes need it). T04 builds `SystemClock`, `TestClock` and `SleepWatch`, and takes `Activity` over. **Consumers:** every task.

## Declarations (`src/clock/`)

```rust
/// Wall time, UTC (POL-26). It can jump either way, and it counts a suspend.
/// Serialises as RFC 3339 in whole seconds with `Z`: `2026-11-24T22:09:23Z` (specs/README).
/// Deserialises any RFC 3339 time. Keeps millisecond precision in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(chrono::DateTime<chrono::Utc>);

impl Time {
    pub fn from_unix_millis(ms: i64) -> Time;
    pub fn unix_millis(self) -> i64;
    pub fn parse(rfc3339: &str) -> Result<Time, TimeError>;
    pub fn rfc3339(self) -> String;                 // whole seconds, `Z`
    pub fn plus(self, d: Duration) -> Time;
    pub fn minus(self, d: Duration) -> Time;
    /// `self - earlier`, zero when `earlier` is later (POL-26: a wall time ahead of now counts as now).
    pub fn since(self, earlier: Time) -> Duration;
    pub fn inner(self) -> chrono::DateTime<chrono::Utc>;
}
// impl Serialize, Deserialize, Display (= rfc3339)

/// Monotonic time since the clock was made. Never goes back. Whether it counts a suspend is the
/// platform's choice (docs/04 §8 item 6): nothing that must survive a sleep is timed on it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mono(Duration);

impl Mono {
    pub const ZERO: Mono;
    pub fn plus(self, d: Duration) -> Mono;
    pub fn since(self, earlier: Mono) -> Duration;  // saturating
    pub fn as_duration(self) -> Duration;
}

pub trait Clock: Send + Sync + 'static {
    fn wall(&self) -> Time;
    fn mono(&self) -> Mono;
    /// Completes once `mono() >= at`. Dropping it cancels the timer.
    fn sleep_until(&self, at: Mono) -> futures::future::BoxFuture<'static, ()>;
}
pub type SharedClock = std::sync::Arc<dyn Clock>;

/// `clock.sleep_until(clock.mono().plus(d))`.
pub fn sleep(clock: &SharedClock, d: Duration) -> futures::future::BoxFuture<'static, ()>;
#[derive(Debug, thiserror::Error)] #[error("timed out")] pub struct Elapsed;
/// Runs `fut` for at most `d` of the clock's monotonic time. Every timeout in circitor is this one (TST-4).
pub async fn timeout<F: Future>(clock: &SharedClock, d: Duration, fut: F) -> Result<F::Output, Elapsed>;

/// The count of work in flight (TST-4). Cheap; always compiled.
#[derive(Clone, Default)] pub struct Activity(/* Arc<AtomicUsize> */);
impl Activity {
    pub fn begin(&self) -> Busy;        // +1 until dropped
    pub fn in_flight(&self) -> usize;
}
pub struct Busy(/* … */);               // Drop: -1
#[cfg(any(test, feature = "test-endpoint"))]
impl Activity {
    /// One piece of work in flight is held by a test's script (a hung answer): don't count it until
    /// the guard is dropped (TST-4: "requests to GitHub not held by a scripted hang").
    pub fn excuse(&self) -> Excused;
}
#[cfg(any(test, feature = "test-endpoint"))] pub struct Excused(/* … */);

/// POL-22's check. One per daemon; the engine calls `check` at every tick, the API at every client request.
pub struct SleepWatch { /* … */ }
impl SleepWatch {
    pub fn new(clock: SharedClock) -> Arc<SleepWatch>;
    /// How far wall and monotonic time moved since the last call. Wall ahead by more than 30 s is a
    /// sleep: returns its length (the gap) and adds it to `total`. A wall clock set back is never a sleep.
    pub fn check(&self) -> Option<Duration>;
    /// All the sleep noticed since the daemon started.
    pub fn total(&self) -> Duration;
    /// Changes whenever `total` does. Waiters re-check their deadline on it (API-12).
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<Duration>;
}

pub struct SystemClock { /* the `Instant` it was made at */ }   // SystemClock::new() -> Arc<SystemClock>; tokio timers inside

// Only with `cfg(any(test, feature = "test-endpoint"))`:
pub trait SettleProbe: Send + Sync { fn settled(&self) -> bool; }
pub struct TestClock { /* … */ }
impl TestClock {
    /// Wall time starts at `start` (TST-5: 2026-11-24T22:00:00Z), monotonic time at zero.
    pub fn new(start: Time) -> Arc<TestClock>;
    /// The daemon under test counts its work here; the clock treats it as a probe.
    pub fn activity(&self) -> Activity;
    pub fn add_probe(&self, probe: Arc<dyn SettleProbe>);   // the fake GitHub adds itself
    /// Settles first. Then moves both clocks by `d`: fires each timer at its own due time, in order (same
    /// instant: in the order they were set), and settles after each before time moves on (TST-4).
    pub async fn advance(&self, d: Duration);
    /// A suspend the monotonic clock doesn't count: wall time jumps by `d` in one step. Then settles.
    pub async fn sleep_wall_only(&self, d: Duration);
    /// A suspend the monotonic clock counts: both jump by `d` in one step; every timer that fell due
    /// fires afterwards, in the order of due time. Then settles.
    pub async fn sleep_both(&self, d: Duration);
    /// Sets wall time back by `d`, alone. Then settles.
    pub async fn set_wall_back(&self, d: Duration);
    /// Waits, in real time, until settled: `Activity::in_flight() == 0`, no timer fired but not yet
    /// polled, and every probe settled, on three checks in a row with a yield between them.
    pub async fn settle(&self);
}
// impl Clock for TestClock
```

## Semantics

- **Two clocks, never mixed (POL-26).** GitHub's windows and retention go by `wall()`: POL-17's hour, POL-25's reset, MOD-8's 60 s, STA-1's and STA-2's durations. They're compared at a tick, never slept on. Everything else goes by `mono()`.
- **`sleep_until` is the only timer.** A component that wants a periodic timer loops over it with absolute due times (`last_sent.plus(interval)`), so a late wake never drifts and a long jump fires it once (TST-8 `sleep`: one poll on waking, not one per missed interval).
- **`timeout`** drops `fut` when the time passes. POL-2's 20 s, OPS-5's 10 s, API-12's deadline and the CLI's limits (CLI-7) all use it, so a scripted hang costs no real time.
- **`Activity`: the rule.** A `Busy` is held only across awaits that end without the clock moving: computing, real I/O to GitHub or `gh`, a lock. **Before awaiting a timer, or anything that may itself await a timer, drop it** (or `InFlight::park`), and take it again on waking. A guard held across such an await stops `settle` from ever returning.
- **`Activity`: who counts what.** The engine's loop and the saver's loop: from waking until parking again; waiting for POL-2's gap, a backoff or the next due time is parking. A request to GitHub in flight is counted by the engine for as long as it awaits the answer, its 20 s timeout included: when the fake holds that answer by script (the fake GitHub's hang, `ScriptedGitHub::hold`), **the fake calls `excuse()`** for as long as it holds it. The API: one guard per request (`InFlight`), parked while a waiter is held, while it waits for the first poll, and while it awaits the poller (`api.md`). The token source: a guard while `gh` runs; a fake `gh` that never exits is excused by the test. The CLI under the harness: none; the test awaits its future.
- **The test clock's fired timers count as work** from the moment they fire until their future is polled. Code that wakes from a timer takes its own `Busy` in that same poll, before its next `.await` (conventions, "Work in flight"), so there's no moment when work is pending and the count is zero.
- **`SleepWatch::check` is cheap and safe to call from any task.** Concurrent calls see one sleep once. It doesn't act on a sleep: the engine does (POL-22), from its tick or when the watch changes.
- **Deadlines that count sleep (API-12, POL-22):** a waiter's deadline has passed when `mono().since(arrived) + (total() - total_at_arrival) >= timeout`.

## The fake

`TestClock` is the fake, and it's T04's product. Until T04 is merged a task has no clock to test with: only tasks whose logic takes time as plain `Time` and `Mono` arguments (T05, T08, T09) are built before it.

## Don't assume

- That `mono()` stops during a suspend, or that it doesn't: POL-22's rule is written to work both ways, and TST-8 `sleep` runs both.
- That real I/O finishes inside `advance`: it does only for work that holds a `Busy` or a probe. A response on its way to a client that nothing counts isn't waited for; the test awaits that client.
- Tokio's paused time. It's never used: the fake GitHub and the socket are real I/O.
