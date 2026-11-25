# The PR record

**Status:** draft 0.1, 2026-11-24. Hardened: pass 1 (2026-11-25), pass 2 (2026-11-25), pass 3 (2026-11-25).

What circitor knows about one PR, as every other file names it. GitHub's side of each field is in [docs/02 §4](../docs/02-github-api.md#4-the-pr-record).

## Identity and fields

- **MOD-1** A PR is named **`owner/repo#number`**, compared without regard to case, and keyed by its GraphQL node ID, which is never shown. `pr` is read from `repository.nameWithOwner` and `number` at every refresh, so it follows a renamed or transferred repository. A found or read-through PR whose node ID is already tracked is that record. (proposal; that the node ID survives a rename or a transfer isn't measured)
- **MOD-2** The record's fields:

| Field | Type | From GitHub | Notes |
|---|---|---|---|
| `pr` | string | `repository.nameWithOwner`, `number` | `owner/repo#number` (MOD-1) |
| `url`, `title` | string | `url`, `title` | |
| `author` | string | `author.login` | `null` for a deleted account |
| `state` | `open`, `merged`, `closed`, `gone` | `state` | `gone` is circitor's own value (MOD-11) |
| `draft` | bool | `isDraft` | The only draft signal (docs/02 §4, observed) |
| `base`, `head` | string | `baseRefName`, `headRefName` | Branch names |
| `head_sha` | string | `headRefOid` | 40 hex characters |
| `created_at`, `updated_at`, `merged_at`, `closed_at` | time or `null` | the same | |
| `mergeable` | `yes`, `no`, `unknown` | `mergeable` | MOD-8 |
| `merge_state` | string | `mergeStateStatus`, lowered | MOD-9 |
| `review` | `approved`, `changes_requested`, `review_required`, `none` | `reviewDecision` | `none` for `null` |
| `checks` | object | the last commit's checks | MOD-3 to MOD-7 |
| `counts` | object | three `totalCount`s | `comments`, `reviews`, `review_threads` |
| `status` | string | derived | MOD-11 |
| `source` | `authored`, `manual`, `asked` | – | Why it's tracked (STA-1) |
| `rev` | integer | – | MOD-13 |
| `fetched_at` | time | – | When the request that last refreshed this record was sent |
| `stale` | bool | – | MOD-16 |
| `error` | `{type, since}` or `null` | – | The error that stopped this PR's last refresh (POL-21, POL-24): the GraphQL error's `type`, as GitHub writes it (not lowered, as CLI-3 prints it), and `since`, the first failure of the current run of this PR's errors, whatever their types. A request that fails as a whole (POL-19, POL-20, POL-23) or a failed check page (POL-8) leaves it as it was. `null` once a refresh is applied; a gone record keeps it. |

  - **A newer read wins.** A refresh sent before the one last applied to the record isn't applied: the later read already holds what it had. Refreshes are ordered by a number the daemon gives each request as it's sent, never by `fetched_at`.
  - **A value GitHub sends that this file doesn't list is passed through, lowered**, but for a check's `phase`, which is always one of MOD-3's three. It never fails decoding. Of a check's results, docs/02 §4 verifies `success`, `failure`, `skipped` and `cancelled`; the others aren't measured. (proposal)

## Checks

- **MOD-3** `checks.items` holds one entry per check on the head commit:

| Field | Check run | Commit status |
|---|---|---|
| `kind` | `run` | `status` |
| `id` | `databaseId` | – |
| `name` | `name` | `context` |
| `phase` | `queued`, `in_progress`, `completed` (`status`, lowered; anything else counts as `queued`) | `completed` unless `state` is `PENDING` or `EXPECTED`, which are `in_progress` (`EXPECTED` isn't measured: proposal) |
| `result` | `conclusion`, lowered; `null` until completed | `success`, `failure` or `error` once completed |
| `url` | `detailsUrl` | `targetUrl` |

  - **Whether a check is required isn't read in v1** (D6): a required check that blocks the merge shows as `merge_state: blocked`.

- **MOD-4** **Each read of a check is believed.** Check runs are matched between reads by `id`, commit statuses, which have no `id`, by `name`. GitHub can return an older view of a check run after a newer one (docs/02 §4, observed once: `IN_PROGRESS`, then `QUEUED`). A move between `queued` and `in_progress` isn't a visible change (MOD-14), so that view makes no event. A step back from `completed` is believed at once: it reads `pending`, which is on the safe side, and is what a re-run looks like. A status posted again as pending is a real change too. Any change of head SHA starts a new list. (proposal; a check run's `databaseId` and re-runs aren't measured, docs/02 §6)
- **MOD-5** **The summary is computed from the items, never taken from GitHub's rollup** (but for a truncated list, MOD-7), which reads `FAILURE` while checks still run (docs/02 §4). `checks.state` is the first that applies:

| `checks.state` | When |
|---|---|
| `none` | No items |
| `pending` | Any item isn't `completed` |
| `failing` | All completed, and any `result` is anything but `success`, `skipped` or `neutral`: `failure`, `error`, `cancelled`, `timed_out`, `action_required`, `startup_failure`, or a value this file doesn't list |
| `passing` | All completed, and every `result` is `success`, `skipped` or `neutral` |

  - `checks` also carries the counts `total`, `completed` and `failed`, and `failed_items`: the failing items' `name` and `url`.
  - **`failing` is reported only once every check has completed.** Before that, `failed` above 0 says a failure is already in.
- **MOD-6** **A checks list is believed only once it has settled.** For a few seconds after a push or a PR's creation GitHub shows no checks where some are about to start (docs/02 §4: up to 9 s), and a list that has begun to fill may not be complete. Settling counts from the **settle start**: the send time of the refresh that first read the record (also one loaded from the state file, STA-8), first showed this `head_sha`, or first showed the PR open again after a close. `checks.settled` becomes true at an applied refresh **sent at least the settle time after the settle start**, on the monotonic clock: **20 s for a list with checks, 60 s for an empty one**, since a repository's CI may report late, also on a PR's first head. The refresh that sets the settle start leaves it false, but for the shortcut below. (proposal: 20 s stays above the 9 s checks take to appear, and 60 s stays within API-12's default 100 s)
  - **It changes only when a refresh is applied, never by the clock alone**, so a `status` it moves takes a revision like any other change (STA-4). Any change of `head_sha`, also back to an earlier one, starts it at false.
  - **A reopening starts it at false too**, and is a new settle start: CI may run again on the same head. (proposal; what a reopening does to checks isn't measured)
  - A record first seen, read through or loaded from the state file, with an `updated_at` older than **that list's settle time plus 5 s by GitHub's clock** (25 s, or 65 s with no checks) counts as settled at once: a push moves `updated_at` within the second, so the head is at least that old. GitHub's clock is the `Date` header of the response that carried the record; a response without one never settles at once. (proposal; the `Date` header isn't recorded in docs/02)
  - A check that first reports later than that can't be foreseen: `passing` can return to `pending` on the same head.
- **MOD-7** **A commit can have more checks than circitor reads** (POL-8), and a list can change between its pages. Whenever fewer items were read than GitHub's count, `checks.truncated` is true, `total` is GitHub's count, `completed` and `failed` count the read items only, and the state is the first that applies: `pending` if any read item isn't completed, or GitHub's rollup says `PENDING` or `EXPECTED`; `passing` if the rollup says `SUCCESS`; otherwise `failing`. **A truncated `failing` can be early:** the rollup reads `FAILURE` while unread checks still run, so MOD-5's rule holds for the read items only. (proposal)

## Mergeability and review

- **MOD-8** **`mergeable: unknown` means "GitHub hasn't worked it out yet"**, never a state to act on (docs/02 §4).
  - For one `head_sha` **and `base`, while the PR is open**, once `yes` or `no` is known the record keeps it while GitHub answers `UNKNOWN`, **for at most 60 s after the last read that gave it**, on the wall clock. It returns to `unknown` when the head or the base changes, or at the first `UNKNOWN` read later than that: the base branch may have moved, which a kept `yes` would hide. (proposal; what a push to the base does isn't measured)
  - After a push the record therefore reads `unknown` until GitHub answers, and both steps are visible changes (MOD-14).
  - On a merged PR `mergeable` and `merge_state` are `unknown` and stay so. That change comes with the `merged` event and adds no `mergeable` kind (MOD-15).
- **MOD-9** `merge_state` passes GitHub's value through, lowered. It follows MOD-8 in keeping the last known value, and on a merged PR. (docs/02 §4; proposal, D6)
  - **Seen:** `clean`, `unstable`, `dirty`, `unknown` (verified) and `blocked` (observed, on other people's repositories only).
  - **Never seen:** `behind`, `has_hooks`, `draft` (documented).
- **MOD-10** `review` is GitHub's `reviewDecision`, lowered; `none` where GitHub gives `null`, as it does where no review is required (docs/02 §4, observed). Individual reviews are reported as new items (MOD-12).

## Status

- **MOD-11** `status` is the one word an agent reads first: the first row that applies. (proposal: the order)

| `status` | When |
|---|---|
| `merged` | `state` is `merged` |
| `closed` | `state` is `closed` |
| `gone` | `state` is `gone` |
| `conflict` | `mergeable` is `no` |
| `checks_failing` | `checks.state` is `failing` |
| `checks_pending` | `checks.state` is `pending`, or it's `none` or `passing` and not settled |
| `changes_requested` | `review` is `changes_requested` |
| `draft` | `draft` is true |
| `review_required` | `review` is `review_required` |
| `blocked` | `merge_state` is `blocked` or `behind` (proposal, D6: `behind` was never seen) |
| `unknown` | `mergeable` is `unknown` |
| `ready` | None of the above: open, mergeable, checks settled and passing or none |

  - A draft with failing checks reads `checks_failing`: the checks are what its author acts on.
  - **`gone` is circitor's own state:** GitHub no longer returns the PR (POL-21). Only an open PR becomes gone; a merged or closed one keeps its state. A gone record keeps the other fields it last had, and `fetched_at` is its last successful read.
  - **`gone` needs the same login.** A `NOT_FOUND` counts towards `gone` (POL-21) only in a poll (POL-4), whose finding gives the login (`viewer { login }`): while that login is the one the PR was last read with, and not when the poll refreshed two or more tracked PRs and every one answered `NOT_FOUND`. A `NOT_FOUND` from a hot or forced refresh (POL-14, POL-15) neither counts nor breaks POL-21's run. Otherwise it's a failed attempt and the record is `stale` (MOD-16). (proposal; that lost access looks like `NOT_FOUND` is assumed, docs/02 §5)
  - **A gone PR that GitHub returns again takes GitHub's state**, with that state's kind: `reopened`, `merged` or `closed` (STA-2).

## What's new

- **MOD-12** For comments and reviews circitor serves **who, when and an ID, never text** (U5). An item is:

| Field | Value |
|---|---|
| `kind` | `comment` (in the conversation) or `review` |
| `id` | GitHub's `databaseId`, the ID the REST API takes (docs/02 §4) |
| `author`, `at` | Login; `at` is a comment's `createdAt` and a review's `submittedAt`. POL-12's and STA-5's comparisons of times use it. |
| `review_state` | For a review: `approved`, `changes_requested`, `commented`, `dismissed` |
| `inline` | For a review: how many comments on the diff it carries |
| `fetch` | The REST path that returns it: `repos/{o}/{r}/issues/comments/{id}`, or `repos/{o}/{r}/pulls/{n}/reviews/{id}` |

  - **A reply in a review thread is a review of its own** (docs/02 §4), so replies arrive as `review` items with `inline: 1`. An item with `inline` above 0 also carries `fetch_inline`, the path of its comments (`…/reviews/{id}/comments`).
  - Items are reported once, as events (MOD-15). The record itself holds only `counts`.
  - **Only new items are reported.** An edit, a deletion or a resolved thread isn't (non-goal; how GitHub shows them isn't measured).
  - Items by the PR's own author and by bots are reported like any other. (proposal; bots aren't measured, docs/02 §4)

## Revisions and events

- **MOD-13** The daemon keeps **one counter**. Each visible change to any record takes the next number, and that record's `rev` becomes it. So `rev` orders changes across PRs, and "everything after revision N" is one comparison.
  - **A revision is taken, and its record and event stored, in one step.** Every answer reads its records, events and cursor from one view, and its cursor is the highest revision in that view. So no revision at or below a cursor appears after it was handed out.
- **MOD-14** A **visible change** is a change to any of: `pr`, `state`, `draft`, `head_sha`, `title`, `base`, `mergeable`, `merge_state`, `review`, `status`, a rise in `counts.comments` or `counts.reviews`, or the checks: `checks.state`, `total`, `completed`, `failed`, `truncated`, `failed_items`, or which items are in the list (a check run by `id`, a commit status by `name`). A record's first appearance and its dropping each take a revision too (`tracked`, `untracked`).
  - **Not a change:** `fetched_at`, `stale`, `error`, `updated_at` by itself, `checks.settled` by itself, a check's `url`, a check moving between `queued` and `in_progress`, and `mergeable` or `merge_state` reading `unknown` where MOD-8 keeps the known value. **Nor is any of these by itself:** `url`, `author`, `head`, `created_at`, `merged_at`, `closed_at` and `source`: `merged_at` and `closed_at` come with a `state` change, and `track` on a tracked PR (API-15) takes no revision.
  - **Not a change either:** any other change in `counts`: a count that falls, or `review_threads` alone. Deletions and resolved threads aren't reported (MOD-12).
- **MOD-15** Each revision is recorded as an **event**: `rev`, `pr`, `at` (the daemon's clock), and `kinds`, one or more of:

| Kind | Carries |
|---|---|
| `status` | `from`, `to` |
| `head` | `from`, `to` (short SHAs) |
| `checks` | The new `checks.state` and counts; `failed_items` when any failed. Present whenever any of MOD-14's checks fields changed. |
| `merged`, `closed`, `reopened`, `gone` | – |
| `draft` | The new value |
| `mergeable` | `mergeable` and `merge_state` |
| `review` | The new `review` |
| `activity` | The new `counts`; the new items (MOD-12), which may be none; `more: true` when some were left out (POL-12) |
| `edited` | The names of the fields that changed: `pr`, `title`, `base` |
| `tracked`, `untracked` | `source` |

  - **Every visible change of MOD-14 has a kind here**, so an event never has none.
  - **One applied refresh (STA-4) makes at most one event per PR:** a count's rise and its items (POL-12), and a PR's check pages (POL-8), are one refresh. When one response changes several PRs, they take revisions in the order of `pr`.
  - A PR's first appearance makes a `tracked` event: what it already had isn't news (STA-5). For a PR loaded from the state file (STA-8) the same event also carries `merged` or `closed` if it was saved open and has ended since, and `activity` for the items STA-5 makes new. Nothing else it already had is news.
- **MOD-16** `stale` is true when either holds:
  - `fetched_at` is older than twice the PR's period (POL-10). **The configured values count, not ones slowed for quota**, so records go stale when polling is slowed or stopped (POL-17, POL-18).
  - the last attempt to refresh it failed (POL-19), or a sleep was noticed since it was fetched (POL-22). A `NOT_FOUND` for the PR is a failed attempt until the PR is `gone` (POL-21).

  A `gone` record is never stale: there is nothing newer to fetch. A stale record is still served; `stale` and `fetched_at` let the agent judge it.
