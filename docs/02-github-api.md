# GitHub API facts

**Status:** measured on github.com on 2026-11-24, with `gh` 2.101.0 and its OAuth token. **Purpose:** the single input for everything the specs say about GitHub. It records what the API returns, what a query costs and how fast a change shows up.

The evidence for each `verified` line is in [measurements](05-measurements.md). The numbered sections are cited from the specs as "docs/02 §3".

Confidence labels:

| Label | Meaning |
|---|---|
| **verified** | Measured on the live API, on the date given |
| **observed** | Seen in a measurement, but its cause or how general it is isn't confirmed |
| **documented** | From GitHub's or the tool's own documentation ([references](references.md)); not measured |
| **assumed** | Not checked at all. Nothing may be built on it until it's measured |

## 1. Access

| Item | Value | Label |
|---|---|---|
| Endpoint | `POST https://api.github.com/graphql` | verified |
| Token | `gh auth token` prints the token of the default host's active account and a newline. Here it starts with `gho_` (gh's OAuth app). Flags: `--hostname`, `--user`. | verified (prefix), documented (flags) |
| Scopes | `gist, read:org, repo, workflow` on this login. A query on a private repository needs `repo` (`X-Accepted-OAuth-Scopes: repo`). | verified |
| Overrides | `GH_TOKEN`, then `GITHUB_TOKEN`, win over the stored login. `gh auth token` prints either when it's set; one set but empty is ignored, and the stored login is used (gh 2.101.0, docs/05 2026-11-25). | documented; verified for `gh auth token` |
| Storage | The system credential store (macOS Keychain, Secret Service on Linux), or plain text in `hosts.yml` under `$GH_CONFIG_DIR`, `$XDG_CONFIG_HOME/gh` or `~/.config/gh` | documented |
| Expiry | gh can issue **expiring tokens that it refreshes itself** (`gh auth login --expiring`). `gh auth token` refreshes before printing, and warns that a captured value can be rotated. | documented |
| With no login | `gh auth token` exits 1, with "no oauth token found for <host>" on stderr and nothing on stdout (gh 2.101.0, docs/05 2026-11-25) | verified |

## 2. Rate limits

| Item | Value | Label |
|---|---|---|
| Primary limit | **5 000 points an hour per user** for GraphQL (`X-RateLimit-Limit: 5000`, resource `graphql`) | verified |
| Shared | Personal access tokens and OAuth apps acting for one user share that user's budget. So circitor shares it with every `gh` call. | documented |
| REST is separate | REST has its own 5 000 requests an hour (resource `core`) | verified |
| **GraphQL search** | **Counts against `graphql`, not against the 30-a-minute `search` bucket** (`X-RateLimit-Resource: graphql`). REST `search/issues` counts against `search` (limit 30). | verified |
| Cost formula | Add the requests needed for every connection, as if each `first`/`last` were filled; divide by 100; round; at least 1 | documented |
| **Cost follows the page size asked for**, not the rows returned | `first: 50` cost 2 with two PRs returned | verified |
| `rateLimit { cost remaining limit resetAt nodeCount }` | Costs nothing extra to ask | verified |
| **Counters lag** | `remaining` read 4 989 and then 4 998 on consecutive queries in one window; REST `/rate_limit` said 4 used while a response header said 18 | observed |
| Node limit | 500 000 nodes a query; `first`/`last` are 1–100 | documented |
| Secondary limits | 100 concurrent requests; 2 000 points a minute on GraphQL, where a query is 1 and a mutation 5; 60 s of CPU time a minute | documented |
| A timed-out query (over 10 s) | Deducts extra points for the next hour | documented |
| REST `304` | **Free**: two conditional requests left `X-RateLimit-Used` unchanged | verified |
| Response when the limit is hit | Not measured | assumed |

### Measured cost

The **full record** is [`frag.graphql`](measurements/2026-11-24/frag.graphql): state, head, mergeability, counts, the last 10 reviews, and the last commit's check rollup with up to 50 checks.

