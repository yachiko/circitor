---
id: T09
title: "Poller cadence: tiers, periods, hot PRs, the forced-refresh windows and staleness"
kind: code
milestone: M1
size: M
status: todo
depends_on: [T02]
requirements: [POL-10, POL-13, POL-14, POL-15, POL-11, MOD-16, NTF-5]
owns: [src/poller/cadence/]
shares: []
---

# T09 · Poller cadence: tiers, periods, hot PRs, the forced-refresh windows and staleness

## Outcome
`poller::cadence` answers, as pure functions over the store's `Target`s: which tier, which PRs are hot, each PR's period, what a poll built now refreshes, when the next poll and hot refresh are due, whether a forced refresh may be sent, whether an `UNKNOWN` gets a follow-up, and whether a record is stale (MOD-16).

## Context
T02 declares these functions with empty bodies, and the types they read: `Target`, `Timing` and `Class` from the store, `Policy` from the poller. The engine (T21, T25) asks them before it builds each request. The routes and the waiters (T22, T23) call `stale` when they build an answer. Nothing here has a clock or I/O: the caller passes `now`.

## Read first
- `plan/conventions.md`
- `plan/contracts/poller.md`: "Inside: cadence" (the signatures), `Tier`, `Policy`. `plan/contracts/state.md`: `Class`, `Timing`, `Target`, and "Don't assume". `plan/contracts/model.md`: `Record`, `Status`, `PrState`, `Source`. `plan/contracts/config.md`: `Config`.
- `specs/02-polling.md#the-poll` POL-10, POL-11; `#cadence` POL-13, POL-14, POL-15; `#quota` POL-23 (what a `Policy` means; T08 computes it).
- `specs/01-model.md#revisions-and-events` MOD-16; `specs/03-state.md#the-tracked-set` STA-1 ("A request names a PR").
- `specs/08-notify.md#while-one-is-open` NTF-5: what a subscription changes in the cadence, and what it doesn't.
- `specs/07-testing.md#what-is-tested` TST-6: the figures these rules must add up to.

