---
id: T15
title: "State store: the tracked set, applying refreshes, revisions and events, retention, cursors"
kind: code
milestone: M1
size: L
status: todo
depends_on: [T04, T05, T43]
requirements: [STA-1, STA-2, STA-3, STA-4, STA-5, STA-9, STA-10, MOD-13, STA-8:a-loaded-pr-keeps-its, STA-8:entries-past-their-keep-done-are-dropped-on-load, NTF-4:a-pr-with-a-subscription-stays-tracked, U3, U7]
owns: [src/state/store/]
shares: []
---

# T15 · State store: the tracked set, applying refreshes, revisions and events, retention, cursors

## Outcome
`state::store::Store` holds every tracked PR: it gives out places, applies refreshes through `model::apply_read`, takes a revision and records an event only for a visible change, keeps ended PRs for `keep_done`, drops what's due at a sweep, and hands readers one consistent `View` with its cursor. It also provides what the state file holds.

## Context
T05 gives `model::apply_read`, `apply_gone` and `counts_rose`; T04 gives the clocks and the logger; T43 gives `github::fake::answers`, which the tests fill the store with. The engine (T21, T25) writes through this store, the routes (T22, T47) and the waiters (T23) read `View`s from it, the notifier (T46) holds a guard per subscription and evaluates on `changed()`, the saver (T16) reads it as a `FileSource`, and T28 builds it from the loaded file. **There's no fake store:** those tasks all test on this one.

