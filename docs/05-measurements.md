# Measurements

A dated log of what was measured on the live GitHub API, newest last. [GitHub API facts](02-github-api.md) is the summary; this is its evidence. Where two entries disagree, the later one wins.

**Rules for a new entry**
- Read-only queries, plus throwaway PRs in `yachiko/experiment` only. Never the owner's other repositories.
- A workflow file goes only on a PR branch that is closed unmerged, so it never reaches `main`.
- Raw logs go in `measurements/<date>/`. Nothing with a token, a title or a body from a private repository is committed.
- Recorded response fixtures for tests don't exist yet. When they're made they go in `testdata/github/`, scrubbed; the testing spec decides their format.

## 2026-11-24: first measurements

**Setup:** `gh` 2.101.0 on Linux, logged in to github.com with the scopes `gist, read:org, repo, workflow`. Test repository: `yachiko/experiment` (private, Actions enabled, no branch protection checked). Budget spent on the first round (down to PR B): **164 of 5 000 GraphQL points**.

**Files** in [`measurements/2026-11-24/`](measurements/2026-11-24/):
- `frag.graphql`: the full PR record used in the cost table.
- `sample.sh`: the sampler. Each run reads one PR directly, `viewer.pullRequests(states: OPEN)` and an `is:pr is:open author:@me` search in one query (cost 1), and prints a line when anything changed.
- `a1.log` to `a3.log`, `b1.log`, `c1.log`, `d1.log`: its output. Each line is the elapsed time, the UTC time and the state. `a1.log` has no elapsed times: the first version of the script needed `bc`, which isn't installed.

### Cost and limits

| # | What | Result |
|---|---|---|
| 1 | `viewer.pullRequests(first: N)` with the full record | N = 10, 20, 30: cost 1. N = 50: cost 2. N = 100: cost 3. Two PRs came back each time. |
| 2 | `search(first: 50)` with the full record | Cost 2 |
| 3 | A light list and an IDs-only list, `first: 100` | Cost 1 each |
| 4 | `nodes(ids:)` with the full record | 2 PRs: cost 1. 40 PRs (the same two IDs repeated): cost 1, 2 480 nodes. |
| 5 | IDs-only list and `nodes(ids:)` in one request | Cost 1 |
| 6 | Which bucket a GraphQL search uses | `X-RateLimit-Resource: graphql`. REST `search/issues`: `search`, limit 30. |
| 7 | A conditional REST request (`If-None-Match`) | `304` twice; `X-RateLimit-Used` stayed at 23 |
| 8 | Counter consistency | `remaining`: 4 994, 4 991, 4 989, then 4 998, with no window reset in between. `/rate_limit` reported 4 used when a response header said 18. |

### Errors

| # | What | Result |
|---|---|---|
| 9 | PR number 9999 | `pullRequest: null`, `NOT_FOUND` |
| 10 | A repository that doesn't exist | `repository: null`, `NOT_FOUND` |
| 11 | `nodes(ids:)` with one made-up ID | The real node, then `null`, and one `NOT_FOUND` with `path: ["nodes", 1]` |

### PR A (#11): a PR with a workflow, closed unmerged

The branch carried a workflow with two jobs on `pull_request`: `fast` (an echo) and `slow` (`sleep 40`).

| UTC | Event or first sample showing it |
|---|---|
| 22:09:22.4 | `git push` returned |
| 22:09:25.1 | `gh pr create` returned |
| 22:09:26 | `OPEN`, `mergeable: UNKNOWN`, rollup `null`. In `viewer.pullRequests`: yes. In search: **no**. |
| 22:09:29 | `MERGEABLE`, `CLEAN`, rollup still `null`. In search: yes. |
| 22:09:34 | Rollup `PENDING`, `UNSTABLE`; both checks `IN_PROGRESS` |
| 22:09:37 | Both checks read `QUEUED` |
| 22:09:40 | `fast` completed, `SUCCESS` |
| 22:10:21 | `slow` completed; rollup `SUCCESS`, `CLEAN`. `updatedAt` still 22:09:23. |
| 22:13:50.6 | Second `git push` returned |
| 22:13:51 | Old head `dce9dbe`, old rollup `SUCCESS` |
| 22:13:54 | New head `cf54ec4`, `updatedAt` 22:13:51, `mergeable: UNKNOWN`, rollup `null`. The search hit shows the new head too. |
| 22:13:57 | `MERGEABLE`, `UNSTABLE`, rollup `PENDING` |
| 22:14:43 | Rollup `SUCCESS`; `updatedAt` still 22:13:51 |
| 22:15:25.4 | Comment added. Same second: `comments: 1`, `updatedAt` 22:15:25. |
| 22:15:36.5 | Converted to draft. At 22:15:38: `isDraft: true`, `updatedAt` 22:15:36, `mergeStateStatus` still `CLEAN`. |
| 22:15:47.9 | Marked ready. Same second: `isDraft: false`. |
| 22:15:59.5 | Closed. Same second: `CLOSED`, gone from `viewer.pullRequests(states: OPEN)`, **still in the `is:open` search**. |
| 22:16:06 | Gone from the search |

