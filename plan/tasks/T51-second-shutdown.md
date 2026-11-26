---
id: T51
title: "Follow-up to T28: a second shutdown request during a stop ends it at once"
kind: code
milestone: M2
size: S
status: done
depends_on: [T28]
requirements: [OPS-7]
owns: [src/daemon/run/]
shares: []
---

# T51 · Follow-up to T28: a second shutdown request during a stop ends it at once

## Outcome
A second stop request of any kind during OPS-7's stop sequence ends it at once, as a second signal already does: a `POST /v1/shutdown` too, also on a connection that was open when the stop began.

## Context
T28 built the stop sequence in `src/daemon/run/life.rs`. A second request is noticed through a `Notify` that only `Running::stop` and the signal loop fire when `StopHandle::times() >= 2`. T22's `shutdown` route calls `StopHandle::request` directly, so a second `POST /v1/shutdown` during the stop isn't seen, and the sequence runs on to its end or its 5 s. `StopHandle` is T02's (`src/daemon/mod.rs`, final): it has `request`, `times` and `wait`, and nothing that wakes on a second request.

## Read first
- `plan/conventions.md`
- `plan/contracts/daemon.md`: `StopHandle`, "Semantics" (one way to stop; the stop's order).
- `specs/06-operations.md#the-daemons-life`: OPS-7 ("a second request during it ends it at once").
- `src/daemon/run/life.rs` and `tests/m2_serve.rs` as T28 left them; T28's Evidence.

## Scope
- While the stop sequence runs, notice `stop.times() >= 2` from any caller: race the sequence against a loop that looks at `times()` every 100 ms of the clock (`clock::sleep`, one named constant), besides T28's `Notify`. On a second request: the same path as T28's second signal (one `warn stop deadline cause=second_request` line, everything dropped, status 0).
- The loop takes no `Busy` across its sleep, and ends with the sequence.
- No change outside `src/daemon/run/`: `StopHandle` stays as it is.

## Acceptance
- A test in `src/daemon/run/tests.rs` or `tests/m2_serve.rs` (T28's file is under `src/daemon/run/`'s owner only for its own tests: add the test in `src/daemon/run/tests.rs`): with a `stop_delay` that holds the sequence, a second `StopHandle::request(StopCause::Request)` from outside `Running` (as T22's route makes it) ends the stop within 100 ms of the clock, with the `second_request` line, and the status is 0.
- `cargo test --features test-endpoint --lib daemon::run::`, `cargo test --features test-endpoint --test m2_serve` and `make verify` pass.

## Traps
- **Internal callers ask once** (T28's brief): the idle loop and the minute's check must not count as a second request.
- Every wait is `clock::sleep`; settle on the test clock, never real time.

## Open questions
- None: decided by the orchestrator (owner's rule of 2026-11-26), recorded in the plan's change log.

Follow-ups: T62

## Evidence
- **2026-11-26**, branch `t51-second-shutdown`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/daemon/run/life.rs` (`second_request()`, a `select!` arm in `Life::run`, `SECOND_LOOK` 100 ms; no `Busy` across its sleep) and `tests.rs` (`ops7_second_request_from_the_handle`: a `StopHandle::request` from outside `Running` during a 10 s `stop_delay` ends the stop within 100 ms of the clock, status 0, the `second_request` line). `cargo test --features test-endpoint --lib daemon::run::`: 7 passed. `make verify` passes on main after the merge, `m2_serve` included, the macOS check included.
