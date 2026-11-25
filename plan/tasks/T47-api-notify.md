---
id: T47
title: "Notify routes: registering a subscription and cancelling it"
kind: code
milestone: M2
size: M
status: todo
depends_on: [T09, T41, T46]
requirements: [NTF-2, NTF-3, NTF-5, API-3]
owns: [src/api/notify/]
shares: []
---

# T47 · Notify routes: registering a subscription and cancelling it

## Outcome
`api::notify::routes()` answers `POST /v1/prs/{owner}/{repo}/{number}/notify` and `DELETE …/notify?socket=…`: registering evaluates once as a `wait` with `timeout=0` does, answers at once when the answer is already known, and otherwise hands the subscription to the notifier and answers `registered: true` after its confirmation was posted (NTF-2); cancelling drops a session's subscriptions on a PR (NTF-3).

## Context
T02 declared `notify::routes()` as a stub and `NotifyBody`, `CancelBody` and `ErrorCode::NoSession`; T43 built `Params`, `respond` (with `window` and `present`) and `router`, which already merges this router; T41 wrote the golden files; T44 built `condition::check` and `evaluate`; T46 built the notifier behind `NotifyHandle`; T15's store has `name_quiet` and the places; T09's rule is behind `present`. This task builds the two routes and nothing else. **It reads API-10 and API-11 through T44 only**, and holds no subscription itself. T48's CLI sends these requests; T28 serves the router.

