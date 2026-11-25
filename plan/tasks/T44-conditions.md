---
id: T44
title: "Conditions: checking and evaluating a wait's or a subscription's condition, once"
kind: code
milestone: M2
size: M
status: todo
depends_on: [T02]
requirements: [API-10, API-11, NTF-4:a-subscription-searches-the-event-history-once]
owns: [src/api/condition.rs]
shares: []
---

# T44 · Conditions: checking and evaluating a wait's or a subscription's condition, once

## Outcome
`api::condition` checks a condition's parameters (`Condition::check`) and evaluates it on a store view (`evaluate`), by API-10's conditions and API-11's outcomes, with `--kind` and `--fail-fast` (D8). It's the one evaluation the waiters, the notify route and the notifier all call.

## Context
T02 declared `Ask`, `Condition`, `Progress` and `evaluate` in `src/api/condition.rs`, with stubs, and built what they read: `View` and its methods, `Record`, `Event`, `EventKind::word`, `Cursor`, `Until`. This task fills the file. **It's pure:** a view goes in, an outcome comes out. The wait route (T23) calls it after every refresh of its PR, the notify route (T47) once when registering, and the notifier (T46) for every open subscription after every applied refresh (NTF-4). None of them reads API-10 or API-11 for itself, so a waiter and a subscription can't disagree.

