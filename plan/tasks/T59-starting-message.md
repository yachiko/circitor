---
id: T59
title: "Follow-up to T22: the starting error gives one retry hint"
kind: code
milestone: M5
size: S
status: done
depends_on: [T22, T35]
requirements: [API-18, CLI-5]
owns: [src/api/server/]
shares: [tests/golden/cli/error_starting.err]
---

# T59 · Follow-up to T22: the starting error gives one retry hint

## Outcome
`circitor list` answered `starting` prints one line with one hint when to try again: `starting: the first poll hasn't finished after 10 s (try again in a minute)`.

## Context
T35's `tests/golden/cli/error_starting.err` reads `starting: the first poll hasn't finished after 10 s; try again in a few seconds (try again in a minute)`: T22's message (`src/api/server/mod.rs`) ends in its own hint, and T10's `render::api_error` adds CLI-5's hint for the codes that exit 9. The hint is the CLI's (CLI-5: "it ends with when to try again"); the message says what's wrong (API-3).

## Read first
- `plan/conventions.md`; `specs/04-api.md`: API-3 (the `starting` row), API-18; `specs/05-cli.md`: CLI-5.
- `src/api/server/mod.rs` (the `starting` message) and its tests; `tests/golden_cli.rs`'s test that writes `error_starting.err`.

## Scope
- The `starting` message becomes `the first poll hasn't finished after 10 s` (the number from the existing constant), with no hint of its own. Update any test in `src/api/server/` that asserts the old text.
- **`shares`:** rewrite `tests/golden/cli/error_starting.err` only, by running `CIRCITOR_UPDATE_GOLDEN=1 cargo test --features test-endpoint --test golden_cli` and keeping **only** that file's change (`git diff --stat` must show no other golden file), and read the new line.
- Look for the same doubled hint in the other `starting`, `github_unavailable` and `rate_limited` messages and report any; change only `starting`'s unless the same fault is plain.

## Acceptance
- `tests/golden/cli/error_starting.err` reads `starting: the first poll hasn't finished after 10 s (try again in a minute)`.
- `cargo test --features test-endpoint --lib api::server::`, `--test golden_cli` and `make verify` pass.

## Open questions
- None: decided by the orchestrator (owner's rule of 2026-11-26).

## Evidence
- **2026-11-26**, branch `t59-starting-message`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/api/server/mod.rs`: the `starting` message is "the first poll hasn't finished after 10 s", and, on the orchestrator's follow-up, the `no_record` `github_unavailable` message lost its "; try again" too (the same doubled hint). `tests/golden/cli/error_starting.err` regenerated alone: `starting: the first poll hasn't finished after 10 s (try again in a minute)`. No route message holds a retry hint now. The executor's second report didn't arrive; the orchestrator checked the worktree's diff itself. `make verify` passes on main after the merge, first try.