Afterwards the branch was deleted: `headRef: null`, `headRefName` and `headRefOid` unchanged.

### PR B (#12): a one-file PR, squash-merged

| UTC | Event or first sample showing it |
|---|---|
| 22:16:38.7 | `gh pr create` returned |
| 22:16:39 | `OPEN`, `UNKNOWN`. In `viewer.pullRequests`: yes. In search: no. |
| 22:16:42 | `MERGEABLE`, `CLEAN`. In search: still no. |
| 22:16:47 | In search |
| 22:16:54.4 | `gh pr merge --squash` returned (`mergedAt` 22:16:53) |
| 22:16:54 | `MERGED`, `mergeable: UNKNOWN`, `mergeStateStatus: UNKNOWN`. Gone from `viewer.pullRequests(states: OPEN)`, still in the `is:open` search. |
| 22:16:56 | Gone from the search |

### Older PRs, read only

- #1, untouched since 2026-09-26: `mergeable: UNKNOWN` on the first read, `MERGEABLE`, `CLEAN` on the next one a few minutes later.
- #4, #5, #6, merged 2026-10-17 by merge commit, squash and rebase: all `MERGED`, `mergeable: UNKNOWN`, `mergeCommit` set, `mergedAt` equal to `closedAt`.
- #8, closed 2026-10-17: `CLOSED`, `MERGEABLE`, `CLEAN`, `mergeCommit: null`.

### PR C (#13): failing, skipped and cancelled checks, and a commit status

The workflow had four jobs: `pass` (an echo), `fail` (`exit 1`), `skipped` (`if: false`) and `slow` (`sleep 300`). Log: `c1.log`.

| UTC | Event or first sample showing it |
|---|---|
| 22:25:52.6 | `gh pr create` returned |
| 22:25:54 | `MERGEABLE`, `CLEAN`, rollup `null`. In search: no. |
| 22:26:01 | Rollup `PENDING`, `UNSTABLE`. `skipped` already `COMPLETED`, `SKIPPED`; the other three `IN_PROGRESS`. In search: yes. |
| 22:26:04 | `pass` `SUCCESS`, `fail` `FAILURE`. **Rollup `FAILURE`, with `slow` still `IN_PROGRESS`.** |
| 22:26:33.3 | Commit status `circitor/manual` posted as `pending`. At 22:26:36 it's in the list as a `StatusContext`, `PENDING`. |
| 22:26:49.0 | The same status posted as `failure`. At 22:26:50: `FAILURE`. |
| 22:27:06.5 | `gh run cancel` returned |
| 22:27:22 | `slow` `COMPLETED`, `CANCELLED`. Rollup still `FAILURE`, `mergeable: MERGEABLE`, `UNSTABLE`. |

`updatedAt` stayed at 22:25:51 throughout.

### PR D (#14): a conflict

A branch cut from the commit before #12, adding the same file with other content. Log: `d1.log`.

| UTC | Event or first sample showing it |
|---|---|
| 22:28:06.9 | `gh pr create` returned |
| 22:28:07 | `mergeable: UNKNOWN`. In search: no. |
| 22:28:10 | **`CONFLICTING`, `DIRTY`**, rollup `null`. In search: yes. |

### PR E (#15): comments and reviews

| UTC | Event or first sample showing it |
|---|---|
| 22:34:21 | `gh pr create` returned. `updatedAt` 22:34:20. |
| 22:34:30 | A review with a body and one inline comment posted (`event: COMMENT`) |
| 22:34:32 | `reviews: 1`, `reviewThreads: 1`. **`updatedAt` still 22:34:20.** |
| 22:34:37 | A reply posted in the thread |
| 22:34:39 | `reviews: 2`. `updatedAt` 22:34:29, the first review's time. |
| 22:34:43 | An issue comment posted. At 22:34:45: `comments: 1`, `updatedAt` 22:34:42. |
| 22:35:18 | A second reply posted |
| 22:35:20, :23, :25 | `reviews: 3`. `updatedAt` still 22:34:42. |
| 22:35:31 | `updatedAt` 22:35:18 |