| Query | Page | Cost | Nodes |
|---|---|---|---|
| `viewer.pullRequests`, full record | 10 | 1 | 620 |
| | 20 | 1 | 1 240 |
| | 30 | 1 | 1 860 |
| | 50 | 2 | 3 100 |
| | 100 | 3 | 6 200 |
| `search`, full record | 50 | 2 | 3 100 |
| `viewer.pullRequests`, light (no reviews, rollup state only) | 100 | 1 | 200 |
| `viewer.pullRequests`, `id` and `updatedAt` only | 100 | 1 | 100 |
| `nodes(ids:)`, full record | 2 PRs | 1 | 124 |
| | 40 PRs | 1 | 2 480 |
| IDs-only list (100) **and** `nodes(ids:)` for 2 PRs, one request | – | 1 | 224 |
| `nodes(ids:)`, full record **plus the last 10 issue comments' IDs** | 40 PRs | 2 | 2 880 |
| the same plus the last 10 reviews' IDs | 40 PRs | 2 | 3 280 |
| the same plus the last 10 review threads, 3 comments each | 40 PRs | 6 | 4 480 |
| full record plus `timelineItems(last: 20)` | 40 PRs | 2 | 3 280 |

All verified 2026-11-24. The full record holds three costed connections per PR (reviews, commits, checks), so its cost fits **round(3 × PRs ÷ 100), at least 1** (observed): about one point up to 49 PRs.

## 3. Finding the user's PRs

| Item | Value | Label |
|---|---|---|
| `viewer.pullRequests(states: OPEN)` | Lists the PRs the user authored, in every repository. **No lag:** a new PR was in it 1 s after creation, and gone at the first sample after a close or a merge. It can't filter by organisation; the owner is in `repository.nameWithOwner`. | verified |
| `search(type: ISSUE)` | Takes the web syntax: `is:pr is:open author:@me`, with `repo:` or `user:` | verified |
| **Search lags on appearing** | A new PR was missing at +1 s in all four PRs. It was there at +3 s (PR D), +4 s (PR A), and +8 s (PRs B and C). | verified (4 samples) |
| **Search lags on disappearing** | After a close, still listed at +0.5 s, gone at +7 s. After a merge, still listed at +0.5 s, gone at +3 s. | verified (1 sample each) |
| Search results are live objects | Only membership lags: a search hit showed the new head SHA at the same sample as the direct read | verified |
| `org:` in a search, and SAML SSO orgs | Not measured | assumed |

## 4. The PR record

Times below are seconds after the action, sampled every 1.5–3 s.

### Mergeability

| Item | Value | Label |
|---|---|---|
| `mergeable` on a new PR | `UNKNOWN` at +1 s and known by +4 s, in three PRs of four; the fourth was already `MERGEABLE` at +1.4 s. **Asking is what starts the computation.** | verified |
| **A conflict** | `mergeable: CONFLICTING`, `mergeStateStatus: DIRTY`, 3 s after the PR was created | verified |
| After a push | Back to `UNKNOWN` at +3 s, `MERGEABLE` at +6 s | verified |
| On a PR untouched for two months | `UNKNOWN` on the first read, `MERGEABLE` when read again a few minutes later | verified |
| On a merged PR | `UNKNOWN` for good, and `mergeStateStatus: UNKNOWN` (three PRs merged a month ago) | verified |
| On a closed PR | Keeps its last value (`MERGEABLE`, `CLEAN`) | verified |
| `mergeStateStatus` | Needs no preview header. Seen: `UNKNOWN`, `CLEAN`, `DIRTY`, and `UNSTABLE` (a check running or failed, none required). | verified |
| On a draft | Stayed `CLEAN`, not `DRAFT`, 2 s after the conversion | observed |
| `BLOCKED` | Seen on public repositories, read as an outsider: with `reviewDecision: REVIEW_REQUIRED` or `CHANGES_REQUESTED`, and with a failed required check | observed |
| `DRAFT` | **Never seen.** 31 draft PRs on public repositories read `BLOCKED`, `CLEAN`, `UNSTABLE`, `DIRTY` or `UNKNOWN`, as the test draft did. `isDraft` is the only draft signal. | observed |
| **A list of idle PRs** | 21 of 30 open PRs of one public repository read `mergeable: UNKNOWN` on the first read | observed |
| `BEHIND`, `HAS_HOOKS` | Not seen | documented |
| Branch protection on the test repository | **Not possible:** GitHub refuses it on a private repository without a paid plan (`403`, "Upgrade to GitHub Pro or make this repository public") | verified |

