---
id: T62
title: "Follow-up to T28 and T51: an answer waiting for room in the socket is work in flight"
kind: code
milestone: M5
size: S
status: todo
depends_on: [T28, T51]
requirements: [TST-4]
owns: [src/daemon/run/]
shares: []
---

# T62 · Follow-up to T28 and T51: an answer waiting for room in the socket is work in flight


> **Partly done (2026-11-26).** The `Writing` wrapper below is merged and fixes the gap reproduced on Linux. **It wasn't enough on macOS:** the fourth dry run ([run 45797842465](https://github.com/yachiko/circitor/actions/runs/45797842465)) failed the same two tests the same way. Both are ignored on macOS only (`cfg_attr(target_os = "macos", ignore = "T62: …")`), on the owner's fallback. **Left:** find the second gap, best on a Mac (T40's machine) or with a diagnostic CI run, then lift the two ignores.

## Outcome
The test clock doesn't settle while the daemon is still writing an answer that the socket can't take yet. A large answer reaches the CLI before a test looks.

## Context
**The third release dry run (T38) failed two tests on `macos-26`**, on 2026-11-26: `tst6_200_prs_cost_at_most_600_and_end_slowed_or_capped` (`tests/quota.rs`, `list --json` over 200 PRs) and `scn_many_checks_truncated_failing` (`tests/scn_checks.rs`). Each found its command not done after the clock settled.

**Why:** TST-4 counts work in flight. A connection's task held no `Activity` guard, so a write parked on a full socket buffer counted as nothing. The CLI, waiting on its read, isn't woken either, so the test clock saw three clean looks half-way through the answer. **Linux's Unix-socket buffer takes the whole answer in one write; macOS's is a few KB**, so only macOS showed it.

**Reproduced on Linux:** with the daemon's `SO_SNDBUF` set to 2048 for a run, both tests fail without this change and pass with it (a temporary edit, reverted).

## Read first
- `plan/conventions.md`, "Work in flight". `specs/07-testing.md`: TST-4.
- `src/daemon/run/serve.rs`: `Conns::serve`, `accept_loop`.

## Scope
- `Conns::serve` wraps the stream in `Writing`: a write, flush or shutdown that returns `Pending` takes an `Activity` guard, held until the next one returns `Ready`. Reads aren't counted, so a connection waiting for a request or a waiter's answer stays idle.

## Out of scope
- The harness (`src/testkit/harness/`): the CLI side needs nothing, since data that reached its socket wakes it.
- A test that shrinks the socket buffer: it would need a test-only hook in the daemon. macOS's CI run covers it.

## Acceptance
- `make verify` passes on Linux.
- Both tests pass on `release.yml`'s `verify (macos-26)`, with the two macOS ignores lifted.

## Open questions
- **A client that stops reading but keeps its end open** holds the guard, and in a test the clock wouldn't settle. Every client in the tests reads to the end or closes; a closed end fails the write, which drops the guard.

## Evidence
- **2026-11-26**, by the orchestrator, on the owner's OK (option 1). Failing run: https://github.com/yachiko/circitor/actions/runs/45795604691.
- **Fourth dry run, 2026-11-26:** with `Writing`, `tst6_200_prs_cost_at_most_600_and_end_slowed_or_capped` and `scn_many_checks_truncated_failing` still fail on `macos-26` with the same messages. Ignored on macOS; everything else passes there.
