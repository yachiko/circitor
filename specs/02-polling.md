# Polling GitHub

**Status:** draft 0.2, 2026-11-25: notify (D7). Draft 0.1, 2026-11-24. Hardened: pass 1 (2026-11-25), pass 2 (2026-11-25), pass 3 (2026-11-25), planning findings (2026-11-25).

When circitor asks GitHub, for what, and what it does when GitHub doesn't answer. Costs and timings are from [docs/02](../docs/02-github-api.md).

## Requests

- **POL-1** **circitor only reads.** It sends GraphQL queries to `https://api.github.com/graphql` and never a mutation. No code path builds one.
- **POL-2** **One request at a time**, never two in flight, with at least 1 s between the end of one and the start of the next. A request times out after 20 s. (proposal: the values; GitHub's own limit is 10 s, docs/02 §2)
- **POL-3** The token comes from the token source (OPS-5). It's read at the daemon's start, again every 10 minutes, and at once when GitHub answers 401 (POL-20). gh may rotate it (docs/02 §1). A periodic read that fails keeps the token in hand, and is logged once; one where gh reports no login drops it (OPS-5).

## The poll

- **POL-4** A poll is **one request doing two things**:
  1. **Finding:** `viewer.pullRequests(states: OPEN, first: 100, orderBy: UPDATED_AT desc)`, asking only for `id`, `number`, `updatedAt` and the repository's name and owner, and `viewer { login }` (MOD-11). This list has no index lag (docs/02 §3). Search isn't used.
  2. **Refreshing:** the full record (POL-6) of every PR already tracked, by node ID; ended PRs less often (POL-10). They're read by `nodes(ids:)` with one shared fragment (docs/02 §2).
  
  A PR found for the first time is refreshed by a second request straight after. It's tracked only once that refresh is applied; until then it's found again at the next poll. **That first refresh also asks POL-12's fields for the PR's last 20 comments and last 20 reviews**, times included. They're the baseline of STA-5 and make no events; the newest time of each kind and the IDs at it are what STA-6 saves. For 40 PRs it costs 2 points, not 1 (docs/02 §2, measured with the last 10; times are fields, not connections). The same goes for a PR first read through (POL-16) or loaded from the state file (STA-8); for a loaded PR, STA-5 compares them with the saved times.

  **Found and read-through PRs are matched to tracked ones by node ID.** When a refresh gives a tracked PR another owner or repository, its `pr` takes the new name, with an `edited` event (MOD-15), and POL-5 applies again. (proposal; what a rename or transfer does to a node ID isn't measured)
- **POL-5** **Owners.** A found PR is tracked only if its repository's owner is in `owners` (OPS-2), compared without case. With `owners` empty, every found PR is tracked. Filtering happens after finding: the query can't do it (docs/02 §3).
  - **The filter holds for tracked PRs too.** An `authored` PR becomes `asked` (STA-1), and leaves as one does, when it's loaded with an owner that isn't in `owners`, or when a refresh shows it open and that poll's finding, read to the last page, didn't list it: it's no longer the user's own (docs/02 §3: the list has no lag). (proposal)
- **POL-6** **The record query** asks for what [01-model](01-model.md) needs: the fields of MOD-2, the repository's owner and name, the three counts, and the last commit's `statusCheckRollup` with its `state` and `contexts(first: 100)`. It doesn't ask for comment or review IDs (POL-12).
- **POL-7** **Batches.** One request refreshes at most 40 PRs, which costs 1 point by GitHub's formula (docs/02 §2). That point was measured for `nodes(ids:)` with a shared fragment, the form POL-4 uses. More PRs go in further requests in the same poll. More than 100 open authored PRs are found page by page, up to STA-3's limit; beyond it a warning is raised (API-5) and the rest are left out. (proposal: the limits)
- **POL-8** **Many checks.** If a commit has more than 100 checks, the further pages are fetched for that PR, up to 300 checks. Beyond that the record is `truncated` (MOD-7). **A PR's check pages are one read.** Its refresh is applied only once every page has answered. If a page fails, the refresh isn't applied: the record keeps its last values and is `stale` (MOD-16). (proposal; 187 on one commit have been seen, docs/02 §4)
- ~~**POL-9** **Required checks.** Each check's `required` comes from `isRequired(pullRequestNumber:)`, which takes the PR's own number, so each PR is its own aliased field in the query, not a shared fragment (docs/02 §4, observed). **A refusal empties the check, not just the field** (docs/02 §4, observed with a wrong number). If a check node comes back `null` with an error whose path ends in `isRequired`, that refresh isn't applied to the PR. The PR is read again without the field, `required` is `null`, and the field is left out for that PR until its head changes. A `null` check node is never read as "no checks". (proposal; `isRequired` isn't measured on the owner's repositories)~~ **Dropped:** a check's `required` isn't read in v1 (MOD-3, D6).
- **POL-10** **A PR that leaves the open list is still refreshed**, because refreshing goes by ID and not by the list. That refresh is what shows `merged` or `closed` (docs/02 §3: the list can't say which). **A PR the open list shows is refreshed in that poll as an open one**, whatever its record says, so the user's own PR reopened, or a gone one back, shows at the next poll (docs/02 §3: the list has no lag).
  - **A PR's period**, the first row that applies (POL-13 says how it's counted): (proposal)

| The PR | Refreshed every |
|---|---|
| Hot (POL-14), whatever its state | `hot_interval` |
| Open, with a subscription (NTF-5), whatever its `source` | Its tier's (POL-13) |
| `merged`, `closed`, `gone`, or `asked` (STA-1) | `idle_interval` |
| Any other | Its tier's (POL-13) |

  - The ended rate still shows a late comment, and the reopening of a PR that isn't the user's own. What the user didn't track isn't followed closely unless a client waits on it or asks for it fresh.
- **POL-11** **`mergeable` unknown.** When a refresh returns `UNKNOWN` for an **open** PR that has a waiter, was named by a request in the last 60 s (STA-1), or is being refreshed for the first time since the daemon started, that PR is refreshed once more 3 s after the answer that read `UNKNOWN`. The PRs of one answer share one request, and the follow-up never causes another. Asking is what makes GitHub compute it. On a new, small PR it took 3–4 s (docs/02 §4); how long a large or long-idle PR takes isn't measured (docs/02 §6). (proposal)
- **POL-12** **What's new.** When a refresh shows `counts.comments` or `counts.reviews` higher than before, one more request fetches that PR's last 20 comments and last 20 reviews: `databaseId`, author, time, and for a review its state and its inline comment count. Items STA-5 counts as new become `activity` items (MOD-12).
  - **They're batched.** The PRs of one poll or one hot refresh that need it share requests, up to POL-7's 40 PRs each, sent straight after it. Each PR's part is applied or fails on its own (POL-21). (proposal; by GitHub's formula 40 PRs' last 20 comments and reviews cost 1 point, not measured, docs/02 §2)
  - **A count change and its items are one change.** A refresh whose `counts.comments` or `counts.reviews` rose is applied only once this request has answered, so one event carries the counts and the `activity` items (MOD-15). If the request fails, the refresh is applied with the old values of those two counts, so the next refresh asks again.
  - Counts are the trigger because they're right at once, where `updatedAt` trails a review by up to 13 s; and asking for IDs on every poll would double its cost (docs/02 §4, §2).
  - **`more: true`** when all 20 items read of a kind are new and that kind's count rose by more than 20; for a PR loaded from the state file (STA-5), when all 20 read are new. The newest 20 are reported. (proposal)
  - A count that goes down makes no items.
  - An older item that slides into the last 20 after a deletion isn't new by STA-5's rule, and makes no event. (proposal; deletions aren't measured, docs/02 §6)

## Cadence

- **POL-13** The poller is in one of two **tiers**, by how recently a client asked for anything:

| Tier | When | A poll every |
|---|---|---|
| **active** | A client request in the last `idle_after`, 10 min | `interval`, 30 s |
| **idle** | Otherwise | `idle_interval`, 5 min |

  - A request arriving in the idle tier moves it to active, and starts a poll at once if the last one is older than `interval`.
  - With no waiter, no subscription and no client for `exit_after`, the daemon exits if the CLI started it (OPS-8).
  - **A subscription on an open PR keeps the active tier** (NTF-5): while one is held, the tier is active whatever the last client request; once it ends, the tier follows the last client request (`idle_after`). One on a PR that has ended doesn't.
  - **Hot isn't a tier:** it's a cadence for single PRs on top of either (POL-14).
  - **Every cadence runs from send times**, on the monotonic clock. A poll is due its interval after the last poll was sent. **Every request of a poll counts as sent when the poll's first request was**, so a PR refreshed in a poll's later request isn't a second short of its period at the next poll. A PR is due when its last refresh, of any kind, was sent at least its period (POL-10) ago, compared when the request is built. The send times of MOD-2 and MOD-6 stay each request's own.
  - Values are (proposal). A poll costs 1 point while it fits one request: up to 40 PRs to refresh (POL-7), none with more than 100 checks (POL-8). Then active costs about 120 points an hour and idle 12 (docs/02 §2). Each further request is one more point.
- **POL-14** **Hot PRs.** A PR is hot while it has a waiter, or for 2 minutes after a request named it (STA-1) while its `status` is `checks_pending` or `unknown`. Hot PRs are refreshed every `hot_interval`, **10 s**, by ID, without finding. Hot PRs share one request, up to POL-7's 40; more go in further requests. **A hot PR is due `hot_interval` after its last refresh, of any kind; a poll's counts.** So its hot refreshes fall between polls: one hot PR adds 240 points an hour, 360 with the active tier (60 % of the budget). (proposal) **A subscription doesn't make its PR hot** (NTF-5).
- **POL-15** **Forced refreshes.** `fresh` (API-7) and a wait's arrival (API-13) refresh that one PR by ID, as a hot refresh does. `refresh` (API-16) runs a whole poll, **which refreshes every tracked PR, whatever its period** (D11). Requests arriving while one is in flight share its result. One PR is force-refreshed only when its last refresh, of any kind, was sent more than 5 s ago, on the monotonic clock, and a whole poll only when the last forced poll was sent more than 10 s ago; a request inside the window gets what's in memory. When capped or in reserve (POL-23) both windows are 30 s. **A forced refresh, a read-through and a `track` go next:** they're sent before the rest of a poll in flight, between its requests (POL-2), so one waits at most for the request in flight. (proposal: the values)
- **POL-16** **Reading through.** A request for a PR that isn't tracked fetches it at once by owner, repository and number. If GitHub returns it, it's tracked with source `asked` (STA-1) and answered. If GitHub answers `NOT_FOUND` for the repository or the PR, the answer is `not_found` (API-3). If GitHub can't be reached, it's `github_unavailable`; another error `type` on the PR or its repository is too, with the type named (API-3); in the `auth` state (POL-20), `auth`; with the limit exhausted (POL-18), `rate_limited`. None of these tracks the PR. A just-created PR is therefore never "not found" for lack of a poll. A read-through is tried even during POL-19's backoff, at most one per step of it. (proposal: trying during the backoff)

## Quota

- **POL-17** **circitor counts what it spends**, from each response's `rateLimit.cost`, over a sliding hour; a response without one counts 1 point, the formula's least (docs/02 §2). GitHub's own counters lag (docs/02 §2), so they're a second opinion.
  - Its budget is **`quota.max_per_hour`, 600 points** (12 % of the user's limit). Above 80 % of it the daemon is `slowed`, at 100 % `capped`: POL-23 says what each sends. (proposal)
- **POL-18** **The user's remaining budget.** When a response's `rateLimit.remaining` is below **`quota.reserve`, 500**, the daemon is in POL-23's `reserve` state. When GitHub answers that the limit is exhausted, it's `exhausted` (POL-25). (proposal; the exhausted response isn't measured, docs/02 §6)
- **POL-23** **The quota states**, and what each one sends and reports. The first row that applies, from the bottom up. (proposal)

| State | When | Polls | Hot refreshes | Forced refreshes, reading through | `daemon.problem` (API-4) |
|---|---|---|---|---|---|
| **normal** | None of the below | By tier (POL-13) | Every `hot_interval` | Served | `null` |
| **slowed** | circitor's count is above 80 % of `quota.max_per_hour` (POL-17) | The tier's interval doubled (`idle_interval` when idle) | `hot_interval` doubled | Served | `null` |
| **reserve** | GitHub's `remaining` is below `quota.reserve` (POL-18) | Every `idle_interval` | Every `idle_interval` | Served, in POL-15's 30 s windows | `quota_low` |
| **capped** | circitor's count is at 100 % of it (POL-17) | None | None | Served, in POL-15's 30 s windows | `quota_low` |
| **exhausted** | GitHub answers that the limit is exhausted (POL-18, POL-25) | None until the reset time | None | Not sent: a tracked PR is answered from memory, an untracked one with `rate_limited` (API-3) | `rate_limited` |
| **secondary** | GitHub answers with a secondary limit (POL-25) | None until it lifts | None | Not sent: as exhausted | `rate_limited` |

  - POL-12's request is sent whenever its refresh was. POL-11's isn't sent when capped or in reserve.
  - **Each warning follows its own condition**, not the state in force: `quota_capped` while circitor's count is at 100 %, `quota_reserve` while `remaining` is below the reserve (API-5). So both can be raised at once.

## Errors

- **POL-19** **Network errors, timeouts and 5xx:** the poll is abandoned: the PRs of the failed request, and of the poll's requests not yet sent, become `stale` (MOD-16); other records keep their state. The next try comes after 5 s, doubling to at most 5 min. The first failure and the recovery are logged, not each retry (OPS-9). Waiters stay registered. (GitHub's 5xx isn't measured, docs/02 §5)
  - **The backoff holds every request**: polls, hot and forced refreshes, POL-11's and POL-12's. A read-through is the one exception, at most one per step (POL-16). Its failure doesn't move the backoff; its success ends it, as any does.
  - **Any request that succeeds ends it**, and a poll follows at once. A wake (POL-22) starts it again at 5 s. Of a poll split into several requests (POL-7), those that answered are applied.
  - **A first poll that fails or can't be sent has finished too** (API-18): one abandoned here or under POL-24, one not sent in the `auth` state (POL-20), and one not sent while `exhausted` (POL-23).
- **POL-20** **401:** the token is read again and the request repeated once. If the repeat gets a 401 too, the daemon is in the **`auth` state**; any other failure of it is handled as POL-19 or POL-24. It's also in it when its first token read ends without one (OPS-5): no request is sent without one. (proposal; docs/02 §5: a 401 has a REST-style body, measured with a made-up token; the answer to an expired one is assumed)
  - Tracked PRs are still answered from memory, `stale`, with `daemon.problem: auth` (API-4).
  - A request that can't be answered without GitHub (reading through, `track`) gets the `auth` error (API-3). Its message names the token source in use.
  - Waiters stay registered.
  - The token is read again every 60 s, and before a request that needs GitHub, at most once in 5 s, so a retry after `gh auth login` works at once.
  - **A read that gives another token than the one refused leaves `auth` for one try:** the request that caused the read is sent, or else a poll. The same token gets one try every 10 minutes (POL-3). A 401 on a try returns to `auth` at once, with no repeat. A success ends `auth`, and polling resumes. Until then `daemon.problem` stays `auth`, also during a try.
- **POL-21** **Partial errors.** A response can hold data and errors together (docs/02 §5). A node that came back is applied only if no error's path points inside it. Otherwise the record keeps its last values and is `stale`. A PR whose node is `null` with a `NOT_FOUND` error on its path, on every refresh for 5 minutes and at least three of them, becomes `gone` if it was open and MOD-11's conditions hold (in a poll, under the same login, and not every PR of the poll `NOT_FOUND`): its repository was deleted or the token lost access. **A PR whose refresh keeps failing** with any error for 5 minutes raises a warning (API-5) naming the PR, the error's `type` and the token source in use: a token that sees less looks like a deleted repository. (proposal: three, and 5 minutes; that lost access looks like `NOT_FOUND` is assumed, docs/02 §5) The 5 minutes go by the monotonic clock (POL-26). Only the PR's own errors count, a `null` node with an error on its path: a request that fails as a whole (POL-19, POL-20, POL-23) neither counts nor breaks a run.
- **POL-22** **Sleep and wake.** A sleep is noticed by comparing how far the wall clock and the monotonic clock moved since the last check: a gap over 30 s. The check runs at every tick (POL-26) and client request. On a sleep:
  - every record is `stale` until its next refresh (MOD-16)
  - POL-19's backoff starts again at 5 s
  - a poll starts at once if a waiter or a subscription (NTF-4) is held or a client asked within `idle_after`
  - every waiter's deadline is checked: its timeout on the monotonic clock plus the sleep noticed since it arrived. A `wait` that outlived it ends on waking (API-12); a clock set back doesn't lengthen it.
  
  (proposal; how tokio's timers behave across a suspend isn't checked, docs/04 §8)
- **POL-24** **Anything else.** Any other HTTP status, and a 200 without `data`, are handled as POL-19. **An error inside one PR stays with that PR:** a `null` node whose error isn't `NOT_FOUND` is handled as POL-21, and the rest of the response is applied. The status and the error's `type` are logged, never the body (OPS-9); a PR's error once while it lasts. Only `NOT_FOUND` counts towards `gone`. (proposal)
- **POL-25** **Rate-limit answers** are told apart before POL-19 and POL-24. (proposal; documented in GitHub's GraphQL rate-limit page, [references](../docs/references.md); docs/02 §2 labels the response `assumed`, §6 item 6)
  - **Exhausted** (POL-18): an answer without `data` whose `x-ratelimit-remaining` header is `0`. **An answer with data is applied as usual**; if its `remaining` is `0`, the daemon is `exhausted` from then on. The reset time is its `x-ratelimit-reset` header, or an hour later without one.
  - **A secondary limit:** a 403, or a 200 with an error, whose message names a secondary rate limit. Nothing is sent, read-throughs and forced refreshes included, until its `Retry-After` has passed; without one, for 60 s, doubling on each one in a row up to `idle_interval`. Every secondary limit counts towards the row, with a `Retry-After` or without; a request that succeeds ends it. The wait goes by the monotonic clock (POL-26).
- **POL-26** **Clocks.** (proposal; the monotonic clock and tokio's timers across a suspend aren't checked, docs/04 §8)
  - **GitHub's own windows, and the daemon's retention, go by the wall clock:** POL-17's hour, POL-25's reset time, MOD-8's 60 s, and STA-1's and STA-2's durations, checked at each tick, not slept on. A wall time ahead of now, after the clock was set back, counts as now.
  - **Every other duration goes by the monotonic clock**, for example POL-2's, POL-11's, POL-13's tiers, POL-14's, POL-15's, POL-19's and a waiter's timeout (API-12). A subscription's `for` runs as a waiter's timeout does, and is checked at the tick (NTF-4).
  - **The tick** is a timer every 10 s on the monotonic clock, beside the poll, hot and backoff timers. At each tick the sleep check (POL-22), the quota state (POL-23), POL-25's reset time, STA-1's and STA-2's durations, and every subscription's socket, expiry and `problem` (NTF-4) are checked. It sends nothing to GitHub by itself.