### Checks

| Item | Value | Label |
|---|---|---|
| `statusCheckRollup` with no checks | **`null`**, not an empty rollup | verified |
| **New PR, workflow about to start** | `null` at +1 s and +4 s, `PENDING` at +9 s. At +4 s the PR read `MERGEABLE`, `CLEAN`, rollup `null`: **the same as a PR in a repository with no CI.** | verified |
| **After a push** | At +0.4 s: the **old** head with its old `SUCCESS`. At +3 s: the new head, rollup `null`. At +6 s: `PENDING`. | verified |
| While checks run | Rollup `PENDING`, `mergeStateStatus: UNSTABLE`; each check run goes `QUEUED`, `IN_PROGRESS`, `COMPLETED` with a `conclusion` | verified |
| Out-of-order reads | Both check runs read `IN_PROGRESS` and then, 3 s later, `QUEUED` | observed |
| When all pass | Rollup `SUCCESS`, `mergeStateStatus: CLEAN` | verified |
| **One check fails while another still runs** | **The rollup reads `FAILURE` at once**, and stayed so for the 78 s the other check kept running. So a rollup that isn't `PENDING` doesn't mean the checks are done: each check's `status` does. | verified |
| A failed check, none required | `conclusion: FAILURE`; `mergeable` stays `MERGEABLE`, `mergeStateStatus: UNSTABLE` | verified |
| A skipped job (`if: false`) | `COMPLETED` with `conclusion: SKIPPED`, from the first read that has checks | verified |
| A cancelled run | `COMPLETED` with `conclusion: CANCELLED`, 16 s after `gh run cancel` returned | verified |
| A commit status (the older statuses API) | In the same list as a `StatusContext`, with `context` and `state` (`PENDING`, `FAILURE`) and no `status` or `conclusion`. Visible within 3 s. | verified |
| **Many checks on one commit** | Up to **187** on a PR of a public repository; 25 on another. One page holds 100 at most. | observed |
| Which checks are required | `isRequired(pullRequestNumber:)` on a check run or status answers it, also for an outsider on a public repository: 3 of 18 on one PR. **It takes the PR's own number**, so it can't sit in a fragment shared by several PRs; a wrong number gives `NOT_FOUND` and empties the node. | observed |
| `baseRef.branchProtectionRule` | `null` for an outsider | observed |
| A required check not yet reported (`EXPECTED`); a re-run | Not measured | documented |

### What moves `updatedAt`

| Change | Moves it? | Label |
|---|---|---|
| Push, issue comment, to draft, to ready, close, merge | Yes, within the second | verified |
| **A review, or a reply in a review thread** | **Yes, but late.** A review posted at :29 hadn't moved it at :32 and had by :39. A reply posted at :18 hadn't at :25 and had by :31. `reviews.totalCount` was right at the first read both times. | verified (1 sample each) |
| **A check starting or finishing, or a commit status** | **No.** It stayed at the PR's creation time through a whole check cycle, a failure, a cancel and two commit statuses. | verified |

### Comments and reviews

