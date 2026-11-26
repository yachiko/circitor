---
id: T29
title: "M3: the harness, and the CLI against a running daemon"
kind: integration
milestone: M3
size: L
status: done
depends_on: [T07, T20, T25, T26, T27, T28, T45]
requirements: [TST-5, TST-11, TST-16]
owns: [src/testkit/harness/, tests/m3_cli.rs]
shares: []
---

# T29 · M3: the harness, and the CLI against a running daemon

## Outcome
`testkit::harness::Harness` starts the daemon in-process on the test clock with the fake GitHub, a fake token source and a fake session socket, stops, crashes and restarts it, runs CLI commands as functions with their output and exit code, and ends every test with TST-11's sweep. Proven by the CLI's commands against it, a command that starts its daemon through the fake spawner included.

## Context
Everything exists: the daemon (`daemon::run::start`, T28) sending through the real client (T07), the CLI (`cli::run`, T27), the fake GitHub (T20), the fakes of T43 (`FakeSpawner`, `FakeTokenSource`, through T27 and T24), `FaultFs` (T16), and the fake session socket (T45), which the daemon's real poster (T46, through T28) writes to. T26 has already run the engine and the client against the fake in one process. This task joins them into the one fixture T30 to T35 write every scenario on, and is the first time the CLI meets a real daemon.

**It's also where the daemon first runs with the engine's demand side** (T25: hot refreshes, forced refreshes, reading through). T28 depends on T21 only, so its tests never see them; `m3_waiter_makes_hot` and `m3_show_reads_through` below are their first end-to-end proof. T42 (`gone`, `pr_failing`) isn't a dependency here: the scenario tasks that need it depend on it themselves. **Neither are the notify routes and the `notify` command (T47, T48):** this task binds the session socket, hands it to every command and sweeps what it recorded; T49 is the first to register through it. **A bug it finds in another task's code is reported, not fixed here.**

## Read first
- `plan/conventions.md` ("Tests", "Work in flight")
- `plan/contracts/testkit.md` (all of it: what the harness must offer, the fake session socket and the markers among it); `plan/contracts/notify.md`: `RealPoster`, and "The fake"; `plan/contracts/daemon.md`: `Deps`, `start`, `Running`, "Semantics"; `plan/contracts/cli.md`: `CliEnv`, `Output`, `Interrupt`, the fakes; `plan/contracts/clock.md`: `TestClock`, `SettleProbe`, "Semantics", "Don't assume"; `plan/contracts/config.md`: `Env`, `Platform`, `resolve`; `plan/contracts/github.md`: `FakeTokenSource`; `plan/contracts/state.md`: `FaultFs`; `plan/contracts/log.md`: `BufferSink`
- `specs/07-testing.md#test-doubles`: TST-5, TST-16, TST-4 ("settled"); `specs/07-testing.md#what-is-tested`: TST-11; `specs/07-testing.md#rules`: TST-1, TST-2
- `specs/05-cli.md#for-tests`: CLI-13; `specs/05-cli.md#the-daemon`: CLI-7, CLI-9 (what the spawner and version cases must show)
- `specs/04-api.md#starting-up`: API-18 (why a `list` waits before the first poll); `#waiting`: API-13; `specs/02-polling.md#cadence`: POL-14, POL-16 (what the two demand tests check); `specs/06-operations.md#configuration`: OPS-2
- The module docs of `src/testkit/server/` (T20 chose the names) and `tests/m2_serve.rs` (how T28 wires a daemon in a test)

