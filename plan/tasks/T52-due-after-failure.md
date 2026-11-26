---
id: T52
title: "Follow-up to T09: a PR whose last refresh failed is due at the next poll (D12)"
kind: code
milestone: M1
size: S
status: in-progress
depends_on: [T09]
requirements: [POL-19]
owns: [src/poller/cadence/]
shares: []
---

# T52 · Follow-up to T09: a PR whose last refresh failed is due at the next poll (D12)

## Outcome
`cadence::in_poll` puts a PR whose last refresh failed in the next poll, whatever its period, so the try that ends a backoff refreshes the records the failure left stale.

## Context
T26 found that after a failed poll the backoff's successful try asks for none of the failed PRs: they were marked sent at the failed poll's send time (P10), so by POL-13 they aren't due for another period, and stay stale meanwhile. **D12** (POL-19, `specs/decisions.md`) settles it: a PR whose last refresh failed is due at the next poll. The store already marks such a PR: `Timing::attempt_failed` is set by `store.attempt_failed` (whole-request failures and node errors alike) and cleared by an applied refresh (T15).

## Read first
- `plan/conventions.md`
- `plan/contracts/poller.md`: "Inside: cadence"; `plan/contracts/state.md`: `Timing`.
- `specs/02-polling.md`: POL-10, POL-13, POL-19 (D12's sentence); `specs/decisions.md`: D12.
- `src/poller/cadence/` as T09 left it, with its tests.

## Scope
- In `in_poll`: a PR with `timing.attempt_failed` set is due, as a `Loaded` or never-sent one is, except a `First` PR, which stays out (its first refresh is the engine's own request). Nothing else changes: `hot_due`, `stale`, `period` and the rest stay as they are.
- A `// D12:` comment at the rule, beside `// POL-19:`.

## Acceptance
- A test `d12_a_failed_pr_is_due_at_the_next_poll` in `src/poller/cadence/tests.rs`: an open PR sent 1 s ago with `attempt_failed` is in `in_poll`; the same without it isn't; a merged one sent 1 s ago with it is in; a `First` one with it isn't.
- T09's tests still pass unchanged.
- `cargo test --features test-endpoint --lib poller::cadence::`, `cargo test --features test-endpoint --test m1_poll` and `make verify` pass. If an engine or M1 test now fails because it counted the old behaviour, report it: don't change it (it's another task's file).

## Traps
- `attempt_failed` is also set by a node's error (POL-21): such a PR is then in every poll until a refresh is applied. That's D12 as decided: it costs no request (one request a poll still holds, P10).

## Open questions
- None: decided by the orchestrator (owner's rule of 2026-11-26).
