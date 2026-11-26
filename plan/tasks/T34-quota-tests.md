---
id: T34
title: "Quota tests: what an hour costs in each tier and state"
kind: test
milestone: M5
size: M
status: in-progress
depends_on: [T29]
requirements: [TST-6, U1]
owns: [tests/quota.rs]
shares: []
---

# T34 · Quota tests: what an hour costs in each tier and state

## Outcome
TST-6's eight groups of figures hold on the harness, counted from the fake GitHub's request log: the points an hour costs active, idle, hot and at 200 PRs, each quota state's cadence and `daemon.problem`, and the exhausted and secondary-limit cases.

## Context
The quota is the reason circitor exists (U1). T08 and T09 prove the limits and the cadence as pure functions; this task proves the **figures of TST-6 on the whole daemon**, counted from what the fake GitHub received. It runs on T29's harness, beside the scenario files (T30 to T33). T26 has a smoke test of the first two figures; the figures themselves are owned here.

## Read first
- `plan/conventions.md`: Tests, Requirement tracing, Report.
- `plan/contracts/testkit.md` (the harness; the fake's `requests()`, `points()`, `set_rate`, its exhausted and secondary-limit faults), `plan/contracts/clock.md` (`advance`, `sleep_wall_only`), `plan/contracts/poller.md` (`QuotaState`, `Policy`: what each state sends), `plan/contracts/api.md` (`StatusBody`, `DaemonBlock`, the error body).
- The module docs of `src/testkit/harness/` and `src/testkit/server/`: the real names of the controls.
- `specs/07-testing.md#what-is-tested`: TST-6, all eight bullets. `specs/00-overview.md`: U1.
- `specs/02-polling.md#cadence`: POL-10, POL-13 to POL-15. `#quota`: POL-17, POL-18, POL-23. `#errors`: POL-25, POL-26.
- `specs/03-state.md`: STA-3. `specs/04-api.md`: API-3, API-4, API-5 (the two quota warnings). `specs/05-cli.md`: CLI-5's code 9. `specs/06-operations.md`: OPS-2.
- `docs/02-github-api.md` §2: the cost formula, the measured table, and "Response when the limit is hit: assumed".

## Scope
`tests/quota.rs`: TST-6's eight bullets, one test or a few per bullet, each on its own harness and ending with `h.finish()`.

- **How points and requests are counted.** Points are the difference of `h.github().points()` across the measured hour. A poll is a request that held a finding (`requests()` says so, testkit.md); a poll of more than one request is still one poll. A PR's refreshes are the requests naming its node ID, by their monotonic times. Never count from the daemon's own `cost_last_hour`; compare it with the fake's sum as one more assertion.
- **The basic world** (TST-6): 10 tracked PRs, settled, with passed checks, `mergeable` known, nothing changing, so no follow-up (POL-11) and no item request (POL-12) is sent. OPS-2's defaults, set through `Harness::start(opts)`. **The first poll is two requests** (the finding, then the found PRs' first refresh, which costs 2). **Every later poll is one request of 1 point:** every request of a poll marks its PRs as sent at the poll's send time (plan decision P10, `plan/contracts/poller.md`), so all 10 fall due together and ride in the finding's request. So an active hour is about 120 points and an idle one about 12 (POL-13). **The measured hour** begins at the send time of the second poll, the first of one request.
- **Active and idle** (POL-13). Active: a `list` straight after the start, then one every 5 minutes for the hour: at most **130** points, at least **110** polls. Idle: no client request at all. **The tier at start is idle** until the first client request (`plan/contracts/poller.md`), so `status --json` reads `tier: idle` from the start, and the first two polls are `idle_interval` apart: at most **15** points, at least **11** polls.
- **One hot PR** (POL-14). Active tier, a background `wait --until merged --timeout 3600` on one PR: the hour costs at most **250** points more than the active hour without it. Idle tier: see Open questions; built as the 3 000 s of that same wait after the tier went idle, with no other client request, against the idle rate: at most 350 × 3 000 ÷ 3 600 = **291** points more than idle alone.
- **A waited-on PR** (POL-14). In the active hour it's refreshed at least **230** times, and every gap between its consecutive refreshes, of any kind, is 10 s to 12 s (a poll in flight can hold a hot refresh for POL-2's second).
- **`show` and hot** (POL-14). A `show` of a PR reading `checks_pending` (its checks running): for the next 120 s its refreshes are about 10 s apart, then back to the tier's. A `show` of a `ready` PR: no refresh outside polls.
- **Ended and asked** (POL-10). In the active tier, a merged PR and an `asked` one (taken in with one `show`) are each refreshed once per `idle_interval`: over 50 minutes, 9 to 11 times.
- **Slowed and capped** (POL-17, POL-23). Config: `quota.max_per_hour = 60`, so the states are reached in minutes; TST-6 lets the figures scale. Above 48 points in the hour: polls are 60 s apart and a waited-on PR's hot refreshes 20 s. At 60: no poll and no hot refresh is sent; two `show --fresh` 10 s apart send one refresh, and two `refresh` 15 s apart one poll (POL-15's 30 s windows: a `refresh` is served while capped); a `show` of an untracked PR and a `track` are sent, with their POL-12 request when a count rose.
- **200 PRs** (STA-3). OPS-2's defaults, 200 tracked PRs, a `list` every 5 minutes: the hour costs at most **600** points, and by its end the daemon ran slowed (polls 60 s apart) or capped (`quota_capped` in `status --json`).
- **Problems and warnings** (API-4, API-5). `daemon.problem` reads `null` when normal and slowed, `quota_low` in reserve and capped, `rate_limited` when exhausted and under a secondary limit, `github_unavailable` in a backoff, `auth` in the auth state. With `remaining` under the reserve **and** a 401 that stays: `auth`. With the count at 100 % and `remaining` under the reserve: both `quota_capped` and `quota_reserve` are in `warnings`; put `remaining` back: only `quota_capped`; let the hour slide past: none.
- **Reserve** (POL-18, POL-23). `set_rate` with `remaining` 400: polls and a waited-on PR's refreshes are `idle_interval` apart. With the cap as well: none are sent.
- **Exhausted** (POL-25, POL-23). An answer without `data` and `x-ratelimit-remaining: 0`, reset in 20 minutes: no request before the reset time, and requests again within an `idle_interval` after it; `show` of a tracked PR exits 0 from memory; `show` of an untracked one exits **9**, `rate_limited`, `error.until` the reset time. Without `x-ratelimit-reset`: nothing for an hour. A `sleep_wall_only` past the reset: a request is sent again within an `idle_interval`, where without the jump none would be.
- **Secondary** (POL-25). As a 403 and as a 200 with an error, each naming a secondary rate limit: with `Retry-After: 90`, nothing for 90 s; without one, nothing for 60 s, then 120 s after the next in a row, up to `idle_interval`; a read-through meanwhile exits 9, `rate_limited`, and sends nothing.
- **U1's share here:** `u1_many_clients_cost_as_one`: five background waits on one PR and a `list` and a `show` every 10 s from "other sessions" cost, over 10 minutes, no more points than one wait alone over the same 10 minutes, give or take one poll. The agents' reads come from memory; the waits share one refresh.

