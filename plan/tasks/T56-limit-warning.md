---
id: T56
title: "Follow-up to T21: a finding page left unread at the limit raises pr_limit (D15)"
kind: code
milestone: M5
size: S
status: todo
depends_on: [T42, T32]
requirements: [STA-3, POL-7]
owns: [src/poller/engine/]
shares: [tests/scn_tracking.rs]
---

# T56 · Follow-up to T21: a finding page left unread at the limit raises pr_limit (D15)

## Outcome
When the finding stops paging because every place is taken and another page follows, the poll counts that as found PRs left out: `pr_limit` is raised, and cleared by a poll whose finding is read to its end with none left out (STA-3, POL-7, API-5, D15).

## Context
T32's `scn_many_prs_205_warning` (TST-8 `many-prs`, 205 open PRs) is ignored: in `src/poller/engine/poll.rs`, `steps`, the finding stops paging once `places_free() == 0`, so the five oldest are never seen, nothing is left out and `pr_limit` is never raised. **D15** (STA-3, `specs/decisions.md`) keeps stopping the paging and counts the unread page as PRs left out.

## Read first
- `plan/conventions.md`; `plan/contracts/poller.md` ("Inside: the engine").
- `specs/03-state.md`: STA-3 (D15's words); `specs/02-polling.md`: POL-7; `specs/04-api.md`: API-5's `pr_limit` row; `specs/decisions.md`: D15.
- `src/poller/engine/` as T42 left it, its tests, and `tests/scn_tracking.rs`'s `scn_many_prs_205_warning`.

## Scope
- Where the finding stops paging for want of a place while its page has `next`, mark the poll as having left PRs out, as when a found PR gets no place. The rest of `pr_limit`'s raising and clearing is T21's, unchanged: cleared by a poll that reads the finding to its end and leaves none out.
- **`shares`:** remove the `#[ignore]` on `scn_many_prs_205_warning` in `tests/scn_tracking.rs`, and change nothing else in that file.

## Acceptance
- An engine test `d15_an_unread_page_at_the_limit_raises_pr_limit` (in `src/poller/engine/tests/`): with every place taken by tracked PRs and the finding's page 2 not asked, `pr_limit` is raised; once the extra PRs close and a poll reads the finding to its end with none left out, it's cleared.
- `scn_many_prs_205_warning` passes un-ignored; T21's, T25's and T42's engine tests pass unchanged.
- `cargo test --features test-endpoint --lib poller::engine::`, `--test scn_tracking` and `make verify` pass.

## Open questions
- None: decided by the orchestrator (owner's rule of 2026-11-26).
