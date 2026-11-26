---
id: T55
title: "Follow-up to T09: a record marked slept is due at the next poll (D14)"
kind: code
milestone: M1
size: S
status: done
depends_on: [T52]
requirements: [POL-22]
owns: [src/poller/cadence/]
shares: []
---

# T55 · Follow-up to T09: a record marked slept is due at the next poll (D14)

## Outcome
`cadence::in_poll` puts a PR whose `timing.slept` is set in the next poll, whatever its period, so the poll POL-22 starts on waking refreshes the records the sleep made stale.

## Context
T31 found that after `sleep_wall_only` the poll that starts on waking sends only the finding: on the monotonic clock no PR is due yet (POL-13, POL-26), so every record stays stale for one more period. **D14** (POL-22, `specs/decisions.md`) settles it. The store sets `Timing::slept` on a sleep (`store.mark_slept`, T21 calls it) and clears it with an applied refresh (T15). T52 did the same for `attempt_failed` (D12), in the same function: follow its pattern.

## Read first
- `plan/conventions.md`; `plan/contracts/poller.md` ("Inside: cadence"); `plan/contracts/state.md` (`Timing`).
- `specs/02-polling.md`: POL-13, POL-22 (D14's words); `specs/decisions.md`: D12, D14.
- `src/poller/cadence/` as T52 left it.

## Scope
- In `in_poll`: a PR with `timing.slept` set is due, as one with `attempt_failed` is (D12), a `First` PR excepted. `// POL-22, D14:` at the rule.

## Acceptance
- `d14_a_slept_pr_is_due_at_the_next_poll` in `src/poller/cadence/tests.rs`: an open PR sent 1 s ago with `slept` is in; without it, out; a merged one with it, in; a `First` one with it, out.
- T09's and T52's tests pass unchanged. `cargo test --features test-endpoint --lib poller::cadence::`, `cargo test --features test-endpoint --lib poller::engine::`, `--test scn_waits` and `make verify` pass. If a test of another task's now fails because it counted the old behaviour, report it with its name and what it asserted: don't change it.

## Open questions
- None: decided by the orchestrator (owner's rule of 2026-11-26).

## Evidence
- **2026-11-26**, branch `t55-due-after-sleep`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/poller/cadence/mod.rs` (`// POL-22, D14:` in `in_poll`) and `tests.rs` (`d14_a_slept_pr_is_due_at_the_next_poll`). `cargo test --features test-endpoint --lib poller::`: 111 passed; `--test scn_waits`: 24 passed; no other task's test counted the old behaviour. `make verify` passes on main after the merge, at a load of about 8.
- **Noted:** the executor's own `make verify` once stopped on `ops17_list_from_memory_within_50_ms` (median 62.9 ms) under a load of about 8.5 from other worktrees, and passed when the load dropped. OPS-17's figures need a quiet machine (as T36 ran alone); the orchestrator keeps them in `make verify` and reruns once when only an `ops17_` test fails under the swarm's load.
