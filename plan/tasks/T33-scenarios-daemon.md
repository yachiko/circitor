---
id: T33
title: "Scenarios: the daemon's life and failures"
kind: test
milestone: M5
size: L
status: done
depends_on: [T29]
requirements: [TST-8]
owns: [tests/scn_daemon.rs]
shares: []
---

# T33 · Scenarios: the daemon's life and failures

## Outcome
Eleven of TST-8's scenarios run on the harness and pass: `cursor-reset`, `github-down`, `token-rotated`, `sleep`, `restart`, `starting`, `first-run`, `restart-offline`, `crash`, `files-removed`, `clock-back`.

## Context
T29's harness runs the whole daemon in-process on the test clock, against the fake GitHub (T14, T20), with a fake token source and the CLI's commands as functions; it can stop, crash and restart the daemon on the same directories. This task plays the eleven TST-8 rows about **the daemon's own life and its failures**: starting, restarting, crashing, GitHub down, the token, sleep, and the clock. T30, T31 and T32 play the other rows. Its failures are how bugs in T13, T15, T16, T21, T22 and T28 are found.

## Read first
- `plan/conventions.md`: Tests, Requirement tracing, Report.
- `plan/contracts/testkit.md`; `plan/contracts/clock.md` (`advance`, `sleep_wall_only`, `sleep_both`, `set_wall_back`, `SleepWatch`); `plan/contracts/daemon.md` (`start`'s order, `crash`, `StopCause`); `plan/contracts/state.md` (`FileEntry`, the saver); `plan/contracts/github.md` (`FakeTokenSource`, `TokenRead`); `plan/contracts/api.md` (bodies, `StatusBody`, the error body); `plan/contracts/log.md` (who logs what); `plan/contracts/cli.md` (`Output`, exit codes, `FakeSpawner`).
- The module docs of `src/testkit/harness/`, `src/testkit/server/` and `src/testkit/github/`: the real names of the controls. `tests/m3_cli.rs` and `tests/m2_serve.rs` show tests driving the harness and a restart.
- `specs/07-testing.md#what-is-tested`: TST-8, the eleven rows named in the Outcome. `#test-doubles`: TST-3 to TST-5.
- `specs/02-polling.md#errors`: POL-19, POL-20, POL-22, POL-26; `#requests`: POL-3; POL-16.
- `specs/03-state.md`: STA-2, STA-5, STA-7 to STA-11. `specs/04-api.md`: API-3, API-4, API-12, API-18.
- `specs/06-operations.md`: OPS-4, OPS-5, OPS-7, OPS-8, OPS-9, OPS-16. `specs/01-model.md`: MOD-15 (a loaded PR's first event), MOD-16.
- `docs/02-github-api.md` §5 (the 401's body; 5xx isn't measured). `docs/04-platform.md` §8 item 6: why `sleep` runs both ways.

## Scope
**TST-8's share here: these eleven rows**, in `tests/scn_daemon.rs`. The row is the spec; each entry below adds only the world's concrete values and what to assert. **Where a row and the requirement it cites differ, the requirement wins** (TST-8): then assert the requirement and report the row.

How every scenario is written:
- **One `#[tokio::test] async fn scn_<name>()` per row**, hyphens as underscores, with `// TST-8: <name>` and the row's IDs (`// POL-19:`) above it. A row with several worlds may be several functions named `scn_<name>_<part>`, so an `#[ignore]` costs one part. Each starts its own harness and **ends with `h.finish()`**.
- **Script the world, the faults, a held answer, the token source and the config before the daemon's first request**, through `Harness::start(opts)` (testkit.md). Half of these rows are about that first request.
- **Look through the CLI:** `--json` output parsed as JSON, `Output.code`, `status --json` for `daemon.problem`, `waiters`, `tracked`, `cost_last_hour` and `warnings`; `h.state_file()`, `h.log()`, `h.exited()` and `h.github().requests()` for the rest. **A log line is found by its event word** (`plan/contracts/log.md`: `problem begin code=…`, `problem end`, `sleep seconds=…`, `stop cause=…`, `token read failed`), never by a sentence. Assert text only where an entry quotes a line; the layout is T35's.
- **Advance and look; never sleep.** A command that needs clock time runs under `cli_bg`; the test advances, then awaits it.
- Keep the spec's values as named constants (`BACKOFF_FIRST = 5 s`, `BACKOFF_MAX = 300 s`, `REQUEST_TIMEOUT = 20 s`, `STARTING_WAIT = 10 s`, `TOKEN_EVERY = 10 min`, `HISTORY = 500`, `SLEEP_GAP = 30 s`): most are (proposal).

Names: login `octocat`; the user's PRs are `octo/app#N`, other people's `else/lib#N`. "Settled PR": `updatedAt` two minutes old at the first read. `<epoch>-0` is the cursor before any revision (STA-9).

The scenarios:

- **`cursor-reset`** (STA-9). (a) Take `c1` from `list --json`; `h.restart()` (each start has its own seed, so the epoch is new). `changes --since c1 --json`: `reset: true`, `records` holding every tracked PR, and the new epoch's events. Its text starts `reset: cursor too old, current state follows`. `list --since c1 --json`: `reset: true`, every record. `wait <pr> --until changed --since c1`: exit 0 at once, `reset: true`. (b) Take `c2`; let 26 PRs flip `draft` at every poll until over 500 events follow it. `changes --since c2 --json`: `reset: true`, `events: []`, every record. A cursor of the same epoch with a `rev` above the counter: `reset: true`. `--since nonsense`: exit 2, `bad_request`.
- **`github-down`** (POL-19, OPS-16, POL-16). World: three settled PRs, a background `wait --until merged --timeout 900`, a `list` every 2 minutes. 5xx from `t` for 10 minutes. Assert: requests at `t` + 0, 5, 15, 35, 75, 155, 315 s and none before 615 s (each gap double the last, capped at 300 s). `list` exits 0 with every record `stale: true`, `daemon.problem: github_unavailable`, `daemon.until` the next try. `h.log()` gains one `problem begin code=github_unavailable` line, none per retry, and one `problem end` line. The wait is still held (`waiters: 1`). Clear the fault: the next try succeeds, a poll follows at once, records aren't stale, `problem: null`; merge the PR: the wait exits 0.
  - *Read-through:* during the backoff `show else/lib#9`: one request, exit 9, `github_unavailable` with `until`. Again in the same step: no request, exit 9. After the next failed try: one more is sent.
  - *The same,* for 60 s each, with: a hang (the request is abandoned 20 s after it was sent and the next comes 5 s later, at no cost in real time); a refused connection; a body that isn't JSON; a redirect, whose target's hit count stays 0 (OPS-16).
- **`token-rotated`** (POL-20). (a) One 401; the token source then gives `testkit::MARKER_TOKEN_2`. The fake's log shows the repeat carrying it in `Authorization`. Assert: `h.token().reads()` rose by one, the request was repeated once and answered, `problem: null`. (b) 401 until cleared. Assert: after the repeat, `daemon.problem: auth`; `list` exits 0, records `stale`, and its text ends with a `problem: auth` line before the cursor; `show octo/app#1` exits 0 from memory; `show else/lib#9` exits **7**, `auth`, its message naming the token source; a background wait stays held. Clear the fault and give the other marker token, so the read gives another token than the one refused: within 60 s `problem: null` and polls resume. No log line or answer holds the 401's body.
- **`sleep`** (POL-22, TST-4). Its last sentences, on a subscription, are T49's. World: PR #1 with a background `wait --until merged --timeout 600`; PR #2 scheduled to merge one hour on; a `list` just before. Run it twice: `sleep_wall_only(2 h)` and `sleep_both(2 h)`. Assert both ways: the wait exits 3, `outcome: timeout`; exactly one poll is sent on waking and the next not before an `interval` later; after it `show octo/app#2` reads `merged`. Wall-only: `h.log()` has one `sleep seconds=7200` line. Stale, wall-only, with GitHub answering 5xx at the wake: every record reads `stale: true` though its `fetched_at` is seconds old on the monotonic clock; cleared by the next applied refresh.
- **`restart`** (STA-5, STA-8, MOD-15, API-18). World: `octo/app#1` (`authored`, one comment) and `else/lib#2` (tracked by hand), both saved. `h.stop()`. With no daemon: #1 merges and gets 2 more comments; advance 10 minutes. Restart with #1's node answering an error for the first request only. Assert: `list --json` exits 0 with `pending: ["octo/app#1"]` and no record of it. A background `wait octo/app#1 --until merged` is held, and no by-name request for #1 is in the log. After the next poll: the wait exits 0; #1's first event holds `tracked` (`authored`), `merged` and `activity` with exactly the 2 new comments, `more: false`, and no other kind; `else/lib#2` reads `source: manual`.
- **`starting`** (API-18). Start with GitHub's first answer held (`opts`, as `hold_next()` does later). `list` at 0 s in the background: not done at 9 s; at 10 s it exits **9**, `error.code: starting`. The same for `changes`. `status` exits 0 at once throughout. Second world: the token source slow by 8 s: a `list` begun at 0 s exits 0 with records once the first poll is done, before 10 s.
- **`first-run`** (API-18, POL-3, POL-19, POL-20, OPS-5).
  - *No login* from the start: `status` exits 0 at once; `list`, with no advance, exits **7**, naming `gh auth login`, and not `starting`. Give a token: a `list` begun then exits 0 with records within 5 s.
  - *A slow token read* (8 s): `list` and `show else/lib#9` in the background aren't done at 7 s and are after it; `status` at 1 s exits 0 with `daemon.problem: null`.
  - *GitHub refusing connections* from the start: `list` exits **9**, `github_unavailable`, with `error.until` set, within 1 s, not after 10.
  - *Periodic reads:* a running daemon; the source answers a failure: after 10 minutes `reads()` rose, polls still succeed, `problem: null`, one `token read failed` line. Then it answers no login: after the next read `daemon.problem: auth`, and `show else/lib#9` exits 7.
- **`restart-offline`** (STA-5, STA-8). World: `octo/app#1` (`authored`, one comment), `else/lib#2` (`manual`), `octo/app#3` merged and seen at `T`; all saved. Stop; keep the file's content as `F`. GitHub answers 5xx. `restart_with` `exit_after = 300`. Assert: `list` exits 9, `github_unavailable` (API-18: no request of the first poll was answered); `status --json`'s `tracked` counts the three loaded PRs. With no further request the daemon exits by itself 300 s on: `h.exited()` is `Some(0)`, the log holds `stop cause=idle`; `h.state_file()` then equals `F`, entry for entry. Clear the fault; 2 comments land on #1; restart. Assert: #1's first event holds `tracked` and `activity` with those 2 comments; #2 reads `manual`; #3 reads `merged` and its `ended.seen_at` in the file is still `T`.
- **`crash`** (STA-7, OPS-4, POL-4). World: `octo/app#1` tracked and saved. `octo/app#2` is opened. Advance to the next poll's send time; `hold_next()`; advance 1 s, so #2's first refresh is in flight and held; `h.crash()`. Assert: the state file is byte for byte what it was, and the socket file is still there. `h.restart()`: `status` answers on the same path; after the first poll `list` holds #1 and #2, and #2 has a `tracked` event in the new epoch.
- **`files-removed`** (STA-11). (a) Remove the socket file under the running daemon; advance 60 s: `h.exited()` is `Some(0)` and the log holds `stop cause=files`. `list` through the spawner that starts the in-process daemon (`cli_bg`, then advance): exit 0, one spawn. (b) Replace `circitor.lock` with a new file (another inode): stopped within 60 s. (c) Remove the socket and bind a socket of the test's own at its path; advance 60 s: the daemon has stopped and the test's socket is still there, the same inode.
- **`clock-back`** (POL-26, STA-2, POL-22). Its last sentence, on a subscription's expiry, is T49's. Config: `keep_done = 7200`. World: a merged PR seen at `s`; a background `wait --until merged --timeout 100` on an open PR, begun at `w`; `cost_last_hour` reads `n`. At `w + 10 s`, `set_wall_back(1 h)`. Assert: no `sleep` line in the log and no record newly stale; `cost_last_hour` is still at least `n`, and 30 minutes on hasn't fallen below it; the wait isn't done at `w + 99 s` and exits 3 at `w + 100 s` on the monotonic clock; the merged PR is still tracked while the wall clock reads under `s + 7200 s`, and is dropped at the first tick after.

## Interfaces
- **Consumes:** `testkit::harness::Harness` (`start(opts)` with config, faults, a held answer and the token's first answer; `clock()`, `github()`, `token()`, `log()`, `state_file()`, `paths()`, `cli`, `cli_bg`, the spawner variants, `stop`, `crash`, `restart`, `restart_with`, `exited()`, `finish`), `testkit::server::FakeGitHub` (`world`, every fault of TST-3, aimed faults, `hold_next()`, the redirect's hit count, `requests()`), `testkit::MARKER_TOKEN_2`, `token::fake::FakeTokenSource` (`set`, `set_delay`, `reads`), `clock::TestClock` (`advance`, `sleep_wall_only`, `sleep_both`, `set_wall_back`).
- **Provides:** nothing.

## Out of scope
- The other TST-8 rows: T30 (checks), T31 (a PR's life and waits), T32 (tracking and polling; `retention` and `tiers` among them).
- **Real processes** (TST-9): signals, a real stale socket, two CLIs, the real spawner, modes and permissions: T36. Here the daemon is in-process and `crash` is a drop.
- The state file's torn and failed writes (TST-10): T16. The exhausted and secondary limits (TST-6): T34.
- The backoff, the auth state and the sleep check as unit tests: T08, T13, T04, T21.
- The CLI's exact text and every exit code (TST-7): T10, T35.
- The harness and the fake GitHub: T29, T20, T14. A missing control is reported, not added.
- Fixing what a scenario finds in `src/`: the task named in the `#[ignore]`.

## Acceptance
- `tests/scn_daemon.rs` starts with `#![cfg(feature = "test-endpoint")]` and holds every scenario above as `scn_<name>` or `scn_<name>_<part>`, each asserting everything its entry lists, with the entry's values.
- `cargo test --features test-endpoint --test scn_daemon -- --list` shows at least one test for each of the eleven names; `grep -c 'TST-8' tests/scn_daemon.rs` is at least 11.
- No `sleep`, no `tokio::time`, no `std::thread` in the file: the 20 s hang, the 10 s `starting` wait and the 2 h sleep cost no real time. Every test ends with `finish()`, so TST-11's sweep and the fake's rules (POL-1, POL-2) ran for each, across restarts.
- The whole file runs in under two minutes of real time.
- `cargo test --features test-endpoint --test scn_daemon` and `make verify` pass. An ignored test is listed under Open items with its reason.

## Traps
- **A scenario that fails because production code is wrong is kept:** mark it `#[ignore = "T28: <what's wrong>"]` with the owning task's ID, and report it. Don't weaken the assertion, change the world to fit, or touch `src/`.
- **Every control used here is promised by `plan/contracts/testkit.md`.** One the code doesn't have is reported; write the scenario as far as it goes and mark it `#[ignore = "T29: no control for …"]`. Don't build it yourself.
- **An awaited `cli()` hangs when the command waits on the clock:** every `wait`, every command that spawns, a `list` before the first poll has finished, anything behind a held answer or a slow token read. Use `cli_bg`, advance, then await (testkit.md).
- **`starting`, `auth` and `github_unavailable` are three answers to a `list` at start** (API-18). Which one depends on whether the first poll is running, couldn't be sent, or failed with no request answered. A hang becomes `github_unavailable` once it's abandoned at 20 s: look for `starting` at 10 s.
- **`pending` shows in `list` only once the first poll has finished with some request answered** and a loaded PR's first refresh not applied. `restart` stages that with an error on one node; a GitHub that's wholly down gives an error instead.
- **`sleep_both` isn't noticed as a sleep:** the two clocks moved together (`SleepWatch`). The same outcomes come from the due timers, the records' age and the wait's own deadline. Assert the sleep's log line wall-only.
- **A wait with a passed deadline ends as `timeout`** on waking, also when its PR merged meanwhile: that's why `sleep` puts the wait and the merge on different PRs.
- **`status` is the only command that isn't a client request** (API-5). `restart-offline`'s idle exit and `github-down`'s tier depend on which commands the test runs, and when.
- **Only a log line's event word is an interface** (`plan/contracts/log.md`); its `key=value` pairs beyond the ones quoted here are each task's. Count lines by word between two points of the test.
- **The fake GitHub's clock never goes back** (TST-3). After `set_wall_back` its `Date` is an hour ahead of the local clock.
- **A second daemon in one process is refused the lock** like any other (`plan/contracts/daemon.md`): `restart` after a `crash` works because the crash drops the descriptor.
- The 401's body and every error message hold the fake's markers. `finish()` sweeps the log and the state file; assert the API's messages yourself.

## Open questions
- **`restart-offline`:** `list` answers `github_unavailable` while no request of the first poll was answered (API-18), so the loaded PRs are seen through `status` and the file, as the row says.
- **GitHub's 5xx, its answer to an expired token and tokio's timers across a suspend aren't measured** (docs/02 §6, docs/04 §8 item 6). POL-19's values, POL-20, POL-22 and POL-26 are (proposal). Test them as written.
- **`first-run`'s "within 5 s" after a login** rests on POL-20's read before a request that needs GitHub. If a `list` after `auth` doesn't cause that read, report it against the row.
- D5 and D6 don't touch this task.

## Evidence
- **2026-11-26**, branch `t33-scenarios-daemon`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `tests/scn_daemon.rs` only (no `tokio::time`, no thread): 25 tests covering the eleven rows (in parts), each ending in `h.finish()`; the subscription sentences of `sleep` and `clock-back` are T49's. **None ignored; no bug found in `src/`.** `cargo test --features test-endpoint --test scn_daemon`: 25 passed, 4 runs. `make verify` passes on main after the merge.
- **The executor's three questions, kept as built by the orchestrator (owner's rule of 2026-11-26):** a `list` at the instant of a login answers `auth` (POL-20's 5 s between reads wins over the row, TST-8's rule); a read-through arriving during a poll goes before its remaining requests (POL-15: "go next"); `status.tracked` counts every place held, first refreshes in flight included (`View::tracked`, `contracts/state.md`).
- Readings: `github-down`'s "a poll follows at once" is the recovering try itself, a poll; `crash` runs a `list` first for the active tier; `restart`'s first event's kinds compared as a set.
