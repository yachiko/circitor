---
id: T56
title: "Follow-up to T21: a finding page left unread at the limit raises pr_limit (D15)"
kind: code
milestone: M5
size: S
status: done
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

## Evidence
- **2026-11-26**, branch `t56-limit-warning`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- **First pass** built D15 as first written (stop at the first page once every place is taken); the executor found a full set of 200 tracked PRs then never clears `pr_limit`, and the orchestrator **revised D15** (stop at the first page holding a found PR with no place). Built in `src/poller/engine/poll.rs` `steps`; `d15_an_unread_page_at_the_limit_raises_pr_limit` (205 PRs: pages 1 and 2 read, page 3's five left out, cleared once they close). T21's `pol15_goes_next` had one assertion on the old rule, changed with a comment; `scn_many_prs_205_warning` (T32's) un-ignored, its clear step waiting 345 s for the idle tier's next poll (the row's 45 s was a slip). `--lib poller::engine::`: 71 passed; `--test scn_tracking`: 42 passed, 0 ignored, twice.
- **After the merge main's `make verify` failed on `ops17_no_cpu_between_polls`** (8 to 40 ticks): the test's window opens at a poll's first answer, and the new second finding page lands inside it. Reverting `poll.rs` made it pass 3 times; follow-up **T58** fixes the window.
