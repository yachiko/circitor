---
id: T25
title: "Poller engine: hot PRs, forced refreshes, reading through, track, the mergeable follow-up"
kind: code
milestone: M2
size: L
status: done
depends_on: [T21]
requirements: [POL-11, POL-14, POL-15, POL-16, API-13]
owns: [src/poller/engine/]
shares: []
---

# T25 · Poller engine: hot PRs, forced refreshes, reading through, track, the mergeable follow-up

## Outcome
The engine serves demand: hot PRs are refreshed every `hot_interval`, `refresh_pr` forces one PR's refresh and shares one in flight, `read_through` and `track` fetch an untracked PR by name ahead of a poll in flight, and an `UNKNOWN` mergeable gets its one follow-up. `PollerHandle` is complete.

## Context
T21 built the loop, the gate, the poll, the error handling and the status, and left three stubs marked `// T25`, an open command enum, and one place where every read is applied. This task fills them, in the same directory. T42 follows, filling the calls T21 left for a PR's run of errors. The routes (T22) call `read_through`, `track` and `refresh_pr`; the waiters (T23) call `refresh_pr` and `nudge`. T28 wires the engine beside this task, on T21's stubs; the harness (T29) is the first to run a waiter against this task's demand side.

## Read first
- `plan/conventions.md`
- `plan/contracts/poller.md`: `PollerHandle`'s doc comments and the three bullets under it (what each call may wait for, what each refusal is), `Refusal`, `Refreshed`, "Inside: the engine". `plan/contracts/state.md`: `lookup`, `reserve`, `release`, `set_manual`, `places_free`, `targets`, `apply`, and "The same PR under a new name". `plan/contracts/github.md`: `Query::lookup`, `LookupAnswer`, `ScriptedGitHub::hold`.
- `src/poller/engine/` as T21 left it: the gate, the function that applies a read, the items and check-page steps, the command enum. Reuse them; don't write a second gate.
- `specs/02-polling.md#the-poll` POL-11, POL-12 ("They're batched"); `#cadence` POL-14, POL-15, POL-16; `#quota` POL-23 (the table's "Forced refreshes, reading through" column); `#errors` POL-19 ("The backoff holds every request"), POL-20, POL-21.
- `specs/01-model.md#identity-and-fields` MOD-1; `#status` MOD-11 ("`gone` needs the same login"); `specs/03-state.md#the-tracked-set` STA-1, STA-3, and `#the-state-file` STA-8 (a loaded PR's first refresh).
- `specs/04-api.md#basics` API-3 (each refusal's message); `#reading` API-7; `#waiting` API-9, API-13; `#changing-the-set` API-15.
- `specs/07-testing.md#what-is-tested` TST-8's rows `forced`, `read-through`, `mergeable-unknown`, `track-untrack`, `tiers`: the scenarios this must later pass.

## Scope
- **Hot refreshes (POL-14's share here).** Add the hot timer to the loop's wait: `cadence::hot_due(...).next`. When PRs are due, refresh them by ID, 40 a request, with no finding; a PR with no record is asked as `RecordAndItems`. Check pages and POL-12's items follow as in a poll, straight after. `nudge` makes the loop look again. Which PRs are hot and when is T09's share.
- **Forced refreshes (POL-15's share here).** `refresh_pr(id)`, in this order: when that PR is in the request in flight, whatever started it, or already waits in the demand queue, the call joins it and returns its result; else `NotSent` when `cadence::may_force_pr` says the window holds, when `policy.demand` is false, in a backoff, or when `Auth::token(true)` gives no token; else one request for that PR. `Applied` or `Failed` is what that request did to this PR. The windows are T09's share; `poll_now` and its `shared` are T21's.
- **A forced poll refreshes every tracked PR (D11, POL-15; decided after T21).** The poll `poll_now` starts (not one it joins, and not an ordinary one) refreshes every tracked PR with a place, whatever `cadence::in_poll` says is due, besides its finding: 40 a request as in any poll, with check pages and items after. Every request still marks its PRs sent at the poll's send time (P10). Build it in the poll T21 wrote, behind one flag the `poll_now` path sets; cadence (T09) is unchanged. `Shared`, the 10 s window and the outcomes stay T21's.
- **Demand goes next (POL-15).** A forced refresh, a read-through and a `track` are sent before the rest of a poll in flight, between its requests, and before a hot refresh that's due. The gate's 1 s still holds.
- **Reading through (POL-16, whole).** `read_through(name)`:
  - `store.lookup` first. A PR with a record: `Ok`, nothing sent. Untracked and `places_free()` at 0: `Limit`, nothing sent (STA-3).
  - Refusals before sending: `Auth` in the `auth` state after `Auth::token(true)`; `RateLimited` with `until` when `policy.demand` is false; in a backoff, `GithubUnavailable` with `until` unless `Backoff::take_read_through` gives this step's one.
  - Send `Query::lookup`. `Found`: fetch further check pages if any, then `store.reserve` just before `store.apply`, because the ID is known only from the answer (source `asked`; a loaded PR keeps its saved source; an ID already tracked under another name is that PR, MOD-1); apply with `ItemsOutcome::Read` and no login; answer its ID. `NotFound`: `NotFound`. `Error { type_ }`: `GithubUnavailable`, the message naming the type and the token source. A failed request: `GithubUnavailable`.
  - Nothing but `Found` tracks the PR. Two calls for one name share one request.
- **`track` (API-15's poller side).** With a record: `store.set_manual`, nothing sent, also at the limit. A loaded PR with no record: `set_manual` first, so the saved source is `manual` even when GitHub is down (`contracts/poller.md`), then read it through. Untracked: as `read_through`, reserving as `manual`.
- **API-13's share here: what a waiter gets from the poller.** Its PR is hot (above), and its arrival refresh is `refresh_pr`, sent ahead of a poll in flight. What follows the arrival goes by the `Policy`: a hot refresh every `hot_interval`, twice that slowed, `idle_interval` in reserve, none when capped, rate-limited, in a backoff or in `auth`. The arrival refresh itself is `NotSent` in the last three; capped, it's still served, in the 30 s window (POL-23). Calling `refresh_pr` and `nudge` at the right moments, and answering a wait that may only time out, are T23's share.
- **The follow-up (POL-11's share here).** After a refresh of any kind is applied whose read said `UNKNOWN` for an open PR, ask `cadence::wants_follow_up` with the PR's fresh `Target` (`first_since_start`: this PR hadn't been refreshed since the engine started). Those PRs are refreshed once more 3 s after that answer, together in one request, as `Record`. **A follow-up's own reads never ask again.** Not sent in a backoff, nor when the policy forbids it. `Target::mergeable_unknown` is the read's value, not the record's: MOD-8 may keep `yes` there.
- **Errors in these requests.** They go through T21's handling: a failing node is `store.attempt_failed` and the rest is applied. **Pass every node result of a hot, forced or follow-up refresh through T21's per-node call, marked as not a poll**: T42 needs that to keep such a `NOT_FOUND` out of a PR's run (MOD-11). A read-through that fails during a backoff leaves the backoff's step as it was. A whole-request failure outside a backoff starts one (POL-19). A 401 gets POL-20's one repeat.
- 3 s is one named constant (proposal).

## Interfaces
- **Consumes:** everything T21 built in `poller::engine`; `cadence::{hot_due, may_force_pr, wants_follow_up, batches}`; `Backoff::take_read_through`; `Auth::token`; `Store::{lookup, reserve, release, set_manual, places_free, targets, apply}`.
- **Provides:** the rest of `impl PollerHandle for Engine`: `read_through`, `track`, `refresh_pr`, and `nudge` with an effect. No signature changes.

## Out of scope
- The gate, the poll (but for D11's flag above), `poll_now`, the tier, the tick, the status, the backoff's own tries: T21. Change them only where this brief says a hook goes; report anything else.
- What a PR's errors add up to (`gone`, `pr_failing`, the `pr error` line): T42. Leave its three calls empty.
- Deciding when a route reads through or forces (`fresh`, a wait's arrival, a loaded PR's wait held without one): T22, T23.
- `untrack`, the `asked` PR's 60 minutes, source strength: T15. The cadence and window arithmetic: T09.
- HTTP codes and exit codes for each refusal: T22, T10. The scenarios: T31, T32. TST-6's hot figures: T34.

## Acceptance
Unit tests in `src/poller/engine/`, set up as T21's: `ScriptedGitHub`, the real `Store` (waiters through `store.add_waiter`, naming through `store.name`), `TestClock`, OPS-2's defaults.
- `d11_forced_poll_refreshes_every_pr`: in the idle tier, a `poll_now` 60 s after an ordinary poll sends the finding and every tracked PR (three PRs: one request with all three); an ordinary poll 60 s after that one asks none of them; a fourth PR beyond 40 goes in a second request.
- `pol14_hot_between_polls`: with a waiter on one tracked PR in the active tier, after a poll sent at t that PR alone is asked at t + 10 s and t + 20 s, by ID, with no finding; the poll at t + 30 s counts as its refresh, so the next is at t + 40 s. When the waiter is dropped and `nudge` called, they stop.
- `pol14_named_while_pending`: a PR named now with status `checks_pending` is refreshed every 10 s for 120 s, then not; a `ready` one never.
- `pol14_many_hot`: 41 hot PRs are two requests.
- `pol15_window`: two `refresh_pr` 4 s apart send one request; the second is `NotSent`. A third, 5.001 s after the first was sent, sends.
- `pol15_shared`: with the request held (`ScriptedGitHub::hold(activity)`), three `refresh_pr` for one PR send one request and all return `Applied`; when it fails, all return `Failed`.
- `pol15_goes_next`: with 200 tracked PRs, a `refresh_pr` for a PR of the poll's third batch, arriving during the first request, is the next entry in `calls()`; the poll's later batches follow, without that PR.
- `pol15_not_sent`: `NotSent` at once in a backoff, in `auth`, while exhausted; when capped it's sent, and the window is 30 s.
- `pol16_read_through`: an untracked name sends one `lookup`; the PR is tracked `asked` with one `tracked` event and no `activity`; the call returns its ID. A second call sends nothing.
- `pol16_refusals`: `NotFound` for a missing PR, nothing tracked; `GithubUnavailable` naming `FORBIDDEN` and the token source for that error; `Limit` with 200 tracked, `calls()` unchanged; `Auth` in `auth`; `RateLimited` with `until` while exhausted. No message holds a title.
- `pol16_one_per_backoff_step`: in a backoff, the first `read_through` of a step is sent, the second answers `GithubUnavailable` with `until`; after the next failed try, one more is sent. One that succeeds ends the backoff and a poll follows at once.
- `pol16_loaded_and_renamed`: a loaded PR read through keeps its saved `manual` source; a lookup whose answer has the ID of a PR tracked under another name returns that ID, with no new place taken.
- `api15_track`: `track` of an untracked PR sends one `lookup` and leaves it `manual`; a second `track` sends nothing, also with 200 tracked; `track` of an untracked PR at 200 is `Limit`; `track` of a loaded PR while GitHub is unreachable answers `GithubUnavailable`, and the store's file content already says `manual`.
- `api13_waiter_cadence`: with a waiter, the PR is asked every 20 s when slowed and every 300 s in reserve. Capped, no hot refresh is sent, and `refresh_pr` is still sent once per 30 s. Exhausted, nothing is sent and `refresh_pr` is `NotSent`.
- `pol11_follow_up`: a refresh that reads `UNKNOWN` for a PR with a waiter is followed 3 s later by one request for it; that one reading `UNKNOWN` again asks nothing more until the next hot refresh, which gets its own. At start, three PRs reading `UNKNOWN` in the first poll share one follow-up. An ended PR, a PR with no waiter that wasn't named in the last 60 s and isn't new since start, a capped daemon, and one in reserve get none. A record kept at `yes` by MOD-8 while the read says `UNKNOWN` still gets one.
- `pol21_hot_node_error`: a `NOT_FOUND` on a hot refresh marks that PR's attempt failed, sets `error.type`, and leaves the other hot PRs of the request applied.
- T21's tests still pass unchanged.
- `cargo test --features test-endpoint --lib poller::engine::` and `make verify` pass.

## Traps
- **Sharing is by PR, not by caller.** A PR that's in the request in flight, a poll's included, is joined, not refused by the 5 s window: the window would hand back what's in memory a moment before the fresh answer lands. A PR in a poll's batch that isn't sent yet isn't joined: its forced refresh goes next, and `in_poll`, asked again, then leaves it out of that batch.
- **A caller may go away.** Each async method can be dropped half-way: the request it started still runs and is applied, and the other callers sharing it still get their result. Answer through a channel the loop owns, never by running the request in the caller's future.
- **Every refresh, of any kind, moves `last_sent`** (`store.mark_sent`): hot refreshes fall between polls only because a poll's send counts. A hot, forced or follow-up request, or a read-through, marks with its own send time, also between a poll's requests; only a poll's own requests take the poll's (plan decision P10, `contracts/poller.md`).
- **The follow-up never chains.** Mark its request so the apply hook skips it; the next regular refresh may ask again.
- **Check the limit before sending**, and again at `reserve`: a poll can take the last place while the lookup is in flight. Then answer `Limit` and apply nothing. The number is `state::MAX_PLACES`; don't write 200.
- **`Busy` as in T21:** kept across `send`, dropped before any timer, the gap and `Auth`'s reads. A caller's future never holds one: the loop does the work.
- A refusal's message is built here from API-3's wording: the error `type`, the token source, a time. Never GitHub's text.
- `read_through` for a `First` PR (found, its first refresh in flight) joins that refresh rather than sending a second.

## Open questions
- **POL-11, POL-14, POL-15 and POL-16 are (proposal)**; how long `mergeable` stays `UNKNOWN` on a large PR isn't measured (docs/02 §6 item 7), nor what a rename does to a node ID (MOD-1). Build the spec as written.
- **Readings this brief makes, to report if they don't fit:** the follow-up's 3 s count from the answer; PRs of one answer share one follow-up; a failed read-through in a backoff doesn't move its step; the 5 s window is looked at only after a request in flight that holds the PR.
- If T21's code has no clean place for the demand queue or the apply hook, report it rather than restructuring the loop.

## Evidence
- **2026-11-26**, branch `t25-engine-demand`, executor on **Opus, medium**; reviewed and merged by the orchestrator.
- `src/poller/engine/` (`mod.rs`, `run.rs`, `poll.rs`, `gate.rs`, new `demand.rs`, `hot.rs`, `tests/demand.rs`, `tests/mod.rs`); no public signature changed. `cargo test --features test-endpoint --lib poller::engine::`: 55 passed (T21's 39 unchanged); run 10 times by the orchestrator after rebasing: 0 failures. `make verify` passes on main after the merge (485 lib tests), the macOS check included.
- **Beyond the brief's hooks, reported and accepted** (inside T25's own directory; T21's tests all pass): `poll.rs`'s check-page and items loops became `pages_and_items`/`fetch_pages`/`fetch_items` for reuse; `PollState` gained `poll` and `every` (D11's flag), and outside a poll a step marks its own send time and skips `between_requests`; every `attempt_failed` goes through `failed_pr`, which also answers joined callers; `gate.rs` gained `flight_begin` (a `refresh_pr` arriving during the token read or the 1 s gap joins the request) and a `keep_backoff` guard; the loop's order is demand, `poll_now`, the poll, follow-ups, hot refreshes.
- **Orchestrator decisions (by recommendation, owner's rule of 2026-11-26):** a read-through's whole-request failure maps as POL-16 reads (a 401 to `auth`, a rate limit to `rate_limited`, else `github_unavailable`), over the brief's "only `GithubUnavailable`"; `d11_forced_poll_refreshes_every_pr` uses `asked` PRs, `interval` 60 s and a client request after the forced poll, since the brief's "ordinary poll 60 s after, in the idle tier" can't happen with the defaults (the brief's slip).
- Readings: the 3 s follow-up counts from when the read is applied (the items answer for a held read); PRs of one answer share one follow-up; a failed read-through in a backoff doesn't move it; the 5 s window is checked only after looking for the PR in flight; `refresh_pr` of an untracked ID is `NotSent`; after `stop`, `refresh_pr` is `Failed` and a read-through `GithubUnavailable`; a `track` joining another's read-through calls `set_manual` once it answers `Ok`.
