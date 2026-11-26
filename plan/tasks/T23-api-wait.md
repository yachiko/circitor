---
id: T23
title: "Waiters: the wait route, its conditions, outcomes, deadline and clean end"
kind: code
milestone: M2
size: L
status: done
depends_on: [T09, T15, T41, T44]
requirements: [API-9, API-10, API-11, API-12, API-13, API-14, U6]
owns: [src/api/wait/]
shares: []
---

# T23 · Waiters: the wait route, its conditions, outcomes, deadline and clean end

## Outcome
`api::wait::routes()` holds a `wait` open: it registers the waiter, sends the arrival cursor at once, evaluates its condition after every refresh of its PR, and answers `satisfied`, `ended` or `timeout` by API-10 to API-12, leaving nothing behind when the client goes away.

## Context
T02 declared `wait::routes()` as a stub; **T43 built the API's shared parts** (`Params`, `respond` with API-8's `window`, `router`, `InFlight`) and `FakePoller`, and **T44 built `api::condition`**: `Condition::check` and `evaluate`, the one reading of API-10 and API-11, which the notifier's subscriptions use too (NTF-4). A fault you find in either is reported, not fixed; T41 wrote the golden files; T15's store has the waiter and naming guards and the `changed()` channel; T09's rule is behind `respond::present`. This task builds the one route that holds a request open (U6). The poller is seen as `PollerHandle` only. T22 builds the other routes; T28 serves the router and is what notices a client that goes away; T27's CLI reads the head, then the body.

