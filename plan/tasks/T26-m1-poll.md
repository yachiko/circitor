---
id: T26
title: "M1: the real client against the fake GitHub, polling end to end"
kind: integration
milestone: M1
size: M
status: in-progress
depends_on: [T07, T20, T21]
requirements: [POL-4, POL-6, POL-13, TST-12]
owns: [tests/m1_poll.rs]
shares: []
---

# T26 · M1: the real client against the fake GitHub, polling end to end

## Outcome
In one process on the test clock, the engine polls the fake GitHub through the real client: PRs are found and tracked with the right status and revisions, a push and a merge are seen as the specs say, and an idle hour's points stay inside the budget. The first proof that T07's queries and decoding agree with a schema-checked server.

## Context
Milestone M1's checkpoint. Until now the client (T07) was tested on hand-written fixtures and T41's schema, the fake GitHub (T14, T20) with hand-written queries, and the engine (T21) on `ScriptedGitHub`. Here the three meet for the first time: the fake shares no type with the client, so a wrong field name, alias or path on either side fails here. This task writes one test file and no production code.

## Read first
- `plan/conventions.md`: "Tests", and the rule for a test that fails because another task's code is wrong.
- `plan/contracts/testkit.md`: what `FakeGitHub` offers, "Semantics". `plan/contracts/poller.md`: `EngineDeps`, `Engine::spawn`, `PollerHandle`. `plan/contracts/state.md`: `StoreDeps`, `Store::new`, `View`. `plan/contracts/clock.md`: `TestClock`, `advance`, `settle`, `SleepWatch`. `plan/contracts/log.md`: `Logger`, `BufferSink`, `Warnings`. `plan/contracts/github.md`: `FakeTokenSource`.
- The rustdoc of `src/testkit/server/` and `src/testkit/github/`: the fake's and the world's real method names.
- `specs/02-polling.md#the-poll` POL-4, POL-6, POL-10; `#cadence` POL-13.
- `specs/01-model.md#checks` MOD-6 (when `settled` turns true: it decides which refresh reads `ready`); `#revisions-and-events` MOD-15.
- `specs/07-testing.md#what-is-tested` TST-6's first line, TST-12.
- `docs/05-measurements.md`, 2026-11-24: PR A's second push (22:13:50 to 22:14:43) and PR B's merge. `docs/02-github-api.md` §3, §4.

## Scope
- **The wiring**, written once as a helper in the file: a `TestClock` at 2026-11-24T22:00:00Z; `FakeGitHub::start(clock)` and its `client()` as the `GitHub`; `FakeTokenSource`; a `Logger` on a `BufferSink`; `Warnings`; `SleepWatch`; the clock's `activity()`; a real `Store` from an empty `FileContent`; `Config::default()`; `Engine::spawn`. No API, no socket, no daemon: T28 adds those.
- **POL-4's share here: the poll on the wire.** T21 proved the engine's steps on a scripted fake. This proves the same poll as GitHub would see it: one request that finds and refreshes, a second straight after for newly found PRs, each valid against the schema and each costed.
- **POL-6's share here:** the record the real query brings back, through the fake's resolvers and T07's decoding, fills every field of the model. T07 has the query's text.
- **POL-13's share here:** the tiers as counted from the fake's request log. T09 has the arithmetic, T21 the engine's side.
- **TST-12's share here:** the shapes T07's fixtures pin are the shapes the fake returns: every answer in these tests decodes. A mismatch between T07 and T14 is reported with the field and both sides' view, not patched in the test. Recording real responses is T37's.
- **Scripting.** Worlds are built with T14's methods and timed from the measurements. Time moves only by `clock.advance`; `engine.client_request()` stands in for a client where a test needs the active tier.

## Interfaces
- **Consumes:** `testkit::server::FakeGitHub`, `github::client::Client` (through `client()`), `poller::engine::{Engine, EngineDeps}`, `poller::PollerHandle`, `state::store::{Store, StoreDeps}`, `clock::{TestClock, SleepWatch}`, `token::fake::FakeTokenSource`, `log::{Logger, BufferSink}`, `status::Warnings`, `poller::cadence::stale`.
- **Provides:** `tests/m1_poll.rs`. T29 runs after it, but nothing builds on its code; T28 and T29 write their own wiring.

## Out of scope
- Any fix in `src/`. A failure caused by T07, T14, T20 or T21 is an `#[ignore = "Tnn: …"]` on the test and an open item, with the request and the answer that disagreed.
- Hot refreshes, forced refreshes, reading through, the `UNKNOWN` follow-up: T25, first run end to end in the harness (T29). T28 runs beside T25, on T21's stubs. A PR going `gone`: T42.
- Waits, the API, the CLI: T28, T29. TST-8's scenarios: T30 to T33. TST-6's full figures: T34.
- Faults and error classes on the wire (`github-down`, `token-rotated`): T33. Keep to one fault here, as a smoke test.

