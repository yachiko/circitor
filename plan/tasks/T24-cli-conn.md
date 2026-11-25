---
id: T24
title: "CLI and the daemon: connecting, starting, replacing, stopping, request limits"
kind: code
milestone: M3
size: L
status: todo
depends_on: [T06, T17, T18, T41]
requirements: [CLI-7, CLI-9, CLI-8, CLI-10, OPS-4:the-cli-also-tries-the, OPS-5:a-spawned-daemon-uses-gh-s-login, OPS-9:the-log-file-is-rotated, U9]
owns: [src/cli/conn/]
shares: []
---

# T24 · CLI and the daemon: connecting, starting, replacing, stopping, request limits

## Outcome
`cli::conn` gets a command its daemon: it connects by CLI-7's rules, starts a detached daemon when there's none and watches it come up, replaces an older one (CLI-9), sends requests under CLI-7's limits with CLI-10's one retry, and carries out `stop` (CLI-8).

## Context
T41 gives the golden answers the tests' stub daemon serves. T17 gives the connection and the probe, and through it T43 gives `cli::fake::FakeSpawner`; T18 the lock's reader and its test, T06 the paths, the private directory and the config check. T27 calls the five functions here and nothing below them; T29 runs them against a real in-process daemon, T36 against real processes. **Everything here runs on the injected clock**, so its tests cost no real time.

## Read first
- `plan/conventions.md`
- `plan/contracts/cli.md`: the block "`src/cli/conn/` (T24)" (the signatures you build), `CliEnv`, `Spawner`, `Spawned`, the fakes; `plan/contracts/transport.md` (all of it); `plan/contracts/daemon.md`: `instance::read_holder`, `instance::is_held`, `Holder`; `plan/contracts/config.md`: `Paths`, `ensure_private_dir`, `load`; `plan/contracts/api.md`: `Request`, `StatusLite`; `plan/contracts/clock.md`: `timeout`, `sleep`, `TestClock`, "Don't assume"
- `specs/05-cli.md#the-daemon`: CLI-7 to CLI-10; `specs/05-cli.md#waiting`: CLI-6's "one deadline" bullet (what `budget` is)
- `specs/06-operations.md#files`: OPS-3, OPS-4; `#the-token`: OPS-5's second bullet; `#the-daemons-life`: OPS-7, OPS-9's rotation bullet
- `specs/03-state.md#one-daemon`: STA-11 (the lock's test, the lock file's line); `specs/04-api.md#basics`: API-2; API-17
- `docs/03-clients.md` §2 (the two sandbox keys), §6 item 1

