---
id: T43
title: "The API's shared parts and the simple fakes"
kind: infra
milestone: M1
size: M
status: done
depends_on: [T02]
requirements: [API-1, CLI-13]
owns: [src/api/request.rs, src/api/params.rs, src/api/respond.rs, src/api/router.rs, src/github/fake.rs, src/token/fake.rs, src/poller/fake.rs, src/cli/fake.rs]
shares: []
---

# T43 · The API's shared parts and the simple fakes

## Outcome
The parts of the API both halves of the server and the CLI share are real: `Request` spells every route, `Params` is the strict parser, `respond` builds the envelope, the error body and API-8's event window once, and `router` has its layer and its `unknown_route` fallback. The simple fakes (`ScriptedGitHub`, `FakeTokenSource`, `FakePoller`, `FakeSpawner`, `FakeGit`) work behind their traits. Tasks from the fourth wave on test against these.

## Context
T02 left the four API files and the four `fake.rs` files as declarations with `unimplemented!("T43")`; the types (`api::types`, `Until`'s words), `AppState`, `NoFlush` and `Activity` with `excuse` are T02's and work. This task builds what both halves of the server share and what the fourth wave tests with. **T22 and T23 use every part of it on every route**, and can't change it: a fault they find comes back to you as a follow-up. T15, T13, T17 and every later test of the engine, the routes and the CLI drive your fakes.

## Read first
- `plan/conventions.md`: "Code rules" (unknown input, JSON names, errors), "Work in flight", "Tests".
- `plan/contracts/api.md`, all of it: the declarations of `router.rs`, `params.rs` and `respond.rs`, `Request`'s methods, the Semantics, **"What T43 can and can't test"** and "The fake".
- `plan/contracts/github.md`: the trait, `Query`, `Answer`, `Failure`, and "The fakes (T43)". `plan/contracts/poller.md`: "Outside" and "The fake (T43)". `plan/contracts/cli.md`: `Spawner`, `Spawned`, `Git`, `Branch`, and "The fakes".
- `plan/contracts/clock.md`: "`Activity`: the rule" and "who counts what" (the fake's hold is excused). `plan/contracts/state.md`: `Cursor`, `View`, `Store::new` and `StoreDeps` (for `test_state`), `NoFlush`.
- `specs/04-api.md#basics` (API-1 to API-4) and each route's path and parameters: API-6 to API-10, API-15 to API-17. `specs/04-api.md#reading`: API-8, which `window` builds. `specs/08-notify.md#subscriptions`: NTF-2 (the body's keys and their types) and NTF-3 (the cancel's `socket`).
- `specs/03-state.md#cursors`: STA-9 (too old) and STA-10 (the history). `specs/05-cli.md#for-tests`: CLI-13.
- `src/api/`, `src/github/fake.rs`, `src/token/fake.rs`, `src/poller/fake.rs`, `src/cli/fake.rs` as T02 left them.

## Scope
- **`request.rs`: `Request::method`, `path_and_query` and `body`** (the `impl` block for T02's enum). Every route of 04-api and the two of 08-notify, spelt once. Parameters in the order of the variant's fields, an `Ask`'s as api.md lists them (`until`, `sha`, `rerun_since`, `fail_fast`, `kind`, `since`); a flag only when set, as `1`; an `Option` only when `Some`. Every path segment and value percent-encoded outside `[A-Za-z0-9._~-]`. `Track`'s body is `{"pr":"owner/repo#12"}`. **`Notify`'s body is one JSON object**, built with `serde_json`: `until`, the `Ask`'s other words only as set (`fail_fast` as `true`, `kind` as its one string), `for` and `limit` only when `Some`, `socket`, and `token` as a string or `null` (NTF-2). `NotifyCancel` is a `DELETE` with `socket` in the query (NTF-3). Every other body is `None`.
- **`params.rs` (API-1: this task's share is the strict parser, with `pr_from_path`, and the `unknown_route` fallback in `router.rs`; T22 and T23 call them on every route and write the route-specific messages).** `Params` as declared: split on `&` and the first `=`, then percent-decode; a bad escape or invalid UTF-8, a name given twice, a flag other than `1` or `0`, a number that isn't one or is out of `min..=max` (never clamped), a cursor not in STA-9's form (through T02's `Cursor::from_str`), and anything left at `finish`, are each `bad_request` **naming the parameter**. `pr_from_path` refuses a number that isn't a positive integer, naming it.
- **`respond.rs`.**
  - `daemon_block` (API-4, from `PollerStatus.problem`); `refusal` (each `RefusalCode` to the `ErrorCode` of the same name, message and `until` kept).
  - `ok`, `ok_at`, `envelope` and `error` (API-2, API-3, API-4): `version`, `cursor`, `daemon`, then the body's keys, compact on one line; an error is `ErrorBody` with `ErrorCode::http()`'s status. **Build them over private inner functions that take the version, the daemon block and the cursor**, so they're tested without an `AppState`.
  - `present`: `stale` from `poller::cadence::stale`, `checks.items` kept or left out.
- **`window` (API-8, once for T22 and T23).** Through `View::cursor`, `too_old` and `events_after`, which T02 built (pure over the view's public fields), so STA-9's rule exists once. Without `since`: no events. A cursor from another epoch gets every event of this one when `events` starts at revision 1 (or `rev` is 0), else none; any other too-old cursor gets none; both are `reset`. `pr` filters by `Event.id`. `limit` cuts, sets `more_events`, and the cursor becomes the last returned event's.
- **`router.rs`.**
  - `router(state)`: `server::routes()` merged with `wait::routes()` and `notify::routes()`, **both fallbacks** (an unknown path and a known path with another method answer `unknown_route` through `error`: no bare 404 or 405), the layer of api.md's Semantics (`activity.begin()` into an `InFlight` in the request's extensions, `sleep.check()`; never `client_request`), then the state.
  - `InFlight`: `park` drops its `Busy`, `resume` takes one again, each any number of times, a repeat doing nothing.
  - **`test_state(clock)`** (test builds only): `AppState` over a real empty `Store` (fixed seed, `FileContent` empty), `FakePoller`, `NoFlush`, `notify::NoNotify` (T02's; T47 puts a real notifier in its place), OPS-2's default `Config`, a `BufferSink` logger, `TEST_VERSION`, mode `spawned`; its `activity` is the test clock's own (`TestClock::activity`), so `InFlight` is counted where `settle` looks. It can't run before T04 and T15: it only compiles here.
- **The fakes (CLI-13: this task's share is the fakes, so a command and the daemon can be driven without the process's `exec`, `git`, token or GitHub; `CliEnv` and the traits are T02's).** Each as its contract's fake section says, with those signatures, under `cfg(any(test, feature = "test-endpoint"))`:
  - `github::fake::ScriptedGitHub` with `calls` and `hold`, and `github::fake::answers` (`pr_read`, `answer_with`, a `Rate`): plain values, fixed times from 2026-11-24T22:00:00Z, the node ID made from the name so two calls agree.
  - `token::fake::FakeTokenSource`; `poller::fake::FakePoller` with its declared defaults and a counter per call; `cli::fake::FakeSpawner` in its three forms with `spawns`; `cli::fake::FakeGit`.

## Interfaces
- Consumes: `api::types`, `api::{AppState, DaemonInfo}`, `notify::NoNotify`, `state::{Cursor, View, Store, StoreDeps, FileContent, NoFlush}`, `poller::{PollerHandle, PollerStatus, Refusal, FirstPoll}`, `clock::{Activity, Busy, Excused, SharedClock, TestClock}`, `github` and `token` traits and types, `cli::{Spawner, Spawned, Git, Branch}`, `testkit::{MARKER_TOKEN, TEST_VERSION}` (T02).
- Provides: everything api.md declares in `request.rs`, `params.rs`, `respond.rs` and `router.rs`, `api::test_state`, and the five fakes, unchanged in name and signature. Private helpers are yours; a new public item goes through the orchestrator.

## Out of scope
- `api::types`, `Until::as_str` and `parse`, `ErrorCode::http`, `AppState`, `NoFlush`, `Activity`: T02. The golden files: T41.
- The routes, their messages and their tests, and **testing `router`'s layer, its fallbacks and `present` through `router`**: T22. The `wait` route, its two-step answer and `circitor-cursor`: T23.
- `Store`: T15. `TestClock`, `SleepWatch`, `Logger`: T04. `cadence::stale`: T09 (T02's stub returns `false`).
- The fake GitHub server: T14, T20. CLI tests' stub daemon: T24. `FaultFs`: T16. The harness: T29. Sending a `Request` over the socket: T17.

## Acceptance
Unit tests beside the code. Where one needs a clock, a ten-line `Clock` inside the test module is enough (T04's isn't merged).
- `api1_params`: an unknown parameter is `bad_request` naming it; a name given twice; `checks=1`, `checks=0` and absent give `true`, `false`, `false`, and `checks=true` and `checks=` are refused; `timeout=3601` with a range of 0 to 3 600 is refused, not clamped, and `timeout=3600` passes; `timeout=abc`; `since=k3x9a2bq` (no `-`); `since=%zz`; `a+b` decodes to `a+b`; `finish` names the one left.
- `api1_request_spells_every_route`: for each `Request` variant, the method, the exact path and query, and the body; `List { all: true, since: None, checks: true }` is `GET /v1/prs?state=all&checks=1`; `Track`'s body is `{"pr":"owner/repo#12"}`; a `since` holding a space and a `&` survives `path_and_query` then `Params::parse` unchanged. A `Wait` with `fail_fast` and one with `kind: "activity,merged"` carry `fail_fast=1` and `kind=activity%2Cmerged`, and neither key when unset.
- `ntf2_notify_request`: `Notify` is `POST /v1/prs/o/r/12/notify` with no query; its body parses as JSON with `until`, `socket` and `token` always there (`token` `null` without one), `fail_fast: true` only when set, `for` and `limit` only when `Some`, and a path holding a quote survives. `NotifyCancel` is `DELETE /v1/prs/o/r/12/notify?socket=%2Frun%2Fuser%2F1000%2Fcc-socks%2F1.sock`, with no body.
- `api1_pr_from_path`: `o`, `r`, `12` give `o/r#12`; `x`, `-1`, `0` and an empty number are `bad_request` naming the number.
- `api3_error_body` (inner function): each `ErrorCode` gives its status and `{"version":…,"error":{"code":…,"message":…,"until":null}}`. `api3_refusal_codes`: five refusals, five codes, message and `until` kept.
- `api2_api4_envelope` (inner function): keys in the order `version`, `cursor`, `daemon`, then the body's; no newline inside; `daemon_block` with a `rate_limited` problem carries its `message` and `until`, and without one has all three `null`.
- `api3_unknown_route_for_a_path_and_a_method`: the private helper that adds both fallbacks, over a stand-in `Router` with one `GET /v1/x` route, answers `GET /v1/y` and `POST /v1/x` with 404 and the `unknown_route` body, through `tower::ServiceExt::oneshot`.
- `api8_window`, over hand-built `View`s with events 1 to 5 of two PRs (cursors written by their `rev`): no `since` gives no events and the view's cursor; `-2` gives 3, 4, 5 and the view's cursor; `-2` with `limit=2` gives 3, 4, `more_events`, cursor `-4`; `-5` gives none, not reset; `-6` is reset with none; with the history at 3 to 5, `-1` is reset with none and `-2` isn't; another epoch gives all five and reset, and none with the history at 3 to 5; `pr` keeps one PR's events, matched by `id` after a rename.
- `in_flight_parks_and_resumes_many_times`: against an `Activity`, 1 at the start; `park`, `park` give 0; `resume`, `resume` give 1; dropped, 0.
- One test per fake, through its trait:
  - `scripted_github_answers_and_holds`: queries recorded in order, answers from the handler; under `hold`, with a `Busy` held beside it, the next `send` stays pending and `in_flight()` reads 0 until the `Release` is dropped, then 1; the send after it isn't held; a held `send` dropped early leaves nothing excused. `answers::pr_read("o/r#1")` twice gives equal values.
  - `fake_token_source`: `MARKER_TOKEN` by default, the set read next, reads counted; with a delay, pending until its clock's timer fires.
  - `fake_poller_defaults_and_counts`: its declared defaults; each closure runs; each call counted.
  - `fake_spawner_three_forms`: `exits(78, Some("bad config"))` appends that line to the file it's given and reads exited with 78; `never_binds` never exits; `starts` runs its closure; `spawns` counts all three. `fake_git`: answers as built, whatever the directory.
- `cargo test --features test-endpoint --lib api::`, `… --lib github::fake`, `… --lib token::fake`, `… --lib poller::fake`, `… --lib cli::fake` and `make verify` pass. `cargo build --release` passes without the feature. `grep -rn 'unimplemented!("T43")' src` prints nothing.

## Traps
- **axum's `fallback` doesn't catch a known path with another method**: that route answers a bare 405. Set `method_not_allowed_fallback` too (check its name in the locked axum 0.8), after the merge.
- **Filter events by `Event.id`, never by `pr`:** a rename changes the name, and `id` is `#[serde(skip)]`, so build test events with it set.
- **`+` is a plus, not a space**: the CLI encodes a space as `%20`. Split before decoding, or an encoded `&` or `=` splits a value.
- **Don't call `Store::new` in a test**: it's T15's stub, and panics. `View`'s methods are T02's and real: a hand-built `View` works. `test_state` compiles here and runs from T22 on.
- `InFlight` sits in the request's extensions: it must be `Clone + Send + Sync`, with a `std::sync::Mutex<Option<Busy>>` inside, never held across an `.await`.
- **The hold's `Excused` lives inside the `send` future**, so a dropped request stops excusing. `FakeTokenSource`'s delay waits on its clock and holds no `Busy` (clock.md).
- `make verify` lints without the feature too: a helper used only by a fake needs the fake's `cfg`. Closures a fake stores are `Send + 'static` (and `Sync` where cli.md says).
- No `unwrap` outside tests: serialising these types can't fail; say so in one line where you rely on it.

## Open questions
- **The contracts are silent on:** `FakePoller`'s `read_through`, `track` and `poll_now` without a closure; `FakeTokenSource`'s `kind()` and `gh_path()`; `test_state`'s clock type; whether a body ends with a newline. Build the smallest thing that fits (a `github_unavailable` refusal, `Polled` at cost 0, `Gh` and `gh`, `Arc<TestClock>`, no trailing newline) and list each under "Contract changes needed".
- **Don't re-derive STA-9 in `window`:** call `View::too_old`. A hand-built `View` in your tests works because its methods read only its fields.
- STA-10's 500 is (proposal): `window` holds no constant of its own; the caller passes `limit`.

## Evidence
- **2026-11-26**, branch `t43-api-shared-fakes` (commit 3b02929), executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- Only the eight owned files changed; no public signature changed (`InFlight`'s private field only). `grep -rn 'unimplemented!("T43")' src` prints nothing. Every test the Acceptance names exists. No `gh`, network or session-socket use.
- After rebasing onto T05: `make verify` passes in the worktree and on main (96 lib tests, 9 golden), the macOS check included; `cargo build --release` passes.
- Choices where the contracts are silent (accepted): `FakePoller::idle_since` is `Mono::ZERO`; `read_through` and `track` without a closure refuse `github_unavailable`, `poll_now` is `Polled` at cost 0; `FakeTokenSource` defaults to `Gh` and `gh`, and reads the set value when its delay ends; `test_state` takes `Arc<TestClock>`, logger `Info` over a `BufferSink`; bodies have no trailing newline; the unknown-route message is `no such route`; `ScriptedGitHub` records a query before the hold; `Params::finish` names every parameter left; `Params::parse` skips empty segments and reads a bare name as `""`; numbers are digits only (`+5`, `-1` refused). The notify body is written key by key to keep NTF-2's order.
- Not run here: `api::test_state`, `present`, the layer and the fallbacks through the real `router`: T22's to test, once T04 and T15 are in.
