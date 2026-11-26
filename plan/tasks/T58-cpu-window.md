---
id: T58
title: "Follow-up to T36: OPS-17's CPU window starts after the poll's last request"
kind: test
milestone: M4
size: S
status: done
depends_on: [T36, T56]
requirements: [OPS-17]
owns: [tests/real/]
shares: []
---

# T58 · Follow-up to T36: OPS-17's CPU window starts after the poll's last request

## Outcome
`ops17_no_cpu_between_polls` measures the daemon's CPU over 5 s that really fall between two polls: from after the poll's **last** request has been answered, not its first.

## Context
`tests/real/perf.rs`'s `next_poll` returns once `status`'s `last_poll` moves, which happens when the poll's **first** request is answered (T21: `last_poll_at` is the poll's first send time, set at the first answer). With 200 tracked PRs a poll has more requests after it, one a second (POL-2). Since D15 (T56) the finding reads its second page when every PR on page 1 is tracked, so a request lands inside the window and its decoding costs 8 to 40 ticks: main's `make verify` fails. The orchestrator checked that with T56's `poll.rs` reverted the test passes, and that the daemon doesn't spin: the window is the fault, not the code.

## Read first
- `plan/conventions.md`; `tests/real/perf.rs` and `tests/real/fixture.rs` as T36 left them (the fake GitHub's `requests()` is reachable from the fixture).
- `specs/06-operations.md`: OPS-17 ("between two polls").

## Scope
- Start the window only once the poll is over: after `last_poll` moves, wait until the fake GitHub's request log has had no new request for at least 1.5 s (POL-2's gap is 1 s), then read the ticks, sleep 5 s, read again. Keep the check that no new poll started inside the window, and add one that **no request at all** reached the fake inside it.
- Change nothing else in `tests/real/`.

## Acceptance
- `cargo test --features test-endpoint --test real ops17_no_cpu_between_polls` passes 5 times in a row; the whole `--test real` suite once; `make verify` passes.

## Open questions
- None: decided by the orchestrator (owner's rule of 2026-11-26).

## Evidence
- **2026-11-26**, branch `t58-cpu-window`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `tests/real/perf.rs` only: a `quiet()` helper (no request at the fake for 1.5 s); `ops17_no_cpu_between_polls` measures 5 s after the first poll's last request and asserts no new poll, **no request at all** in the window, and at most 2 ticks. **Deviation, accepted:** with `interval` 10 a 200-PR poll (about 7 requests over 6.5 s) leaves under 4 s before the next, so the test has its own `CONFIG_CPU` with `interval` 20 and measures after the first poll (a second would pass the fixture's 30 s limit). 5 runs of the test and the whole `--test real` (29) passed. `make verify` passes on main after the merge, first try.
