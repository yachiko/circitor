---
id: T22
title: "API routes: list, show, changes, track, untrack, refresh, status, shutdown, and starting up"
kind: code
milestone: M2
size: L
status: done
depends_on: [T09, T15, T41]
requirements: [API-1, API-2, API-3, API-4, API-5, API-6, API-7, API-8, API-15, API-16, API-17, API-18, NTF-9]
owns: [src/api/server/]
shares: []
---

# T22 · API routes: list, show, changes, track, untrack, refresh, status, shutdown, and starting up

## Outcome
`api::server::routes()` answers every route but `wait` and the two of `notify`, with strict parameters, the envelope and API-3's errors, from one store view each, reading through and refreshing by way of `PollerHandle`.

## Context
T02 wrote the types; **T43 built the API's shared parts**: `Request`, `Params`, `respond` (with API-8's `window`), `router` with its layer and both fallbacks, `InFlight`, and the fakes `FakePoller` and `api::test_state`. T41 wrote the golden files in `testdata/api/`. T43 couldn't test `router` without a working store: this task does. **A fault you find in T43's files is reported, not fixed.** T15's store holds the state; T09's `cadence::stale` is behind `respond::present`. This task fills `server::routes()`. The poller is seen only as `PollerHandle`: the tests use T43's `FakePoller`, the engine (T21, T25) comes later. T23 builds the `wait` route beside this one, and T47 the notify routes later; T28 serves the router on the socket.

