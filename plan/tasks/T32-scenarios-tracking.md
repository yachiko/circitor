---
id: T32
title: "Scenarios: tracking and polling"
kind: test
milestone: M5
size: L
status: todo
depends_on: [T29, T42]
requirements: [TST-8]
owns: [tests/scn_tracking.rs]
shares: []
---

# T32 · Scenarios: tracking and polling

## Outcome
Eleven of TST-8's scenarios run on the harness and pass: `read-through`, `owners`, `renamed`, `repo-gone`, `pr-error`, `many-prs`, `track-untrack`, `forced`, `tiers`, `bare-pr`, `retention`.

## Context
T29's harness runs the whole daemon in-process on the test clock, against the fake GitHub (T14, T20), with the CLI's commands as functions. This task plays the eleven TST-8 rows about **which PRs are tracked and how they're polled**: reading through, owners, renames, gone and failing PRs, the limits, `track` and `untrack`, forced refreshes, tiers, naming by branch, and retention. T30, T31 and T33 play the other rows. Its failures are how bugs in T15, T21, T22, T25 and T27 are found.

## Read first
- `plan/conventions.md`: Tests, Requirement tracing, Report.
- `plan/contracts/testkit.md`, `plan/contracts/clock.md` (`advance`, `sleep_wall_only`, what "settled" means), `plan/contracts/api.md` (each route's body, `StatusBody`, the error body), `plan/contracts/model.md` (records and events as JSON), `plan/contracts/state.md` (`FileEntry`: what `h.state_file()` holds), `plan/contracts/cli.md` (`Output`, exit codes, `FakeSpawner`, `FakeGit`).
- The module docs of `src/testkit/harness/`, `src/testkit/server/` and `src/testkit/github/`: the real names of the controls. `tests/m3_cli.rs` shows a test driving the harness.
- `specs/07-testing.md#what-is-tested`: TST-8, the eleven rows named in the Outcome. `#test-doubles`: TST-3 to TST-5.
- `specs/02-polling.md#the-poll`: POL-4, POL-5, POL-7, POL-10. `#cadence`: POL-13 to POL-16. `#errors`: POL-21, POL-24.
- `specs/03-state.md#the-tracked-set`: STA-1 to STA-5. `#the-state-file`: STA-7, STA-8.
- `specs/04-api.md`: API-3, API-5 (the warnings), API-7, API-15, API-16. `specs/05-cli.md#commands`: CLI-2. `specs/01-model.md`: MOD-1, MOD-11, MOD-15, MOD-16.
- `docs/02-github-api.md` §2 (the measured costs of `nodes(ids:)`), §3 and §5. `docs/measurements/2026-11-24/b1.log`: a PR is in the open list 1 s after creation, with `mergeable` `UNKNOWN`.

## Scope
**TST-8's share here: these eleven rows**, in `tests/scn_tracking.rs`. The row is the spec; each entry below adds only the world's concrete values and what to assert. **Where a row and the requirement it cites differ, the requirement wins** (TST-8): then assert the requirement and report the row.

How every scenario is written:
- **One `#[tokio::test] async fn scn_<name>()` per row**, hyphens as underscores, with `// TST-8: <name>` and the row's IDs (`// POL-21:`) above it. A row with several worlds may be several functions named `scn_<name>_<part>`, so an `#[ignore]` costs one part. Each starts its own harness and **ends with `h.finish()`**.
- **Script the world, and set the config, before the daemon's first poll**, through `Harness::start(opts)` (testkit.md); `no_daemon` gives a harness whose daemon the fake spawner starts, and counts its spawns.
- **Look through the CLI:** `--json` output parsed as JSON, `Output.code`, events from `changes --since <cursor> --json`, counts and warnings from `status --json`, the file from `h.state_file()`. Assert text only where an entry quotes a line; the layout is T35's.
- **Count requests in `h.github().requests()`**: "sends no request" means its length didn't change. Each entry says whether it held a finding (a poll) and that page's cursor, the node IDs asked, and any by-name lookup. A PR's refreshes are the requests naming its node ID.
- **Advance and look; never sleep.** A command that needs clock time runs under `cli_bg`; the test advances, then awaits it.
- Keep the spec's values as named constants (`BATCH = 40`, `FIND_PAGE = 100`, `TRACKED_MAX = 200`, `GONE_AFTER = 5 min`, `GONE_MIN_READS = 3`, `ASKED_FOR = 60 min`, `PR_WINDOW = 5 s`, `POLL_WINDOW = 10 s`): most are (proposal).

Names: login `octocat`; the user's PRs are `octo/app#N`, other people's `else/lib#N`. "Settled PR": `updatedAt` two minutes old at the first read.

The scenarios:

- **`read-through`** (POL-16). World: the first poll is done; then `octo/app#9` is created at `C` (b1.log: listed at once, `UNKNOWN`). Steps: `show octo/app#9 --json` at `C+1`. Assert: exit 0, a record, `source: asked`; one request by owner, repository and number, with no finding; `status --json` reads `tracked.asked: 1`. Then `show octo/app#9999` and `show octo/nosuch#1`: exit 5, `not_found`, and `tracked.asked` still 1.
- **`owners`** (POL-5, STA-1). Config: `owners = ["acme"]`. World: the user's open PRs `acme/app#1`, `Acme/tools#2` and `else/lib#3`. Assert after the first poll: `list --json` holds #1 and #2 as `authored` and not #3; no request names #3's node. `track else/lib#3`: exit 0, `source: manual`; two polls later it's still listed and was refreshed.
- **`renamed`** (MOD-1, POL-4, POL-5). (a) `octo/app#1` tracked, with a background `wait --until merged`. Rename the repository to `octo/app2`, node IDs kept. Before the next poll, `show octo/app2#1 --json`: the record, `source: authored`, and `status` still counts one tracked PR. After the next refresh: one record, `pr: octo/app2#1`, **one** event with `edited` and `fields: ["pr"]`. The wait is still held; merge the PR: exit 0. (b) Config `owners = ["octo"]`; transfer the repository to `else/app`: the record reads `pr: else/app#1`, `source: asked`, with one `edited` event.
- **`repo-gone`** (POL-21, MOD-11). Keep the tier active with `list` unless a part says idle. "Hidden" means off the open list and `NOT_FOUND` by node.
  - *Gone:* three settled PRs, #2 hidden from `t`. Before the first poll sent 5 minutes after its first `NOT_FOUND`: `state: open`, `stale: true`. From that poll: `state: gone`, `status: gone`, `stale: false`, one event with a `gone` kind, `fetched_at` its last good read. #1 and #3 are never stale and their changes are applied.
  - *Three in under 5 minutes:* after three `NOT_FOUND` polls, still `open` and `stale`; return it before 5 minutes: applied, not stale, never gone.
  - *Fewer than three:* in the idle tier (no client request; look at the end only) the `gone` event's `at` is the third `NOT_FOUND` poll's time, 10 minutes in, not the second's at 5.
  - *Changed login:* `set_login("someone")`, then hide #2: open and stale after 15 minutes.
  - *Every PR:* two tracked PRs, both hidden: neither is gone after 15 minutes.
  - *Alone:* one tracked PR, hidden: gone as in the first part.
  - *Hot:* with a background `wait --until merged` on #2, hidden: the `gone` event's `at` is a poll's send time, the same poll as without the waiter; the wait exits 4, `ended: gone`. And with `NOT_FOUND` aimed at requests without a finding only (testkit.md's aimed faults), for 6 minutes: never gone.
- **`pr-error`** (POL-16, POL-21, POL-24, API-5). World: three settled PRs; #2's node answers `null` with a `FORBIDDEN` error until cleared. Assert: a change to #1 makes its event; #1 and #3 read `stale: false`. #2: `state: open`, `stale: true`, `error.type: "FORBIDDEN"`, `error.since` the first failure; `show octo/app#2` prints `error FORBIDDEN` in its stale mark. `status --json` has no `pr_failing` under 5 minutes in, and one after, whose message names `octo/app#2`, `FORBIDDEN` and the token source, and doesn't hold `MARKER_GITHUB_TEXT`. After 15 minutes it's still `open`. Clear the fault: at the next refresh `error: null`, `stale: false`, the warning gone. A read-through: `show octo/app#5`, with a `FORBIDDEN` fault aimed at its by-name lookup: exit 9, `github_unavailable`, the message naming `FORBIDDEN` and not the marker; it isn't tracked.
- **`many-prs`** (POL-7, STA-3). Worlds of settled PRs with distinct `updatedAt`.
  - *41:* a poll is two requests, naming 40 nodes and 1, each costing 1.
  - *150:* the first poll's finding is two requests (the second with a cursor), and `list --all --json` holds 150 records.
  - *200:* all tracked. `track else/lib#1`: exit 10, `limit`, no request. `wait else/lib#1 --until merged`: exit 10, no request. `show else/lib#1`: **exit 10, `limit`, nothing sent** (STA-3).
  - *205:* the 200 newest by `updatedAt` are tracked; the 5 oldest are in neither `records` nor `pending`; `status --json` has `pr_limit`. Close those five in the world: after the next poll the warning is gone.
- **`track-untrack`** (API-15, API-9, STA-1, STA-3).
  - `track else/lib#1`: one request; `source: manual`; one `tracked` event; `h.state_file()` holds it as soon as the command returns, with no advance (STA-7). A second `track`: exit 0, no request, the cursor unchanged. The same second `track` in a world with 200 tracked PRs.
  - `untrack else/lib#7`, never tracked: exit 0, no request, `record: null`.
  - `untrack octo/app#1`, `authored`: nothing changes, no event.
  - `track octo/app#1` (now `manual`), then `untrack`: `source: authored`, no event, the file says `authored`. `untrack else/lib#1`: dropped, one `untracked` event, `list --since <cursor>` names it in `untracked`, the file doesn't hold it.
  - `track else/lib#2`, a background `wait --until merged` on it, then `untrack`: `source: asked`, the wait still held; merge it: the wait exits 0.
- **`forced`** (POL-15, API-16).
  - Two `show --fresh` 3 s apart: one refresh in the log; both answers carry the same `fetched_at`.
  - Two `refresh --json` 5 s apart, the first over 10 s after the last poll: `{outcome: polled, cost: 1}`, then `{outcome: shared, cost: 0}`; one poll in the log.
  - `hold_next()`. Start three `show --fresh` in the background. Release: one request, three answers with one `fetched_at`.
  - 200 tracked PRs, so a poll is five requests. `hold_next()` on the poll's first request; start `show octo/app#N --fresh` for a PR that isn't in that request and wasn't refreshed in the last 5 s; release. The next entry in the log names that one node and has no finding.
- **`tiers`** (POL-13, POL-14). World: two settled PRs. Steps: one `list` at `t0`, then only `status --json`. Assert: `daemon.tier` is `active` until `t0 + 600 s` and `idle` after; polls are 30 s apart before and 300 s apart after. Then, over 30 s after a poll, `list`: a poll is sent at that instant and the tier is `active`. Then a background `wait octo/app#1 --until merged`: for the next 60 s, #1's refreshes are about 10 s apart, each due 10 s after its last of any kind, a poll's counting; #2 is refreshed by polls only.
- **`bare-pr`** (CLI-2). Working directory: `h.git_repo("https://github.com/octo/app.git", "feat")`; every PR below is `octocat`'s with head `feat`.
  - Opened after the last poll, which is over 10 s old: `show` with no PR finds it, exit 0; the log gained one poll and its first refresh.
  - Create the PR; `hold_next()`; start `refresh` in the background (its poll is now in flight), then `show`; release and advance: the poll `show` asks for is `shared`, and it still prints the PR, exit 0.
  - No open match, #3 merged and #4 closed later: `show` prints #4 and says it's ended.
  - Two open matches: exit 2, stderr names both.
  - A working directory that isn't a git repository: exit 2, and nothing was sent: the fake's log is empty and the spawner's count is 0, on a `no_daemon` harness.
- **`retention`** (STA-1, STA-2, STA-5, STA-8). `s` is the send time of the refresh that first showed a PR ended.
  - *24 hours:* a merged PR. Jump the wall clock (`sleep_wall_only`) to `s + 24 h − 30 s`: `list --all` holds it. Advance 40 s: gone from it, one `untracked` event whose `at` is under 10 s after `s + 24 h`, and `list --since <cursor>` names it in `untracked`.
  - *A waiter:* `keep_done = 60`; a background `wait --until changed --timeout 600` on the merged PR; at `s + 120 s` it's still tracked; cancel the wait: dropped, with its event.
  - *`keep_done = 0`:* a background `wait --until merged`; merge: exit 0, `satisfied`, the record `merged`; an `untracked` event follows at the next tick.
  - *Asked:* `show else/lib#1` ends at `a`. At `a + 60 min − 30 s` (wall jump) `tracked.asked` is 1; 40 s later 0, with an `untracked` event. Again with a wait held on it past `a + 61 min` (a plain advance): still 1; it leaves 60 minutes after the wait ended.
  - *Eight days:* three ended PRs; `sleep_wall_only(8 d)`, advance 10 s: three `untracked` events, their revisions in the order of `pr`.
  - *On loading:* `stop()`; jump the wall clock 25 h; `restart()`: the merged PR is in neither `records` nor `pending`, and no event names it.
  - *Dropped, then shown:* a PR with 3 comments, dropped as above; `show` it: `source: asked`, one `tracked` event, no `activity` kind.

## Interfaces
- **Consumes:** `testkit::harness::Harness` (`start(opts)` with config and `no_daemon`, `clock()`, `github()`, `cli`, `cli_bg` and its `cancel()`, the working-directory and spawner variants, `git_repo`, `state_file()`, `stop`, `restart`, `finish`), `testkit::server::FakeGitHub` (`world`, `set_login`, the aimed faults, `hold_next()`, `requests()`), the world's controls listed in testkit.md, "What the world must be able to play", `clock::TestClock` (`advance`, `sleep_wall_only`), the JSON of `plan/contracts/model.md`, `api.md` and `state.md`.
- **Provides:** nothing.

## Out of scope
- The other TST-8 rows: T30 (checks), T31 (a PR's life and waits; `gone-returns`, `baseline`), T33 (the daemon's life; `restart`, `sleep`, `clock-back`).
- What an hour costs in each tier and state (TST-6): T34. `tiers` proves the cadence changes, not the figures.
- The CLI's exact text and every exit code (TST-7): T10, T35. `match_branch`'s table and a detached HEAD: T11, T27.
- The store's limits and the engine's batches as unit tests: T15, T21. The state file's faults (TST-10): T16.
- The engine's per-PR error runs as unit tests (`gone`, `pr_failing`): T42.
- The harness and the fake GitHub: T29, T20, T14. A missing control is reported, not added.
- Fixing what a scenario finds in `src/`: the task named in the `#[ignore]`.

## Acceptance
- `tests/scn_tracking.rs` starts with `#![cfg(feature = "test-endpoint")]` and holds every scenario above as `scn_<name>` or `scn_<name>_<part>`, each asserting everything its entry lists, with the entry's values.
- `cargo test --features test-endpoint --test scn_tracking -- --list` shows at least one test for each of the eleven names; `grep -c 'TST-8' tests/scn_tracking.rs` is at least 11.
- No `sleep`, no `tokio::time`, no `std::thread` in the file. Every test ends with `finish()`, so TST-11's sweep and the fake's rules (POL-1, POL-2) ran for each.
- The whole file runs in under three minutes of real time; say in the report which test is slowest.
- `cargo test --features test-endpoint --test scn_tracking` and `make verify` pass. An ignored test is listed under Open items with its reason.

## Traps
- **A scenario that fails because production code is wrong is kept:** mark it `#[ignore = "T21: <what's wrong>"]` with the owning task's ID, and report it. `gone`, `pr_failing` and a PR's error are T42's (the engine's per-PR error runs): `repo-gone` and `pr-error` name T42. Don't weaken the assertion, change the world to fit, or touch `src/`.
- **Every control used here is promised by `plan/contracts/testkit.md`.** One the code doesn't have is reported; write the scenario as far as it goes and mark it `#[ignore = "T20: no control for …"]`. Don't build it yourself.
- **An awaited `cli()` hangs when the command waits on the clock:** every `wait`, every command that spawns, and any command whose GitHub request must wait out POL-2's 1 s gap or needs a second request (a found PR's first refresh). Use `cli_bg`, advance, then await (testkit.md).
- **Looking is a request.** `show`, `track` and `wait` name the PR (STA-1): they restart an `asked` PR's 60 minutes and can make it hot. `list` keeps the tier active. Only `status` touches nothing: read `tracked` and `warnings` from it in `retention`, `tiers` and the idle part of `repo-gone`.
- **A wall-clock jump is a sleep** (POL-22): records go stale, a poll may start, and held waits past their deadline end. Retention goes by the wall clock (POL-26), so a jump is the cheap way to 24 h; where a wait must survive, use a short `keep_done` and a plain advance.
- **Ended PRs hold places** (STA-3). Closing tracked PRs doesn't make room; in `many-prs` it's the left-out PRs that are closed.
- **`source` alone is not a visible change** (MOD-14): `untrack` to `authored` makes no event. Read `source` from the record.
- **`gone` counts polls only**, under one login, never when every PR of the poll is `NOT_FOUND` (MOD-11): each part of `repo-gone` needs its other PRs chosen on purpose.
- **A 200-PR world is slow to start:** two finding pages and five first refreshes, 1 s apart (POL-2). Advance 10 s before the first `list`.
- **GitHub's error text must not reach an answer** (API-3, OPS-9). `finish()` sweeps the log and the state file for it, not API responses: assert it yourself in `pr-error`.
- An event's `kinds` is a list of objects tagged `kind`; the order "of `pr`" is owner, repository, then the number as a number (MOD-1).

## Open questions
- **`many-prs`:** at the limit a `show` of an untracked PR answers `limit` (STA-3, and the row), as `plan/contracts/poller.md` has it.
- **`track-untrack` cites CLI-10**, which is about a wait's reconnect. The part "with a waiter makes it `asked`" is tested against API-9 and API-15.
- **That a node ID survives a rename or a transfer, and that lost access looks like `NOT_FOUND`, aren't measured** (MOD-1, docs/02 §5). STA-1's 60 minutes, STA-2, STA-3, POL-5's second rule, POL-15's windows and POL-21's "three, and 5 minutes" are (proposal). Test them as written.
- **`bare-pr`'s `shared` part** is staged with `hold_next()`. If the CLI's first look already finds the PR, the staging is wrong, not the CLI: don't loop until it happens.
- D5 and D6 don't touch this task.