## Scope
- **Connecting** (`CLI-7:connecting-tries-the-cli-s-socket-path`, `OPS-4:the-cli-also-tries-the`): `paths.socket`, then the socket `read_holder(&paths.lock_file)` names, if it's another path. "Answers" is `transport::probe` (OPS-4's 2 s). The second is tried only when the first is `NoDaemon`; when it answers, one warning line names both paths. Used by `ensure`, `existing`, `reconnect` and `stop` alike. `Link::socket` is the path that answered.
- **No daemon, or an error** (`CLI-7:only-a-missing-socket-file`): `NoDaemon` on both is the only "no daemon". `NotAnswering` starts nothing: exit 6, naming the path and the system's error. `Probe::Denied` starts nothing either: exit 6, naming `sandbox.excludedCommands` and `sandbox.network.allowUnixSockets` instead.
- **Starting** (`CLI-7:before-starting-it`, `OPS-9:the-log-file-is-rotated`, U9's share: the CLI starts the daemon; its idle exit is T28's): `ensure_private_dir` for `state_dir`, and for `socket_dir` when it's `Some`; a `circitor.log` **over** 5 000 000 bytes is renamed to `circitor.log.1`, replacing it; open the log for appending and note its size; `env.spawner.spawn`. A failure in any of these is exit 6 at once, naming the path and the error.
- **Watching** (`CLI-7:the-cli-watches-the-process-it-started`): every 100 ms of the clock, ask `exited()` and probe, for up to 8 s. A non-zero exit ends the wait at once; exit 0 goes on, probing the socket the lock file names; the socket answering ends it. Exit 6 carries up to 20 lines of the log from the noted size on.
- **The real spawner** (`OPS-5:a-spawned-daemon-uses-gh-s-login`): `pub struct RealSpawner` in `src/cli/conn/`. It runs `std::env::current_exe()` with `daemon --spawned`, the environment of the `Env` given without `GH_TOKEN` and `GITHUB_TOKEN` (OPS-5) and **without `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN`** (CLI-7, NTF-9: the daemon's environment never holds a session's socket or token), `/` as working directory, stdin from `/dev/null`, stdout and stderr to the log, and `setsid` in `pre_exec`. Build the command in a function of its own, so a test can read it.
- **`--no-spawn`** (`CLI-7:no-spawn`): with `no_spawn`, no daemon is exit 6 and nothing is started or replaced; an older daemon is used, with a warning.
- **Replacing (CLI-9).** After connecting, compare `StatusLite.version` with `env.version` by `semver` precedence (`CLI-9:versions-are-compared-as-semantic-versions`). Older and `mode` `spawned`: check the config (`config::load`, `CLI-9:before-replacing`), send `Shutdown`, wait up to 8 s for `is_held` to turn false, start your own as above. Kept, each with one warning line: a lock still held, a config that fails (naming the key), a newer daemon, a `mode` other than `spawned`, a version that doesn't parse (`CLI-9:a-newer-daemon`). `CLI-9:at-most-one-replacement-per-command`: the `Link` remembers. `CLI-9:checking-first-means-no-answer-is-thrown-away`: the status is read before any other request. `Link::older_unreplaced` is true for an older daemon that was kept; T10 and T27 turn it into `CLI-9:a-bad-request-or-unknown-route-from`.
- **`budget`** (CLI-6's deadline, kept by T27): with `Some(b)`, no wait in `ensure` or `reconnect` outlasts `b`. Out of budget is the same `Err` as out of the 8 s; T27 tells them apart by its own clock.
- **Requests** (`CLI-7:every-request-has-a-limit`, your share: every request but a wait; `CLI-10:any-other-command-whose-connection`): `Link::request` connects, sends and reads the body inside `env.request_limit` (90 s). Past it: exit 6, `daemon not answering on <path>`. A `Dropped`, or a connect that fails, is tried once more as a new command: connect again by the rules above, start a daemon if the `Link` came from `ensure` without `no_spawn`, replace none, all inside the same limit. A second drop is exit 6. The `Link` keeps the paths and the flags, as `request` isn't given them.
- **`Link::open`** (a wait): connect and send, return the head. No limit, no retry; a connect that fails is `Dropped`. **The wait's limits and CLI-10's rule for a wait are T27's.**
- **`reconnect`** (CLI-10, for a wait that had its cursor): connect as above every 100 ms for up to 8 s, inside `budget`. It starts nothing and replaces nothing.
- **`existing`** (CLI-8): connect only. `Ok(None)` is "not running"; nothing is started or replaced.
- **`stop`** (CLI-8, `CLI-8:stop-returns-once-the-daemon-has-exited`, `CLI-8:stop-ends-a-daemon-that-doesn-t-answer`): no daemon on both paths is `Ok(false)`. The second-socket warning goes to `warn`. Otherwise send `Shutdown` without reading the status first (API-2), under `env.request_limit`. Answered, or dropped: wait up to 8 s for the lock to be released, then `Ok(true)`. No answer in time, or the lock still held: `SIGTERM` to the lock file's process, `SIGKILL` 5 s later, **each only while `is_held` is true**. Then wait up to 2 s more for the lock (this brief's value); still held is exit 6, naming the process. `Denied` is exit 6 and signals nothing. Send signals through a private function argument, so a test can record them.
- **Messages.** `ConnError.stderr` and `warn` hold whole lines. An OS error's text goes last on its line, after `: ` (T35 replaces it). Each duration and size above is one named constant.

