---
id: T31
title: "Scenarios: a PR's life and waits"
kind: test
milestone: M5
size: L
status: todo
depends_on: [T29, T42]
requirements: [TST-8]
owns: [tests/scn_waits.rs]
shares: []
---

# T31 · Scenarios: a PR's life and waits

## Outcome
Thirteen of TST-8's scenarios run on the harness and pass: `merged`, `closed`, `reopened`, `conflict`, `mergeable-unknown`, `base-changed`, `head-moved`, `rerun`, `waiter-limit`, `new-review`, `changed-kind`, `baseline`, `gone-returns`.

## Context
T29's harness runs the whole daemon in-process on the test clock, against the fake GitHub (T14, T20), with the CLI's commands as functions. This task plays the thirteen TST-8 rows about **a PR's life and the waits on it**: ending, reopening, mergeability, a moved head, re-runs, the waiter limit, and what's new. T30, T32, T33 and T49 play the other rows. Its failures are how bugs in T05, T15, T23, T25 and T42 are found. **T42 is a direct dependency** (`gone-returns` needs a PR to go `gone`); the harness (T29) doesn't bring it.

## Read first
- `plan/conventions.md`: Tests, Requirement tracing, Report.
- `plan/contracts/testkit.md`, `plan/contracts/clock.md` (`advance`, `sleep_wall_only`, what "settled" means), `plan/contracts/model.md` (records, events, items as JSON), `plan/contracts/api.md` (`WaitBody`, the error body), `plan/contracts/cli.md` (`Output`, exit codes), `plan/contracts/state.md` (`FileEntry`: what `h.state_file()` holds).
- The module docs of `src/testkit/harness/`, `src/testkit/server/` and `src/testkit/github/`: the real names of the controls. `tests/m3_cli.rs` shows a test driving the harness.
- `specs/07-testing.md#what-is-tested`: TST-8, the thirteen rows named in the Outcome. `#test-doubles`: TST-3 to TST-5.
- `specs/04-api.md#waiting`: API-9 to API-14. `specs/05-cli.md`: CLI-5, CLI-6.
- `specs/01-model.md#mergeability-and-review`: MOD-8. `#status`: MOD-11. `#whats-new`: MOD-12. `#revisions-and-events`: MOD-14, MOD-15. `#checks`: MOD-4, MOD-6.
- `specs/02-polling.md`: POL-10, POL-11, POL-12, POL-21, POL-23, and POL-13 to POL-15. `specs/03-state.md#the-tracked-set`: STA-2, STA-5.
- `docs/02-github-api.md` §4 ("Mergeability", "Comments and reviews", "Other fields") and §3. `docs/measurements/2026-11-24/a2.log`, `a3.log`, `b1.log`, `d1.log`, and the PR B, D and E tables of `docs/05-measurements.md`: the timings below come from them.

## Scope
**TST-8's share here: these thirteen rows**, in `tests/scn_waits.rs`. The row is the spec; each entry below adds only the world's concrete values and what to assert. **Where a row and the requirement it cites differ, the requirement wins** (TST-8): then assert the requirement and report the row.

How every scenario is written:
- **One `#[tokio::test] async fn scn_<name>()` per row**, hyphens as underscores, with `// TST-8: <name>` and the row's IDs (`// API-11:`) above it. A row with several worlds may be several functions named `scn_<name>_<part>`, so an `#[ignore]` costs one part. Each starts its own harness and **ends with `h.finish()`**.
- **Script the world, and set the config, before the daemon's first poll**, through `Harness::start(opts)` (testkit.md).
- **Look through the CLI:** `--json` output parsed as JSON, `Output.code`, events from `changes <pr> --since <cursor> --json`, the file from `h.state_file()`. Assert text only where an entry quotes a line; the layout is T35's.
- **Read send times from `h.github().requests()`**, never from an assumed cadence: each entry has the clock's monotonic time, whether it held a finding, the node IDs with what was asked of each, and any by-name lookup. A PR's refreshes are the requests naming its node ID.
- **Advance and look; never sleep.** Every held `wait` runs under `cli_bg`; the test advances, then awaits it.
- Keep the spec's values as named constants (`KEEP_MERGEABLE = 60 s`, `FOLLOW_UP = 3 s`, `HEAD_MOVED_AFTER = 60 s`, `WAITERS_MAX = 64`, `GONE_AFTER = 5 min`): all are (proposal).

