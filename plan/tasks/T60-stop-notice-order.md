---
id: T60
title: "Follow-up to T49: find a stop's notice by its PR, not its place"
kind: test
milestone: M5
size: S
status: done
depends_on: [T49, T57]
requirements: [TST-8, NTF-10]
owns: [tests/scn_notify.rs]
shares: []
---

# T60 · Follow-up to T49: find a stop's notice by its PR, not its place

## Outcome
TST-8's `notify-stop` row: `scn_notify_stop_ask_again_replacement` asks again for `octo/app#1` whatever order the stop's notices arrive in.

## Context
**The first CI run (T38) failed on this test**, on PR #1's `ci.yml` run of 2026-11-26. It took `notices(&h)[3]` as `octo/app#1`'s `stopped` notice. A stop posts every notice at once (NTF-10, `join_all` in `src/notify/notifier.rs`), so the order the fake session receives them in isn't fixed. On the 4-core runner, `octo/app#3`'s came first, and the test registered `#3`. Locally it passed 30 times in 30. **The code is right:** NTF-10 says "together" and fixes no order.

## Read first
- `plan/conventions.md`; `specs/08-notify.md`: NTF-10. `tests/scn_notify.rs`: `three_registered`, `ask_again` and the `notify-stop` tests.

## Scope
- A helper, `stopped_notice(h, pr)`, finds the notice that says `notify for <pr> is cancelled`, and fails naming the PR if there's none.
- `scn_notify_stop_ask_again_replacement` uses it for `octo/app#1`. `scn_notify_stop_ask_again_no_daemon` keeps `[3]`: any PR's notice serves it.

## Out of scope
- `src/notify/`: the order of a stop's posts stays unfixed, as NTF-10 allows.
- The other tests in `tests/scn_notify.rs`.

## Acceptance
- `cargo test --features test-endpoint --test scn_notify` and `make verify` pass.
- The next CI run on Linux is green on this test.

## Open questions
- None.

## Evidence
- **2026-11-26**, by the orchestrator, on branch `ci-release` (PR #1), on the owner's OK. Failing run: https://github.com/yachiko/circitor/actions/runs/45790109951.