Interpretations: "an hour" is 3 600 s of the test clock from a poll's send time. "About 10 s" is 10 s to 12 s. A figure with "at most" is asserted as written, not with a margin of the test's own.

## Interfaces
- **Consumes:** `testkit::harness::Harness` (`start(opts)` with config, `clock()`, `github()`, `cli`, `cli_bg`, `finish`), `testkit::server::FakeGitHub` (`requests()`, `points()`, `set_rate`, the exhausted, secondary and 401 faults, `world`), `clock::TestClock` (`advance`, `sleep_wall_only`).
- **Provides:** nothing.

## Out of scope
- `Quota`, `Policy` and `Backoff` as tables: T08. Tiers, periods and windows as tables: T09.
- That the fake costs a query as GitHub does (`tests/cost/`): T14. The M1 smoke test of the first two figures: T26.
- The scenarios `tiers`, `forced`, `many-prs` (T32) and `mergeable-unknown`'s "no follow-up when capped or in reserve" (T31).
- The CLI's `rate_limited` line as text (TST-7): T10, T35.
- Measuring the real quota against GitHub: T40 (TST-13 step 4), by hand.
- Fixing what a figure finds in `src/`: T08, T09, T21 or T25, named in the `#[ignore]`.

## Acceptance
- `tests/quota.rs` starts with `#![cfg(feature = "test-endpoint")]`. Its tests carry `// TST-6:` (and the POL or API ID each proves) and names like `tst6_active_hour_costs_at_most_130`:
  - active: ≤ 130 points, ≥ 110 polls; idle: ≤ 15 points, ≥ 11 polls
  - one hot PR: ≤ 250 more in the active hour; ≤ 291 more over 3 000 idle seconds
  - a waited-on PR: ≥ 230 refreshes in the active hour, every gap 10 s to 12 s
  - `show` of a `checks_pending` PR: hot for 120 s; of a `ready` one: not
  - a merged and an `asked` PR: once per `idle_interval`
  - slowed: both intervals doubled; capped: only demand, `refresh` included, in 30 s windows
  - 200 PRs: ≤ 600 points, slowed or capped reached
  - each state's `daemon.problem`; `auth` over `quota_low`; the two warnings raised together and cleared apart
  - reserve: `idle_interval` for polls and hot refreshes; reserve with the cap: none
  - exhausted: silent before the reset, sending again within an `idle_interval` after it, exit 9 with `until`; no reset header: an hour; a wall jump past the reset resumes
  - secondary: 403 and 200-with-error; `Retry-After`; 60 s doubling without one
  - `u1_many_clients_cost_as_one`
