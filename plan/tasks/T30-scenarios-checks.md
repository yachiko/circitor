---
id: T30
title: "Scenarios: checks and settling"
kind: test
milestone: M5
size: L
status: todo
depends_on: [T29]
requirements: [TST-8]
owns: [tests/scn_checks.rs]
shares: []
---

# T30 · Scenarios: checks and settling

## Outcome
Twelve of TST-8's scenarios run on the harness and pass: `push-old-head`, `no-ci`, `first-check-early`, `late-ci`, `one-fails-others-run`, `skipped-and-cancelled`, `commit-status`, `check-out-of-order`, `settled-at-once`, `many-checks`, `unlisted-values`, `head-back`.

## Context
T29's harness runs the whole daemon in-process on the test clock, against the fake GitHub (T14, T20), with the CLI's commands as functions. This task plays the twelve TST-8 rows about **checks and settling** on it. T31, T32 and T33 play the other rows in their own files. Nothing is built on this file: its failures are how bugs in T05, T21, T23 and T25 are found.

## Read first
- `plan/conventions.md`: Tests, Requirement tracing, Report.
- `plan/contracts/testkit.md` (the harness, the fake GitHub, what the fake's world does by default), `plan/contracts/clock.md` (`TestClock::advance`, what "settled" means), `plan/contracts/model.md` (the record's and the events' JSON), `plan/contracts/api.md` (each route's body), `plan/contracts/cli.md` (`Output`, the exit codes).
- The module docs of `src/testkit/harness/`, `src/testkit/server/` and `src/testkit/github/`: the real names of every control used below. `tests/m3_cli.rs` shows a test driving the harness.
- `specs/07-testing.md#what-is-tested`: TST-8, the twelve rows named in the Outcome. `specs/07-testing.md#test-doubles`: TST-3 to TST-5.
- `specs/01-model.md#checks`: MOD-3 to MOD-7. `#status`: MOD-11. `#revisions-and-events`: MOD-14, MOD-15. `#identity-and-fields`: MOD-2's last bullet.
- `specs/02-polling.md`: POL-2, POL-8, and POL-13 to POL-15 (when a refresh is sent).
- `specs/04-api.md#waiting`: API-10 to API-12. `specs/05-cli.md`: CLI-5, CLI-6.
- `docs/02-github-api.md` §4, "Checks" and "What moves `updatedAt`". `docs/measurements/2026-11-24/a1.log`, `a2.log` and `c1.log`, with the PR A and PR C tables of `docs/05-measurements.md`: the timings below come from them.

## Scope
**TST-8's share here: these twelve rows**, in `tests/scn_checks.rs`. The row is the spec; each entry below adds only the world's concrete values and what to assert. **Where a row and the requirement it cites differ, the requirement wins** (TST-8): then assert the requirement and report the row.

How every scenario is written:
- **One `#[tokio::test] async fn scn_<name>()` per row**, hyphens as underscores, with `// TST-8: <name>` and the row's IDs (`// MOD-6:`) above it. A row with several worlds may be several functions named `scn_<name>_<part>`, so an `#[ignore]` costs one part. Each starts its own harness and **ends with `h.finish()`**.
- **Script the world before the daemon's first poll**, through `Harness::start(opts)` (testkit.md: the world's closure, the skew, config values). A PR added after the start is found one poll later, and isn't "first seen" when the test thinks.
- **Look through the CLI:** `--json` output parsed as JSON, `Output.code`, and events from `changes <pr> --since <cursor> --json`. Assert text only where an entry quotes a line; the layout is T35's.
- **Read send times from `h.github().requests()`**, never from an assumed cadence: each entry has the clock's monotonic time, whether it held a finding, and the node IDs with what was asked of each (the record, the items, a check page). A PR's refreshes are the requests naming its node ID. Its **settle start** (MOD-6) is the send time of the request that first read the record, first showed the head, or first showed it open again.
- **Advance and look; never sleep.** A command that needs clock time runs under `cli_bg`, then `advance`, then is awaited.
- Keep the spec's values as named constants at the top of the file (`SETTLE_WITH_CHECKS = 20 s`, `SETTLE_EMPTY = 60 s`, `SETTLE_MARGIN = 5 s`, `CHECK_PAGE = 100`, `CHECKS_READ_MAX = 300`): most are (proposal).

