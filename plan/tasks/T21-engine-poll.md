---
id: T21
title: "Poller engine: the loop, the gate, the poll, errors, the tick"
kind: code
milestone: M1
size: L
status: todo
depends_on: [T08, T09, T13, T15]
requirements: [POL-2, POL-4, POL-5, POL-7, POL-8, POL-10, POL-12, POL-13, POL-19, POL-20, POL-21, POL-22, POL-23, POL-24, POL-25, POL-26, STA-3, API-18, API-4, API-16, OPS-9:logged, OPS-9:circitor-log-debug-adds-one-line-per-request-to, U2]
owns: [src/poller/engine/]
shares: []
---

# T21 · Poller engine: the loop, the gate, the poll, errors, the tick

## Outcome
`poller::engine::Engine` runs: one request at a time through the gate, a poll that finds, refreshes, pages checks and fetches new items and applies them to the store, every error class handled as 02-polling says, the tier's cadence, the tick, the reaction to a sleep, and the status the API reports. `poll_now` works; the per-PR demand calls are stubs for T25.

## Context
This is where the poller's parts meet. T08 gives `Quota` and `Backoff`, T09 the cadence functions, T13 `Auth`, T15 the store, T04 the clock, the logger, `Warnings` and `SleepWatch`; `GitHub::send` is behind the trait (T07 is the real one). The tests' fakes, `ScriptedGitHub` and `FakeTokenSource`, are T43's, reached through T13 and T15. The routes and waiters (T22, T23) and the daemon (T28) see only `PollerHandle`. Two tasks then build in this same directory: T25 adds demand, and T42 adds what a PR's errors add up to (`gone`, `pr_failing`, its log line). T26 runs this engine against the fake GitHub.

## Read first
- `plan/conventions.md`
- `plan/contracts/poller.md`, all of it: the outside trait is what you implement, the inside seams are what you drive; "`client_request` in the `auth` state"; who builds what under "Inside: the engine". `plan/contracts/state.md`: `Store`'s methods under "the tracked set" and "refreshes", `Target`, `Timing`, `Class`, `Applied`, `MAX_PLACES`, and "Semantics". `plan/contracts/github.md`: `Query`, `Answer`, `Failure`, the fakes. `plan/contracts/model.md`: `PrRead`, `ChecksRead::append_page`, `ItemsOutcome`, `ReadMeta`. `plan/contracts/clock.md`: all. `plan/contracts/log.md`: the event words, `Warnings`.
- `specs/02-polling.md`, the whole file: this task is its centre. The other tasks' shares are under Scope.
- `specs/01-model.md#identity-and-fields` MOD-2 ("A newer read wins"; `error`); `specs/03-state.md#the-tracked-set` STA-3, STA-4.
- `specs/04-api.md#basics` API-3 (the messages), API-4, API-5 (the warnings table); `#changing-the-set` API-16; `#starting-up` API-18.
- `specs/06-operations.md#the-daemons-life` OPS-9; `specs/00-overview.md` U2.
- `docs/02-github-api.md` §2 (what a request costs), §3 (the open list has no lag), §5 (partial errors).