## Read first
- `plan/conventions.md`
- `plan/contracts/state.md`: every declaration, and "Semantics"; `plan/contracts/model.md`: `Before`, `Carry`, `Merged`, `ReadMeta`, `Event`, `PrName`'s order; `plan/contracts/clock.md`: wall against monotonic time; `plan/contracts/log.md`: "Who logs what"; `plan/contracts/config.md`: `keep_done`, `owners`
- `specs/03-state.md#the-tracked-set`: STA-1 to STA-5; `#the-state-file`: STA-6 (what an entry holds) and STA-8's two parts named below; `#cursors`: STA-9, STA-10
- `specs/01-model.md#revisions-and-events`: MOD-13; MOD-14 and MOD-15 for what takes a revision
- `specs/04-api.md#changing-the-set`: API-15's `untrack` rules; `specs/02-polling.md`: POL-5 (owners), POL-26 (which clock)
- `specs/00-overview.md`: U3, U7
- `specs/08-notify.md`: NTF-4's part "a PR with a subscription stays tracked", NTF-2's 64 and its naming rule, NTF-5
- `src/github/fake.rs` (T43's): the `answers` helpers the tests fill the store with

## Scope
- **Per tracked PR** the store keeps: ID, name, `source`, `Class`, `Option<Arc<Record>>`, `Carry`, `Timing`, the login it was last read with, the retention clock (wall), the asked clock (wall), the count of open naming requests, the count of waiters, the count of subscriptions, and for a `Loaded` one its saved `FileEntry`.
- **The tracked set (STA-1, U3).** `reserve` keeps the stronger source. `set_manual` and `untrack` change the source without a revision (MOD-14). `untrack` carries out API-15 whole: this task's share of API-15 is the rule, the route is T22's. "The user's own" is read as: `record.author` equals the PR's login, without case; with no login it isn't.
- **Naming requests and the asked hour (STA-1).** `name()` sets `Timing::named` and counts an open request; `Named`'s `Drop` restarts the asked clock. **`name_quiet()` does the same without touching `Timing::named`**: it's for the two notify routes, whose requests count for the 60 minutes only, never for POL-11's 60 s or POL-14's 2 minutes: STA-1 states that rule once, and POL-11, POL-14 and NTF-2 cite it. An `asked` PR is due 60 minutes of wall time after that clock, with no open request, no waiter and no subscription. As state.md says, the clock also starts whenever a PR becomes `asked` (a read-through, `demote_unlisted`, `untrack` with a waiter), so one no request ever named still leaves. **Interpretation:** a load outside `owners` starts it too.
- **`demote_unlisted` (POL-5's share here).** It takes the slice the engine passes (state.md): the `authored` PRs whose refresh this poll applied and showed open, and which its finding didn't list. The store **checks each one again** and changes only a PR that is still `authored` and open: it becomes `asked`, starts its hour, leaves `file_content()`, and `changed()` moves. A `manual`, `asked` or ended PR in the slice is left as it is. **A PR not in the slice is never touched**, whatever its source: one merged since the last poll is unlisted and still reads open until its refresh. Choosing the slice is T21's.
- **Places (STA-3).** `state::MAX_PLACES`, held by every class. `reserve` on a tracked ID takes none and never fails. `release` frees a `First` place only. Leaving found PRs out, and the warning, are T21's.
- **Applying (STA-4).** As state.md's "Semantics" says. `Before::Loaded` gets the saved source, `saved_open` from the saved `ended` being `null`, and a `Carry` whose `seen` is the saved one. A read for an ID that holds no place is `Discarded`. `Timing::fetched` is `meta.sent_mono`. `Target::mergeable_unknown` is the read's value, not the record's (state.md).
- **STA-5's share here:** the store keeps `Carry::seen` between reads, puts it in the file's entry, and hands the saved one to the first refresh. The rule for what's new is T05's.
- **Revisions and events (MOD-13, MOD-15).** One counter from 0. Non-empty `kinds` take the next number: the record's `rev`, the event (with `at` from the wall clock and the PR's `id`) and the history are changed under one lock hold. Empty `kinds` change nothing a reader orders by.
- **Retention (STA-2, U7).** The clock starts at `meta.sent_wall` of the read that first shows `merged` or `closed`, or `mark_gone`'s `seen_at`; it's cleared on `open` and started again on a change to another ended state. `mark_gone` on a `Loaded` entry saved open changes the saved entry only: no event, `Unchanged` (state.md). For a `Loaded` entry it runs from the saved `seen_at`, and a first refresh that confirms the saved state keeps that time (STA-8).
- **`sweep`.** By wall time, through `Time::since` (POL-26). Drops due `asked` and ended PRs, `Loaded` ones included, in the order of `pr`, each with an `untracked` event and a log line. Never one with a waiter or a subscription (STA-1): the `Drop` of the last `Waiting` or `Subscribed` on it carries out a drop that fell due meanwhile, with its event. `sweep` is the only place a due PR is dropped: never `apply`.
- **STA-8, the part "a loaded PR keeps its saved entry":** until its first refresh is applied, every `file_content()` carries the entry as loaded. The one change is `set_manual`, which sets its saved `source` (API-15). `login_of` answers the saved login.
- **STA-8, the part "entries past their `keep_done` are dropped on loading":** in `new`, without an event; the counter stays 0. Also in `new` (POL-5): a loaded `authored` entry whose owner isn't in `owners` becomes `asked`, leaves the file's content, and starts its hour.
- **Cursors (STA-9).** The epoch is 8 characters of `[a-z0-9]` from `fastrand` seeded with `deps.seed`. `View`'s methods are T02's, pure over its fields: the store's job is to fill `epoch`, `rev` and `events` so they're right (every revision has one event, so events were lost exactly when the oldest held `rev` is above 1).
- **History (STA-10).** The last `state::MAX_EVENTS` events, across all PRs, as a shared `Arc<[Event]>` rebuilt on change.
- **Waiters.** `add_waiter` counts at most `state::MAX_WAITERS` in all (API-14's number; the route is T23's) and sets `Timing::waiters`. `last_waiter_ended()` is the monotonic time of the latest `Waiting` drop, for T28's idle exit (OPS-8).
- **Subscriptions (NTF-4's part "a PR with a subscription stays tracked"; this is the store's whole share of notify).** `add_subscription` counts at most `state::MAX_SUBSCRIPTIONS` in all, apart from the waiters, and sets `Timing::subscriptions`, which POL-10's period reads and nothing about being hot does (NTF-5). `Subscribed`'s `Drop` takes it away and sets `last_subscription_ended()`; **it is the end of a request that named the PR** (STA-1, NTF-4): an `asked` PR's 60 minutes start again, and an ended PR, of any source, is kept until 60 minutes of wall time after it even past `keep_done`; a drop that fell due meanwhile is carried out at the later of the two. `open_subscriptions()` counts the subscriptions on PRs whose record is `open` (POL-13, NTF-5: the engine's tier, active while it's above 0; once it's 0 the tier follows the last client request, so no time is kept for it). `subscriptions()` is the count over every PR, without building a view. `untrack` of a `manual` PR with a subscription makes it `asked`, as with a waiter (API-15). **The store knows nothing else of a subscription:** its condition, its session and its notice are T46's. Which request replaces which, and so takes no new place, is T46's too: it then keeps the guard it has.
- **`Untracked::record`** for a dropped PR is its last record with the `rev` of its `untracked` event.
- **`FileSource`.** Every tracked PR but `asked` and `First` ones (STA-6; POL-4: a found PR is tracked only once its first refresh is applied), sorted by `pr`. `file_changed()` moves only when `file_content()` would differ.
- **`View`.** `pending` is `Loaded` and `First`, sorted; `tracked` counts every place held, by source; `subscriptions` is the count over every PR.
- **Logging (OPS-9, through `deps.logger`):** one `info` line when a PR is tracked (its first applied refresh), untracked (every drop) and gone, with the name and the source.
- **Named constants.** The three limits are T02's, in `src/state/mod.rs`: use them, never the numbers. STA-1's 60 minutes is one constant of this module.