## Read first
- `plan/conventions.md`
- `plan/contracts/api.md`: the whole file, above all `NotifyBody`, `CancelBody`, `Request::Notify`, "Declarations (`src/api/condition.rs`)" and, in "Semantics", the notify routes and `checks.items`; `plan/contracts/notify.md`: `NotifyHandle`, `NewSubscription`, `Registered`, `RegisterError`, "Semantics" (`register`), the fake; `plan/contracts/state.md`: `lookup`, `places_free`, `name_quiet`, `View`, `Class`; `plan/contracts/poller.md`: "Outside" and "The fake"; `plan/contracts/clock.md`: "`Activity`: who counts what"
- `specs/08-notify.md#subscriptions`: NTF-1 (node ID, the name registered under), NTF-2 (every part, its order above all) and NTF-3; `#while-one-is-open`: NTF-5
- `specs/04-api.md#waiting`: API-9 (the arrival's order), API-12 (`timeout=0`), API-13; `#basics`: API-1, API-3 (`limit`, `no_session`), API-19's two rows; `#reading`: API-8 (`limit`)
- `specs/03-state.md`: STA-1 ("a request names a PR"), STA-3, STA-8 (a loaded PR)
- In `testdata/api/` (T41's, there once T41 is merged): `notify-registered.json`, `notify-already.json`, `notify-cancelled.json`, `error-no_session.json`, `error-limit.json`
- `src/api/wait/` if T23 is merged: how the wait does its arrival. You share T43's and T44's functions with it, not its code.

## Scope
- **`POST …/notify` (NTF-2), in this order:**
  1. `poller.client_request()`. The body, strict (`deny_unknown_fields`, mapped to `bad_request` naming the key): `until` and `socket` required; `token` a string or `null`; `for` a number, 60 to 86 400, default `notify::DEFAULT_LIFE`; `limit` 1 to `state::MAX_EVENTS`; `sha`, `rerun_since`, `since` strings, `fail_fast` a JSON boolean, `kind` one string. They become an `Ask` and go through `Condition::check`. A `socket` that isn't an absolute path is `bad_request`.
  2. `state.notify.look(socket)`, NTF-2's `stat`: anything but `Looked::Socket` is **`no_session`** (409), its message naming the reason (API-3): missing, not a socket, another user's, or the system's error. **NTF-2's order puts it straight after the parameters:** a bad socket never reaches GitHub, and gets `no_session` even when the answer would be known.
  3. STA-3: an untracked PR with `places_free() == 0` is `limit`. Nothing is sent.
  4. The read-through, when the PR has no record: **also for one loaded from the state file**, unlike a wait's (NTF-2). A refusal answers as API-3.
  5. `store.name_quiet(id)`, held until the answer (STA-1's 60 minutes only). Take the view's cursor: where the answer's events start without `since`.
  6. The arrival refresh, `poller.refresh_pr(id)`; `NotSent` and `Failed` count as failed, and the evaluation follows at once (API-9, API-13).
  7. One `store.view()`, and `condition::evaluate` with `Progress::new(arrival's monotonic time)`.
  8. **An outcome (`satisfied` or `ended`):** `state.notify.drop_same(…)`: an earlier subscription that is the same request (NTF-2) is dropped, and `replaced` is what it returns. Answer 200 with `registered: false`, `replaced`, `expires_at: null`, the outcome and reason, the record through `present(…, true)`, and the events as `respond::window` gives them from `since` or step 5's cursor, with `limit`; with `reset`, `records` holds the record without `checks.items`. Nothing is kept, nothing posted: `registered: false` with `replaced: true` is a valid answer (NTF-2).
  9. **No outcome, in the `auth` state** (`poller.status().problem` with the code `auth`): the `auth` error (API-3), nothing kept (NTF-2: nothing can be told until a person logs in). An answer already known (step 8) comes first, and **`auth` comes before the limit** (step 10's `register`).
  10. **No outcome:** `state.notify.register(NewSubscription { … })`, with the checked condition, the progress from step 7, the name as given, `cursor` as `since`, or this view's cursor without `since` or **when `since` is too old on it** (`View::too_old`: answered with the reset, then let go, NTF-2), and `socket_id` from step 2. NTF-2's "one step" is `register`'s (`contracts/notify.md`): don't take another view in between. `Limit` is `limit`; `NoSession(e)` is `no_session` naming `e`; `Stopping` never answers (below). `Registered`: 200 with `registered: true`, `replaced`, `expires_at`, `outcome` and `reason` `null`, the record, and the events since `since` (none without it), through `window`, **all from step 7's view**; a too-old `since` gives `reset: true` and `records`, as a wait's answer does.
- **Never hot (NTF-5's share here; STA-1 states the rule, which POL-11, POL-14 and NTF-2 cite).** No `store.name`, no `store.add_waiter`, no `poller.nudge`: `name_quiet` names the PR for STA-1's 60 minutes only, never for POL-11's 60 s or POL-14's 2 minutes. The arrival refresh is the one forced refresh a registration causes.
- **`DELETE …/notify?socket=…` (NTF-3).** `client_request()`; `socket` required, through `Params`; `store.lookup` for the node ID, **never a read-through, and never `starting`** (API-18); `name_quiet` when it's tracked; `state.notify.cancel(name, id, socket)`, which also finds a subscription registered under that name whatever its PR is called now (NTF-1). An untracked PR with no such subscription answers `cancelled: 0`. 200 with `CancelBody`. Idempotent.
- **API-3's share here:** `no_session` (409) and `limit` for subscriptions. The code and the body's shape are T02's and T43's.
- **`Stopping`.** The daemon is closing this connection without an answer (OPS-7): park the `InFlight` and await a future that never completes. Don't answer an error.
- **Work in flight (clock.md).** `InFlight::park()` before awaiting `look`, the read-through, the arrival refresh and `register`; `resume()` after each.
- **One view per answer, the registration's (MOD-13, NTF-2):** the view of step 7, for an outcome and for a kept one alike. Its record, events and cursor are that view's, so the answer's `cursor` is the subscription's (NTF-8). **Don't take a view after `register` returns:** a refresh applied while the confirmation was posted belongs in the notice, not the answer.

## Interfaces
- **Consumes:** `api::{AppState, Params, pr_from_path, bad_request, ok, ok_at, window, Window, error, refusal, present, InFlight}`, `api::types::{NotifyBody, CancelBody, Ask, Until, ApiError, ErrorCode}`, `api::condition::{Condition, Progress, evaluate}`; `notify::{NotifyHandle, Looked, NewSubscription, Registered, RegisterError, SessionToken, DEFAULT_LIFE, MIN_LIFE, MAX_LIFE}`; `state::store::{Store, View}`, `state::MAX_EVENTS`; `poller::{PollerHandle, Refreshed}`.
- **Provides:** `api::notify::routes() -> axum::Router<AppState>` with the two routes. Nothing else is public.

## Out of scope
- The registry, the confirmation, replacing, the 64, the loop and every notice: T46. This task calls `NotifyHandle`.
- API-10 and API-11: T44. API-8's window: T43. The `wait` route: T23; no code is shared with it but T43's and T44's.
- `subscriptions` in `status`: T22. The tier and the idle exit while one is open: T09, T21, T28.
- `src/api/mod.rs`, `types.rs`, `router.rs`: T02's and T43's. The golden files: T41's. The command: T48.

## Acceptance
Unit tests in `src/api/notify/`, through `tower::ServiceExt::oneshot` on `api::router`. The state is `api::test_state(clock)`'s with `state.notify` replaced by a `Notifier::spawn` over the same store and a `RecordingPoster`; the store is filled through `reserve` and `apply`.
- `ntf2_strict_body`: no `until`, no `socket`, an unknown key, `fail_fast: 1`, `kind: ["activity"]`, `for: 59`, `for: 86401`, `limit: 0`, `sha` with `until: "merged"`, a body that isn't JSON, and `socket: "relative/s.sock"`: each 400 `bad_request` naming it, with the poller and the poster untouched.
- `ntf2_no_session`: with `RecordingPoster::gone(path)`, 409 `no_session` before any read-through, also for an untracked PR (`read_through` never called) and for a PR whose answer is already known (failed and settled checks: still `no_session`, not `registered: false`); with the confirmation answered `Failed`, 409 `no_session` naming the error, `status`' count still 0. A `socket` that names a regular file (through `RealPoster` in a `ShortDir`): 409 `no_session`.
- `ntf2_auth`: with `FakePoller::set_status` reporting `auth` and running checks: the `auth` error, nothing kept, nothing posted, also with 64 already held (`auth` before `limit`); with the checks failed and settled: 200 `registered: false`.
- `ntf2_since_too_old`: a `since` from another epoch on running checks: 200, `registered: true`, `reset: true`, `records` holds the record; after `for: 60` has passed, the `expired` notice's `--since` is the answer's cursor, not the `since` sent.
- `ntf2_registered`: running checks, `until: "checks-done"`: one confirmation is in `posts()` before the response is returned; the body has `registered: true`, `replaced: false`, `expires_at` 86 400 s after the clock's wall time, `outcome` and `reason` `null`, the record with `checks.items`, `events: []`; `view.subscriptions` is 1.
- `ntf2_already_known`: failed and settled checks: 200, `registered: false`, `expires_at: null`, `outcome: "satisfied"`, the record; nothing posted, nothing kept. A closed PR with `until: "merged"`: `ended`, `closed`.
- `ntf2_evaluates_as_timeout_0`: `refresh_pr` was called once; an untracked PR calls `read_through` once; a `Loaded` PR with no record calls it too; with `refresh_pr` answering `NotSent`, the evaluation runs on the record in memory.
- `ntf2_replaced`: the same body again answers `replaced: true` with a second confirmation and one subscription; the same with `sha` upper-cased and `fail_fast: false` added is a replacement too. A body whose answer is now already known answers `registered: false`, `replaced: true`, the earlier one is gone and nothing is posted.
- `ntf2_limit_after_the_evaluation`: with 64 held, a 65th that would be kept is 409 `limit`; one whose answer is already known is 200; with 32 held for one socket, a 33rd for it is 409 `limit`; an untracked PR at 200 places is 409 `limit` with no read-through.
- `ntf5_never_hot`: after registering on a `checks_pending` PR, its `Timing::named` is unset, `Timing::waiters` 0, `nudge` wasn't called; `Timing::subscriptions` is 1.
- `ntf2_events_since`: with `since` and three later events, `registered: true` carries the three; with `limit: 2`, two and `more_events: true`.
- `ntf2_answer_from_the_registration_view` (MOD-13): with `RecordingPoster::stall` held on the confirmation, a refresh that makes the PR's checks fail is applied; once released, the answer's `record` still shows them running, its `cursor` is the one before that refresh, and the outcome notice is posted after the confirmation.
- `ntf3_cancel`: answers `cancelled: 1`, then `cancelled: 0`; no `socket` is 400; an untracked PR answers `cancelled: 0` and `read_through` wasn't called; with `FakePoller::set_first_poll(Running)` it still answers at once; nothing is posted. After a refresh renames the PR, a cancel by the old name answers `cancelled: 1`.
- `ntf9_token_not_in_answers`: registering with `MARKER_SESSION_TOKEN`: no response body and no log line holds it; the poster got it.
- `api19_shapes`: the bodies have the key names of `notify-registered.json`, `notify-already.json` and `notify-cancelled.json`, at every level and in order; the errors those of `error-no_session.json` and `error-limit.json`.
- `cargo test --features test-endpoint --lib api::notify::` and `make verify` pass.

## Traps
- **`store.name` makes the PR hot for two minutes** (POL-14). These routes use `name_quiet`; `ntf5_never_hot` is what catches the wrong one.
- **Check the 64 and the 32 after the evaluation, by calling `register`**, never before: an answer already known is never refused (NTF-2).
- **A registration that isn't kept still replaces** (NTF-2): call `drop_same` on an outcome.
- **Park, or the test clock hangs:** the read-through, the arrival refresh and the confirmation can each wait on a timer.
- **`fail_fast` is a boolean in a body and `1` in a query.** Don't route the body through `Params`.
- The token arrives in the body: deserialise it straight into `SessionToken`, never into a `String` you keep. An error message names `socket`, never the token.
- A handler's future is dropped when the client goes away: the `Named` guard's `Drop` is the request's end. A subscription `register` has already kept stays kept.
- `test_state` holds `NoNotify`: replace `state.notify` before building the router, or every registration answers nothing.

## Open questions
- **NTF-2's values, its 64 and its 32 are (proposal).** Built as written; the numbers are T46's and T15's constants.
- **Plan decision P15, built:** a client that goes away after `register` leaves its subscription in place.
- **`no_session`'s message** names the system's error (API-3): pass `look`'s and `RegisterError::NoSession`'s text on, and add nothing from the request but the word `socket`.