Names: login `octocat`, PRs `octo/app#N`. "Settled PR" means: `updatedAt` two minutes old at the first read (MOD-6's shortcut). `P` is a push, on GitHub's clock; SHAs `A` (`dce9dbe…`) and `B` (`cf54ec4…`).

The scenarios:

- **`merged`** (POL-10, API-11). World: a settled PR with one check `slow` still `IN_PROGRESS`; merged at `M` (b1.log: `MERGED`, `mergeable` and `mergeStateStatus` `UNKNOWN`, gone from the open list at the same sample). Steps: `wait --until merged` and `wait --until checks-done` in the background, then the merge. Assert: the first exits 0, `outcome: satisfied`, line `merged`; the second exits 4, `outcome: ended`, `reason: merged`, line `ended: merged`. The record: `state: merged`, `status: merged`, `mergeable: unknown`, `merge_state: unknown`, `merged_at` set. Its event has a `merged` kind and **no `mergeable` kind** (MOD-8). After `M`, a request that names its node ID and whose finding doesn't list it is what showed it. `list` leaves it out; `list --all` has it.
- **`closed`** (API-11). World: the same PR, closed unmerged at `K` (a3.log: `CLOSED`, `MERGEABLE`, `CLEAN` kept). Three background waits. Assert: `--until merged` exits 4, `ended`, `closed`, line `ended: closed`; `--until checks-done` exits 4, `ended`, `closed`; `--until changed` exits 0, `satisfied`, line `changed`. The record: `state: closed`, `mergeable: yes`; the event has a `closed` kind.
- **`reopened`** (POL-10, STA-2, MOD-6). Config: `keep_done = 300`. World: a settled PR with passed checks, closed at `K` and seen closed; reopened at `R = K + 60 s`. Keep the tier active with `list`. Assert: after the first poll sent after `R`, `state: open`, an event with a `reopened` kind, `checks.settled: false`, `status: checks_pending`; `ready` only from a refresh sent at least 20 s after the one that showed it open. The retention clock is cleared: `h.state_file()` reads `ended: null` for it once the saver has written, and at `K + 300 s` plus two ticks it's still listed, with no `untracked` event.
- **`conflict`** (MOD-11, docs/02 §4). World: a PR created at start − 1 s reading `UNKNOWN`; from `C+3`, `CONFLICTING` and `DIRTY` (d1.log). Assert after 10 s: `mergeable: no`, `merge_state: dirty`, `status: conflict` (above `checks_pending`: its empty checks list hasn't settled).
- **`mergeable-unknown`** (MOD-8, POL-11, POL-23). Six parts; sample with `list`, never `show`, where a part says "no follow-up".
  - *Kept:* a settled PR read `MERGEABLE` at `r0`; from then GitHub answers `UNKNOWN` on the same head. A refresh sent under 60 s after `r0`: `mergeable: yes`, no event, the cursor unchanged. The first refresh sent over 60 s after `r0` (keep clear of the boundary): `mergeable: unknown`, one event with a `mergeable` kind. No request follows either by 3 s.
  - *After a sleep:* read `MERGEABLE`, script `UNKNOWN`, `sleep_wall_only(120 s)`: the first read on waking gives `unknown`. **A sleep under 60 s keeps `yes`:** MOD-8 has no sleep signal, only its 60 s of wall time since the last `yes`, which is what the row's "a sleep longer than 60 s" says. Don't test a short sleep as `unknown`.
  - *After a push,* with a background `wait --until merged` holding the PR hot: new head and `UNKNOWN` from `P+3`, `MERGEABLE` from `P+6` (a2.log). Assert two events: `head` with `mergeable` (`unknown`), then `mergeable` (`yes`). The refresh that read `UNKNOWN` is followed by one request naming the node 3 s after its answer (POL-11).
  - *Twenty reads:* with that waiter, `UNKNOWN` for 20 reads. Among the node's refreshes, every gap is 3 s or at least `hot_interval`, and a 3 s gap never follows a 3 s gap.
  - *At start:* five PRs all `UNKNOWN` on their first read, `MERGEABLE` after. The log shows the finding, their first refresh, then **one** request 3 s later naming all five, and no fourth for them.
  - *Reserve and capped:* with `set_rate` putting `remaining` at 400, a waiter and `UNKNOWN`: no request follows the reading refresh by 3 s. The same with `quota.max_per_hour = 60` once `status --json` shows `quota_capped`: the wait's arrival refresh reads `UNKNOWN` and nothing follows it.
- **`base-changed`** (MOD-8). World: a settled PR read `MERGEABLE` on base `main`. Ten seconds later its base is `develop` and GitHub answers `UNKNOWN`. Assert at the next refresh, well inside the 60 s: `base: develop`, `mergeable: unknown`; one event holding `edited` with `fields: ["base"]` and a `mergeable` kind. Contrast with *Kept* above.
- **`head-moved`** (API-11). (a) A PR at `A` with a running check; `wait --until checks-done --sha <A>` in the background; push `B` (served from `P+3`). Assert: exit 4, `ended`, `reason: head_moved`, line `ended: head moved to cf54ec4, expected dce9dbe`. (b) `wait --until checks-done --sha deadbeef --timeout 100` on a PR that stays at `A`: not done 59 s after it began; it ends at the first refresh sent at least 60 s after, exit 4, `head_moved`, line `ended: head moved to dce9dbe, expected deadbee`.
- **`rerun`** (API-10). World: a settled PR at `A` whose check `build` (one `databaseId`) reads `COMPLETED`/`FAILURE`; `checks_failing`. Take the PR's cursor `c` from `changes <pr> --json`. Re-run at `R`: the same check reads `QUEUED` at `R+5`, `IN_PROGRESS` at `R+8`, `SUCCESS` at `R+45`. Assert: before `R`, `wait --until checks-done --timeout 0` exits 8 (the previous result). `wait --until checks-done --rerun-since c`, begun at `R+1` in the background, isn't done at `R+44` though the record read `failing` and settled; it ends after `R+45`, exit 0, `checks passed (1/1)`, with two `checks` kinds in its events. With no re-run at all, `--rerun-since c --timeout 20` exits 3 and its `again:` line repeats `--rerun-since c`.
- **`waiter-limit`** (API-14). World: one settled PR. Steps: 64 background `wait --until merged --timeout 3600`. Assert: `status --json` reads `waiters: 64`; a 65th exits **10**, `error.code: limit`, one stderr line. `cancel()` one of the 64 (the client closes; it returns once the daemon has seen it): `waiters: 63`; a new wait is held, `waiters: 64`. End by cancelling them all; `waiters: 0`.
- **`new-review`** (POL-12, MOD-12). World (PR E): a settled PR with no comments and no reviews. At `V` reviewer `hubot` submits a review, `CHANGES_REQUESTED`, with one inline comment; at `V+7` someone replies in the thread, which GitHub shows as a second review, `COMMENTED`. Refresh after each. Assert per step: exactly one more request names the node straight after the refresh that saw `counts.reviews` rise, and it's the one asking the items; **one** event, with an `activity` kind whose `counts.reviews` is the new count, `more: false`, and one item: `kind: review`, its `id`, `author`, `at`, `review_state`, `inline: 1`, `fetch: repos/octo/app/pulls/<n>/reviews/<id>`, `fetch_inline` ending `/comments`. The reply's item reads `review_state: commented`, `inline: 1`, and its own `id`.
- **`changed-kind`** (API-10, API-11; D8). World: a settled PR with one check `IN_PROGRESS`. At `K` the check passes; at `K+40` reviewer `hubot` submits a review, `COMMENTED`. Three more worlds end with a merge at `K+40` in place of the review. Steps, each wait begun before `K` in the background with `--timeout 120`:
  - `wait --until changed --kind activity`: not done 20 s after `K`, though a `checks` and a `status` event were recorded; it ends after `K+40`, exit 0, `outcome: satisfied`, line `changed`, and its events hold the `activity` kind with the review's item.
  - `wait --until changed` (no `--kind`), beside it: ends at the check's refresh, exit 0.
  - With the merge: `--kind activity` ends exit **4**, `outcome: ended`, `reason: merged`, line `ended: merged`; `--kind activity,merged` ends exit 0, `satisfied`; a close in place of the merge gives `ended: closed` for `--kind activity`.
  - `wait --until changed --kind foo`: exit 2 at once, `bad_request` naming `kind`; `--kind tracked` the same. `--kind activity` with `--until merged` is a usage error of the CLI's own, exit 2 with no request sent.
- **`baseline`** (STA-5). World: a PR with 5 comments and 2 reviews before the daemon first sees it; another, someone else's, with 5 comments, taken in with `track`. Assert: each PR's only event since `<epoch>-0` is `tracked` (`authored`, `manual`), with no `activity` kind; `counts.comments: 5`. Then a sixth comment: one `activity` event with exactly one item, the new comment's `id` and `fetch: repos/…/issues/comments/<id>`. `h.state_file()` holds `seen.comment.at` equal to the newest comment's time.
- **`gone-returns`** (MOD-11, STA-2). World: three settled PRs; keep the tier active with `list`. Hide #1 and #2 (off the open list, `NOT_FOUND` by node) until both read `gone` (POL-21: 5 minutes of polls). Return #1 open and #2 merged. Assert #1, at the next poll: `state: open`, a `reopened` kind, `checks.settled: false`, `status: checks_pending`, and `ended: null` in the state file. Assert #2, at its next refresh (an ended PR's period is `idle_interval`): `state: merged`, a `merged` kind, and `ended` in the state file reading `merged` with a `seen_at` equal to that refresh's send time, not the time it went gone.

## Interfaces
- **Consumes:** `testkit::harness::Harness` (`start(opts)` with config, `clock()`, `github()`, `cli`, `cli_bg` and its `cancel()`, `state_file()`, `finish`), the world's controls listed in testkit.md, "What the world must be able to play", `testkit::server::FakeGitHub` (`world`, `set_rate`, the unknown-for-N-reads and old-head controls, `requests()`), `clock::TestClock` (`advance`, `sleep_wall_only`), the JSON of `plan/contracts/model.md`, `api.md` and `state.md`.
- **Provides:** nothing.

## Out of scope
- The other TST-8 rows: T30 (checks, settling; `fail-fast` among them), T32 (tracking, polling; `repo-gone`, `retention` and `track-untrack` among them), T33 (the daemon's life; `sleep`, `restart`), T49 (notify).
- The waiters' rules as unit tests, the disconnect over a real socket: T23, T28. The derivations as tables (TST-15): T05.
- Quota states and their figures (TST-6): T34. Here only "no follow-up when capped or in reserve".
- The CLI's exact text and every exit code (TST-7): T10, T35.
- The harness and the fake GitHub: T29, T20, T14. A missing control is reported, not added.
- Fixing what a scenario finds in `src/`: the task named in the `#[ignore]`.

## Acceptance
- `tests/scn_waits.rs` starts with `#![cfg(feature = "test-endpoint")]` and holds every scenario above as `scn_<name>` or `scn_<name>_<part>`, each asserting everything its entry lists, with the entry's values.
- `cargo test --features test-endpoint --test scn_waits -- --list` shows at least one test for each of the thirteen names; `grep -c 'TST-8' tests/scn_waits.rs` is at least 13.
- No `sleep`, no `tokio::time`, no `std::thread` in the file. Every test ends with `finish()`, so TST-11's sweep and the fake's rules (POL-1, POL-2) ran for each. No waiter is left when a test ends.
- The whole file runs in under two minutes of real time.
- `cargo test --features test-endpoint --test scn_waits` and `make verify` pass. An ignored test is listed under Open items with its reason.

## Traps
- **A scenario that fails because production code is wrong is kept:** mark it `#[ignore = "T23: <what's wrong>"]` with the owning task's ID, and report it. `gone` and a gone PR's return are T42's (the engine's per-PR error runs): `gone-returns` names T42. Don't weaken the assertion, change the world to fit, or touch `src/`.
- **Every control used here is promised by `plan/contracts/testkit.md`.** One the code doesn't have is reported; write the scenario as far as it goes and mark it `#[ignore = "T20: no control for …"]`. Don't build it yourself.
- **An awaited `cli()` hangs when the command waits on the clock:** every `wait`, and any command whose GitHub request must wait out POL-2's 1 s gap or needs a second request (POL-12's items). Use `cli_bg`, advance, then await (testkit.md).
- **The condition is checked before the ends** (API-11). A `checks-done` wait on a PR whose checks are done and settled is `satisfied` even when it has merged. `merged` and `closed` need checks that aren't done: hence the running `slow`.
- **A wait's arrival forces a refresh** unless the PR's last refresh is under 5 s old (POL-15), and a waiter makes its PR hot (POL-14). Both change the request log: count around them.
- **A wall-clock jump ends held waits:** POL-22 adds the sleep to their deadline. Use `sleep_wall_only` only where no wait must survive it; `reopened` uses a short `keep_done` instead.
- **`gone` needs polls, not hot refreshes** (MOD-11), under the same login, and not every PR of the poll: hence the third PR in `gone-returns`. A `NOT_FOUND` alone on a hidden PR that the open list still shows isn't what a deleted repository looks like: hide it from the list too.
- **`cancel()` is a client closing its connection**, and returns once the daemon has seen the close (testkit.md). If `waiters` hasn't fallen right after, that's a finding (API-14), not a reason to advance.
- **A cursor from one PR's `changes` or `wait` covers that PR only** (API-8). `<epoch>-0` is the cursor before any revision (STA-9).
- **MOD-8's 60 s go by the wall clock, POL-11's 3 s by the monotonic one** (POL-26). Under a plain `advance` they move together; only the sleep part tells them apart.
- An event's `kinds` is a list of objects tagged `kind` (MOD-15). Titles hold the fake's marker; never assert on a title.
- **`--kind` is checked by the daemon, the pairing of flags by the CLI.** An unknown kind reaches the daemon and comes back as `bad_request` (exit 2 with an API error's line); `--kind` with `--until merged` never leaves the CLI.

## Open questions
- **Reopening, a re-run, a push to the base and what a gone PR's return looks like aren't measured** (docs/02 §6; MOD-4, MOD-6, MOD-8, STA-2, API-10 and API-11's `head_moved` are (proposal)). The fake plays what the specs assume; test them as written.
- **`conflict`** is tested against MOD-11's order and docs/02 §4's `CONFLICTING`, which the row now cites.
- **`--kind` is a (proposal)** (API-10). A waiter's condition is evaluated by `api::condition` (T44): a `changed-kind` failure names T44 or T23.
- **Capped is reached by lowering `quota.max_per_hour` to 60**, its smallest value, and polling until the count is there. That's this task's choice; T34 owns the quota states themselves.
- **D6** (required checks and reviews unmeasured) touches `blocked` and `behind` only: no scenario here reads them.