## Interfaces
- **Consumes:** `transport::{probe, connect, Conn, Head, Probe, Dropped}`, `daemon::instance::{read_holder, is_held}`, `config::{Paths, Env, ensure_private_dir, load}`, `api::types::{Request, StatusLite}`, `clock::{timeout, sleep}`, `cli::{CliEnv, Spawner, Spawned}`, `semver`, `nix`.
- **Provides:** `ConnError`, `Reply`, `Link`, `ensure`, `existing`, `reconnect`, `stop`, as `plan/contracts/cli.md` declares them; `RealSpawner` (T27's `CliEnv::real` uses it); and, under `cfg(test)`, `pub(crate) mod stub`: the stub daemon below, which T27's tests reuse.

## Out of scope
- A wait's deadline, its limits, its drop rule and its endings; `status`' and `stop`'s output; `CliEnv::real`: T27.
- Text of answers and of API errors: T10. Parsing `--no-spawn`: T11.
- The lock, the socket file and the probe themselves: T18, T17. The daemon's side of a start, a stop and the idle exit: T28.
- Real processes, `setsid` seen from outside, real signals: T36.

## Acceptance
Unit tests in `src/cli/conn/`, on `TestClock`, a `ShortDir`, T43's `FakeSpawner` and a stub daemon: a few lines of server on a Unix socket that answers `testdata/api/` files (T41's) per route with a version and `mode` of the test's choice, and can accept without answering, close without answering, and count its requests. It implements `clock::SettleProbe`.
- `cli7_first_path_answers`; `ops4_lock_file_socket_answers_with_a_warning`: the warning holds both paths.
- `cli7_no_daemon_spawns_once`; `cli7_no_spawn_is_exit_6`: zero spawns.
- `cli7_not_answering_starts_nothing`: a stub that never answers gives exit 6 after 2 s, zero spawns. `cli7_denied_names_the_sandbox_keys`: a socket in a directory of mode 000 (skipped as root).
- `cli7_state_dir_is_0700`; `ops9_rotates_over_5_000_000_bytes`: 5 000 001 bytes are renamed over an older `.1`; 5 000 000 stay.
- `cli7_child_exits_1`: `exits(1, Some(line))` gives exit 6 before the clock has moved 1 s, with that line; with 25 lines written after the spawn, the last 20 and none from before. `cli7_child_exits_0_waits_for_the_holder`. `cli7_never_binds`: still waiting at 7.9 s, exit 6 at 8 s. `cli6_budget_caps_the_wait`: with 3 s, `Err` at 3 s.
- `cli9_older_spawned_is_replaced`: daemon `0.1.0+aaa`, CLI `0.2.0+bbb`: one `POST /v1/shutdown`, one spawn, the next request goes to the new stub. `cli9_build_metadata_is_ignored`: `0.2.0+aaa` against `0.2.0+bbb`, no warning. `cli9_newer_is_kept`, `cli9_foreground_is_kept` (`older_unreplaced` true), `cli9_unparsable_is_kept`, `cli9_bad_config_keeps_it` (no shutdown sent, the key named), `cli9_lock_held_after_8s_keeps_it`, `cli9_replaces_once`: each with its warning.
- `cli7_request_limit_90s`: exit 6 at 90 s with `daemon not answering on <path>`. `cli10_one_retry`: a drop then an answer is `Ok` after two requests; two drops are exit 6.
- `cli10_reconnect`: a stub that appears at 3 s is `Ok`; none is `Err` at 8 s, or at 2 s with that budget; zero spawns.
- `cli8_existing_none`; `cli8_stop_not_running`; `cli8_stop_waits_for_the_lock`: released at 3 s gives `Ok(true)` and no signal. `cli8_stop_signals`: with `request_limit` 2 s and a held lock, `SIGTERM` at 2 s, `SIGKILL` 5 s later; released in between, no `SIGKILL`; never to this process.
- `ops5_spawned_env`: the real spawner's command, built from an `Env` that has all four, has no `GH_TOKEN`, `GITHUB_TOKEN`, `CLAUDE_CODE_MESSAGING_SOCKET` or `CLAUDE_CODE_MESSAGING_TOKEN`, keeps `GH_CONFIG_DIR` and `XDG_CONFIG_HOME`, and runs in `/`.
- `cargo test --features test-endpoint --lib cli::conn::` and `make verify` pass.

## Traps
- **`semver`'s `Ord` compares build metadata.** Use `cmp_precedence`.
- **Never signal blindly.** Read the holder and test `is_held` right before each signal. No holder, a process ID of 0 or 1, or this process's own: no signal. Under the harness the daemon runs in the test's process.
- A regular file at the socket path refuses a connection, so it reads as "no daemon"; the daemon you start then stops on it (OPS-4). That's right: report its log lines.
- The stub's and the CLI's socket I/O isn't work the test clock counts. Spawn the call, advance to the moment under test, then await it; never put an answer and a limit inside one advance.
- The child must outlive the CLI and hold none of its streams: no `kill_on_drop`, nothing inherited. `pre_exec` is the one `unsafe` in the crate: allow it on that function only, with a comment.
- Every wait is `clock::timeout` or `clock::sleep`: no tokio timer, no `std::process::Command`.
- The lock file's socket path may hold spaces. Use `read_holder`; don't parse the line.
- The log's tail is the daemon's own lines; print them as they are, never the whole file.

## Open questions
- CLI-7's values and rules, and CLI-9, are (proposal): build them as written. What a sandbox's refusal looks like isn't measured (docs/03 §6 item 1): `Probe::Denied` is `EACCES` and `EPERM`.
- **Not said by the specs, built as Scope says, and reported to the planner:** that `stop` skips the status read; the 2 s after `SIGKILL`; `--no-spawn` with an older daemon; a `mode` that is neither word; that the second path isn't tried when the first doesn't answer.