## Read first
- `plan/conventions.md`
- `plan/contracts/api.md`: "Declarations (`src/api/condition.rs`, T44)" with its five bullets, and `Ask`, `Until`, `Outcome`, `Reason`, `ApiError`, `ErrorCode`; `plan/contracts/state.md`: `View`, `PrView`, `Timing` (`fetched`), `Cursor`; `plan/contracts/model.md`: `Record`, `Checks`, `PrState`, `Event`, `EventKind` and `word`
- `specs/04-api.md#waiting`: API-10 and API-11, every bullet; API-9's first bullet (when the first evaluation comes)
- `specs/08-notify.md#while-one-is-open`: NTF-4, the part "a subscription searches the event history once"; NTF-2's part "a `since` that is too old when registering"
- `specs/03-state.md#cursors`: STA-9 (too old), STA-10 (the history's 500)
- `specs/01-model.md`: MOD-5 (`checks.state`, `failed`), MOD-6 (`settled`), MOD-7 (a truncated `failing`), MOD-15 (the kinds)
- `specs/decisions.md`: D8

## Scope
- **`Condition::check` (API-10's parameters).** `sha` is 7 to 40 hex characters, kept as given. `sha`, `rerun_since` and `fail_fast` with another `until` than `checks-done`, and `kind` with another than `changed`, are `bad_request` naming the parameter. `kind` is split on commas: every word must be one of `KIND_WORDS` (so `tracked` and `untracked` are refused), and an empty list or an empty word is refused. `since` and `rerun_since` parse through `Cursor::from_str` (STA-9's form). The error is an `ApiError` with `ErrorCode::BadRequest`, a message that names the parameter and `until: None`: **build the value yourself**, `api::bad_request` is T43's and may still be a stub.
- **`evaluate` (API-10, API-11).** As `plan/contracts/api.md` declares it, on the view's record for `id`. No record: `None`, and nothing in `progress` changes.
  - **The condition first, then the ends** (API-11's first bullet). `merged`: `state` is `merged`. `checks-done`: API-10's row, with `sha` compared as a prefix without case, and a truncated `failing` holding (MOD-7). `changed`: the record's `rev` is after the point.
  - **The point** is `since`, or without one `progress.baseline`: set to the record's `rev` by the first evaluation that sees a record, which then answers `None`.
  - **`kind` (D8).** Holds when `progress.kind_seen`. With `kind`, a merge, a close or `gone` whose word isn't named ends the wait with that reason; one that is named satisfies it like any event of a named kind.
  - **`fail_fast` (D8).** With `checks.failed` above 0, the first part of `checks-done` and `settled` count as held; `sha` and `rerun_since` still have to. API-10 says so (D9): the failure stands in for `checks.settled` too, so it answers at the first refresh that shows it.
  - **`rerun_since`.** Holds when `progress.rerun_seen`, or when `progress.rerun_dropped`.
  - **The ends.** `closed` and `gone` end `merged` and `checks-done`; `merged` ends `checks-done`. A state's reason comes before `head_moved` (API-11's "both ended and moved its head"). `head_moved`, for `checks-done` with `sha`: `sha_matched` and no longer matching; or never matched and `timing.fetched` is at least `HEAD_MOVED_AFTER` after `progress.began`. Never the clock: there's none here.
- **What it remembers (NTF-4's part, and API-10's cursors).** The first evaluation that sees a record judges `since` and `rerun_since` with `View::too_old`: a too-old `since` sets `held_at_once` (`changed` then holds, with or without `kind`); a too-old `rerun_since` sets `rerun_dropped`. It then reads this PR's events after the point with `View::events_after`; **every later call reads only those after `progress.looked`**, and sets `looked` to the view's `rev`. `kind_seen` and `rerun_seen` only ever go from `false` to `true`. So a cursor that goes too old while the caller holds on changes nothing.
- **Shares.** API-10 and API-11: this task has the rule. T23 has the wait's parameters as a query, its deadline and `timeout`, and when it calls; T47 the same for `notify`'s body. NTF-4: this part is the memory; the notifier's loop, its ends and its notices are T46's.

## Interfaces
- **Consumes:** `api::types::{Ask, Until, Outcome, Reason, ApiError, ErrorCode}`, `state::{Cursor}`, `state::store::{View, PrView, Timing}`, `model::{Record, ChecksState, PrState, Event, EventKind, NodeId}`, `clock::Mono` (T02).
- **Provides:** `api::condition::{KIND_WORDS, Condition, Progress, HEAD_MOVED_AFTER, evaluate}`, exactly as `plan/contracts/api.md` declares them. A new public item goes through the orchestrator.

## Out of scope
- The `wait` route, its two-step answer, its deadline and `timeout` (API-9, API-12): T23. Turning a query into an `Ask`: T23. A JSON body into one: T47.
- The answer's events, `reset` and `more_events`: `respond::window`, T43. This task only says whether `rerun_since` was dropped (`progress.rerun_dropped`); the caller sets `reset`.
- The notifier's loop, expiry, the socket check and every notice: T46. Exit codes and the outcome line: T10.
- `View`'s methods, `EventKind::word`, `Cursor`: T02's. A fault in them is reported.

## Acceptance
Unit tests in `src/api/condition.rs`, over hand-built `View`s (its fields are public) and records built in the test:
- `api10_check`: `sha` of 6 and 41 characters and `1a2b3cz` are refused, `1A2b3c4` kept as given; `sha`, `rerun_since` and `fail_fast` each with `until=merged`, and `kind` with `until=checks-done`, are `bad_request` naming it; `kind=activity,merged` gives both words in that order; `kind=tracked`, `kind=foo`, `kind=` and `kind=activity,` are refused; `since=abc` is refused.
- `api10_api11_table`, one case per row of API-10 and API-11: `merged`; `checks-done` for `passing`, `failing` and `none`, each unsettled too; `sha` matching by prefix and without case, and not matching; merged with checks done is `satisfied`, with checks pending `ended`, `merged`; closed and gone end `merged` and `checks-done` and satisfy `changed`; a truncated `failing` holds.
- `api10_changed_baseline`: without `since`, the first call sets `baseline` and answers `None`; a later view with a higher `rev` answers `satisfied`. With `since`, a record whose `rev` is above it answers at the first call.
- `api10_kind`: with `kind=activity`, a `checks` event after the point answers `None`, an `activity` one `satisfied`; a merge answers `ended`, `merged`, and with `kind=activity,merged` `satisfied`; a close and `gone` likewise with their own words.
- `api10_fail_fast`: `checks.state` `pending` with `failed` 1 answers `satisfied` with the flag and `None` without; the same unsettled (API-10, D9); with `sha` not matching, `None`; with `failed` 0 it's `checks-done` as ever.
- `api10_rerun_since`: no `checks` event after it answers `None` though the checks are done and settled; one after it answers.
- `api11_head_moved`: matched, then another head: `ended`, `head_moved`. Never matched: `None` with `timing.fetched` 59 s after `began`, `ended` at 60 s. A closed PR whose head also moved: `closed`.
- `ntf4_memory`: after a first call on a view holding events 1 to 5, a second on a view whose history starts at event 400 (the cursor is now too old) still answers by what was seen: a `kind` seen at event 3 holds; one not seen doesn't start holding; `rerun_seen` stays. A later event of a named kind, above `looked`, is found.
- `sta9_too_old_at_first`: a `since` from another epoch holds `changed` at the first call, with and without `kind`; a too-old `rerun_since` sets `rerun_dropped` and that part holds.
- `no_record_is_none`: a view without the PR answers `None`, and `progress` is unchanged.
- `cargo test --features test-endpoint --lib api::condition::` and `make verify` pass.

## Traps
- **Mark what was looked at.** Reading `events_after(point)` on every call is the history search NTF-4 forbids, and after 500 events it reads a `kind` out of nothing or loses one.
- **`baseline` isn't the cursor in a header.** It's the record's `rev` at the first evaluation; the caller makes that call after its arrival refresh (API-9, API-10).
- **`satisfied` for `checks-done` covers passing and failing**, and with `fail_fast` a `pending` state too: the record says which. Exit code 8 is T10's.
- Filter events by `Event.id`, never by `pr`: a rename changes the name, and `id` is `#[serde(skip)]`, so build test events with it set.
- Compare a SHA without case and as a prefix; never print or change it here.
- No clock, no `Store`, no `AppState`: T15 and T43 aren't merged when you build.

## Open questions
- **API-10's `kind`, `fail_fast` and `rerun_since`, API-11's 60 s and a truncated `failing` holding are (proposal).** Built as written, the 60 s as `HEAD_MOVED_AFTER`.
- **A wait is evaluated with the same memory as a subscription.** API-10 writes a wait's rule as a search from `since`; over a wait's hour at most the two agree, and one function is what NTF-4 asks for. Say so in your report if a case differs.