## Scope
- **The loop.** One task. It waits on the command channel, on the sleep watch, and on one timer set to the earliest of: the next poll, the backoff's next try, `Auth::due`, the tick. **U2:** polling is the only source of data.
- **The gate (POL-2).** Every request goes through one function: the token from `Auth`, the 1 s since the last request ended, a request number, the send times, `clock::timeout` of 20 s around `GitHub::send`. It builds each read's `ReadMeta` with that request's own send times, calls `store.mark_sent` (next bullet), counts the cost into `Quota`, and writes OPS-9's `debug` line (`request`, with its kind, cost and duration).
- **A poll's send time (plan decision P10, `contracts/poller.md`).** Every request of a poll marks its PRs as sent at the poll's own send time, its first request's: `store.mark_sent(ids, poll_sent)`. A request outside a poll marks with its own. `ReadMeta` keeps each request's real send times: settling and "a newer read wins" (MOD-2) go by those. Without this a PR first refreshed in a poll's second request is due again before the next poll's second request, and every poll takes two requests for good (POL-13, TST-6).
- **What each result feeds.** An answer: `Quota::spent` and `observed` from its `rate` (no `rate`: count 1), `Quota::answered`, `Backoff::succeeded`, `Auth::accepted`. `Unauthorized`: **POL-20's share here**, below. `Exhausted`, `Secondary`: `Quota` (**POL-25's share here**: feeding it and sending nothing meanwhile; recognising is T07's, the timing T08's). `Unavailable` or the timeout: POL-19.
- **The poll (POL-4).** Its first request is the finding's first page and the first 40 due PRs (`cadence::in_poll`, `listed: None`). Then, each its own request, in this order: further finding pages; further batches; PRs the list shows that weren't refreshed yet (the second `in_poll` pass, below); the first refresh of newly found PRs; check pages; items. PRs with no record (`Loaded`, newly found) are asked as `RecordAndItems`. A found PR whose ID is tracked is that PR (MOD-1).
- **Found PRs (POL-5's first part, POL-7, and STA-3's share here).** Filter by `owners` without case, after finding. `store.reserve` a place for each new one, newest `updated_at` first, while `places_free()` allows; the rest are left out and `pr_limit` is raised, cleared by a poll that leaves none out. Stop paging once every place is taken. A first refresh that fails gives the place back (`store.release`). The count of places and `Limit` are T15's; the read-through's check is T25's. **POL-5's second part:** once, at the poll's end, and only when the finding was read to its last page, call `store.demote_unlisted(unlisted_open)` (`contracts/state.md`). It holds the `authored` PRs whose refresh **this poll applied and showed open**, and which the finding, after the owners filter, didn't list: so a PR transferred to another owner is demoted. **A PR merged since the last poll is unlisted and still reads open in the store until its refresh is applied:** it isn't passed, and keeps `authored`. The demotion itself is T15's.
- **POL-10's share here.** The second `in_poll` pass, with the open list once read, so a PR the list shows is refreshed in that poll as an open one; and refreshing by ID, so a PR that left the list is still refreshed and shows `merged` or `closed`. Which PRs are due, and the periods, are T09's.
- **Batches (POL-7).** `cadence::batches`: 40 a request.
- **Check pages (POL-8).** While a read has `next_page` and fewer than 300 items, fetch `ChecksPage` and `append_page`; apply once every page answered. A failed page: `store.attempt_failed(id, None, …)`, nothing applied.
- **New items (POL-12, whole but for T05's and T25's shares).** When `store.counts_rose`, hold that PR's refresh, fetch `Items` for all such PRs of the poll in batches of 40 straight after, then apply with `ItemsOutcome::Read`; when its part fails, with `ItemsOutcome::Failed`. Which items are new, and `more`, are T05's. T25 reuses this for hot and forced refreshes.
- **Applying.** One function applies a read through `store.apply`, with the poll's `viewer.login`. PRs of one response are applied in the order of `pr` (MOD-15). Keep it the one place: T25 hooks the follow-up there.
- **Whole-request failures (POL-19's share here; POL-24's first sentence).** Abandon the poll; `store.attempt_failed(id, None, …)` for the PRs of the failed request and of the batches not sent; `Backoff::failed`. The backoff's try is a poll. `problem begin code=github_unavailable` with the status or the failure's kind at the first failure, `problem end` at the recovery, nothing per retry. After any success during a backoff, a poll follows at once. Requests of the poll that answered stay applied. T08 has the times, T25 the read-through.
- **One node's error (POL-21's and POL-24's shares here).** For `NodeAnswer::NotFound` and `Error { type_ }`: the other nodes of the response are applied, and this one is `store.attempt_failed(id, Some(type), at)`, with `NOT_FOUND` as the type for `NotFound`. **Nothing more.** What a PR's errors add up to (the run that makes it `gone`, `pr_failing`, the `pr error` log line) is T42's.
- **A place for T42.** Put three calls where it can fill them, in a file of their own with empty bodies marked `// T42`: one per node result (the PR's ID, the kind of request: a poll or not, and applied, `NotFound` or `Error` with its type, with the request's send times); one per finished poll (its login, or none when the finding failed; which tracked PRs it refreshed and how each answered); one per tick.
- **Obeying the quota (POL-23's share here).** Before a poll or `poll_now`, take `Quota::policy`: no polls when it says none; nothing at all while `demand` is false. An answer with `remaining` 0 is applied, then the poll stops. At the tick, raise and clear `quota_capped` and `quota_reserve`, each on its own condition (`Quota::capped`, `in_reserve`). The table itself is T08's; demand under it is T25's.
- **The tier (POL-13's share here).** **The engine starts idle:** `cadence::tier` gets `None` until the first `client_request` (`contracts/poller.md`). `idle_since()` answers the start time until then, for OPS-8 only; never feed it to `tier`. `client_request` records the time, never waits, and wakes the loop: the tier turns active, a poll starts if `cadence::poll_due` has passed, and one `tier` line marks each change. The arithmetic is T09's; the idle exit is T28's.
- **POL-20's share here.** A 401: `Auth::unauthorized`, then the request once more with `Retry`'s token, once; `InAuth` ends it. In `auth`, tracked PRs are answered from the store, `daemon.problem` is `auth` (below) and `first_poll` turns `Unanswered(Auth)` when nothing was answered. When to read, the `auth` state and its message are T13's; a read-through's `auth` error is T25's.
- **`client_request` in the `auth` state** (`contracts/poller.md`). When the last token read ended 5 s ago or more (`Auth::last_read_ended()`: the loop publishes it after every read, and `client_request` reads that copy, never `Auth`), `client_request` itself sets a `first_poll` of `Unanswered` back to `Running` before it returns; the loop then asks `Auth::token(true)`. No token: `Unanswered` again at once. A token: a poll, then `Finished`. The handle shares what that needs with the loop: the `auth` flag and the published `last_read_ended`.
- **Sleep (POL-22's share here).** React when `SleepWatch`'s total rises, whoever called `check`: a `sleep` line with its length, `store.mark_slept`, `Backoff::woke`, and a poll at once if a waiter is held or a client asked within `idle_after`. Detection is T04's; waiters' deadlines are T23's.
- **The tick (POL-26's share here).** Every 10 s on the monotonic clock: `SleepWatch::check`, the quota warnings, `store.sweep`, T42's call, the status. It sends nothing.
- **`poll_now` (API-16).** `Shared` with cost 0 inside `cadence::may_force_poll`'s window. **One arriving while a poll is in flight joins it:** it answers when that poll is done, `Shared`, cost 0 (`contracts/poller.md`). Else `auth`, `github_unavailable` or `rate_limited` at once, cost 0, when that problem holds; else a whole poll, `polled` or the problem that stopped some of it, with the points of every request sent. It runs when capped, in the 30 s window.
- **`first_poll` (API-18's share here).** `Running` until the finding and every first refresh of a found or loaded PR are applied or have failed; then `Finished`, or `Unanswered` with the problem's `Refusal` when no request was answered (POL-19's last part), until one is. Waiting 10 s and `starting` are T22's.
- **`status` (API-4's share here).** Publish a `PollerStatus` before every `.await`: the tier, `last_poll_at`, `last_poll`, the problem by API-4's precedence with API-3's message and `until` (`Quota::resume_at`, or `Backoff::next_try_wall`), the cost, GitHub's last `remaining` and reset, the login, the token source and `gh_path`. The envelope is T22's.
- **OPS-9's "Logged" list, this task's rows,** with `contracts/log.md`'s words: `sleep`, `tier`, `problem begin` and `problem end` for a backoff and a rate limit, `login` at the first poll and when it changes; and `request` at `debug`.
- **Stubs for T25:** `read_through` and `track` answer `GithubUnavailable`, `refresh_pr` answers `NotSent`, each marked `// T25`. Leave the command enum open for them, and take pending demand **between a poll's requests**.

## Interfaces
- **Consumes:** `poller::limits::{Quota, Backoff}`, `poller::cadence::*`, `poller::auth::Auth` (`Auth::new(source, clock, logger)`), `state::store::Store`, `github::{GitHub, Query, …}`, `clock::{SharedClock, Activity, SleepWatch, timeout}`, `log::Logger`, `status::Warnings`, `config::Config`.
- **Provides:** `poller::engine::{Engine, EngineDeps}` and `impl PollerHandle for Engine`, as `plan/contracts/poller.md` declares them; and, inside the engine, the three calls T42 fills and the apply function T25 hooks.

## Out of scope
- **A PR's run of errors, `store.mark_gone`, `pr_failing`, the `pr error` line: T42.** Here a failing node is marked failed and left.
- Hot refreshes, `refresh_pr`, `read_through`, `track`, sharing a refresh in flight, the `UNKNOWN` follow-up: T25.
- The arithmetic of limits, cadence and the token's life: T08, T09, T13. Fix nothing there: report it.
- The merge, revisions, events, retention, demotion: T05, T15. Queries and decoding: T07.
- Routes, the envelope, `starting`, waiters and their deadlines: T22, T23. Start order, stopping the daemon, the idle exit: T28.
- The engine against the fake GitHub server: T26. TST-6's figures: T34.

## Acceptance
Unit tests in `src/poller/engine/`: `ScriptedGitHub`, the real `Store`, `TestClock`, `FakeTokenSource`, a `BufferSink`, OPS-2's defaults. Each `#[tokio::test]`. Log lines are matched by their event word.
- `pol2_gate`: a poll's two requests are sent at least 1 s apart; with one held (`ScriptedGitHub::hold(activity)`), `calls()` stays at 1 whatever arrives; a held request is given up after 20 s of the clock.
- `pol4_first_poll`: two found PRs: request 1 finds, request 2 asks both as `RecordAndItems`; both are tracked `authored`, with `tracked` events in `pr` order. A failed first refresh frees the place and the PR is found again at the next poll. A loaded PR is in request 1 as `RecordAndItems`.
- `pol5_owners`: with `owners = ["Acme"]`, `acme/x#1` is tracked and `other/x#1` isn't; a tracked `authored` PR now listed under `other/` becomes `asked`. No PR is demoted when page 2 of the finding fails.
- `pol5_merged_keeps_authored`: a tracked `authored` PR merged between two polls is unlisted at the next one, refreshed by ID in it and read `merged`: it keeps `source: authored`, and `store.file_content()` still holds it as `authored`. An unlisted PR whose refresh fails in that poll keeps `authored` too.
- `pol7_batches_and_pages`: 41 tracked PRs are two requests (40, 1); 150 found are two finding pages, the second with the first's cursor.
- `sta3_places`: with 198 places taken and 5 new PRs found, the 2 newest by `updated_at` get the places and `pr_limit` is raised; with every place taken, page 2 isn't asked; a later poll that leaves none out clears the warning.
- `pol8_check_pages`: 187 checks are read over two pages and applied once; 400 stop at 300 items; a failed second page applies nothing, marks the attempt failed and leaves `error` as it was.
- `pol12_items`: a raised count sends one `Items` request straight after, and one event carries the items; a failed part applies the refresh with the old counts, and the next refresh asks again; 41 PRs with raised counts are two requests.
- `pol13_one_request_per_poll` (P10): three found PRs, the active tier kept by `client_request`. The first poll is two requests; the next three, with the world unchanged, are one request each in `calls()`, each holding the finding and all three PRs. After the first poll a found PR's `Timing::last_sent` is the first request's send time, and its `Timing::fetched` the second's.
- `pol10_listed_as_open`: a tracked PR whose record is `merged`, sent 60 s ago, is refreshed in the poll whose finding lists it.
- `pol19_backoff`: an `Unavailable` abandons the poll and marks its PRs' attempts failed; tries follow 5, 10 and 20 s apart, reaching 300 s; the log has one `problem begin` and, at the recovery, one `problem end`; a poll follows the first success at once. A first poll that fails is `Unanswered` with `GithubUnavailable` and an `until`. A status 418 does the same (POL-24).
- `pol20_repeat`: a 401 reads the token again (`reads() == 2`) and sends the request once more; a second 401 gives `problem: auth`, and a first poll `Unanswered` with `Auth`.
- `pol20_client_request_in_auth`: with no login and `first_poll` `Unanswered`, a `client_request` 5 s after the last read sets it to `Running` before it returns, and it's `Unanswered` again once the read ends; one 2 s after reads nothing and changes nothing. After the source gives a token, the next `client_request` leads to a poll and `Finished`.
- `pol21_node_errors_stay_with_the_pr`: of three nodes, one `NotFound` and one `Error { "FORBIDDEN" }`: the third is applied; the two keep their values, are stale by `cadence::stale`, and carry `error.type` `NOT_FOUND` and `FORBIDDEN` (POL-21, POL-24). Neither becomes `gone`, however long it lasts: that's T42.
- `pol25_limits`: after `Exhausted` with a reset 20 minutes ahead nothing is sent until then, `problem` is `rate_limited` with that `until`, and `poll_now` answers `RateLimited`, cost 0, at once; an answer with `remaining` 0 is applied first. After `Secondary` with no `Retry-After`, nothing for 60 s. One `problem begin code=rate_limited` and one `problem end`.
- `pol23_policy_obeyed`: with 481 points spent, polls are 60 s apart; `quota_capped` is raised at 600 points and no poll is sent, yet `poll_now` runs; `quota_reserve` follows `remaining` under 500, and both can be raised at once; each clears on its own condition at a tick.
- `pol13_tiers`: with no `client_request` the engine is idle from the start: `status().tier` is `Idle` and the second poll is 300 s after the first. Polls are 30 s apart while `client_request` keeps coming, 300 s apart after 600 s without one; a request in the idle tier starts a poll at once when the last is over 30 s old; each change logs one `tier` line.
- `pol22_sleep`: after `sleep_wall_only(2 h)`, the next tick or `client_request` notices it: one `sleep` line with its length, every record slept, a running backoff restarted at 5 s, and one poll when a client asked within `idle_after` or a waiter is held, none otherwise. `sleep_both(2 h)` sends exactly one poll, not one per missed interval.
- `pol26_tick`: between two polls of the idle tier the ticks send nothing; an ended PR past `keep_done` is dropped at a tick.
- `api16_poll_now`: `Polled` with the sum of the costs; a second within 10 s is `Shared`, cost 0; one arriving during a held poll doesn't answer until that poll is done, then `Shared`, cost 0, with no request of its own; during a backoff, `GithubUnavailable` at once.
- `api18_first_poll`: `Running`, then `Finished` once the first refreshes are applied.
- `api4_status`: `auth` wins over `rate_limited`, over `github_unavailable`, over `quota_low`; `until` and `last_poll_at` follow; `status()` answers while the first token read (8 s) is still running.
- `ops9_log`: with `debug`, one `request` line per request with its kind, cost and duration; a `login` line at the first poll and when the login changes; no line holds a title given to `pr_read`.
- `engine_stops`: `stop()` returns with a request held, and nothing is sent after.
- `cargo test --features test-endpoint --lib poller::engine::` and `make verify` pass.

## Traps
- **`Busy`, by `contracts/clock.md`'s rule.** Take one first thing after each wake. Keep it across `GitHub::send` and its 20 s timeout: a request in flight is the engine's work, and a scripted hold excuses itself. **Drop it before every other wait:** the timer, POL-2's 1 s gap, the channel, and `Auth::token`, `unauthorized` and `periodic`, whose read may wait on a timer. A guard held across one of those stops `advance` from ever returning.
- **Timers are absolute.** Compute each due time from the last send (`cadence::poll_due`), sleep until the earliest, and build the sleep again each turn: a 2 h jump then fires one poll.
- **Two send times, kept apart.** `ReadMeta` takes the request's own; `mark_sent` takes the poll's for every request of a poll (P10). Using the request's in `mark_sent` brings back two requests a poll for good; using the poll's in `ReadMeta` breaks MOD-2's order. Ask `store.targets()` and `in_poll` afresh before each later request.
- **`demote_unlisted` goes by what this poll applied,** not by the store's state before it: collect the PRs it refreshed and read open, and call once at the end.
- **A sleep can be noticed by the API's `check`, not yours**: yours then returns `None`. Go by the watch.
- **`client_request` is called from a route's task, not from the loop.** What it reads and sets (`first_poll`, the times) is shared state behind a short lock or atomics; it never awaits and never takes the store's lock for long.
- **`Auth::token` and `send` can each take many seconds**: `client_request` and `status` must not wait for them. Publish the status first.
- Log and warn with the error's `type` and the status only: never a title, never GitHub's text (OPS-9).
- `stop()` must drop a request in flight and a running token read: abort the loop's task and wait for it.
- `Policy.poll_every` being `None` doesn't refuse `poll_now`: only `demand: false`, a backoff and `auth` do.
- Don't start T42's work "while you're there". Its three calls are empty here; a gone PR, a warning for a failing PR and its log line are tested in T42.

## Open questions
- **Nearly every rule is a (proposal)**; POL-25's answers are `assumed` and a 5xx isn't measured (docs/02 §5, §6). Build the spec as written.
- **Readings this brief makes, to report if they don't fit:** an answer without `rateLimit` counts 1 point; an abandoned poll marks the PRs it was about to refresh; the backoff's try is a poll.
- **"At least its period ago" is exact** (POL-13). P10 keeps polls to one request: a PR is due again one period after the poll that refreshed it, whichever request carried it. If the steady state still shows two requests a poll, report it.