## Scope
- **`Harness::start(opts)`** (TST-5, `TST-5:unless-a-test-says-otherwise`). Defaults: `mode` `spawned`; `Config::default()` with `exit_after` `None`; a `BufferSink` at `info`; `TestClock` at 2026-11-24T22:00:00Z; a fixed seed; `FakeTokenSource` giving `MARKER_TOKEN`; `FaultFs`; `signals: false`; `TEST_VERSION` for the daemon and the CLI. `start` ends with `clock.settle()`.
- **`opts` sets everything that must hold before the daemon's first request**, exactly the list in `plan/contracts/testkit.md`: a closure that scripts the world; config values (`owners`, `keep_done`, `exit_after`, `quota.*`, the intervals); faults and a held answer in force from the first request; the token source's first answer and its delay; the fake GitHub's clock skew; the daemon's version and `mode`; and **`no_daemon`**: nothing runs yet, the default spawner starts the in-process daemon when a command asks, and the test can read its count of spawns.
- **Paths.** One `ShortDir`. Build an `Env` with `XDG_STATE_HOME` and `XDG_CONFIG_HOME` inside it and no `XDG_RUNTIME_DIR`, and resolve it once with `config::resolve(&env, Platform::Linux)`: the daemon gets those `Paths`, every command gets that `Env`, so both agree without `HOME` and the socket path stays short.
- **The session socket (TST-16's share here: "a command is given its path and a marker token in its environment"; the socket itself is T45's).** `start` binds a `testkit::session::FakeSession` named `session` in the `ShortDir`, on the test clock, also with `no_daemon`. `session()` hands it out; `new_session(name)` binds another. **Every command's `Env` holds `cli::notify::SOCKET_VAR` set to its path and `TOKEN_VAR` set to `MARKER_SESSION_TOKEN`**; `env()` gives that `Env`, and the environment variant can take the two out or change them. The sockets live as long as the harness: a restart and a crash leave them as they are.
- **The daemon.** `Deps` as `plan/contracts/daemon.md` lists them, with `clock.activity()`, the fake GitHub's `client()`, `RealProbe` and **`notify::RealPoster`**: the daemon posts to the fake session socket for real. `stop()` (the exit status), `crash()`, `restart()` and `restart_with(config)` on the same directories and the same `FaultFs`; a daemon with another version or `mode` in place of the first. **The daemon's own exit is visible:** `exited() -> Option<i32>`, from `Running::exit_status` (an idle exit, a stop for files not its own), and a `restart()` works after it. **Every start, restart and crash calls the fake GitHub's `new_daemon()`**, and **each start gets another seed** (the fixed seed plus the start's number): the store's epoch comes from the seed alone, and a restart must give a new one (STA-9).
- **Commands** (`TST-5:each-command-takes-its-inputs`, CLI-13). `cli(&[…])` awaits `cli::run` with a `CliEnv` on the test clock: UTC, the `ShortDir` as working directory, the `Env` above, `TEST_VERSION`, a spawner, `cli::commands::RealGit`, `Interrupt::never()`, 90 s. Variants set the working directory, the environment, the CLI's version and the spawner.
- **`cli_bg(…)`**, for a command the test advances time under. Its handle says whether the command is done without awaiting, gives the `Output` when awaited, and has **`cancel()`**: the command's future is dropped, so its connection closes, and `cancel` returns only once the daemon has seen the close (`Running::open_connections` back to what it was). A test can read `waiters` right after.
- **Background commands and settling.** The test clock doesn't count a command's socket I/O. The harness adds a `SettleProbe` that's unsettled while a background command has been woken and not yet polled: then an answer the daemon sent is read before time moves on. It follows `plan/contracts/clock.md`'s rule: a command parked on a timer or on the socket is settled, or `settle` never returns. `advance` settles before its first step. Work the fake GitHub holds by script is excused by the fake itself: nothing to build for it.
- **Spawners** (TST-5): four ready-made ones on `FakeSpawner`, as `plan/contracts/cli.md` declares it: `starts(f)`, whose closure starts the in-process daemon on a task and returns a `Spawned` that follows it (the harness keeps its `Running`); `exits(1, Some(<an OPS-1 message>))`; `exits(0, None)`; `never_binds()`.
- **`git_repo(origin, branch)`** (`TST-5:the-harness-can-make-a`): `git init` in a new directory under the `ShortDir`, that branch checked out, `origin` set. Run `git` through `tokio::process`.
- **`api_raw(method, path_and_query, body) -> (status, body)`**: one request over the socket, written with `hyper`, for what the CLI can't send (an unknown parameter, `timeout=3601`; TST-7). And the accessors: `clock()`, `github()`, `token()`, `fs()`, `log()`, `state_file()`, `paths()`.
- **`finish()`** (TST-11, your share: the sweep; the fake GitHub's markers are T20's). It fails if a background command is still running. With a daemon running it fetches a `status`, a `list` (`state=all`, `checks=1`) and a `changes`, then stops it. It searches for `MARKER_TOKEN` and `MARKER_TOKEN_2` in the log, the state file, every `Output` a command returned (`--json` output is the API's body), every `api_raw` answer and those three answers; **and in the same places for `MARKER_SESSION_TOKEN`** (NTF-9: a session's token is a marker too). It searches the log and the state file for `MARKER_TITLE`, `MARKER_GITHUB_TEXT` and `MARKER_CHECK` (never `MARKER_LOGIN` there: the state file keeps a PR's login, STA-6). **It calls `notices()` on every fake session socket**, so a malformed line fails the test, and searches each notice's text for `MARKER_TITLE`, `MARKER_GITHUB_TEXT`, `MARKER_CHECK` and `MARKER_LOGIN` (NTF-8: nothing a GitHub user wrote is in a notice). A notice's auth line may hold the session's token: that's where it belongs. Then the fake GitHub's `check()`. A `Harness` dropped without `finish()` panics, unless the thread already panics.
- **`tests/m3_cli.rs`**: the cases under Acceptance.

## Interfaces
- **Consumes:** `daemon::{Deps, Running, Mode, StopCause}` (with `Running::exit_status` and `Running::open_connections`) and `daemon::run::start`, `cli::{run, CliEnv, Output, Interrupt}` and `cli::fake::FakeSpawner`, `testkit::server::FakeGitHub`, `testkit::session::FakeSession`, `notify::RealPoster`, `cli::notify::{SOCKET_VAR, TOKEN_VAR}`, `testkit::{ShortDir, TEST_VERSION, MARKER_TOKEN, MARKER_TOKEN_2, MARKER_SESSION_TOKEN, MARKER_TITLE, MARKER_GITHUB_TEXT, MARKER_CHECK, MARKER_LOGIN}`, `token::fake::FakeTokenSource`, `state::file::FaultFs`, `log::BufferSink`, `clock::TestClock`, `config::{Env, Platform, resolve, Config}`.
- **Provides:** `testkit::harness::Harness` with what `plan/contracts/testkit.md` lists, and nothing beside it, for T30 to T36 and T49. Where the contract gives no name, it's yours: document each in the module's docs, with one example test.

## Out of scope
- The scenarios: T30 to T33, and notify's: T49. Quota figures: T34. Golden output and each exit code: T35. Real processes, the real spawner, signals: T36.
- The fake session socket itself: T45. The notifier, its routes and the `notify` command: T46, T47, T48. No test here runs `notify`: it may still be T02's stub.
- The fake GitHub, the daemon, the engine's demand side, the CLI: T20, T28, T25, T24, T27. A test that fails on their code is kept, marked `#[ignore = "Tnn: …"]` and reported.
- `src/testkit/mod.rs` is T02's: add files under `src/testkit/harness/` only.

## Acceptance
`tests/m3_cli.rs`, each test on `#[tokio::test]`, each ending in `h.finish()`. The world has `o/r#1`, open, with passing checks, `MERGEABLE` and `updatedAt` two minutes before the start (so it reads `ready`, and no follow-up or naming keeps it hot), by the fake's login, and `o/x#2`, another author's.
- `m3_each_command`: after an advance of 5 s (the first poll), `list`, `show o/r#1`, `changes`, `track o/x#2`, `untrack o/x#2`, `refresh`, `status` and `stop` each exit 0; `list`'s last line starts `cursor: `; `status --json` decodes into `Envelope<StatusBody>` with `mode` `spawned`; after `stop` the socket file is gone.
- `m3_wait_is_satisfied`: `wait o/r#1 --until merged` in the background, the world merges the PR, the clock advances: exit 0, the outcome line `merged`.
- `m3_waiter_makes_hot` (POL-14, API-13; the test T28 gave up): a background `wait o/r#1 --until merged` begun 20 s after the start (outside POL-15's 5 s window), then 60 s of advance. In `h.github().requests()`, a request naming `o/r#1`'s node ID comes within 1 s of the wait's start (the arrival refresh), and from it on every gap between consecutive requests naming it, polls included, is 10 s to 12 s (at least five gaps). After `cancel()` and 60 s more, no request without a finding names it.
- `m3_show_reads_through` (POL-16): `show o/x#2` (untracked, another author's) in the background, then an advance: exit 0, its line names `o/x#2`; the log holds one request with a by-name lookup of `o`, `x`, `2` and no finding; `status --json` then reads `tracked.asked` 1.
- `m3_cancel_frees_the_waiter`: a background `wait` is held (`status` shows `waiters` 1, the handle isn't done); right after `cancel()`, `waiters` is 0.
- `m3_wait_times_out`: `wait o/r#1 --until merged` with nothing changing ends at 100 s with exit 3 and the three lines `timeout after 100s: o/r#1 …`, `again: circitor wait o/r#1 --until merged --since <c>`, `cursor: <c>`; `status` then shows `waiters` 0.
- `m3_command_starts_the_daemon`: with `no_daemon`, `list` exits 0 and the spawner's count is 1; a second `list` leaves it at 1.
- `m3_spawner_fails`: `exits(1, Some(…))` gives exit 6 before the clock has moved 1 s, with the OPS-1 message on stderr; `never_binds()` and `exits(0, None)` with no holder give exit 6 at 8 s; `--no-spawn` gives exit 6 with zero spawns.
- `m3_older_daemon_is_replaced`: daemon `1.2.2+a`, the CLI at `TEST_VERSION` (`1.2.3+test`): one spawn, and `status` shows `1.2.3+test`. `m3_other_daemons_are_kept`: a newer one, an older `foreground` one and one with version `banana` each answer `list` with exit 0, a warning on stderr and zero spawns (CLI-9).
- `m3_bare_pr`: in `git_repo("https://github.com/o/r.git", <the PR's head>)`, `show` prints `o/r#1`'s line; outside it, exit 2.
- `m3_token_outcomes`: with the token source set to no login, `list` exits 7 and stderr names `gh auth login`; set to slow, `status` answers while the read is held.
- `m3_crash_and_restart`: after `crash()` the socket file is still there; `restart()` serves `list` again, with `o/r#1` in `records` or in `pending`, and a cursor from before it gets `reset: cursor too old, current state follows` (another seed, another epoch).
- `m3_own_exit_and_restart`: `restart_with` a config whose `exit_after` is 1 s; after an idle advance `exited()` is `Some(0)`; `restart()` then serves `status`.
- `m3_opts_hold_from_the_first_request`: a fault set in `opts` is met by the daemon's first request (the fake's first recorded request got it); a world scripted in `opts` is what the first `list` shows.
- `m3_api_raw`: `GET /v1/prs?nope=1` and a wait with `timeout=3601` each answer 400 with `bad_request`.
- `m3_sweep_finds_a_token`, `m3_sweep_finds_the_second_token`, `m3_sweep_finds_a_title_in_the_log`: `MARKER_TOKEN` planted in a command's output, `MARKER_TOKEN_2` planted among the kept `api_raw` answers, and `MARKER_TITLE` in the log buffer each make `finish()` panic (`#[should_panic]`). `m3_drop_without_finish_panics`.
- `tst16_a_command_has_the_session`: `h.env()` holds `CLAUDE_CODE_MESSAGING_SOCKET` equal to `h.session().path()` and `CLAUDE_CODE_MESSAGING_TOKEN` equal to `MARKER_SESSION_TOKEN`; the socket exists with `no_daemon` too, and is the same file after `restart()` and `crash()`; `new_session("b")` is another path in the same directory.
- `tst16_the_daemon_can_post_to_it`: a notice the test posts to `h.session().path()` through a `notify::RealPoster` of its own, with the marker token, is in `h.session().notices()` with that token and the clock's time; `finish()` passes.
- `m3_sweep_finds_a_session_token`, `m3_sweep_finds_a_title_in_a_notice`, `m3_sweep_finds_a_malformed_notice` (`#[should_panic]`): `MARKER_SESSION_TOKEN` planted in the log buffer; a notice whose text holds `MARKER_TITLE`, and one holding `MARKER_LOGIN`, posted to `h.session()`; a plain-text line posted to it. Each makes `finish()` panic.
- `cargo test --features test-endpoint --test m3_cli` and `make verify` pass.

## Traps
- **The clock never moves by itself.** An awaited `cli()` hangs if its command waits on the clock: a `list` before the first poll, every `wait`, every spawn, and a `show` that reads through (its request waits out POL-2's 1 s gap). Those are `cli_bg`, an advance, then the await.
- **Titles belong in CLI output and API answers.** `MARKER_TITLE` is searched in the log and the state file only; the token everywhere.
- `stop` on a daemon that doesn't answer signals the lock file's process, which here is the test itself. T24 refuses its own process; don't write a harness test that leans on that path. It's T36's.
- Everything is on one current-thread runtime. A future given to `tokio::spawn` must be `Send`; if `run`'s or `start`'s isn't, use a `LocalSet` inside the harness, and say so in the report.
- A restart must reuse the `FaultFs`, the `ShortDir` and the fake GitHub, and make a new `Deps`: the old lock is released only when the old `Running` is gone.
- `finish()` reads the state file after the stop, when the last write has landed; after a crash it reads what's there.
- The sweep proves nothing if it never fails: keep the `should_panic` tests.
- An idle exit in a test needs `exit_after` of 1 s, which only a test build's range check allows (TST-1); build the `Config` by hand, don't load it.
- No real `gh`, no real token, no host but the loopback (TST-1). The real `git` is run only inside the `ShortDir`.
- **No real session socket.** The shell that runs you has `CLAUDE_CODE_MESSAGING_SOCKET` set: the harness builds every `Env` itself and never reads the process's, or a test's notice lands in a live session.

## Open questions
- **"Every API response" (TST-11):** a command's answers pass inside `cli::run`, where the harness can't see them. `plan/contracts/testkit.md` settles what's swept: every `Output`, every `api_raw` answer, and three answers fetched at `finish()`.
- **TST-5 gives the daemon its paths "directly, not through `HOME`"**, while the CLI finds them through its `Env`. Built: XDG variables under `/tmp`, resolved once for both.
- Whether tokio's timers count a suspend (docs/04 §8 item 6) doesn't touch the harness: the test clock plays both (TST-4).

## Evidence
- **2026-11-26**, branch `t29-harness`, executor on **Opus, medium**; reviewed and merged by the orchestrator. **M3's checkpoint.**
- `src/testkit/harness/` (`mod.rs`, new `command.rs`, `spawn.rs`, `raw.rs`) and `tests/m3_cli.rs`. **Checked by the orchestrator:** the harness builds every `Env` itself, setting `CLAUDE_CODE_MESSAGING_SOCKET` to its own `FakeSession` in the `ShortDir` and the token to `MARKER_SESSION_TOKEN`; it never reads the process's environment; the daemon's `RealPoster` can only post to that fake socket.
- `cargo test --features test-endpoint --test m3_cli`: 26 passed, every test ending in a passing sweep; the executor ran it 11 times (3 under `--test-threads=64`), the orchestrator 5 more after rebasing: 0 failures. `make verify` passes on main after the merge (576 lib, 26 m2_serve, 26 m3_cli, 8 m1_poll, 17 fake_github, 9 golden), the macOS check included. No `LocalSet` needed.
- Names chosen (documented in the module): `Opts` builders, `Harness::{take_hold, spawner, spawns, dir, restart_as}`, `Cmd` with `.cwd/.env/.var/.without/.version/.spawner/.interrupt`, `Bg::{is_done, cancel}` and `.await`, `spawner_bad_config`, `spawner_exits_ok`, `spawner_never_binds`, `SEED`, `START`.
- Choices: a daemon the harness's spawner starts is `spawned` at `TEST_VERSION` (`Opts::version`/`mode` apply to daemons the harness starts itself); the spawner's task holds `Busy` across `daemon::start`; `finish()` and `stop()` advance the clock in 100 ms steps when settled and not done, up to 60 s; the sweep also reads the on-disk `circitor.log` and searches notices for the tokens too.