## Read first
- `plan/conventions.md`
- `plan/contracts/api.md`: the whole file, "Declarations (`src/api/condition.rs`, T44)" with its bullets above all; `plan/contracts/state.md`: `View`, `PrView`, `Timing`, `lookup`, `places_free`, `name`, `add_waiter`, `changed`, and "Semantics"; `plan/contracts/poller.md`: "Outside" and "The fake"; `plan/contracts/clock.md`: `SleepWatch`, "Deadlines that count sleep", "`Activity`: who counts what"
- `specs/04-api.md#waiting`: API-9 to API-14; `#reading`: API-8 (`limit`, a too-old cursor, a cursor for one PR)
- `specs/03-state.md`: STA-3 (the limit), STA-8 (a loaded PR's wait), STA-9 (judged when the answer is built)
- `specs/02-polling.md#errors`: POL-22's last point; `specs/00-overview.md`: U6
- In `testdata/api/` (T41's, there once T41 is merged): `wait-satisfied.json`, `wait-timeout-no-record.json`, `error-limit.json`; `plan/contracts/testkit.md`: `golden::check`

## Scope
- **Arrival, in API-9's order:**
  1. `poller.client_request()`; the parameters (API-1): `until` (required), `since`, `sha`, `rerun_since`, `fail_fast` (a flag) and `kind`, taken with `Params` into an `Ask` and checked by **`Condition::check`** (API-10's rules are T44's: which goes with which `until`, the SHA's form, the kind words); `timeout` (0 to 3 600, default 100, API-12), `limit` (1 to `state::MAX_EVENTS`).
  2. The limits: an untracked PR with `places_free() == 0` is `limit` (STA-3); `view.waiters` at `state::MAX_WAITERS` is `limit` (API-14). Nothing is sent.
  3. The read-through, if the PR has no record and isn't `Loaded`. A refusal answers as API-3.
  4. `store.add_waiter` and `store.name`, `poller.nudge()`; then the status line and headers go out, with `circitor-cursor` set to the view's cursor at that moment.
  5. The arrival refresh, `poller.refresh_pr`, unless the PR is `Loaded` without a record. `NotSent` and `Failed` count as failed: the first evaluation follows at once (API-9, API-13).
  6. The first evaluation.
- **The body completes later.** The response is returned at step 4 with a body that yields once, when the wait ends. **The body owns the waiter:** its guards, its `InFlight`, its future. No detached task.
- **Evaluating (API-10, API-11: this task's share is when; the rule is T44's).** `api::condition::evaluate(&cond, &view, &id, &mut progress)`, called after the arrival refresh, on every `store.changed()`, and at the deadline, each time on a fresh view. **Don't read API-10 or API-11 for yourself here:** `--kind`, `--fail-fast`, `head_moved` and the order of condition and ends are all inside it. One `Progress::new(arrival's monotonic time)` per wait, kept in the body.
  - **The first call comes after step 5**, so `changed` without `since` takes its baseline there, not at the header's cursor.
  - **`progress.rerun_dropped`** (a too-old `rerun_since`) puts `reset: true` in the answer (API-10).
- **A `Loaded` PR (STA-8)** is held with no read-through and no arrival refresh until the view has its record, and first evaluated then.
- **The deadline (API-12).** From the request's arrival: the read-through and the arrival refresh count. Passed when `mono().since(arrived)` plus the sleep noticed since arrival reaches `timeout`; re-checked when `SleepWatch` changes. At the deadline the condition is evaluated once more on the view, then `timeout`.
  - **During the read-through:** drop its future (the poller finishes it) and answer a plain 200, `timeout`, `record: null`. **Interpretation:** it carries `circitor-cursor` too.
  - **`timeout=0`** has no deadline around steps 3 and 5: it evaluates once after them and answers.
- **The answer** is always 200 once the waiter is registered, built from one view: `outcome`, `reason`, the record through `respond::present` (or `null`), and the events, `more_events`, `reset` and the body's cursor **as `respond::window` gives them** on that view, for this PR's ID, with `limit`, and with `since` or, without one, the header's cursor. Don't re-implement the `since`, `limit` or too-old rule. Because `window` runs on the answer's view, too old is judged when the answer is built (STA-9); with `reset`, `records` holds the record, without `checks.items` (API-19: `present(…, false)`), while `record` keeps them. **The late body is `respond::envelope(state, cursor, body)`**; `ok` is for the whole 200 sent before a waiter is registered.
- **API-13's share:** the waiter makes the PR hot through `add_waiter` and `nudge()`, again when it leaves; and step 5. The refreshes are T25's.
- **API-14's share:** the 64 at step 2, and the clean end: everything the wait holds is released in `Drop`, whenever the handler's future or the body is dropped. Closing connections at a stop is T28's.
- **Work in flight (clock.md, "`Activity`: the rule"):** `InFlight::park()` whenever the wait awaits (the poller, `changed()`, the deadline), `resume()` when it wakes.
- **Named constants** for API-12's 100 and 3 600. API-11's 60 s is T44's.

## Interfaces
- **Consumes:** `api::{AppState, Params, pr_from_path, bad_request, ok, envelope, window, Window, error, refusal, present, InFlight}`, `api::condition::{Condition, Progress, evaluate}` (T44), `api::types::{WaitBody, Ask, Outcome, Reason, Until, CURSOR_HEADER}`; `state::store::{Store, View, Waiting, Named, Class}`; `state::{MAX_WAITERS, MAX_EVENTS}`; `poller::{PollerHandle, Refreshed}`; `clock::{SleepWatch, sleep}`.
- **Provides:** `api::wait::routes() -> axum::Router<AppState>` with `GET /v1/prs/{owner}/{repo}/{number}/wait`. Nothing else is public.

## Out of scope
- Every other route, and API-18's wait for the first poll (`wait` doesn't wait for it): T22.
- API-8's window rule itself, the `unknown_route` fallbacks, `Params`, `respond` and `FakePoller`: T43's. `src/api/mod.rs` and `types.rs`: T02's.
- **What a condition means and when it ends** (API-10's and API-11's tables, `kind`, `fail_fast`): T44, with its table test. The notify routes, which evaluate the same way without a waiter: T47.
- The 64 and the 200 themselves, dropping a PR when its last waiter ends: T15. Hot refreshes, forced refreshes and their windows: T25.
- Noticing that a client closed, and closing connections at a stop: T28. The CLI's deadline, `again:` line and reconnect: T27.

## Acceptance
Unit tests in `src/api/wait/`, through `oneshot` on `api::router`, reading the body as a stream, on `TestClock`:
- `api10_api11_through_the_route`, a few cases end to end (the table is T44's): a `merged` wait answers when the PR merges; a `checks-done` wait on a closed PR answers `ended`, `closed`; `fail_fast=1` answers `satisfied` at the first failed check while another runs, and without it the wait goes on; `kind=activity` isn't answered by a `checks` event and is by an `activity` one, and a merge answers it `ended`, `merged`.
- `api9_arrival_order`: `timeout=3601`, `sha=xyz`, `sha` with `until=merged`, `fail_fast=1` with `until=merged`, `kind=foo`, `fail_fast=true`, and no `until` are 400 with the poller untouched; an untracked PR at 200 places is 409 `limit` with no read-through; a refused read-through is its API-3 error; otherwise the head is 200 with `circitor-cursor` equal to the cursor at registration, before any body.
- `api9_loaded_is_held`: no `read_through`, no `refresh_pr`; the body completes after the first `apply`; answered before it, `record: null`.
- `api10_changed_baseline`: the arrival refresh changes the record and the wait goes on; the next change answers `satisfied`, with both events.
- `api11_head_moved`: matched, then another head: `ended`, `head_moved`. Never matched: not on a refresh sent 59 s after arrival, yes on one sent at 60 s. With no refresh, the clock at 200 s of a 3 600 s wait ends nothing.
- `api12_timeout`: the default ends at 100 s of the clock, not at 99 s; `timeout=0` answers after the arrival refresh with the clock unmoved; `sleep_wall_only` of 2 hours, then a `SleepWatch::check`, ends a 100 s wait as `timeout`; `set_wall_back` of an hour doesn't lengthen it; a read-through that outlasts the deadline (a `PollerHandle` of the test's own that sleeps on the clock) gives 200, `timeout`, `record: null`.
- `api13_hot`: once the head is out, `Timing::waiters` is 1, `nudge` and `refresh_pr` were each called once; with `NotSent` and a merged record, a `merged` wait answers at once.
- `api14_64_and_cleanup`: with 64 held, the 65th is 409 `limit`; dropping one body leaves `view.waiters` at 63 and lets the next in; dropping a response during the arrival refresh leaves no waiter.
- `api19_shapes`: the bodies have the key names of `wait-satisfied.json` and `wait-timeout-no-record.json`, at every level and in order; a refusal has `error-limit.json`'s. A whole-body comparison, where a test can make one, goes through `testkit::golden::check`.
- `sta9_reset_at_answer`: 501 events while a wait is held: its answer has `reset: true` and `records`, whose record has no `checks.items` while `record` does (API-19; `wait-reset.json`'s shape). `api8_limit`: `limit` reaches `window`: 5 events with `limit=2` give 2, `more_events: true`, and the second's cursor in the body.
- `cargo test --features test-endpoint --lib api::wait::` and `make verify` pass.

## Traps
- **Two different numbers.** The header's cursor (step 4) is where the answer's events start; `changed`'s baseline is taken after step 5. Mixing them makes a wait answer on its own arrival refresh.
- **`refresh_pr` returning `Applied` doesn't mean a change**, and `changed()` moves for other PRs too: always evaluate on a fresh view.
- **Mark the watch seen before you evaluate** (`borrow_and_update`), then await `changed()`: the other order loses a refresh applied in between.
- **Cleanup lives in `Drop`.** The server drops the body when the client closes; there's no other signal. A waiter left behind keeps its PR hot and tracked for ever.
- A held wait that keeps its `InFlight` unparked stops `TestClock` from settling, and every scenario with it.
- **The deadline is the monotonic clock plus noticed sleep**, never the wall clock (POL-22, POL-26).
- `respond::ok` returns a whole response, and a registered wait's head is already out: its body is `respond::envelope`'s bytes, with `Window::cursor`. A registered wait never answers with an error status.
- `FakePoller`'s closures answer at once: a slow read-through needs a `PollerHandle` written in the test.

## Open questions
- **(proposal):** API-12's 100 and 3 600, API-14's 64. Built as written. API-10's and API-11's own proposals are T44's.
- **A wait's `record` carries `checks.items`; a reset's `records` don't** (API-19).

## Evidence
- **2026-11-26**, branch `t23-api-wait`, executor on **Opus, medium**; reviewed and merged by the orchestrator.
- `src/api/wait/mod.rs` and a new `tests.rs`; no public signature changed. `cargo test --features test-endpoint --lib api::wait::`: 11 passed, 6 runs, no flake. `make verify` passes on main after the merge (408 lib tests), the macOS check included.
- Choices: a deadline that passes during the read-through answers with `circitor-cursor` and the events `window` gives for an ID matching none; a tracked `First` PR without a record is read through; the limit errors are the executor's words ("Too many waiters: 64 are held."). No whole-body golden file (shapes compared with T41's files instead).
