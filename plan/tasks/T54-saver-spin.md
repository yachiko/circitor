---
id: T54
title: "Follow-up to T16: the saver spins on a moving clock and never writes"
kind: code
milestone: M4
size: S
status: todo
depends_on: [T16]
requirements: [STA-7, OPS-17]
owns: [src/state/file/]
shares: []
---

# T54 · Follow-up to T16: the saver spins on a moving clock and never writes

## Outcome
The state-file saver writes a changed content at the first chance and then sleeps until its next due time, on the real clock as on the test clock: no busy loop, and `state.json` written while the daemon runs (STA-7).

## Context
T36 found it with real processes (gdb: 33 107 turns of the loop in 7 s, no write; one worker at 100 % CPU; `ops17_no_cpu_between_polls` ignored, and the 50 ms `list` figure failing about one run in five under the load). In `src/state/file/saver.rs`, while `last_attempt` is `None`, `Task::due()` returns `self.clock.mono()`. `Task::run` tests `self.clock.mono() >= self.due()`: on a clock that moves between the two reads, the left side is read first and is smaller, so the write is never made; then `sleep_until(self.due())` is ready at once, and the loop turns again. The test clock hides it: its time doesn't move between two reads.

## Read first
- `plan/conventions.md`
- `plan/contracts/state.md`: the saver's lines under "Semantics"; `plan/contracts/clock.md`: `Clock`, `Mono`.
- `specs/03-state.md`: STA-7. `specs/06-operations.md`: OPS-17's CPU figure.
- `src/state/file/saver.rs` and `tests.rs` as T16 left them; T16's and T36's Evidence.

## Scope
- **No due time is "now".** With no attempt yet, the content is due at once: `due()` gives a time that is never after any reading of the clock (`Mono::ZERO`), so the comparison holds and the write is made. Keep the 5 s spacing and the 60 s retry as they are.
- **Read the clock once per turn** where a decision and a sleep depend on it, so a moving clock can't make them disagree.
- Look for the same pattern elsewhere in `src/state/file/` and fix it there too; report any you see outside it (don't change other tasks' files).

## Acceptance
- `sta7_writes_on_a_moving_clock` in `src/state/file/tests.rs`: a `Clock` of the test's own whose `mono()` moves forward 1 ms at every read (wrapping `TestClock` or standing alone), a saver with a pending change: the file is written at once, and the saver's loop turns a bounded number of times (count `StateFs` calls or loop wakes: at most a handful, not thousands) before it parks.
- T16's tests still pass unchanged.
- `cargo test --features test-endpoint --lib state::file::` and `make verify` pass.
- With T36's real-process tests merged by then, the orchestrator re-runs `ops17_no_cpu_between_polls` (un-ignored) and the suite five times.

## Traps
- Don't hide the bug by adding a minimum sleep: the busy loop must be impossible, not slowed.
- The saver still takes `activity.begin()` on waking and drops it before every wait (TST-4).

## Open questions
- None: a follow-up fix, decided by the orchestrator (owner's rule of 2026-11-26).