| Item | Value | Label |
|---|---|---|
| Three kinds | **Issue comments** (the conversation), **reviews** (a state and an optional body), and **review comments** (on a line of the diff, grouped in threads) | verified |
| Counts | `comments.totalCount`, `reviews.totalCount` and `reviewThreads.totalCount` are right at the first read, 2 s after the action | verified |
| **A reply in a thread creates a review of its own** | `reviews.totalCount` went from 1 to 2 to 3 over two replies; each reply's `pullRequestReview` is a new one, in state `COMMENTED` | verified |
| A review by the PR's author | Allowed with `event: COMMENT`; its state reads `COMMENTED` | verified |
| IDs | **`databaseId` in GraphQL is the `id` REST uses**, for all three kinds | verified |
| Fetching one by ID | `repos/{o}/{r}/issues/comments/{id}`, `repos/{o}/{r}/pulls/comments/{id}`, `repos/{o}/{r}/pulls/{n}/reviews/{id}`; a review's own comments at `…/reviews/{id}/comments` | verified |
| Listing what's new over REST | `issues/{n}/comments?since=` and `pulls/{n}/comments?since=` return those created or changed since the time given | verified |
| **`timelineItems` misses thread replies** | With two reviews and an issue comment, it returned one review and the comment. Its `totalCount` ignores `itemTypes`. | verified |
| Edited and deleted comments, resolved threads, bot comments | Not measured | documented |

### Other fields

| Item | Value | Label |
|---|---|---|
| `headRefOid` | The new SHA is visible 0.4–3 s after `git push` returns | verified |
| Head branch deleted | `headRef` is `null`; `headRefName` and `headRefOid` stay | verified |
| Merged | `state: MERGED`, `merged: true`, `mergedAt` equal to `closedAt`, `mergeCommit` set. Visible in the first read after `gh pr merge` returned. | verified |
| Closed unmerged | `state: CLOSED`, `mergedAt: null`, `mergeCommit: null` | verified |
| `isDraft`, `comments.totalCount` | Change in the first read after the action | verified |
| `reviewDecision` | `null` where no review is required. On public repositories: `REVIEW_REQUIRED`, `APPROVED`, `CHANGES_REQUESTED`. | observed |
| A review's `state` | `COMMENTED`, `APPROVED`, `CHANGES_REQUESTED` seen | observed |
| Auto-merge, the merge queue, PRs from forks | Not measured: none of 60 public PRs had auto-merge or a queue entry | documented |

## 5. Errors

GraphQL answers `200` with partial `data` and an `errors` array. `gh api` exits non-zero when `errors` is present, even with data.

| Case | Response | Label |
|---|---|---|
| PR number that doesn't exist | `data.repository.pullRequest: null`, error `type: NOT_FOUND`, `path: ["repository","pullRequest"]` | verified |
| Repository that doesn't exist | `data.repository: null`, error `NOT_FOUND` | verified |
| One unknown ID in `nodes(ids:)` | `null` at that index, error `NOT_FOUND` with `path: ["nodes", 1]`; **the other nodes are returned** | verified |
| A repository the token can't see | Expected to look like one that doesn't exist | assumed |
| **A bad token** | **HTTP `401`** with `{"message": "Bad credentials", "status": "401"}`: a REST-style body, with no `data` or `errors` | verified |
| No token at all | `403` with an "API rate limit exceeded" message for the IP address: the unauthenticated limit, which was already used up here | observed |
| An expired token, the rate limit hit with a valid token, 5xx | Not measured | assumed |

## 6. Open items

Each needs a measurement before a spec may rely on it:

1. Required checks and reviews **as they change**: `BEHIND`, `EXPECTED`, and what an approval moves. The values were only observed on other people's public repositories; watching a change needs a public repository of the owner's with branch protection.
2. A search with `org:`, and an organisation that enforces SAML SSO. Not needed for v1: the owner has no organisation (log D15).
3. Auto-merge and the merge queue.
4. An edited or deleted comment, and a resolved thread: what moves, and whether a count goes down.
5. A re-run of a failed check, and more than 50 checks on one commit.
6. The response to an exhausted primary limit, a secondary limit and an expired token.
7. How long `mergeable` stays `UNKNOWN` on a large or long-idle PR.
8. Search lag under load: every lag above comes from a few samples in a quiet repository.
9. GitHub Enterprise Server.