## Read first
- `plan/conventions.md`
- `plan/contracts/api.md`: the whole file; `plan/contracts/state.md`: `View`, `PrView`, `Untracked`, `lookup`, `name`, `untrack`, `Flush`, "Don't assume"; `plan/contracts/poller.md`: "Outside" and "The fake"; `plan/contracts/daemon.md`: `StopHandle`; `plan/contracts/log.md`: `Warnings`, `Logger::failing_since`; `plan/contracts/clock.md`: `timeout`, "`Activity`: who counts what"
- `specs/04-api.md#basics`: API-1 to API-5, API-19; `#reading`: API-6 to API-8; `#changing-the-set`: API-15 to API-17; `#starting-up`: API-18
- `specs/03-state.md`: STA-1 ("a request names a PR"), STA-3 (the limit), STA-7 (written before it's answered), STA-9 (too old)
- `specs/01-model.md#revisions-and-events`: MOD-13's one view, MOD-15's `untracked`
- `testdata/api/*.json` (T41's): the shape of every answer; `plan/contracts/testkit.md`: `golden::check`

## Scope
**Every handler:** `poller.client_request()` first (not `status`, API-5; not `shutdown`); parameters through `Params`, ended with `finish()` (API-1); the store changed, if at all; then **one `store.view()`**, from which the records, events and cursor all come (MOD-13); `respond::ok` (`ok_at` for a cut `limit`) or `respond::error`, which add `version` (API-2), `cursor` and `daemon` (API-4). A route with a PR in its path holds `store.name(id)` from the moment it knows the ID until it answers (STA-1).
- **Shares.** API-1, API-2 and API-3: T43 built `Params`, the envelope, the error body and the `unknown_route` fallback; this task uses them on every route, writes the messages of `bad_request` and `starting`, and is where the fallback and the layer are first tested through `router`. **`unknown_route` is `router`'s** (api.md, "Semantics"): an unknown path, and a known path with another method. This task adds no fallback of its own; it only tests both through `router`. A `Refusal` passes through `respond::refusal` unchanged. **API-4:** what the problem is, is T21's; here every 200 carries the block.
- **`GET /v1/prs` (API-6).** Sort: open first, then `updated_at` newest first, then `pr`. `state` is `open` or `all`. With `since`: the records with `rev` after it whatever their `state`, and `untracked` from the history's `Untracked` events after it whose `id` has no record and whose name isn't `pending` in this view. `checks.items` only with `checks=1`.
  - A too-old `since` gives `reset: true`, **the records `state` asks for** (API-6: the open ones by default, all of them with `state=all`), and `untracked: []`.
- **`GET /v1/prs/{owner}/{repo}/{number}` (API-7).** `store.lookup`; no record: `poller.read_through`. `fresh=1`: `poller.refresh_pr`, then answer from memory whatever it returned.
- **`GET /v1/changes` and the one for a PR (API-8).** **API-8's share here is the route; the rule is `respond::window`'s (T43).** The events, `more_events`, `reset` and the answer's cursor all come from **`respond::window(&view, since, pr, limit)`**: don't re-implement the `since`, `limit` or too-old rule. `limit` is parsed as 1 to `state::MAX_EVENTS`. The answer goes through `respond::ok_at` with `Window::cursor`. The route adds the records in scope (with `reset`, all of them, **without `checks.items`**: API-19) and `pending`. The PR's route reads an untracked PR through, passes its ID to `window`, and its `pending` is `[]`.
- **`POST /v1/track`, `DELETE /v1/track/…` (API-15).** This task's share is the two routes: the `untrack` rules are in `Store::untrack` (T15), fetching and `manual` in `PollerHandle::track` (T25). `track`: body `{"pr": "owner/repo#12"}`, strict; `poller.track`; then `state.flush.flush()` **whether it succeeded or was refused**: a loaded PR's saved source is set before its read-through (T25), and STA-7 wants that written before the answer. A refusal then answers its error; undo nothing. `untrack`: `store.untrack`, never the poller; `flush()` when `file_changed` (STA-7). Its record is the one in the view when the PR is still tracked, `Untracked::record` as it is when it was dropped (it carries its `untracked` event's `rev`, and `stale: false`), and `null` otherwise.
- **`POST /v1/refresh` (API-16).** This task's share: the route. `poller.poll_now()`, then HTTP 200 with its `outcome` and `cost`, also when the outcome is a problem. What a poll is and when it's `shared` are T21's.
- **`GET /v1/status` (API-5).** From `info`, `poller.status()`, the view (`tracked`, `waiters`, `subscriptions`), `config.owners` and `warnings.list()`, plus a `log_unwritable` warning while `logger.failing_since()` is set. It always answers at once. **NTF-9's share here: `subscriptions`**, the number of open ones, from `View::subscriptions`; nothing else about a subscription is ever in an answer of this task's.
- **`POST /v1/shutdown` (API-17).** This task's share: answer, **then** `stop.request(StopCause::Request)`. As api.md says: the response's body asks for the stop from its `Drop`, so the stop can't start before the server has taken the whole answer. The stop itself is T28's.
- **Starting up (API-18).** `list` and `/v1/changes` wait on `poller.first_poll()` for up to 10 s of the clock (one constant). `Running` at the end: `starting`. `Unanswered(refusal)`: that error, at once. `Finished`: the answer, with `pending`. Routes with a PR in their path, `status` and `shutdown` don't wait. What "finished" means is T21's.
- **Work in flight (clock.md, "`Activity`: the rule").** A handler calls `InFlight::park()` before it awaits `first_poll()` or any async `PollerHandle` method, and `resume()` after: the engine counts that work itself.

## Interfaces
- **Consumes:** `api::{AppState, Params, pr_from_path, bad_request, ok, ok_at, window, Window, error, refusal, present, InFlight}` and `api::types::*`; `state::store::{Store, View}`; `state::{Flush, MAX_EVENTS}`; `poller::{PollerHandle, FirstPoll, Refreshed}`; `daemon::{StopHandle, StopCause}`; `clock::timeout`.
- **Provides:** `api::server::routes() -> axum::Router<AppState>` with the eight routes above. Nothing else is public.

## Out of scope
- The `wait` route: T23. The notify routes, `no_session` and the subscriptions' `limit`: T47. `src/api/mod.rs` and `types.rs`: T02's, final. `request.rs`, `params.rs`, `respond.rs`, `router.rs` and `poller::fake`: T43's; report a fault in them, don't fix it. The golden files: T41's.
- API-8's window rule (`since`, `limit`, too old, another epoch): `respond::window`, tested by T43. The `unknown_route` fallbacks: `router`, T43.
- `stale`: `respond::present`, on T09's rule. The `untrack` rules and the limits: T15.
- Sending anything to GitHub, POL-15's windows, the messages of `auth`, `github_unavailable` and `rate_limited`: T21, T25.
- Serving on the socket, the stop: T28. The CLI's text and exit codes: T10, T27.

## Acceptance
Unit tests in `src/api/server/`, through `tower::ServiceExt::oneshot` on `api::router` over the `AppState` of `api::test_state(clock)` (it returns the state and its `FakePoller`), the store filled through `reserve` and `apply`:
- `api1_router_fallback_and_layer`: `/v1/nope` and `/v2/prs` are 404 `unknown_route` with `version`; so are a known path with another method, `POST /v1/prs/o/r/1/wait`, `DELETE /v1/prs` and `PUT /v1/status`, never a bare 405. With a `PollerHandle` of the test's own, `activity.in_flight()` is 1 inside `client_request` and 0 once the response is dropped. After `sleep_wall_only` of 1 hour, one `status` request makes `sleep.total()` 1 hour.
- `api1_encoded_value`: a `since` sent as `<epoch>%2D3` is read as `<epoch>-3`, and `%zz` is `bad_request`.
- `api19_shapes`: each route's body has the key names of its golden file, at every level and in order, with every list and flag there when empty. The values are the test's own, so this is a comparison of shapes; a whole-body comparison, where a test can make one, goes through `testkit::golden::check`, never a file read and compare of its own.
- `api1_strict`: an unknown parameter, a parameter twice, `checks=2`, `limit=0`, `limit=501`, `state=closed`, `since=abc`, a `track` body with an unknown key, one that isn't JSON, a `pr` that isn't `owner/repo#n`, and a path number `x`: each 400 `bad_request` naming it.
- `api2_api4_envelope`: every 200 has `version`, the view's `cursor` and `daemon`; with a problem set on `FakePoller`, `daemon.problem`, `message` and `until` carry it. Every error has `version`.
- `api3_errors`: a read-through refused as `NotFound` is 404, `Auth` 503, `GithubUnavailable` and `RateLimited` 503 with `until`, `Limit` 409; `GET /v1/track` is 404 `unknown_route`. A PR with a record answers 200 while the poller reports a problem.
- `api5_status`: `tracked` by source; `subscriptions` is 0, and 2 with two `Subscribed` guards taken from the store, while `waiters` stays 0; `log_unwritable` appears while the logger fails; `client_request` wasn't called; it answers while the first poll is `Running`.
- `api6_list`: the order; `open` by default, `all` adds merged, closed and gone; `since` lists a PR merged after the cursor, and names in `untracked` a PR dropped after it, but not one tracked again; a too-old cursor gives `reset: true` with the open records, and with `state=all` every record; `checks.items` is absent without `checks=1` and `failed_items` present; `pending` is sorted; empty lists are `[]`.
- `api7_show`: with `checks.items`; no record calls `read_through` once; `fresh=1` calls `refresh_pr` once and answers 200 on `Failed`; `Timing::named` is set.
- `api8_changes`, the one test of `window` through the route: no `since` gives no events and the view's cursor; 5 events with `limit=2` give the 2 oldest, `more_events: true` and the second's cursor in the envelope; the PR's route gives only its own events and `pending: []`; a cursor older than the history gives `reset: true` and every record in scope, none with `checks.items`. The rest of the rule is T43's test.
- `api15_track_untrack`: `track` calls `poller.track` and `flush`, and answers `source: manual`; a refused `track` answers its error and has called `flush` too; `untrack` of an untracked PR answers `record: null` and calls neither the poller nor `flush`; of a `manual` one that's dropped, its record, and `flush`; of an `authored` one, no change.
- `api16_refresh`: `polled` with its cost; `shared` with 0; `github_unavailable` is still HTTP 200.
- `api17_shutdown`: a 200 with the envelope and no other key; `stop.requested()` is `None` until the body has been read and dropped, then `Request`.
- `api18_starting`: with the first poll `Running`, `list` answers `starting` (503) when the clock reaches 10 s, and answers 200 at 3 s if it finishes then; `Unanswered` with an `auth` refusal is 503 `auth` at once; `show`, `status` and `shutdown` don't wait.
- `cargo test --features test-endpoint --lib api::server::` and `make verify` pass.

## Traps
- **Park, or the test clock hangs.** A handler that holds its `InFlight` while it waits for something only a later timer ends (the 10 s of API-18, a read-through over several requests 1 s apart) stops `TestClock` from ever settling.
- **`respond::ok` writes the view's cursor.** `changes` answers through `ok_at` with `Window::cursor`, which is the last returned event's when `limit` cut the list.
- **Take the view after the last change**, once. Two views in one answer can show a revision without its event.
- `View.prs` isn't sorted, and `Record.stale` isn't set: sort here, and go through `present`.
- Match `untracked` by the event's `id`, not its name: a PR can be renamed (MOD-1).
- A handler's future is dropped when the client goes away: the `Named` guard's `Drop` is the request's end.
- The response types aren't `deny_unknown_fields`; the server's own `track` body is.
- No message holds GitHub's text: pass `Refusal::message` on, and write your own from names and numbers.

## Open questions
- **(proposal):** the names of API-5 and API-19, API-5's codes, API-16, `rate_limited`. Built as written.
- **`checks.items` (API-19):** in every single `record` (`show`, `track`, `untrack`), in `list`'s records only with `checks=1`, and never in a reset's `records`. `present`'s last argument says which.
- **At STA-3's limit** a `show` of an untracked PR answers `limit` and nothing is sent, as TST-8's `many-prs` row now says too.

## Evidence
- **2026-11-26**, branch `t22-api-routes`, executor on **Opus, medium**; reviewed and merged by the orchestrator.
- `src/api/server/` (`mod.rs`, new `reading.rs`, `changing.rs`, `status.rs`, `tests.rs`); no public signature changed. `cargo test --features test-endpoint --lib api::server::`: 14 passed, among them `api1_router_fallback_and_layer` (T43's fallbacks and layer through `router`) and `api19_shapes` (key order at every level against T41's files). `make verify` passes on main after the merge (397 lib tests), the macOS check included. No fault found in T43's files.
- Choices: a read-through or `track` that answers `Ok(id)` with no record in the view then answers `github_unavailable` with the route's own message; `status`, `refresh` and `shutdown` refuse unknown query parameters too; the shutdown body is a `Full` whose drop guard asks the stop (whether the written bytes survive the stop is T28's to prove).
