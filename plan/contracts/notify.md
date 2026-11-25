# Seam: notify

**Purpose.** Telling a Claude Code session once (D7, U14): the subscriptions the daemon holds without a connection (NTF-1 to NTF-5), the one way a notice leaves the daemon (NTF-6), the words of every notice (NTF-7, NTF-8), and what a stop does with what's open (NTF-9, NTF-10). **Everything that posts is a (proposal)**: the message format is known from Claude Code's debug log only, and only Linux is measured (docs/07).

**Producer:** T02 declares everything below, with `NoNotify` real and the rest as stubs. **T46 builds `src/notify/`**: the registry, the loop, the texts, the real poster and `fake::RecordingPoster`. **Consumers:** T47 (the two routes, through `NotifyHandle`), T28 (`Notifier::spawn` with its deps, `stop(asked)`, the idle exit), T48 (`text::watching` for the CLI's `registered:` line), T22 (only `View::subscriptions`, see `state.md`), T29 and T49 (the harness hands the daemon a `RealPoster`).

## Declarations (`src/notify/`)

```rust
pub const DEFAULT_LIFE: Duration = Duration::from_secs(86_400);  // NTF-2's `for`
pub const MIN_LIFE: Duration = Duration::from_secs(60);
pub const MAX_LIFE: Duration = Duration::from_secs(86_400);
pub const PER_SOCKET: u32 = 32;                                   // NTF-2: at most 32 for one socket
pub const POST_LIMIT: Duration = Duration::from_secs(2);          // NTF-6: connecting and writing together. Never a second try
pub const STOP_LIMIT: Duration = Duration::from_secs(1);          // NTF-10: for all the `stopped` notices together
pub const TICK: Duration = Duration::from_secs(10);               // POL-26's tick: every socket, `for` and `problem` is looked at
pub const PROBLEM_AFTER: Duration = Duration::from_secs(600);     // NTF-4's `problem`: 10 minutes in `auth`, from `PollerStatus::problem_since`
pub const CATCH_UP: Duration = Duration::from_secs(60);           // NTF-4: an expiry waits at most this for a refresh
// The 64 is `state::MAX_SUBSCRIPTIONS`, beside the other limits (`state.md`).

/// A session's token (NTF-9). As `token::Token`: no `Display`, no `Serialize`, `Debug` prints `SessionToken(…)`.
/// It lives in its subscription and in the poster's one write, and nowhere else.
#[derive(Clone)] pub struct SessionToken(/* String */);
impl SessionToken { pub fn new(value: String) -> SessionToken; pub fn expose(&self) -> &str; }
// impl Deserialize (the route's body holds one), Debug

/// Which socket a path names: its device and inode (NTF-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct SocketId { pub dev: u64, pub ino: u64 }
/// What one `stat` of a session's path saw (NTF-2, NTF-4, NTF-6). The texts are the system's (`io::Error`'s) or circitor's own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Looked {
    Socket(SocketId),            // a socket owned by the user the daemon runs as
    Other(SocketId, String),     // something else is there: a regular file, another user's socket. Which, in words
    Missing,                     // nothing at the path
    Failed(String),              // the `stat` failed otherwise
}

/// How one post ended; the texts are the system's error (`io::Error`'s). `SessionGone` and `Failed` both end
/// the subscription, and nothing is tried again (NTF-6): they differ only in the log's `reason=`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Posted {
    Done,                    // the message line is written in full. Not "delivered": nobody can know that (docs/07)
    SessionGone(String),     // a missing socket, a refused connection or a denied permission
    Failed(String),          // a connect or a write that failed otherwise (a full queue's `EAGAIN` included), before the line was written in full
}
/// The session's socket, seen from the daemon (NTF-2, NTF-4, NTF-6). The one door out of the daemon
/// besides `github::GitHub::send` (OPS-16).
#[async_trait::async_trait]
pub trait Poster: Send + Sync + 'static {
    /// A `stat`, never a connection.
    async fn look(&self, socket: &Path) -> Looked;
    /// One connection: connect; with a token, `{"type":"auth","token":"<token>"}` and a newline;
    /// `{"type":"user","message":{"role":"user","content":"<text>"}}` and a newline; close. Nothing is read.
    /// **It returns `Done` in the poll that writes the line's last byte, and awaits nothing after it**
    /// (the close is the drop). So a `post` that hasn't returned when the caller's timeout fires hasn't
    /// written the line in full, and the subscription ends (NTF-6). No timeout inside.
    async fn post(&self, socket: &Path, token: Option<&SessionToken>, text: &str) -> Posted;
}
pub struct RealPoster;                                            // `tokio::fs::metadata`, `tokio::net::UnixStream`
impl RealPoster { pub fn new() -> Arc<RealPoster>; }

/// A subscription to be kept (NTF-1). The route has checked every value and evaluated once (NTF-2).
pub struct NewSubscription {
    pub id: NodeId,                          // it holds its PR by node ID (NTF-1)
    pub pr: PrName,                          // the name it was registered under: NTF-3's cancel matches it too
    pub cond: Condition,                     // `api::condition` (`api.md`)
    pub progress: Progress,                  // from the route's one evaluation: `began` is the registration
    pub cursor: Cursor,                      // NTF-8's: `cond.since`, or, without one or with one too old on
                                             // that view (let go, NTF-2), the cursor of the view the route evaluated on
    pub life: Duration, pub life_given: bool, // `for`; whether the request held it (`Ask again` repeats it only then)
    pub socket: PathBuf, pub socket_id: SocketId,   // what `look` saw when registering
    pub token: Option<SessionToken>,
}
#[derive(Clone, Debug, PartialEq)] pub struct Registered { pub expires_at: Time, pub replaced: bool }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    Limit,                   // 64 are held, or 32 for this socket, and this isn't a replacement (NTF-2): API-3's `limit`
    NoSession(String),       // the confirmation wasn't posted: API-3's `no_session`, its message naming this, the system's error
    Stopping,                // `stop` has begun: the connection is about to be closed without an answer (OPS-7)
}
/// NTF-4's `problem`: what only the user can end. `as_str` is API-4's code, `auth`, or `pr_failing` (API-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ProblemWhy { Auth, PrFailing }
impl ProblemWhy { pub fn as_str(&self) -> &'static str; }
/// How a subscription ends (NTF-4's table). `as_str` is the word the log line holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End { Satisfied, Ended(Reason), Expired, Stopped, Problem(ProblemWhy), Cancelled, Replaced, SessionGone }
impl End { pub fn as_str(&self) -> &'static str; }       // `satisfied`, `ended`, `expired`, `stopped`, `problem`, `cancelled`, `replaced`, `session_gone`

/// What the API sees of the notifier, so the routes' shared test state needs no notifier (`api.md`).
#[async_trait::async_trait]
pub trait NotifyHandle: Send + Sync + 'static {
    /// `Poster::look`, for NTF-2's check of `socket` when registering: anything but `Socket` is `no_session`.
    async fn look(&self, socket: &Path) -> Looked;
    /// Ends, as `Replaced`, the subscription that is "the same request" (NTF-2, Semantics below) in any
    /// state, posting nothing. `true` if there was one. For a request whose answer is already known:
    /// nothing is registered, the earlier one still goes, and the answer has `registered: false` with
    /// `replaced: true` (NTF-2).
    fn drop_same(&self, id: &NodeId, socket: &Path, ask: &Ask) -> bool;
    /// Keeps it, in place of the one `drop_same` would end if there is one; posts the confirmation and
    /// returns once that post has ended (NTF-2).
    async fn register(&self, new: NewSubscription) -> Result<Registered, RegisterError>;
    /// NTF-3: drops that socket's subscriptions whose PR is `id`, or that were registered under `name`
    /// (without case), in any state. Their number. Posts nothing; never waits.
    fn cancel(&self, name: &PrName, id: Option<&NodeId>, socket: &Path) -> u32;
    /// The subscriptions held now, those whose confirmation or notice is being posted included. Equal to
    /// `store.view().subscriptions`, which `status` shows (API-5).
    fn count(&self) -> u32;
}
pub struct NoNotify;          // built by T02: `look` is `Failed`, `register` is `Err(Stopping)`, the rest `false` and 0

pub struct NotifyDeps { pub store: Arc<Store>, pub clock: SharedClock, pub logger: Logger,
                        pub activity: Activity, pub poster: Arc<dyn Poster>,
                        pub poller: Arc<dyn PollerHandle>,  // `status()`: the `auth` problem with its `problem_since`, and the tier (`stale`)
                        pub sleep: Arc<SleepWatch>,         // `for` counts the sleep noticed (POL-22)
                        pub warnings: Warnings,             // `pr_failing` for a PR (`log.md`, `since`)
                        pub config: Arc<Config> }           // `cadence::stale` reads it
pub struct Notifier { /* the registry, the loop's task, the notices on their way */ }
impl Notifier {
    /// Starts the loop. It starts with no subscription: none is ever saved or loaded (NTF-9).
    pub fn spawn(deps: NotifyDeps) -> Arc<Notifier>;
    /// OPS-7's hook (NTF-10): takes every subscription out at once, posts to all of them together, and
    /// returns when every post has ended or `STOP_LIMIT` has passed, whichever is first. `asked` is false
    /// for a stop the daemon chose itself (`StopCause::NotOwnFiles`, STA-11): its notices' command then has
    /// no `--no-spawn`. Nothing is tried again. Those not written by then are dropped and logged in one
    /// line, also when this future is dropped.
    pub async fn stop(&self, asked: bool);
    /// `Running::crash` (TST-5): ends the loop and drops everything. Posts nothing, logs nothing (NTF-10).
    pub fn abort(&self);
}
// impl NotifyHandle for Notifier
```

## Declarations (`src/notify/text.rs`): the words

Pure. No clock, no store. **These functions are the only place a notice's words exist.** Durations are `cli::render::duration` (CLI-3: 86 400 s reads `1d`), the outcome line is `cli::render::outcome_line`, the flags are `cli::render::flags`: called, never copied (`cli.md`).

```rust
/// What a notice says of its subscription, in the client's own words.
#[derive(Clone, Debug, PartialEq)]
pub struct Told { pub pr: PrName, pub cond: Ask, pub life: Duration, pub life_given: bool, pub cursor: Cursor }

/// NTF-7's one rule for the condition's words, which the confirmation and the CLI's `registered:` line
/// both hold: `<pr> until <until>`, then, each only where given and in this order, ` on <sha7>` (the
/// first 7 characters, lower case), ` after <rerun_since>`, ` fail-fast`, ` kind <kind>` and
/// ` since <since>`, each as given; then `, for <life>` (`render::duration`). For example
/// `yachiko/circitor#12 until checks-done on 1a2b3c4, for 1d` and
/// `yachiko/circitor#12 until changed kind review since k3x9q2mz-418, for 2h`.
pub fn watching(pr: &PrName, cond: &Ask, life: Duration) -> String;
/// NTF-7: `circitor: watching <watching>. If you can read this, notices reach this session.`
pub fn confirmation(told: &Told) -> String;
/// NTF-8: `circitor: <pr> <outcome line>. Details: circitor changes <pr> --since <cursor>`. With
/// `history: false` (the cursor is too old as the notice is built, STA-9):
/// `circitor: <pr> <outcome line>. History too old; details: circitor show <pr>`.
pub fn outcome(told: &Told, end: (Outcome, Option<Reason>), record: Option<&Record>, history: bool) -> String;
/// `circitor: notify for <pr> expired after <life>; it's <status><stale>. Ask again: <ask again>`. `stale`
/// is `cli::render::stale_mark`'s ` (stale, 2d5h old)` when the record is stale, "" otherwise (NTF-8).
pub fn expired(told: &Told, status: Status, stale: &str) -> String;
/// `circitor: can't read <pr> from GitHub (<why>); notify is cancelled. Details: circitor status`, `<why>`
/// being `ProblemWhy::as_str` (NTF-8): API-4's code or `pr_failing`, never GitHub's text.
pub fn problem(told: &Told, why: ProblemWhy) -> String;
/// `circitor: the daemon is stopping; notify for <pr> is cancelled. Ask again: <ask again>`, then
/// ` --no-spawn` when `no_spawn` (a stop someone asked for), and nothing after it otherwise (NTF-10).
pub fn stopped(told: &Told, no_spawn: bool) -> String;
/// `circitor notify <pr> `, `cli::render::flags(&told.cond)`, ` --for <seconds>` only if it was given,
/// then ` --since <told.cursor>`: the subscription's cursor, as CLI-6's `again:` sets it (NTF-8).
pub fn ask_again(told: &Told) -> String;
```

## Semantics

- **One loop evaluates; notices go out apart from it and apart from the engine's gate.** A post never delays a poll, and delays no answer but its own registration's (NTF-6, POL-2). Each post runs inside `clock::timeout(POST_LIMIT)`. A slow post holds no other.
- **A subscription's three states.** Each holds its `Subscribed` guard, counts in `count()` and against the limits, and has its socket checked at every tick:
  - **confirming**, from its entry until its confirmation's post has ended. It's evaluated as an open one; **an end found meanwhile waits for the confirmation**: its notice is posted after it, and not at all if the confirmation isn't written (NTF-2).
  - **open**: evaluated at every wake.
  - **telling**: it has ended, and its one notice is being posted (NTF-6). Never evaluated again; its text was built when it ended.
  **It leaves the registry, and its guard is dropped, when its notice's post has ended, however it ended**, and at once for an end that posts nothing (cancelled, replaced, session gone). **There's no other state: nothing waits to be tried again** (NTF-6, NTF-10). That drop is the subscription's end for STA-1 (`state.md`): the PR stays for 60 minutes after it, so the notice's command is answered from memory.
- **"The same request" (NTF-2)** is compared as the daemon reads it: the same `socket`, the PR by node ID, `until`, `sha` lower-cased as given (a short and a full SHA differ), `fail_fast` (left out is `false`) and `kind` as a set of words. `rerun_since`, `since` and `for` aren't in it. Requests that differ in any of the six are held side by side.
- **NTF-2's order, and its one step.** The route (T47) checks the parameters, then `look`s at `socket` (anything but `Socket` is `no_session`, so a bad socket never reaches GitHub and gets `no_session` even when the answer would be known), then reads through and runs the arrival refresh, then evaluates on a view `V`. With an outcome it answers from `V`; without one it answers `auth` in the `auth` state, and otherwise calls `register`, which checks the limits and enters it. **The answer is built from `V`** (MOD-13): its record, events and cursor are `V`'s, so its `cursor` is the subscription's; a refresh applied while the confirmation is posted is in the notice, not the answer. `register` enters the subscription under the registry's lock, and the loop's next evaluation of it reads every event after `V` (`progress.looked`, `api.md`): **a refresh applied between `V` and the entry is evaluated as if it came just after the entry**, never lost and never seen twice, which is what "no refresh applied between" asks. The route checks the limits only by calling `register`, after its evaluation and the `auth` check, so an answer already known is never refused.
- **`register`**, in this order, under the registry's lock but for the post:
  1. After `stop` has begun: `Stopping`.
  2. The one the request replaces, if any (`drop_same`'s rule, in any state): it ends as `Replaced`, nothing posted, and `replaced` is `true`. **A replacement takes no new place.** Otherwise, with `state::MAX_SUBSCRIPTIONS` held, or `PER_SOCKET` for this `socket`: `Limit`.
  3. `store.add_subscription(id)` (`state.md`). It's entered as confirming, with `expires_at` = the wall time now plus `life`, and its expiry's base: `clock.mono()` and `sleep.total()` now.
  4. The confirmation is posted and awaited, once, never tried again. `Posted::Done`: it's open (or, if it ended meanwhile, telling, and its notice goes now); `Registered`. **Anything else, a timeout included: it leaves, with any notice that waited for it**, the one it replaced stays gone, and the answer is `NoSession(error)` (NTF-2).
  5. The loop is woken.
- **The loop** wakes on `store.changed()`, on a registration, on `sleep.subscribe()`, and every `TICK` while any subscription is held; with none it keeps no timer. At each wake it takes **one `store.view()`** and walks the confirming and open subscriptions in the order registered. The first that applies ends it:
  1. `api::condition::evaluate(&cond, &view, &id, &mut progress)`: **the function the waiters call** (NTF-4, `api.md`). `Some((outcome, reason))` is `Satisfied` or `Ended(reason)`; its notice is `text::outcome` on that view's record, with `history` false when `view.too_old(&cursor)`.
  2. On a tick: **the socket**, `poster.look(socket)`. `Missing`, or a `Socket` or `Other` with another `SocketId` than `socket_id`, is `SessionGone`: nothing is posted (NTF-4). `Failed` changes nothing at that tick, and is logged once for that subscription. A `stat`, never a connection.
  3. On a tick: **`problem`** (NTF-4, POL-26). `warnings.since(PrFailing, id)` is `Some`: so one registered while its PR has `pr_failing` raised posts at the first tick. Or `poller.status().problem` has the code `auth` and `clock.mono()` is `PROBLEM_AFTER` or more past `poller.status().problem_since` (`poller.md`): **the 10 minutes run from when `daemon.problem` became `auth`**, not from when this loop first saw it, and a try with a new token that fails doesn't restart them (POL-20 keeps `daemon.problem` at `auth` through the try). Notice: `text::problem`.
  4. On a tick: **`for` (NTF-4, POL-22, POL-26):** it has passed when `mono().since(base) + (sleep.total() - total at base) >= life`, the rule of a waiter's deadline (`clock.md`); a wall clock step moves nothing. **Expiry never beats a catch-up:** when it's first found passed at a wake where `sleep.total()` has moved since the previous wake, or while the view's record is stale (`poller::cadence::stale` with `poller.status().tier` and `config`), it's held: evaluated as ever, it expires at the first wake on which its PR's `timing.fetched` is later than when it was held and step 1 gave nothing, or once `CATCH_UP` has passed since it was held, as it is. Otherwise `Expired` at once. Notice: `text::expired` with the view's record's `status`, and `cli::render::stale_mark` of that record with `stale` set as `present` sets it.
  So `expired` comes up to `TICK` late, and up to `CATCH_UP` after a sleep or on a stale record; `problem` up to `TICK` after its 10 minutes.
- **A subscription never searches the event history after its registration (NTF-4).** The route's evaluation searched it once; `Progress` remembers what was found and what was looked at (`api.md`), so a cursor that goes too old while it's held neither satisfies it nor drops a part, and events recorded while the confirmation was posted are looked at.
- **This tick is the notifier's own timer**, with POL-26's period: the engine doesn't know the notifier (plan decision P15).
- **Posting a notice (NTF-6).** First `poster.look(socket)`, as at a tick: `Missing` or another `SocketId` is the session gone, and nothing is posted; `Failed` doesn't stop the post. Then `post` inside `timeout(POST_LIMIT)`. **Whatever it gives, the subscription leaves; nothing is ever tried again:**
  - `Done`: **once `post` has returned `Done` a notice is never sent again**, whatever happens to it in the session. A peer that doesn't read isn't a failure: the line fits the socket's buffer.
  - `SessionGone`, `Failed`, or `POST_LIMIT` passing before `post` returned: logged (`notice not posted`, `reason=` `session_gone`, `failed` or `timeout`). A full listen queue fails at once with `EAGAIN` on Linux (measured 2026-11-25, not yet in docs/05): `Failed`.
- **`cancel`, `drop_same` and `count` never wait** and never post. `cancel` matches the node ID the route found without reading through (`store.lookup`), or the name the subscription was registered under, so a renamed PR's subscription is still found by its old name (NTF-3). A telling one is already on its way out: `cancel` and `drop_same` don't count it, and its post isn't stopped.
- **`stop`** is called once, by T28, after the engine has stopped and before the connections are closed (OPS-7, `daemon.md`). From its first instant `count()` is 0, `register` answers `Stopping` and the loop evaluates nothing. It posts, all together and inside one `STOP_LIMIT` of the clock: `text::stopped(told, asked)` to every open one, and the same to every confirming one once its confirmation was written. **A telling one's post already under way is awaited inside the same `STOP_LIMIT`**, in place of `stopped`. Nothing is tried again. Those not written when `STOP_LIMIT` passes are dropped and logged in one line (NTF-10). It needs no outside timeout and leaves the stop's 5 s to the final write.
- **The name in a notice** is the record's `pr` in the view the notice is built from, so a renamed PR is named as `changes` will name it (MOD-1). A subscription's PR always has a record: the route read it through (NTF-2).
- **Nothing a GitHub user wrote is in a notice but the PR's name (NTF-8).** `text` takes no title, no check's name, no login and no URL, so it can't print one: `Told` holds a `PrName`, a cursor and an `Ask` whose every word `Condition::check` has checked (a hex SHA, cursors, kind words); the outcome line holds counts and short SHAs; `status`, the stale mark and the problem's code are circitor's own words. A notice ends without a newline; the poster adds the line's.
- **circitor posts as what it is** (NTF-6): the two lines above and nothing else. No permission mode, no reply address, no `from`.
- **The session token** goes from the route's body into `NewSubscription`, from there to `Poster::post`, and is dropped with the subscription (NTF-9). No log line, error, `Debug` output or API answer holds it. `Posted`'s and `Looked`'s texts are `io::Error`s or circitor's words: they hold no token.
- **Nothing is saved.** `state::FileContent` has no place for a subscription (STA-6, NTF-9), and `Notifier::spawn` takes none.
- **Logging (OPS-9; the words are in `log.md`):** a subscription registered, and ended with `End::as_str` (and `why=` for `problem`); a notice that couldn't be posted, with why (`session_gone`, `failed`, `timeout`, `stop`); a socket whose `stat` failed otherwise, once. Never a notice's text, a token or a socket's path.

## The fake

- **`notify::fake::RecordingPoster`** (T46; under `cfg(any(test, feature = "test-endpoint"))`): `new(clock: SharedClock)`; `posts() -> Vec<Post>` with `Post { socket: PathBuf, token: Option<String>, text: String, at: Time }`, every post asked for, in order, a stalled one included from the moment it's asked; `answer(Posted)` for every post from now on, `answer_next(Posted)` for one; `stall(activity: &Activity) -> Release`, which makes every post wait until the `Release` is dropped and excuses each from the work in flight meanwhile (`clock.md`), for the 2 s and 1 s limits; `look` answers `Looked::Socket` with one fixed `SocketId` per path, until `gone(path)` (`Missing`), `swapped(path)` (another `SocketId`) or `stat_fails(path)` (`Failed`). Good for the notifier's, the routes' and the daemon's own tests (T46, T47, T28).
- **`notify::NoNotify`** (T02): the handle behind `api::test_state`, so T22 and T23 never need a notifier.
- **The fake session socket** (`testkit::session::FakeSession`, T45, `testkit.md`) is the other end of `RealPoster` in the harness: it shows what was really written.

## Don't assume

- That `Posted::Done` means the session read the notice. A held, a refused and a delivered one look the same from here (docs/07). Only the agent can tell, which is what the confirmation is for (NTF-7).
- That a post to a session socket can be made to hang in a test: on Linux a connect to a full listen queue fails at once with `EAGAIN` (`testkit.md`, NTF-6). `RecordingPoster::stall` is how the limits are tested.
- That a failed post gets another chance: NTF-6 has no second try, at a stop or anywhere else. A notice that isn't written ends its subscription, logged.
- That macOS behaves as Linux does here: nothing about the session socket is measured there (docs/07).
- That a session's socket path is short: NTF-2 puts no limit on it. One the system can't connect to is `no_session`.
- That a `stat` that fails means the session is gone: only a missing path or another device and inode does (NTF-4).
- That a subscription on an ended PR holds the active tier: only one on an open PR does (NTF-5, `poller.md`). It still keeps the daemon from its idle exit. Nor that the tier stays active after the last one ends: it then follows the last client request (POL-13).
- That `problem`'s 10 minutes start when the notifier first notices `auth`: they start at `PollerStatus::problem_since` (NTF-4).
- That `expired` is posted at `expires_at` to the second (a tick, a sleep and a stale record each delay it), that `expires_at` moves with the wall clock, or that a subscription outlives its daemon: a restart, a crash and an idle exit that never comes (NTF-5) are all in NTF-9 and NTF-10.