## Interfaces
- **Consumes:** `model::{apply_read, apply_gone, counts_rose, Before, Carry, Merged, Event, EventKind}`; `clock::SharedClock` (`wall()` for retention and `at`, `mono()` for `Timing`); `log::Logger`; `config::Config` (`keep_done`, `owners`); `state::{FileContent, FileEntry, Ended, FileSource, Cursor, Limit, MAX_PLACES, MAX_WAITERS, MAX_EVENTS}` from `src/state/mod.rs`.
- **Provides:** everything under "Declarations (`src/state/store/`)" in state.md, `last_waiter_ended`, `name_quiet` and the three subscription methods included, and `impl FileSource for Store`.

## Out of scope
- What a read changes and which kinds it makes: T05. Don't compare records here.
- Reading and writing `state.json`, and when: T16. Loading order at start: T28.
- When to refresh, what's stale, POL-21's count towards `gone`, POL-7's warning: T09, T21, T25. Which PRs `demote_unlisted` gets: T21. `Record.stale` stays `false` in the store.
- API-6's order, parameters, HTTP answers: T22. Evaluating a wait: T23. Everything about a subscription but its guard and its count: T46, T47.

## Acceptance
Unit tests in `src/state/store/`, on `TestClock`, filled through `reserve` and `apply` with `github::fake::answers`:
- `sta1_stronger_source_kept`: `reserve` as `asked` on a `manual` PR keeps `manual`, and takes no place.
- `sta1_asked_leaves_after_60_min`: dropped at the first `sweep` 60 minutes of wall time after its `Named` guard dropped; not while a guard is open; not with a waiter; one never named leaves 60 minutes after it became `asked`.
- `sta1_drop_waits_for_the_last_waiter`: an ended PR past `keep_done` with a waiter survives `sweep`; dropping `Waiting` drops it, with an `untracked` event.
- `sta2_keep_done`: a PR merged at t is in the view at t + 24 h − 1 s and gone after the sweep at t + 24 h; reopened, its clock is cleared; closed then merged, it starts again. A wall jump of 8 days drops every ended PR at one sweep, `o/r#9` taking its revision before `o/r#10`. The wall clock set back 1 hour drops nothing early. With `keep_done` 0 the merged record is in the view after `apply`, and leaves only at the next `sweep`.
- `sta3_places`: 200 reserves, the 201st is `Limit`; a tracked ID is `Ok` at the limit; `release` frees a `First` place and nothing else.
- `sta4_apply`: no place gives `Discarded` and no event; the same read twice gives `Unchanged`, the same `rev`, and `changed()` moved both times; a new head gives `Changed { rev: n + 1 }` and one event.
- `mod13_one_step`: over a seeded random run of every method, each view has `rev` equal to its newest event's, and no record's `rev` above it.
- `sta5_seen_is_carried`: `seen` after a read is in `file_content()`; a loaded entry's `seen` makes only later items `activity` in its first event.
- `sta8_loaded_entry_unchanged`: equal in `file_content()` after `mark_sent`, `attempt_failed` and `sweep`; its first refresh keeps the saved source; a confirmed `merged` keeps the saved `seen_at`. `sta8_expired_dropped_on_load`: no event, cursor `<epoch>-0`.
- `sta9_cursor`: the epoch's form; the same seed gives the same epoch, another seed another; too old for another epoch, a `rev` above the counter, and `rev` 0 once 501 events were made (`rev` 1 isn't).
- `sta10_500_events`; `api14_64_waiters` (the 65th is `Limit`, a dropped guard frees one); `ops8_last_waiter_ended` (`None` at first, then the clock's monotonic time at the latest guard's drop).
- `ntf4_a_subscription_keeps_the_pr`: an ended PR past `keep_done` with a `Subscribed` guard survives `sweep`, and so does an `asked` one past its 60 minutes; once the guard is dropped, each survives the `sweep` 59 minutes on and is dropped, with an `untracked` event, by the first one at or after 60 minutes (STA-1's part "a request names a PR"). `untrack` of a `manual` PR with a guard makes it `asked`.
- `ntf2_64_subscriptions`: the 65th `add_subscription` is `Limit` while 64 waiters are also held; a dropped guard frees one; `subscriptions()`, `View::subscriptions` and the PR's `Timing::subscriptions` agree; `last_subscription_ended()` is `None` at first, then the monotonic time of the latest drop. A subscription in `file_content()`: there's none, and `file_changed()` doesn't move for one.
- `ntf5_open_subscriptions`: two guards, one on an open PR and one on a merged one: `open_subscriptions()` is 1; an `apply` that shows the open one merged makes it 0; a guard taken on an open PR makes it 1, and its drop 0 again. `subscriptions()` counts both throughout.
- `ntf2_name_quiet`: `name_quiet` leaves `Timing::named` as it was, and its guard's drop restarts an `asked` PR's 60 minutes as `name`'s does.
- `api15_untracked_record_rev`: a dropped PR's `Untracked::record` has the `rev` of its `untracked` event.
- `api15_untrack_table`: every case of API-15, with `file_changed`. `pol5_demote_unlisted`: in one call, an open `authored` PR in the slice becomes `asked`, leaves `file_content()`, moves `changed()`, and is dropped at the first `sweep` 60 minutes later; a `manual` PR and a merged `authored` one in the slice keep their `source`; an open `authored` PR left out of the slice keeps `authored` and its file entry. `gone_starts_retention`; `gone_on_a_loaded_entry`: the saved entry says `gone` from `seen_at`, no event, `Unchanged`, and the file's content changed.
- `file_changed_only_on_content`: a read that changes only checks doesn't move it.
- `ops9_lines`: tracked, untracked and gone are logged; `testkit::MARKER_TITLE` in a read's title is never in the log.
- `cargo test --features test-endpoint --lib state::store::` and `make verify` pass.

## Traps
- **Retention and the asked hour go by the wall clock, `Timing` by the monotonic one.** Never convert. A wall time ahead of now counts as now: `Time::since` saturates.
- **The methods take `&self`, the guards outlive the call:** keep the state behind an inner `Arc` the guards clone. No lock across an `.await`; a guard's `Drop` takes the lock briefly.
- `changed()` moves on `Unchanged` too: waiters and subscriptions are evaluated after every applied refresh (STA-4). `file_changed()` doesn't.
- The order "of `pr`" is `PrName`'s `Ord`: `o/r#9` before `o/r#10`. Names compare without case; after a rename only the new name is found (MOD-1).
- A log line holds the name, never the title (OPS-9).
- `apply` with `keep_done` 0 must not drop: the waiters see the end first (STA-2).

## Open questions
- **(proposal) values:** 60 minutes, `keep_done`, 200, the two 64s, 500. Built as written, one constant each. Reopening isn't measured (docs/02): built as STA-2 says.
- **`untrack` of a `Loaded` `manual` entry** (API-15 needs the author; there's no record). Build: `asked` while it has a waiter, otherwise dropped with `untracked`; `record` is `None`. Report it.
- **`View::pending` and API-18.** The spec says "found or loaded"; a `First` place taken by a read-through is neither. Build the contract's "`Loaded` and `First`".