Names: login `octocat`, PRs `octo/app#N`. `C` is a PR's creation and `P` a push, on GitHub's clock. SHAs: `A` starts `dce9dbe`, `B` starts `cf54ec4` (a2.log). "Settled PR" means: `updatedAt` two minutes old at the first read, so MOD-6's shortcut settles it at once.

The scenarios:

- **`push-old-head`** (API-10). World: a settled PR at head `A` with `fast` and `slow` passed, reading `ready`. Push at `P`: the old head with its old `SUCCESS` is served for 3 s; from `P+3` head `B`, no checks, `UNKNOWN`; `P+6` both checks `QUEUED`, `MERGEABLE`; `P+9` `IN_PROGRESS`; `P+15` `fast` passes; `P+52` `slow` passes (a2.log). Steps: at `P`, `wait --until checks-done` (no `--sha`) and, in the background, `wait --until checks-done --sha <B> --timeout 100`. Assert: the first ends at once, exit 0, `checks passed (2/2) on dce9dbe`, `head_sha` `A` (documented behaviour). The second isn't done at `P+50`; it ends after `P+52`, exit 0, `outcome: satisfied`, `head_sha` `B`, `checks.state: passing`, `checks passed (2/2) on cf54ec4`. Its events hold a `head` kind from `dce9dbe` to `cf54ec4`.
- **`no-ci`** (MOD-6). World: a PR created at start − 1 s, no checks ever, `MERGEABLE` (b1.log). Part one, no waiter: sample `list --json` every 5 s for 120 s. At each sample take `r`, the PR's latest refresh sent by then, and `s`, its settle start: `status` is `ready` exactly when `r − s ≥ 60 s`, else `checks_pending`. At least one sample falls where the clock is past `s + 60 s` and `r − s < 60 s`, and reads `checks_pending`: the clock alone never settles it. Part two: `wait --until checks-done` in the background ends exit 0, `satisfied`, `checks.state: none`, `settled: true`, line `no checks reported on <sha7>`, at a refresh sent at least 60 s after `s` and not before.
- **`first-check-early`** (MOD-6, MOD-11). World: `C` = start − 3 s. At `C+3` a commit status `ci/external` reads `SUCCESS`. At `C+8` check runs `fast` and `slow` appear `IN_PROGRESS`; `fast` passes at `C+15`, `slow` at `C+56` (a1.log). Assert: after the first read (one passed status, nothing else) `checks.state: passing`, `settled: false`, `status: checks_pending`; `wait --until checks-done --timeout 0` exits 3, `outcome: timeout`. A background `wait --until checks-done` isn't done at `C+55` and ends after `C+56`, exit 0, `checks passed (3/3)`. No `list` sample ever reads `ready` before that.
- **`late-ci`** (MOD-6, MOD-11). Four worlds: a **new PR** whose checks appear at `C+6` and at `C+40`; a **new head** (a settled, `ready` PR pushed at `P`, new head from `P+3`) whose checks appear at `P+6` and at `P+40`. Each time the checks run 10 s and pass. Assert in each: from the first read of the new PR or head until the checks pass, every `list` sample reads `checks_pending`, never `ready`; a background `wait --until checks-done --timeout 120` (with `--sha <new>` after a push) isn't done one second before they pass, and ends exit 0, `passing`. The 40 s worlds are why an empty list takes 60 s to settle.
- **`one-fails-others-run`** (MOD-5, CLI-5). World (c1.log): at `C+8` `pass`, `fail`, `slow` `IN_PROGRESS` and `skipped` `COMPLETED`/`SKIPPED`; at `C+11` `pass` `SUCCESS`, `fail` `FAILURE`; `slow` runs 78 s more and passes at `C+89`. Steps: `wait --until checks-done --timeout 120` in the background. Assert between `C+12` and `C+88`: `checks.state: pending`, `failed: 1`, `failed_items` names `fail`, `status: checks_pending`, and the wait isn't done. After `C+89`: exit **8**, `outcome: satisfied`, `checks.state: failing`, `total: 4`, `completed: 4`, `failed: 1`, line `checks failed (1 of 4) on <sha7>`.
- **`skipped-and-cancelled`** (MOD-5). Two settled PRs. One has `pass` `SUCCESS` and `skipped` `SKIPPED`: `checks.state: passing`, `failed: 0`, `status: ready`, `wait --until checks-done --timeout 0` exits 0. The other has `pass` `SUCCESS` and `slow` `IN_PROGRESS`, then `COMPLETED`/`CANCELLED` 16 s later (c1.log): then `checks.state: failing`, `failed: 1`, `failed_items` names `slow`, `status: checks_failing`, the same wait exits 8.
- **`commit-status`** (MOD-3, MOD-4). World: a settled PR with check run `pass` passed, `ready`. At `S` a commit status `circitor/manual` reads `PENDING`; at `S+16` `FAILURE`; at `S+40` it's posted again as `PENDING` (c1.log, the re-post is this row's own). Refresh after each step (`show --fresh`, over 5 s apart). Assert, in `checks.items` of `show --json`: one list holds `{kind: run, name: pass, id: <number>}` and `{kind: status, name: circitor/manual, id: null}`; the status reads `phase: in_progress`, `result: null` with `checks.state: pending`; then `phase: completed`, `result: failure` with `failing` and `failed_items` naming it; then `in_progress` and `pending` again. `changes` shows three events, each with a `checks` kind.
- **`check-out-of-order`** (MOD-4, MOD-14). World (a1.log): `fast` and `slow` read `IN_PROGRESS`; 3 s later one read gives both as `QUEUED` (the fake's out-of-order control); then `IN_PROGRESS` again. Steps: refresh, take the cursor `c`, advance 6 s, refresh. Assert: `changes <pr> --since c --json` has `events: []` and the same cursor; the record's `rev` is unchanged; `checks.state: pending`; the items read `queued` (each read is believed).
- **`settled-at-once`** (MOD-6). Four worlds, each a PR with `fast` and `slow` passed. (a) `updatedAt` 2 minutes old: `ready` and `settled: true` after its first refresh, with exactly one refresh of it in the log; then `h.restart()`: `ready` again after its first refresh, and no `status` kind in its events. (b) `updatedAt` 10 s old: `checks_pending` until a refresh sent at least 20 s after the settle start, `ready` from that one. (c) `updatedAt` 2 minutes old and `without_date_header`: as (b). (d) GitHub's clock skewed 5 minutes ahead of the local one, `updatedAt` 10 s old by GitHub's clock: as (b): the age is taken from `Date`, never from the local clock.
- **`many-checks`** (POL-8, MOD-7, CLI-6). Settled PRs, one world each. Pages: **100** checks are one request for that PR's refresh, **101** two, **187** two with `total: 187`, `truncated: false`; **300** three, not truncated; **301** `truncated: true`, `total: 301`, 300 items in `show --json`; **400** `truncated: true`, `total: 400`, `completed` counting read items only. A failed second page: with 187 read once, fail one check in the world and make the next refresh's check-page request answer 5xx (a fault aimed at it, or set for one request between the two pages): the record equals the one before (same `rev`, `fetched_at`, `checks`) and `stale: true`; with the fault gone the next refresh applies and makes one event. Truncated states: 400 checks whose first 300 passed, the unread ones running, all passed, or one failed: `checks.state` reads `pending`, `passing`, `failing`. On the last, `wait --until checks-done` exits 8 and its outcome line holds `(truncated: unread checks may still run)`.
- **`unlisted-values`** (MOD-2, MOD-5). World: a settled PR with `pass` `SUCCESS`, `boot` concluding `STARTUP_FAILURE` and `odd` concluding `FOO`, a value GitHub's schema doesn't have. Assert: `show --json` decodes, exit 0; the items read `result: "startup_failure"` and `result: "foo"`, `phase: completed`; `checks.state: failing`, `failed: 2`, `failed_items` names both; `status: checks_failing`; `wait --until checks-done --timeout 0` exits 8.
- **`head-back`** (MOD-6). World: a settled PR at `A`, checks passed, `ready`. Push `B` at `P` (served from `P+3`), force-push back to `A` at `P+12` (served from `P+15`); `A` keeps its passed checks. Steps: `show --fresh` at `P+4` and at `P+16`, so both heads are read. Assert after the second: `head_sha` `A`, `checks.state: passing`, `settled: false`, `status: checks_pending`; it reads `ready` only from a refresh sent at least 20 s after the one that showed `A` again. The events hold two `head` kinds, `A` to `B` and `B` to `A`.

## Interfaces
- **Consumes:** `testkit::harness::Harness` (`start(opts)`, `clock()`, `github()`, `cli`, `cli_bg`, `restart`, `finish`), `testkit::server::FakeGitHub` (`world`, the old-head, out-of-order and 5xx faults, `without_date_header`, the skew, `requests()`), the world's controls listed in testkit.md, "What the world must be able to play", `clock::TestClock::advance`, the JSON of `plan/contracts/model.md` and `api.md`.
- **Provides:** nothing.

## Out of scope
- The other TST-8 rows: T31 (a PR's life, waits), T32 (tracking, polling), T33 (the daemon's life).
- The derivations as tables, one case per row of MOD-5, MOD-7 and MOD-11 (TST-15): T05. Here each is seen once, end to end.
- The CLI's exact text and every exit code (TST-7): T10, T35. Quota figures (TST-6): T34.
- The harness and the fake GitHub: T29, T20, T14. A missing control is reported, not added: `src/testkit/` isn't yours.
- Fixing what a scenario finds in `src/`: the task named in the `#[ignore]`.

## Acceptance
- `tests/scn_checks.rs` starts with `#![cfg(feature = "test-endpoint")]` and holds every scenario above as `scn_<name>` or `scn_<name>_<part>`, each asserting everything its entry lists, with the entry's values.
- `cargo test --features test-endpoint --test scn_checks -- --list` shows at least one test for each of the twelve names; `grep -c 'TST-8' tests/scn_checks.rs` is at least 12.
- No `sleep`, no `tokio::time`, no `std::thread` in the file. Every test ends with `finish()`, so TST-11's sweep and the fake's rules (POL-1, POL-2) ran for each.
- The whole file runs in under a minute of real time.
- `cargo test --features test-endpoint --test scn_checks` and `make verify` pass. An ignored test is listed under Open items with its reason.

## Traps
- **A scenario that fails because production code is wrong is kept:** mark it `#[ignore = "T05: <what's wrong>"]` with the owning task's ID, and report it. Don't weaken the assertion, change the world to fit, or touch `src/`.
- **Every control used here is promised by `plan/contracts/testkit.md`.** One the code doesn't have is reported; write the scenario as far as it goes and mark it `#[ignore = "T20: no control for …"]`. Don't build it yourself.
- **An awaited `cli()` hangs when the command waits on the clock:** every `wait`, a `list` before the first poll has finished, and any command whose GitHub request must wait out POL-2's 1 s gap or needs a second request (a check page). Use `cli_bg`, advance, then await (testkit.md).
- **`refresh` isn't a way to re-read one PR:** a poll refreshes only the PRs that are due (POL-13). Use `show --fresh` once its last refresh is over 5 s old (POL-15), or advance two intervals.
- **Looking changes the cadence.** A `show` or a `wait` names the PR and makes it hot while it reads `checks_pending` (POL-14). Sample with `list --json` where the entry counts refreshes. `status` isn't a client request; `list` is, and keeps the tier active.
- **MOD-6's shortcut decides the first read.** A PR whose `updatedAt` is over 25 s old (65 s with no checks) at its first read is settled at once. Set `updatedAt` on purpose in every world. A check never moves `updatedAt` (docs/02 §4).
- **`checks.settled` is not an event and not a revision** (MOD-14): a `status` word it moves is. Don't look for a `settled` kind.
- **`failing` only once every check has completed** (MOD-5): GitHub's rollup reads `FAILURE` at `C+11` in `one-fails-others-run`, and the record must not.
- An event's `kinds` is a list of objects tagged `kind` (plan decision P10). Titles hold the fake's marker; never assert on a title.
- `#[tokio::test]` only, never `start_paused`, never a second thread: settling depends on the current-thread runtime.

## Open questions
- **MOD-6's 20 s, 60 s and 5 s, POL-8's 300, MOD-7's truncated states and MOD-4's belief in each read are (proposal).** Test them as written, through the constants.
- **A re-post of a commit status, a check's `databaseId` and the `Date` header aren't measured** (docs/02 §6, MOD-6). The fake plays what the specs assume.
- **`no-ci` part one needs a sample between the clock passing 60 s and the next refresh.** With OPS-2's defaults there is one. If the engine's cadence leaves none, report it; don't change the intervals to make one.
- D5 and D6 don't touch this task.
