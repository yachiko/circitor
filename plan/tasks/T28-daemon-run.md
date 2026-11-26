---
id: T28
title: "M2: the daemon runs: start order, serving the socket, modes, signals, stop, idle exit"
kind: integration
milestone: M2
size: L
status: done
depends_on: [T06, T07, T12, T16, T17, T18, T21, T22, T23, T46]
requirements: [OPS-6, OPS-7, OPS-8, STA-11, API-17, API-14, TST-1, OPS-9:logged, OPS-5:the-token-is-read-after, NTF-5, NTF-10]
owns: [src/daemon/run/, tests/m2_serve.rs]
shares: []
---

# T28 · M2: the daemon runs: start order, serving the socket, modes, signals, stop, idle exit

## Outcome
`daemon::run::start` brings up a whole daemon in STA-11's order and serves the API on its socket; it stops by OPS-7 within 5 s, telling every session with an open subscription first (NTF-10), exits when idle (OPS-8), and a foreground one takes over from a spawned one (OPS-6). Proven in-process over a real socket: lists, waits that are satisfied, ended and timed out, and a client that disconnects leaving no waiter.

## Context
Everything the daemon is made of exists: paths and config (T06), the instance primitives (T18), the state file (T16) and the store (T15), the engine's loop and poll (T21) with the real client (T07) and token source (T12), the router (T22, T23), the notifier (T46), and the transport (T17) it probes a lock's holder with. This task wires them into `start` and `main`, and owns the daemon's life. **T25 isn't a dependency:** when this task is built, the engine's demand calls (`read_through`, `track`, `refresh_pr`) may still be T21's stubs, answering `GithubUnavailable` and `NotSent`. So every PR in this task's tests comes in through a poll's finding. **The notify routes (T47) aren't a dependency either:** `notify::routes()` may still be T02's empty router, so this task's tests register a subscription through `Running::notifier()`, never over the socket. The harness (T29) starts it through `start`; the CLI (T27) calls `main`; the real-process tests (T36) run the binary.

## Read first
- `plan/conventions.md`
- `plan/contracts/daemon.md`: the whole file; `notify.md`: `Notifier::spawn`, `stop`, `abort`, `NotifyDeps`, `NewSubscription`, and the fake; `state.md`: "Order at start", `load`, `Saver`, `last_waiter_ended`, `last_subscription_ended`, `View::subscriptions`; `poller.md`: `Engine`, `EngineDeps`, `idle_since`, and what T21 builds (its stubs) in "Inside: the engine"; `api.md`: `AppState`, `DaemonInfo`, `router`, the `shutdown` line; `transport.md`: `probe`, `connect`; `clock.md`; `log.md`: this task's rows in "Who logs what"; `config.md`; `github.md`: the two fakes, and "The real ones' constructors"
- `specs/03-state.md#one-daemon`: STA-11, STA-12
- `specs/08-notify.md`: NTF-10 (what a stop tells), NTF-5 (the idle exit), NTF-9 (nothing is loaded)
- `specs/06-operations.md#the-daemons-life`: OPS-6 to OPS-9; `#files`: OPS-3, OPS-4; `#the-token`: OPS-5, the part "the token is read after"; `#configuration`: OPS-1
- `specs/04-api.md`: API-14, API-17
- `specs/07-testing.md#rules`: TST-1; TST-10's line on a failed write at a stop

