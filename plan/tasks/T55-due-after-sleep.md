---
id: T55
title: "Follow-up to T09: a record marked slept is due at the next poll (D14)"
kind: code
milestone: M1
size: S
status: in-progress
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
