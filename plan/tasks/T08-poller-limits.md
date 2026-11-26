---
id: T08
title: "Poller limits: the quota count, the quota states and the backoff"
kind: code
milestone: M1
size: M
status: done
depends_on: [T02]
requirements: [POL-17, POL-18, POL-23, POL-19, POL-25, POL-26]
owns: [src/poller/limits/]
shares: []
---

# T08 · Poller limits: the quota count, the quota states and the backoff

## Outcome
`poller::limits::Quota` counts what circitor spends over a sliding hour, follows GitHub's `remaining`, and gives POL-23's state and its `Policy`; `Backoff` gives POL-19's retry times. Pure code the engine (T21) drives with the clock's times.

## Context
T02 declares `Quota`, `Backoff`, `QuotaState` and `Policy` with empty bodies. The engine's loop (T21, then T25) owns one of each: it tells them what every answer and failure said, and asks them what may be sent now. Cadence (T09) reads the `Policy` this task computes. Nothing here has a clock, a timer or I/O.

## Read first
- `plan/conventions.md`
- `plan/contracts/poller.md`: "Inside: limits" (the signatures to fill), and `Tier`. `plan/contracts/config.md`: `Config`, `QuotaConfig`. `plan/contracts/clock.md`: `Time`, `Mono`, and "Two clocks, never mixed".
- `specs/02-polling.md#quota` POL-17, POL-18, POL-23 (the table is this task); `#errors` POL-19, POL-25, POL-26; `#cadence` POL-15 (the two windows a `Policy` carries) and POL-16 (one read-through per step).
- `specs/07-testing.md#what-is-tested` TST-6's quota lines and TST-8 `clock-back`: T34 and T33 test these rules end to end later.
- `docs/02-github-api.md` §2: the limit, the lagging counters, and that the exhausted answer is `assumed`.

## Scope
- **The count (POL-17).** `spent` records a cost at a wall time; `cost_last_hour` sums the costs under an hour old. A time ahead of now counts as now (POL-26, through `Time::since`), so a clock set back resets nothing.
- **GitHub's `remaining` (POL-18).** `observed` keeps the last reading and its reset time. **The last reading stands until the next answer**: this brief's reading, the spec doesn't say when a low `remaining` stops counting. `in_reserve` is `remaining < quota.reserve`.
- **The state (POL-23),** the first that holds from the bottom of the table up: secondary, exhausted, capped, reserve, slowed, normal. Slowed is a count **above** 80 % of `quota.max_per_hour`; capped is a count **at or above** 100 %.
- **The policy (POL-23), per state and tier:**

| State | `poll_every` | `hot_every` | `demand` | `pr_window`, `poll_window` | `follow_up` |
|---|---|---|---|---|---|
| normal | the tier's: `interval`, or `idle_interval` | `hot_interval` | yes | 5 s, 10 s | yes |
| slowed | the tier's, doubled | `hot_interval` doubled | yes | 5 s, 10 s | yes |
| reserve | `idle_interval` | `idle_interval` | yes | 30 s, 30 s | no |
| capped | `None` | `None` | yes | 30 s, 30 s | no |
| exhausted, secondary | `None` | `None` | no | 30 s, 30 s | no |

  "The tier's, doubled" in the idle tier is `idle_interval` doubled: this brief's reading of "`interval` doubled".