## Acceptance
`tests/m1_poll.rs`, each test `#[tokio::test]`, each ending with the fake's `check()`.
- `m1_found_and_tracked`: a world with two open PRs of the login. `o/r#1` has two passed checks and an `updatedAt` 2 minutes old; `o/r#2` was created 5 s ago with no checks. After the first poll: both are tracked `authored`; `o/r#1` reads `ready` with `checks.settled` true (MOD-6's shortcut, by the `Date` header), `o/r#2` `checks_pending`; the history holds two `tracked` events with revisions 1 and 2 in `pr` order, and the view's cursor is at 2. The fake's log shows two requests: one holding the finding and no node, then one asking both PRs for the record and the items, sent at least 1 s later, costing 1 point each (POL-4). `first_poll()` reads `Finished`.
- `m1_record_fields`: one PR scripted with every field set (a draft, a base other than `main`, a review decision, one comment, one review with a thread, a check run and a commit status). The stored record has each of MOD-2's fields as scripted, lowered; `checks.items` has the run with its `id` and the status without one (POL-6, MOD-3).
- `m1_push`: PR A's second push, with the active tier kept by `client_request`. The world serves the old head (a 40-character SHA starting `dce9dbe`) with its `SUCCESS` for 3 s after the push, then the new head (`cf54ec4…`) with `mergeable` `UNKNOWN` and a `null` rollup, `MERGEABLE` and `PENDING` from +6 s, both checks passed at +52 s. Over the next two minutes the PR's events, in order, hold: a `head` kind from `dce9dbe` to `cf54ec4` with a `status` kind to `checks_pending`; a `checks` kind with state `pending`; a `checks` kind with state `passing`. No event has `to: ready` before the one with `passing`, and the record ends `ready`. A poll that still read the old head made no event.
- `m1_merge`: PR B merges between two polls. The next poll's finding no longer lists it, the same poll refreshes it by ID, and it reads `merged` with `mergeable` `unknown`: one event with the kinds `status` and `merged`, and no `mergeable` kind (POL-10, MOD-8). It stays tracked with `source: authored`, not demoted for leaving the list (POL-5, `contracts/state.md`'s `demote_unlisted`), and `store.file_content()` holds it as `authored`.
- `m1_tiers`: with 10 tracked PRs and nothing changing, the log's poll requests are 30 s apart while `client_request` comes every 5 minutes, and 300 s apart from 10 minutes after the last one (POL-13). Each is one request.
- `m1_hour_in_budget`: the same world, counted from `points()` after the first poll has settled. An idle hour costs at most 15 points with at least 11 polls; an active hour at most 130 with at least 110 (TST-6's first line, as a smoke test).
- `m1_one_bad_answer`: a 500 for one request: the records turn stale by `cadence::stale`, the next try comes 5 s later, and the poll after it clears them (POL-19).
- `m1_rules_held`: across every test, the fake recorded no mutation, no overlap, no gap under 1 s and no query the schema refuses (POL-1, POL-2): that's what `check()` asserts.
- `cargo test --features test-endpoint --test m1_poll` and `make verify` pass. List every ignored test in the report, with its task.

## Traps
- **Never sleep or poll for a state.** `clock.advance(d)` returns when the engine and the fake are quiet; then look. A test that needs real time is wrong.
- **If `advance` panics after 10 s naming work in flight, or a request shows up at the wrong fake time, that's the settle seam** between T04, T20 and T21 (`contracts/clock.md`, "`Activity`: who counts what"), not this test. Ignore the test with the task ID and report what was in flight.
- **Short socket-free setup, but real TCP:** the fake listens on the loopback. Nothing else may be contacted (TST-1).
- The logs give 7-character SHAs; a record's `head_sha` is 40. Script full SHAs that start with the measured ones, and compare events by their short form.
- The engine starts in the idle tier: without `client_request` the second poll is 300 s away.
- `ready` needs `settled`: a list with checks settles at a refresh sent 20 s after the settle start, or at once when `updatedAt` is over 25 s old by GitHub's clock (MOD-6). Choose `advance` steps so the asserted refresh is past it, and don't assert an event's full kind list where a poll's timing decides it.
- An hour of polls counted from the log includes each poll's extra requests: start counting after the first poll, whose first refreshes cost a second request.
- **Every later poll is one request** (plan decision P10: all a poll's requests mark their PRs as sent at the poll's send time). `m1_tiers` and `m1_hour_in_budget` hold by it, with no tolerance. A steady second request a poll is T21's: ignore the test with T21 and report it.
- Tests under `tests/` see only the crate's public items, and only in a test build: the file starts with `#![cfg(feature = "test-endpoint")]`.

## Open questions
- **POL-4 and POL-13 are (proposal); TST-12 is too**, and no recorded fixture exists yet (T37). Build against the synthetic ones and the fake.
- **The cost formula is `documented`, its fit `observed`** (docs/02 §2): the budget figures rest on the fake's costing (T14).
- **This is the first test of the settle seam with real I/O** (the engine's `Busy` across a request, the fake's probe and its excuse). Report what you see, flaky or not: run the file ten times before you report.
- If the fake's world can't script one of these timelines without touching JSON, that's a gap in T14's builders: report it, and don't post-process answers in the test.