## Scope
- **STA-11's share here:** the order at start, what a spawned and a foreground daemon do when the lock is busy, the takeover, and the minute's check. The lock, the holder line, the stale socket and the bind are T18's primitives: call them, don't rebuild them.
- **`start`, in STA-11's order.** Each failed step returns a `StartError` and leaves nothing locked or bound.
  1. `ensure_private_dir` for the state directory and, when set, `socket_dir` (OPS-3).
  2. The lock: `instance::acquire` with `PATIENCE`. An error from opening stops it, naming the path and the system's error.
  3. **Busy, spawned:** probe the socket in the holder's line. It answers: `code()` 0. It doesn't (`Probe::Denied` included), or there's no line: an error, `code()` 1.
  4. **Busy, foreground (OPS-6):** probe the holder. `mode: spawned`: send `Request::Shutdown` and `acquire` again, for 6 s or what's left of `TAKEOVER_PATIENCE`. Repeat while the holder answers `spawned`. A `foreground` holder: an error naming its socket, at once. No lock after 15 s: `code()` 1, naming the holder's socket. Never 0.
  5. `write_holder(pid, socket)`.
  6. `clear_socket`. `Live`: as steps 3 and 4 say for an answering holder. `NotASocket` or `NotAnswering`: an error naming the path (OPS-4).
  7. `state::file::load`, `Store::new`, `Saver::spawn` (`writable: false` after `Loaded::Newer`). 
  8. `instance::bind`; then `Engine::spawn`; then `Notifier::spawn` with `NotifyDeps` (the store, `deps.poster`, the engine as `poller`, the `SleepWatch`, the `Warnings` and the config): it starts with no subscription (NTF-9), and `AppState::notify` is that notifier; then the accept loop. **The token is read by the engine, after the bind (OPS-5, this task's part):** `start` never waits for it.
  9. One `info` line: started, with the version, the mode and the paths (OPS-9).
- **Serving.** An accept loop of this task's own, one task per connection, `hyper-util`'s connection builder around `api::router(state)`. The tasks are kept so they can be aborted. Not `axum::serve`'s graceful shutdown.
- **API-14's share:** a client that closes makes `hyper` drop the handler's future or the body, which is where T23's cleanup runs; and at a stop every connection is closed without an answer. The 64 and the cleanup are T23's.
- **Stopping (OPS-7), once, when `StopHandle` is first asked:** stop accepting and `remove_if_own`; hold for `stop_delay` (TST-1); `Engine::stop`; **`Notifier::stop(asked)`**, with `asked` false for `StopCause::NotOwnFiles` and true for every other cause (NTF-10); abort every connection; `Saver::shutdown` (a failure is logged); drop the lock; exit status 0.
- **NTF-10's share here: where the hook sits.** After the engine has stopped, so no refresh ends a subscription meanwhile, and before any connection is aborted, so each session is told `stopped` while the daemon's clients are still connected. What it posts, to whom, and its 1 s for all of them are T46's: `Notifier::stop` returns by itself, inside the 5 s. **The whole sequence runs inside `clock::timeout` of 5 s:** past it, one line says so, everything is dropped and the status is still 0. A second request during it (`StopHandle::times() >= 2`) does the same at once.
- **API-17's share:** a shutdown request stops as a signal does. T22 asks for the stop only once the server has taken the whole answer; aborting a connection must not cut bytes already handed to the socket.
- **Signals**, when `deps.signals`: `SIGTERM`, `SIGINT` and `SIGHUP` each call `stop.request(StopCause::Signal(n))`; a later `SIGTERM` or `SIGINT` calls it again.
- **Idle exit (OPS-8; NTF-5's share here).** Spawned only, and only with `exit_after` set. It asks for `StopCause::Idle` when `view.waiters == 0`, **`view.subscriptions == 0`**, and `exit_after` has passed since the latest of `poller.idle_since()`, `store.last_waiter_ended()` and `store.last_subscription_ended()`. The loop sleeps on the clock until that time, then looks again; with a waiter or a subscription held it looks again `exit_after` later. No fixed sampling. `status` doesn't count: T22 doesn't report it as a request (API-5, OPS-8).
- **The minute's check (STA-11):** every 60 s of the clock, `LockGuard::still_own` and `Bound::still_own`; either false asks for `StopCause::NotOwnFiles`.
- **`Running`.** `exited` gives the status. `crash` (TST-5) drops everything without OPS-7's sequence: connections and loops aborted, the engine stopped, `Notifier::abort` (**a crash tells nobody**, NTF-10), `Saver::abort` (no write), the listener dropped with its file left, the lock released. `notifier()`, in test builds, hands out the notifier for this task's own tests. Both types live in `src/daemon/run/mod.rs`; `src/daemon/mod.rs` re-exports them.
- **`main(spawned)` is a thin shell over an inner function** that takes an `Env` and a log sink and returns the exit status; `main` passes `Env::from_process()` and `StderrSink` and hands the status to the process. The names are yours (say `run_main(spawned, env, sink) -> i32`). The inner function builds its own runtime. In order: the `SystemClock` and the logger on the sink with `CIRCITOR_LOG` (an unknown value: `info`, and one `log level unknown` `warn` line); `resolve`; `config::load` before any file is touched (OPS-1); real `Deps`; `start`; then `exited`. Every line goes through the `Logger`.
  - **Building `Deps` is a function of its own** (say `deps_from(spawned, &env, &logger) -> Result<Deps, i32>`), so a test can check it without starting anything: `SystemClock`, `Client::from_env(env, version)`, `GhTokenSource::new(gh_path, env, clock, activity)`, `RealFs`, `RealProbe`, `notify::RealPoster::new()`, the level, `stop_delay`, and the seed.
  - **The seed** is the clock's wall time in milliseconds XOR the process ID. Not `fastrand`: T04's guard test fails on an unseeded `fastrand::` call under `src/` outside `src/clock/`.
  - **Before `start`:** a `PathError` (from `resolve`), a `ConfigError` (from `load`) or a `Client::from_env` error is one `error` line holding its `Display`, and exit status 1, with nothing created. A `StartError` is one `error` line and its `code()`.
  - **Interpretation (OPS-7, "status 1 is for a crash"):** `main` sets a panic hook that logs the panic's location, never its message, and exits 1. A panic in a tokio task doesn't end the process otherwise.
- **TST-1's share:** an `Err` from `Client::from_env` (a test build without a loopback `CIRCITOR_TEST_GITHUB`) is one `error` line with its message and exit 1; behind `cfg(feature = "test-endpoint")` only, `CIRCITOR_TEST_STOP_DELAY` seconds become `Deps::stop_delay`.
- **OPS-9's part "Logged", this task's rows:** start; stop with its cause (a signal's number, a shutdown request, idle, files not its own); the 5 s deadline; a failed final write; an unknown `CIRCITOR_LOG`. A CLI replacing the daemon can't be told from `stop` and logs as a shutdown request (OPS-9).

## Interfaces
- **Consumes:** `config::{Paths, Config, load, resolve, ensure_private_dir}`; `daemon::instance::*`; `state::file::{load, Saver, SaverDeps, RealFs}`; `state::store::Store` (`view`, `last_waiter_ended`); `poller::engine::{Engine, EngineDeps}`, `PollerHandle::idle_since`; `notify::{Notifier, NotifyDeps, RealPoster}`; `api::{router, AppState, DaemonInfo}`; `transport::{probe, connect, RealProbe}`; `clock::*`; `log::*`; `github::client::Client::from_env`, `token::gh::GhTokenSource::new`.
- **Provides:** `daemon::run::{start, main, Running, StartError}`, as daemon.md declares them. No test hook beyond `Deps`.

## Out of scope
- Starting a daemon from the CLI, rotating the log, `SIGTERM` and `SIGKILL` from `stop`: T24. `circitor daemon --check-config` and the argument parsing: T27.
- The routes and the waiters, including `status` during a slow token read: T22, T23, T21. Fix nothing there: report it.
- The notifier's inside (what a `stopped` notice says, the 1 s, the loop), the notify routes and the command: T46, T47, T48. Stopping with subscriptions registered through the CLI, and the idle exit after one ends, end to end: T49's `notify-stop`, `notify-keeps-active` and `notify-keeps-ended`.
- The harness, the fake GitHub server and the marker sweep: T29, T20. Real processes and real signals: T36.
- Demand (T25): reading through, `track`, `show --fresh`, hot and forced refreshes. A waiter making its PR hot is T29's `m3_waiter_makes_hot`. No test here needs any of them.

## Acceptance
`tests/m2_serve.rs`: an in-process daemon from `start`, on `TestClock`, with `github::fake::ScriptedGitHub` answering from a small world in the test, `FakeTokenSource`, a `BufferSink`, a `notify::fake::RecordingPoster` as `Deps::poster`, `signals: false`, and a socket in a `testkit::ShortDir`. Requests go through `transport`. A subscription is registered with `running.notifier().register(NewSubscription { … })` on an open PR a poll found, its ID from `github::fake::answers`, after `running.notifier().look(path)` gave its `SocketId` (`RecordingPoster` answers one per path).
- `m2_list_and_show`: two PRs are found; `list` and `show` answer over the socket.
- `m2_wait_satisfied`, `m2_wait_ended`, `m2_wait_timeout`: a `merged` wait whose PR merges; one whose PR closes; one that ends at 100 s of the clock. The merge and the close are seen by a later poll, not a hot refresh. Each gets its head with `circitor-cursor` first.
- `api14_client_drop`: the client drops a held wait: `status` then shows `waiters: 0`.
- `api14_ops7_stop_closes_waiters`: at a stop, a held wait's `body()` is `Dropped`.
- `api17_shutdown_answers_then_stops`: `POST /v1/shutdown` gets a complete 200; `exited()` is 0; the log names a shutdown request.
- `ops7_stop_leaves_no_socket_and_a_state_file`: after a stop the socket is gone, `state.json` loads, the lock is free, and the directory holds `state.json` and `circitor.lock` only (STA-12). `m2_restart_loads_it`: a new `start` names the saved PR in `pending` until its first refresh.
- `ops7_deadline`: with `stop_delay` of 10 s, `exited()` is 0 when the clock reaches 5 s, with the deadline's line. `ops7_second_request`: a second request ends the stop at once.
- `tst10_failed_final_write`: with `FaultFs` failing, the stop logs it and `exited()` is 0.
- `ops8_idle_exit`: spawned, `exit_after` 300 s: no request for 300 s stops it, cause idle, file written. A held wait prevents it; the daemon then exits 300 s after the wait ended, and not at 299 s. `status` calls don't prevent it. A foreground daemon doesn't exit.
- `ntf5_a_subscription_prevents_the_idle_exit`: with one subscription open and no request, the daemon is still running 20 minutes on, and `status` reads `subscriptions: 1`; after `notifier().cancel(…)` (its name, its ID and the socket; a call, not a client request) it exits 300 s later, and not at 299 s.
- `ntf10_stop_tells_each_session_first`: two subscriptions and a held wait, with `RecordingPoster::stall` held. Half a second into the stop, `posts()` holds the two `stopped` notices and the wait's connection is still open (`open_connections()` is 1); once the clock has moved 1 s the wait's `body()` is `Dropped`, `exited()` is 0, the final write was made (`state.json` loads, and holds no socket's path), and no `stop deadline` line is logged. Without the stall both notices are posted and the stop takes no clock time. With no subscription, a stop posts nothing.
- `ntf10_crash_tells_nobody`: after `crash()` with a subscription open, `posts()` holds its confirmation only; a new `start` on the same directories reads `subscriptions: 0`.
- `ntf10_a_stop_it_chose`: with a subscription open, the socket file removed and the minute's check run: the `stopped` notice in `posts()` ends without `--no-spawn`; after `Request::Shutdown` instead, it ends with ` --no-spawn`.
- `ops5_token_after_bind`: with the token read delayed 10 s, `start` returns and `status` answers at once with `problem: null`.
- `sta11_second_spawned`: a second spawned `start` on the same paths fails with `code()` 0 once the clock has moved 6 s; the first still answers.
- `ops6_takeover`: a foreground `start` against a spawned daemon: the first exits 0, the second serves, within 15 s of the clock. `ops6_foreground_holder`: against a foreground one: `code()` 1, its message naming the holder's socket.
- `ops4_stale_socket`: after `crash()`, `start` replaces the socket file and loads the file as last written. `ops4_not_a_socket`: a regular file there fails `start` naming the path, `code()` 1, with the lock free again. `sta11_lock_unopenable`: a directory at the lock's path fails at once, naming it.
- `sta11_files_not_own`: the socket file removed, or the lock file replaced: within 60 s of the clock the daemon stops, and leaves a newer file at the socket's path alone.
- `ops9_start_and_stop_lines`: the start line holds the version and the socket path.
- `cargo test --features test-endpoint --test m2_serve` and `make verify` pass.

Unit tests in `src/daemon/run/`, on the inner function and `deps_from`, which never reach `start`: a hand-built `Env` whose `home` is a `testkit::ShortDir`, and a `BufferSink`.
- `ops1_config_error_before_start`: a `config.toml` at `resolve`'s `config_file` with an unknown key: the inner function returns 1, the sink holds one `error` line naming the key, and the state directory doesn't exist.
- `ops4_path_error_before_start`: `CIRCITOR_SOCKET` of 104 bytes: 1, one `error` line naming `CIRCITOR_SOCKET`, nothing created.
- `ops9_unknown_log_level`: `CIRCITOR_LOG=loud` with the bad config: a `log level unknown` `warn` line, then the `error` line, and 1. With a valid config, `deps_from` gives `level` `Info`.
- `tst1_endpoint_error_before_start`: a valid config without `CIRCITOR_TEST_GITHUB`, and with `http://example.com/graphql`: 1, one `error` line naming `CIRCITOR_TEST_GITHUB`, nothing created.
- `tst1_stop_delay`: with `CIRCITOR_TEST_GITHUB=http://127.0.0.1:1/graphql`, `deps_from` gives `stop_delay` `Some(3 s)` for `CIRCITOR_TEST_STOP_DELAY=3`, and `None` without it.
- `cargo test --features test-endpoint --lib daemon::run::` passes.

## Traps
- **Bind, then spawn the engine, then accept.** `AppState` needs the engine, and the token must not be read before the bind.
- **Don't turn on `half_close`.** `hyper` ends a connection whose client closed mid-response only without it; that's the one signal a waiter gets (API-14).
- **The hook goes between the engine and the connections.** Before `Engine::stop`, a refresh could still end a subscription and post an outcome beside `stopped`; after the connections are aborted, a session would be told later than the waiters were cut.
- **Internal callers ask once.** The idle loop and the minute's check end when a stop begins: a second `request` from them would cut the stop short as a second signal does.
- Every loop here takes `activity.begin()` on waking, and every wait is `clock::sleep`: a stray tokio timer breaks the test clock.
- The test's requests are real I/O the clock doesn't count: await the client's future, or its `Head`, before advancing.
- `ScriptedGitHub` isn't the fake GitHub server: it checks no query. Don't prove the client here.
- `main` is called from outside any runtime. Nothing but `main` calls `std::process::exit`.
- The seed of a real daemon is the wall time in milliseconds XOR the process ID, so two starts never share an epoch (STA-9). Never an unseeded `fastrand::` call: T04's guard test fails on it.
- **T21's stubs may still be in place.** No test here shows an untracked PR, tracks one, sends `fresh=1` or counts on a hot refresh: a PR comes in through a poll's finding, and a wait's arrival refresh may be `NotSent`, which T23 treats as failed.
- **The unit tests never reach `start` and never read the token.** Each stops before `start` on an error, or calls `deps_from` only; `GhTokenSource::new` only builds. `deps_from` succeeds only with a loopback `CIRCITOR_TEST_GITHUB` (TST-1).

## Open questions
- **No holder line after the patience** (STA-11: "counts as no path"). Build: an error, `code()` 1, for both modes.
- OPS-8 doesn't count a `status` request (API-5). `exit_after`'s default is (proposal). How tokio's timers behave across a suspend isn't checked (docs/04 §8 item 6).

Follow-ups: T51

## Evidence
- **2026-11-26**, branch `t28-daemon-run`, executor on **Opus, medium**; reviewed and merged by the orchestrator. **M2's checkpoint.**
- `src/daemon/run/` (`mod.rs`, new `start.rs`, `life.rs`, `serve.rs`, `entry.rs`, `tests.rs`) and `tests/m2_serve.rs`. `StartError` gained its variants. **Checked by the orchestrator:** `Env::from_process` and `RealPoster` appear only on the real `main` path; tests build their own deps, and `run_main` tests pass an `Env` they built and stop before `start`.
- `cargo test --features test-endpoint --lib daemon::run::`: 6 passed; `--test m2_serve`: 26 passed, run 5 times by the orchestrator after rebasing: 0 failures. `make verify` passes on main after the merge (491 lib tests, 26 m2_serve, 17 fake_github, 9 golden), the macOS check included.
- Choices: `deps_from(spawned, &Env, Arc<dyn LogSink>, &Logger)` also does `resolve` and `config::load`; a second request logs `warn stop deadline cause=second_request`; `log level unknown` doesn't carry the value; the test rig gives the first poll 1 s after `start` (POL-2); in step 6 a socket answering `spawned` under a foreground lock gets one shutdown and is polled every 100 ms until the 15 s; no holder line after the patience is `StartError::NoHolder`, code 1, in both modes.
- **Gap, turned into follow-up T51 (orchestrator's decision):** a second `POST /v1/shutdown` during the stop isn't noticed (only `Running::stop` and the signal loop fire the second-request `Notify`), against OPS-7's "a second request during it ends it at once".
- Not run here: real signals, the real `main` and its panic hook (T36); macOS only compiled.