- GraphQL `databaseId` equalled the REST `id` for the issue comment, both review comments and the review. Each was fetched by that ID over REST.
- The reply's `pullRequestReview` wasn't the posted review: a reply makes its own.
- `timelineItems(itemTypes: [ISSUE_COMMENT, PULL_REQUEST_REVIEW])` returned one review and the comment, with `totalCount: 4`. Unfiltered it returned a commit, one review and the comment, also with 4.
- One query for all the IDs of one PR cost 1 point. For 40 PRs on top of the full record: issue comment IDs 2 points, plus review IDs 2, plus threads with their comments 6, the timeline 2. The record alone: 1.

### Tokens

- A made-up `gho_` token: HTTP `401`, `{"message": "Bad credentials", "documentation_url": "…", "status": "401"}`.
- No `Authorization` header: HTTP `403`, "API rate limit exceeded for <address>". The address's unauthenticated limit was already used up, so this isn't the normal answer.

### Left behind in `yachiko/experiment`

- PRs #11, #13, #14 and #15 (closed) and #12 (merged), all titled as throwaway.
- One file on `main`: `circitor-measure/b.txt`, from #12.
- All five `circitor-measure-*` branches are deleted. The workflow was never on `main`.
- Seven short Actions jobs, the longest cancelled after a minute: about 8 billed minutes on a private repository.

### Branch protection, and public repositories read as an outsider

- `GET` on `main`'s protection and on the repository's rulesets in `yachiko/experiment`: `403`, "Upgrade to GitHub Pro or make this repository public to enable this feature." So required checks and reviews can't be set up there.
- Instead, the 30 most recently updated open PRs of `cli/cli` and of `astral-sh/uv` were read once each (cost 1 a query). Nothing was written anywhere. Two more repositories returned nothing usable.

| | `cli/cli` | `astral-sh/uv` |
|---|---|---|
| `mergeStateStatus` | `UNKNOWN` 21, `BLOCKED` 7, `DIRTY` 2 | `CLEAN` 21, `BLOCKED` 5, `UNSTABLE` 3, `DIRTY` 1 |
| `mergeable` | `UNKNOWN` 21, `MERGEABLE` 7, `CONFLICTING` 2 | `MERGEABLE` 29, `CONFLICTING` 1 |
| `reviewDecision` | `REVIEW_REQUIRED` 29, `APPROVED` 1 | `null` 29, `CHANGES_REQUESTED` 1 |
| Rollup | `SUCCESS` 16, `FAILURE` 8, `null` 6 | `SUCCESS` 21, `FAILURE` 9 |
| Most checks on one PR | 25 | 187 |

- Across both, 31 PRs were drafts; none read `mergeStateStatus: DRAFT`.
- No PR had `autoMergeRequest` or `mergeQueueEntry`. `baseRef.branchProtectionRule` was `null` on all.
- `isRequired(pullRequestNumber:)` on one `cli/cli` PR's checks: 3 required (the three `build` jobs), 15 not. With a number that isn't the PR's own, every check node came back `null` with `NOT_FOUND`.

### The owner's Mac

Reported by the owner: `$TMPDIR` is `/var/folders/26/<id>`, the usual per-user temp directory.

## 2026-11-25: `gh auth token` with no login and with the variables

**Setup:** `gh` 2.101.0 on Linux, run locally. No request went to GitHub, and no token was printed: each run compared the output with a marker string, or sent stdout to `/dev/null`.

- **No login** (`GH_CONFIG_DIR` set to an empty directory, `GH_TOKEN` and `GITHUB_TOKEN` unset): exit status 1, nothing on stdout, `no oauth token found for github.com` on stderr. With `--user nosuchuser`, also exit 1, with `no oauth token found for github.com account nosuchuser`.
- **`GH_TOKEN` set** to a marker: `gh auth token --hostname github.com` printed the marker. The same with only `GITHUB_TOKEN` set.
- **`GH_TOKEN` set but empty:** the stored login's token was printed, so an empty variable is ignored.