- On a failure the test prints the request log's send times and costs for the measured span.
- No `sleep`, no `tokio::time` in the file; every test ends with `finish()`. The file runs in under three minutes of real time.
- `cargo test --features test-endpoint --test quota` and `make verify` pass. An ignored test is listed under Open items.

## Traps
- **A figure that fails because production code is wrong is kept:** mark it `#[ignore = "T09: <what's wrong>"]` with the owning task's ID, and report the measured number. Don't raise the limit, and don't trim the world until it passes.
- **`status` isn't a client request, `list` is** (API-5, POL-13). One stray `list` in the idle hour makes it an active one. Look with `status --json` and the fake's log.
- **Looking at a PR changes what it costs.** `show` and `wait` name it: a wait's arrival forces a refresh, and a `show` of a `checks_pending` PR makes it hot (POL-14, POL-15).
- **The world must be quiet.** One PR reading `UNKNOWN` adds a follow-up per refresh (POL-11); an unsettled list reads `checks_pending`. Give every PR an `updatedAt` two minutes old, passed checks and `MERGEABLE`.
- **A wait lasts at most 3 600 s** (API-12), and its arrival is a client request. The CLI's own limit is the deadline plus 10 s (CLI-7): await the command after the hour, don't cancel it before measuring.
- **The cost follows the page size asked, not the rows returned** (docs/02 §2). A poll of 10 PRs is 1 point; 41 to 80 PRs are 2 requests; a first refresh with its last 20 comments and reviews costs 2 for 40 PRs. Start measuring at the second poll, as Scope says.
- **A poll that keeps taking two requests is a finding,** not noise: it means a PR refreshed in a poll's second request was marked sent at that request's own time, against P10. Report it against T21 with the send times; don't measure around it.
- **The daemon starts idle.** Until the first client request its polls are `idle_interval` apart. The active tests send their first `list` straight after `start`, in `cli_bg` (it waits for the first poll, API-18), so the second poll comes `interval` after the first, not 5 minutes.
- **Don't assert a poll at the reset to the second.** After a reset the daemon knows only the last `remaining` it saw, so it may sit in `reserve` (`idle_interval`) until the next answer says otherwise. Assert silence before the reset and sending within an `idle_interval` after.
- **A follow-up moves a hot PR's last send time** (POL-11, POL-14): where a PR reads `UNKNOWN`, its refreshes come 3 s and then 10 s apart, and the hour's count changes. Keep the measured worlds `MERGEABLE`.
- **`set_rate` is GitHub's side** (`remaining`, the reset), apart from what circitor spends: it reaches the reserve at once. The cap is circitor's own count: it's reached only by spending, and it slides off only as the wall-clock hour passes (POL-26).
- **Exhausted and secondary go by different clocks:** the reset time is wall time, `Retry-After` and the 60 s are monotonic (POL-26). Only the wall jump tells them apart.
- An hour is 360 ticks and over 100 polls of real loopback I/O: a second or two each. Don't run more hours than the bullets need.

## Open questions
- **A hot PR for a whole idle hour can't be staged:** a wait is the only way to keep a PR hot that long, it lasts at most 3 600 s (API-12), and its arrival keeps the tier active for `idle_after`. Built: the idle part of one wait, as a rate. Report it as a spec question on TST-6.
- **The exhausted and secondary-limit answers are assumed** (docs/02 §2, §6 item 6; POL-25 is a proposal). The fake plays POL-25; a measurement may change both.
- **Every value is (proposal):** OPS-2's defaults, POL-17's 80 % and 100 %, POL-23's table, STA-3's 200. Keep the figures as constants named after TST-6's bullets.
- D5 and D6 don't touch this task.