## Scope
- **`tier` (POL-13's table).** Active while a client request came less than `idle_after` ago on the monotonic clock; idle otherwise, and with `None`.
- **`last_ask` (POL-13's part "a subscription on an open PR keeps the active tier"; NTF-5's share here).** What the engine passes to `tier` as its `last_request`: `now` while `open` is above 0; otherwise the last client request, `None` when there was none. So the tier is active while a subscription on an open PR is held, whatever the last client request, and **once none is, the tier follows the last client request** (POL-13, NTF-5): an end doesn't count as a request. `open` counts only subscriptions on open PRs (the store's `open_subscriptions`, T15); the engine passes it (T21).
- **`hot` (POL-14's condition).** A waiter, whatever the status; or named less than 120 s ago while the status is `checks_pending` or `unknown`. No status (no record yet) and no waiter isn't hot. **It never reads `timing.subscriptions`** (NTF-5), and `timing.named` isn't set by `notify`'s requests (the store's `name_quiet`): STA-1 states that rule once (the notify routes name their PR for its 60 minutes only, never for POL-14's 2 minutes or POL-11's 60 s), and POL-14 and POL-11 cite it. Nothing more is asked of this function.
- **`period` (POL-10's table, its part "a PR's period"), the first row that applies.** Hot is `hot_interval`; `Class::Open` with `subscribed` is the tier's, whatever its `source`; `Class::Ended` or `Source::Asked` is `idle_interval`; else the tier's `interval` or `idle_interval`. **Always the configured values**, never slowed ones.
- **`stale` (MOD-16, whole).** Never for a `gone` record. True with `timing.attempt_failed` or `timing.slept`; or when `timing.fetched` is more than twice the PR's period ago, the period taken from the record (`state`, `source`), `hot`, `timing.subscriptions` and the tier. The flags are set by T15 and T21; `record.stale` is set by whoever builds an answer (T22, T23).
- **`poll_due` (POL-13: send times).** `last_poll_sent + policy.poll_every`; with no poll sent yet, `Mono::ZERO` (due at once); `None` when the policy has no polls.
- **`in_poll` (POL-10, POL-13's part "every cadence runs from send times").** A `Loaded` PR and one never sent are always due. A `First` PR never is: its first refresh is the engine's own request. Any other is due when `now - timing.last_sent >= period`. With `listed`, a PR in the list is due as an open, tracked one: its tier's period, whatever its record says (POL-10's second sentence). A PR that left the list is still returned by its own period: refreshing goes by ID (POL-10's first). It takes no `Policy`: a quota state changes when a poll is sent, never what a sent poll carries.
- **`hot_due` (POL-14).** Of the hot PRs, bar `First` ones: `now` holds those whose `last_sent` is `policy.hot_every` or more ago, or never; `next` is the earliest due time still ahead. With `hot_every` `None`: nothing, `None`.
- **`may_force_pr`, `may_force_poll` (POL-15's two windows).** True when the last send is more than the policy's window ago, or there was none. Exactly at the window is still inside it. They don't read `policy.demand`: the caller does. **`may_force_poll` takes the send time of the last forced poll**, not of the last poll of any kind (POL-15: "only when the last forced poll was sent more than 10 s ago"); `may_force_pr` takes the PR's last refresh of any kind.
- **`wants_follow_up` (POL-11's conditions).** True when the PR is `Class::Open`, `target.mergeable_unknown` is set, the policy allows it, and it has a waiter, or was named less than 60 s ago, or `first_since_start`. `mergeable_unknown` is what the read last applied said, whatever MOD-8 kept in the record (`contracts/state.md`): POL-11 goes by what the refresh returned.
- **`batches`.** Groups of at most 40, in order: a helper for POL-7, which is T21's.
- **Shares.** Of POL-10, POL-13, POL-14, POL-15, POL-11 and NTF-5 this task has the arithmetic. Reading the store's count of subscriptions and passing `last_ask`'s answer to `tier` is T21's; the idle exit (NTF-5, OPS-8) T28's; keeping the notify routes from naming their PR T47's. Sending polls and moving the tier on a request are T21's; hot and forced refreshes, sharing, the queue and the follow-up request are T25's; T26 proves POL-13 end to end.
- 120 s, 60 s, twice the period and 40 are one named constant each (proposal).

## Interfaces
- **Consumes:** `state::store::{Target, Timing, Class}`, `model::{Record, Status, PrState, Source, NodeId}`, `poller::{Tier, Policy}`, `config::Config`, `clock::Mono`.
- **Provides:** the eleven functions and `HotDue` of `plan/contracts/poller.md`, "Inside: cadence", with those signatures.

## Out of scope
- The loop, the timers, building and sending requests, the tier's log line: T21. Hot and forced refreshes, sharing a request in flight, the follow-up's 3 s: T25.
- Computing a `Policy` from the quota: T08. Filling `Timing` and `Target`: T15.
- Setting `stale` in an answer: T22, T23. OPS-8's idle exit (POL-13's second bullet): T28.
- `status` itself (MOD-11): T05.

## Acceptance
Unit tests in `src/poller/cadence/`, as tables, with OPS-2's defaults and a normal active `Policy` built by hand unless a row says otherwise.
- `pol13_tier`: no request is `Idle`; a request 599 s ago `Active`; 600 s ago `Idle`.
- `pol14_hot`: a waiter makes a `merged` PR hot; named 119 s ago with `checks_pending`, and with `unknown`, hot; with `ready`, not; named 120 s ago, not; no status and no waiter, not.
- `pol10_period`: hot and merged 10 s; merged 300 s in both tiers; an open `asked` PR 300 s; an open `authored` or `manual` PR 30 s active, 300 s idle. **With `subscribed`:** an open `asked` PR 30 s active and 300 s idle; a merged one still 300 s; a hot one still 10 s.
- `ntf5_last_ask`: no request and none open is `None`; one open gives `now`, whatever the last request; none open and a request at 100 s gives 100 s. Through `tier`: with one open the tier is `Active` 2 hours after the last request; with none open, a request 599 s ago is `Active` and one 600 s ago `Idle`, however recently a subscription ended.
- `ntf5_a_subscription_is_never_hot`: `timing.subscriptions` 3, no waiter, never named, `checks_pending`: `hot` is false, `hot_due` holds nothing for it, `wants_follow_up` is false for it.
- `mod16_stale`: a `gone` record never, also with both flags set; `attempt_failed` alone; `slept` alone; an open PR fetched 60 s ago in the active tier isn't stale, 61 s ago is; idle 600 s and 601 s; hot 20 s and 21 s; ended 600 s and 601 s in both tiers.
- `pol13_poll_due`: `None` sent gives `Mono::ZERO`; sent at t gives t + 30 s; a policy without polls gives `None`.
- `pol10_in_poll`: `Loaded` and never-sent are in; `First` is out; an open PR sent 29 s ago is out, 30 s ago in; a merged one 299 s out, 300 s in, in the active tier; an open `asked` one the same; a hot one at 10 s. With `listed` holding a merged and a `gone` PR's IDs, each is in at 30 s. A tracked open PR missing from `listed` is still in at its period.
- `pol13_an_active_hour`: stepping `poll_due` and `in_poll` through one hour with 10 open PRs gives 120 poll times, each with all 10; idle, 12.
- `pol14_hot_due`: a hot PR sent 9 s ago isn't in `now` and `next` is 1 s ahead; at 10 s it's in `now`; with `hot_every` 20 s (slowed) it's due at 20 s; with `None`, nothing.
- `pol15_windows`: a PR sent 5 s ago may not be forced, 5.001 s ago may, never sent may; with a 30 s window, 30 s may not. The same for a poll at 10 s, counted from the last forced poll: `None` (none was ever forced) may, whenever the last ordinary poll was.
- `pol11_follow_up`: with `mergeable_unknown` set: open with a waiter; open and named 59 s ago; open and `first_since_start`: each true. Named 60 s ago with nothing else, false. `Class::Ended`, false. `policy.follow_up` false, always false. With `mergeable_unknown` unset, always false.
- `pol7_batches`: 0 items give no batch, 40 one, 41 two (40 and 1), 200 five.
- `cargo test --features test-endpoint --lib poller::cadence::` and `make verify` pass.

## Traps
- **`stale` and `period` take the configured intervals, not `policy`'s** (MOD-16): records must go stale when polling is slowed or stopped.
- **Right after the tier turns active, records fetched during the idle time are stale** until the poll that follows is applied. That's MOD-16 as written; don't smooth it.
- Durations are compared on `Mono` only. `fetched_at` in the record is a wall time for display: never use it here.
- `>=` for a period ("at least its period ago"), strictly more for a window ("more than 5 s ago") and for staleness ("older than twice").
- A hot PR's refreshes count from its last refresh of any kind, a poll's included: use `timing.last_sent`, not a hot-only time.

## Open questions
- **All of POL-10, POL-11 and POL-13 to POL-15 are (proposal)**: built as written.
- **"For `idle_after` after the last subscription ended"** is how this brief reads NTF-5's "as if a client had just asked" at the end of a subscription. Report it if it doesn't fit.
- **Readings this brief makes, to report if they don't fit:** a hot PR in a poll is due by the configured `hot_interval`, also when hot refreshes are slowed; both window edges are inside the window.
- **"At least its period ago" is exact.** The engine marks every request of a poll as sent at the poll's own send time (POL-13, T21), so a PR carried in a poll's second request is due one period after that poll, and `in_poll` needs no tolerance. Whether real latency needs one isn't in the spec.