- **POL-23's last part, the warnings:** `capped(wall)` and `in_reserve()` are each warning's own condition, apart from the state in force. Raising and clearing them, obeying the policy, and `daemon.problem` are the engine's (T21).
- **Exhausted (POL-25's share here: how long it holds).** After `exhausted(reset_at, at)`, or `observed` with `remaining == 0`: until the reset time on the wall clock, or an hour after `at` without one. `exhausted` sets `remaining` to 0. An `observed` with `remaining` above 0 ends it. Recognising the answer is T07's share.
- **Secondary (POL-25).** Until `Retry-After` has passed; without one 60 s, doubling on each secondary answer in a row, at most `idle_interval`. It runs on the monotonic clock (POL-26: "every other duration"); `at_wall` is only for `resume_at`. `answered` ends the run, so the next one starts at 60 s again. Every secondary answer in a row counts towards the doubling, with a `Retry-After` or not: this brief's reading.
- **`resume_at`:** the reset time while exhausted, the lift time as a wall time while secondary, the later when both hold, else `None`.
- **The backoff (POL-19's share here: the times).** `failed` steps 5 s, 10 s, 20 s, … at most 300 s, on the monotonic clock; `next_try_wall` is the same step on the wall time given. `succeeded` ends it. `woke` (POL-22) restarts a running one: the next try is 5 s after `at`, the failure after that steps to 10 s; on an idle one it does nothing. `take_read_through` (POL-16) is true once per step, and again after the next `failed` or `woke`; when no backoff runs it's true and takes nothing. When to call each, the log lines and the poll after a recovery are T21's share; the read-through itself is T25's.
- **POL-26's share:** which clock each of the above goes by. The two-clock types are T04's, the tick T21's.
- Each value (80 %, the hour, 5 s, 10 s, 30 s, 60 s, 5 s to 300 s) is one named constant: all are (proposal).

## Interfaces
- **Consumes:** `config::{Config, QuotaConfig}`, `clock::{Time, Mono}`, `poller::{Tier, QuotaState, Policy}` (declared in `src/poller/mod.rs` by T02: don't move them).
- **Provides:** `poller::limits::{Quota, Backoff}` with exactly the methods of `plan/contracts/poller.md`.

## Out of scope
- Calling these from the loop, the gate, the tick, `daemon.problem`, the quota warnings, the log: T21. Demand under each policy, and the read-through during a backoff: T25.
- Tiers, periods, due times and the window checks that read `Policy`: T09.
- Telling an exhausted or secondary answer from another failure: T07.
- TST-6's figures on a running daemon: T34.

## Acceptance
Unit tests in `src/poller/limits/`, as tables over plain `Time` and `Mono` values, with OPS-2's defaults (600, 500, 30 s, 10 s, 300 s). No clock, no runtime.
- `pol17_sliding_hour`: a cost spent at t counts at t + 3 599 s and not at t + 3 600 s.
- `pol26_clock_back_keeps_the_count`: after spending at t, `cost_last_hour(t - 1 h)` still holds it; entries spent before and after a step back are all summed.
- `pol23_thresholds`: a count of 480 is `Normal`, 481 `Slowed`, 599 `Slowed`, 600 `Capped`; `capped()` is false at 599 and true at 600, and false again once enough cost is an hour old.
- `pol18_reserve`: `remaining` 499 is `Reserve`, 500 isn't; `in_reserve()` follows.
- `pol23_precedence`: capped and in reserve is `Capped`; in reserve and above 80 % is `Reserve`; exhausted and capped is `Exhausted`; secondary and exhausted is `Secondary`. Under a higher state `capped()` and `in_reserve()` still answer their own condition.
- `pol23_policy_table`: every row of the table above, for both tiers: normal active 30 s and 10 s; normal idle 300 s and 10 s; slowed active 60 s and 20 s; slowed idle 600 s and 20 s; reserve 300 s and 300 s with 30 s windows and no follow-up; capped no polls, `demand` true; exhausted and secondary `demand` false.
- `pol25_exhausted`: with a reset 20 minutes ahead, `Exhausted` one second before it and not at it; `resume_at` is that time. Without a reset: an hour after `at`. A wall time jumped past the reset isn't exhausted (TST-6); it's `Reserve`, by the reading above, until an answer says otherwise. `observed(0, …)` does the same; a later `observed(4000, …)` ends it.
- `pol25_secondary`: `Retry-After` 30 s holds until 30 s later on the monotonic clock, whatever the wall time does, and `resume_at` is `at_wall` + 30 s. Without one, in a row: 60, 120, 240, 300, 300 s. After `answered`, 60 s again.
- `pol19_backoff_steps`: eight failures in a row give 5, 10, 20, 40, 80, 160, 300, 300 s; `next_try` and `next_try_wall` are `at` plus the step; `succeeded` makes `active()` false and both `None`.
- `pol22_woke`: on a backoff at 80 s, `woke` makes the next try 5 s away and the next failure 10 s; on an idle one, nothing.
- `pol16_one_read_through_per_step`: true, then false, then true after the next `failed`, and after a `woke`.
- `cargo test --features test-endpoint --lib poller::limits::` and `make verify` pass.

## Traps
- **Don't prune the hour's entries from the front as if they were in time order.** After the wall clock steps back, later entries carry earlier times. Sum by age; drop only what's truly an hour old.
- **Compare percentages in integers** (`count * 100 > max * 80`): 480 of 600 isn't slowed.
- **Two clocks in one struct.** The hour and the reset are `Time`; the secondary limit and the backoff are `Mono`. Never turn one into the other; the `_wall` values exist only to be shown (API-4's `until`).
- The tier never changes with the quota state: `policy` takes the tier, it doesn't return one.
- `state` and `policy` take `&self`: they must not record anything, the engine calls them often.
- GitHub's counters lag (docs/02 §2): `remaining` may rise between two answers with no reset. Take each reading as it comes.

## Open questions
- **Every rule here is a (proposal)** (POL-17, POL-18, POL-23, POL-25, POL-26): built as written.
- **The exhausted and secondary answers are `assumed`** (docs/02 §2, §6 item 6): nothing here depends on their shape, only on T07's classification.
- **Readings this brief makes, to report if they don't fit:** a low `remaining` stands until the next answer, so the daemon is in reserve after a reset until one arrives; slowed doubles the idle tier's interval too; every secondary answer in a row doubles.

## Evidence
- **2026-11-26**, branch `t08-poller-limits` (commit 26d1a87), executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- One file changed, no public signature changed; every test the Acceptance names exists, with the brief's values. `cargo test --features test-endpoint --lib poller::limits::`: 12 passed. `make verify` passes in the worktree and on main after the merge (108 lib tests), the macOS check included.
- Choices: `spent` drops only entries an hour old by the new entry's time (`Time::since`, so entries ahead after a step back are kept); a `Retry-After` is used as given, uncapped. The brief's three readings are built as written.
