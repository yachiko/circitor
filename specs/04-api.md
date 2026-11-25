# API

**Status:** draft 0.2, 2026-11-25: notify and narrower conditions (D7, D8). Draft 0.1, 2026-11-24. Hardened: pass 1 (2026-11-25), pass 2 (2026-11-25), pass 3 (2026-11-25), planning findings (2026-11-25).

What the daemon answers on its socket. **The API is private to circitor's own CLI** (log D5): agents and people use the CLI ([05-cli](05-cli.md)), and nothing else is promised a stable API in v1.

## Basics

- **API-1** HTTP/1.1 over the Unix socket (OPS-4), JSON bodies, every route under `/v1/`. No authentication beyond the socket's file permissions: only the user can open it. **An unknown query parameter, body key or value is `bad_request` naming it, never ignored**, so a newer CLI's request is never read with another meaning. Flags (`checks`, `fresh`) take `1` or `0`. A value out of range is `bad_request`, never clamped.
- **API-2** Every response carries `version`, the daemon's version string. The CLI compares it with its own (CLI-9).
  - `GET /v1/status`'s `version` and `mode`, and `POST /v1/shutdown`, keep their shape in every version, so any CLI can read and stop any daemon.
- **API-3** **Errors** are `{"version": "…", "error": {"code": "…", "message": "…", "until": "…"}}`, with `until` `null` but for `github_unavailable` and `rate_limited` (API-4), and these codes:

| Code | HTTP | When |
|---|---|---|
| `bad_request` | 400 | A parameter is missing or invalid; the message names it |
| `not_found` | 404 | GitHub answers `NOT_FOUND` for the PR or its repository; a repository the token can't see is assumed to answer the same (docs/02 §5) |
| `unknown_route` | 404 | No such route, by path or by method: the CLI is newer than the daemon (CLI-9), or a bug |
| `auth` | 503 | There is no usable token (POL-20). The message names the token source in use (OPS-5) and its fix: `gh auth login` when gh has no login, or when GitHub refuses gh's token twice (it may have expired); when `gh` fails, can't be run or times out, that failure and `gh_path` (OPS-5); for `GH_TOKEN` or `GITHUB_TOKEN`, which only a foreground daemon's `gh` sees, fixing or unsetting the variable and restarting it. |
| `starting` | 503 | The first poll hasn't finished (API-18) |
| `github_unavailable` | 503 | The PR has no record and GitHub couldn't be reached to read it through, or answered the PR or its repository with an error other than `NOT_FOUND` (POL-16). The message then names the error's `type` and the token source in use (OPS-5), never GitHub's text (OPS-9). |
| `rate_limited` | 503 | The same, with the limit exhausted or a secondary limit in force (POL-23, POL-25); the message says when circitor will ask again. (proposal; GitHub's answer is assumed, docs/02 §2) |
| `limit` | 409 | Too many tracked PRs (STA-3), waiters (API-14) or subscriptions (NTF-2) |
| `no_session` | 409 | The session's socket isn't a socket of the user's, or can't be connected to (NTF-2). The message names the system's error. |

  - **A PR with a record is always answered from memory**, even when GitHub is unreachable; `stale` says so (MOD-16). A PR loaded from the state file has none until its first refresh (STA-8), and is read through, except by `wait`.
- **API-4** Every successful response carries `cursor` (STA-9) and `daemon`: `{tier, last_poll_at, problem, message, until}`.
  - `problem` is `null` or the first that holds: `auth` (POL-20), `rate_limited` (`exhausted` or `secondary`, POL-23), `github_unavailable` (POL-19's backoff, or POL-24 handled as it), `quota_low` (`capped` or `reserve`). The parentheses name the states behind each code; they aren't values.
  - `until` is when circitor will next ask GitHub: the reset time or the end of a secondary limit (POL-25), or the backoff's next try (POL-19); `null` otherwise. CLI-3 prints it. API-3's `github_unavailable` and `rate_limited` errors carry it as `error.until`.
  - `message` says what the problem means and what to do, `null` with none. For `auth`, `github_unavailable` and `rate_limited` it's API-3's message for that code: for `auth`, the fix for the token source in use. CLI-3 prints it.
  - `last_poll_at` is the last poll applied.
- **API-5** `GET /v1/status` returns the daemon's health and counts no client activity for POL-13. Its keys: `version`, `mode` (`spawned` or `foreground`, OPS-6), `started_at`, `last_poll` (`{at, outcome}`: API-16's `polled`, `auth`, `github_unavailable` or `rate_limited`), `cost_last_hour` (points), `github` (`{remaining, reset_at, login}`: GitHub's last reported values and the login it reports), `tracked` (the numbers of tracked PRs by `source`), `waiters`, `subscriptions` (their number, NTF-9), `owners` (in force), `token_source` (the source in use, never the token, OPS-5), `gh_path` (the `gh` that was run last, or `gh_path` as configured before any run, OPS-5), `paths` (`{config, state, socket, log}`: the files' paths, `state` being `state.json`'s), and `warnings`: a list of `{code, message, since}`. The tier is in `daemon` (API-4). (proposal: the names)
  - **Warnings.** `since` is when one was raised. A new warning needs a row here. (proposal: the codes)

| `code` | Raised by | Cleared when |
|---|---|---|
| `quota_capped` | circitor's count at 100 % of `quota.max_per_hour` (POL-17, POL-23) | The count drops below it |
| `quota_reserve` | GitHub's `remaining` below `quota.reserve` (POL-18, POL-23) | `remaining` is back at or above it |
| `pr_limit` | POL-7 and STA-3: found PRs left out | A poll leaves no found PR out |
| `pr_failing` | POL-21, one per PR | That PR's refresh is applied, or it's gone or dropped |
| `state_file_set_aside` | STA-8: a file that doesn't parse | The daemon exits |
| `state_file_newer` | STA-8: a file with a newer `version` | The daemon exits |
| `state_file_unwritable` | STA-7: a write of `state.json` failed | A write succeeds |
| `log_unwritable` | OPS-9: a log line couldn't be written | A line is written again |

- **API-19** **Response bodies.** Beside `version`, `cursor` and `daemon` (API-2, API-4), each route answers with these keys. A list is `[]` when empty and a flag `false` when not set; neither is ever left out. The event's own shape is MOD-15's. (proposal: the names)

| Route | Keys |
|---|---|
| `GET /v1/prs` | `records` (MOD-2), `untracked` and `pending` (lists of `pr`), `reset` |
| `GET /v1/prs/{owner}/{repo}/{number}` | `record` |
| `GET /v1/changes`, `GET /v1/prs/…/changes` | `events` (MOD-15), `records` (on a reset), `pending`, `more_events`, `reset` |
| `GET /v1/prs/…/wait` | `outcome`, `reason`, `record`, `events`, `records` (on a reset), `more_events`, `reset` |
| `POST /v1/prs/…/notify` | `registered`, `replaced`, `expires_at` (`null` when not registered), `outcome`, `reason`, `record`, `events`, `records` (on a reset), `more_events`, `reset` (NTF-2) |
| `DELETE /v1/prs/…/notify` | `cancelled`, a number (NTF-3) |
| `POST /v1/track`, `DELETE /v1/track/…` | `record` |
| `POST /v1/refresh` | `outcome`, `cost` (points) |
| `GET /v1/status` | API-5's |
| `POST /v1/shutdown` | none |

  - **`checks.items`** are in every single `record` (`show`, `wait`, `notify`, `track`, `untrack`), and in a list of `records` only for `list` with `checks=1` (API-6): a reset of `changes` or `wait` leaves them out.

## Reading

- **API-6** `GET /v1/prs` returns the tracked PRs, open ones first, then by `updated_at`, newest first, then by `pr`. `untracked` and `pending` (API-18) are sorted by `pr`.
  - `state=open` (the default) or `all`, which adds merged, closed and gone PRs still kept (STA-2).
  - `since=<cursor>` returns only records whose `rev` is after it, **whatever their `state`**: a PR that merged or closed after the cursor is listed. PRs dropped after it **and not tracked now** are named in `untracked`, from the event history (MOD-15). A too-old cursor (STA-9) gives `reset: true`, and all the records `state` asks for are returned.
  - Each record is as MOD-2, with `checks.items` left out unless `checks=1`; `failed_items` is always there.
- **API-7** `GET /v1/prs/{owner}/{repo}/{number}` returns one record, with `checks.items`.
  - A PR with no record is read through (POL-16).
  - `fresh=1` forces a refresh first (POL-15).
- **API-8** `GET /v1/changes?since=<cursor>` returns the events after the cursor (MOD-15), oldest first, and the new cursor. `GET /v1/prs/{owner}/{repo}/{number}/changes` takes the same parameters for one PR.
  - Without `since` it returns no events, just the cursor: a starting point.
  - `limit=N`, 1 to 500 (STA-10), returns at most N events, oldest first, with `more_events: true` when there are others, and as its cursor that of the last event returned. API-9's answer takes it too. (MOD-15's `more` is about one event's items.)
  - A cursor that's too old (STA-9) gives `reset: true` and the current records of the PRs in scope. It gets no events, except a cursor from another epoch: it gets every event of the current one while the history still holds them all.
  - **A cursor from an answer for one PR** (its `changes`, a `wait`) **covers that PR only:** other PRs' events at or below it weren't in the answer. A call over every PR (`list`, `changes` without a PR) takes a cursor from one over every PR.

## Waiting

- **API-9** `GET /v1/prs/{owner}/{repo}/{number}/wait?until=…` **holds the request open until its condition holds, the PR reaches an end, or the timeout passes.**
  - The condition is first evaluated once the read-through or the arrival refresh (API-13) has been applied or has failed, or at arrival when neither is due, and again after every refresh of that PR (STA-4). **An arrival refresh that isn't sent counts as failed at once:** inside POL-15's window, in POL-19's backoff, in the `auth` state, or when POL-23 sends none. `timeout=0` waits for a refresh that is sent.
  - **A wait's arrival, in order:** the parameters are checked (API-1); STA-3's and API-14's limits; the read-through, if due (POL-16); the waiter is registered and the headers sent; the arrival refresh, if due (API-13); the first evaluation. A refusal before the waiter is registered answers as API-3.
  - A PR with no record is read through first (POL-16), **except one loaded from the state file:** its wait is held, with no read-through and no arrival refresh, until its first refresh is applied (STA-8), and first evaluated then. Answered before that, it has `record: null`; `daemon.problem` says why when a problem holds, and with none, the first refresh hasn't answered yet.
  - **The arrival cursor is sent at once.** When the waiter is registered, the status line and headers go out, with `Circitor-Cursor` set to the cursor then; the body follows when the wait ends. Every 200 answer to a `wait` carries `Circitor-Cursor`: one sent before the waiter was registered (API-12) sets it to the answer's cursor.
  - Once the waiter is registered, the answer is always HTTP 200 with `outcome`, the record, the PR's events since `since` (or after `Circitor-Cursor`; with either too old when the answer is built (STA-9), `reset: true` and the events as API-8 gives them), and the cursor. A wait refused before that (`limit`, or a failed read-through) answers as API-3.
  - **A PR with a waiter stays tracked.** `untrack` makes it `asked` (API-15, STA-1), and `keep_done` drops it only once its waiters have ended (STA-2). At STA-3's limit, a `wait` on a PR that isn't tracked gets `limit` (API-3).
- **API-10** **Conditions:**

| `until` | Holds when | Parameters |
|---|---|---|
| `changed` | The record's `rev` is after `since`. Without `since`: after the revision at arrival, taken after the read-through and the forced refresh (API-13), so it waits for the next change. A too-old `since` (STA-9) holds at once. **With `kind`:** an event of this PR after that point (MOD-15) has one of the kinds named. | `since`, `kind` |
| `checks-done` | `checks.state` is `passing`, `failing` or `none`, **and** `checks.settled` (MOD-6), **and**, if `sha` is given, `head_sha` starts with it, **and**, if `rerun_since` is given, a `checks` event (MOD-15) of this PR is after it. **With `fail_fast`,** `checks.failed` above 0 stands in for the first part: one failure is enough, while others still run (MOD-5). | `sha`, `rerun_since`, `fail_fast` |
| `merged` | `state` is `merged` | – |

  - **`since` is taken by every condition:** it sets where the answer's events start (API-9), and for `changed` it's also the condition. `sha`, `rerun_since` and `fail_fast` are taken by `checks-done` only, and `kind` by `changed` only; with another condition they're `bad_request` (API-1).
  - **`sha` is how an agent that has just pushed avoids the previous commit's result** (docs/02 §4: GitHub serves the old head for up to 3 s). It's 7 to 40 hex characters, compared without case. An agent that pushed should always pass it.
  - While `sha` is given and `head_sha` doesn't match, the condition doesn't hold, whatever the checks say.
  - **`rerun_since` is how an agent that started CI again on the same head avoids the previous run's result:** a re-run, `gh pr ready`, a close and reopen. It passes the cursor it had before. If CI never runs again, the wait times out. A too-old `rerun_since` (STA-9) drops this part, and the answer has `reset: true`. (proposal; a re-run isn't measured, docs/02 §6)
  - **A truncated `failing` may be early** (MOD-7): unread checks may still run, and nothing circitor reads says when they end (the rollup reads `FAILURE` while they run, docs/02 §4). It still holds `checks-done`; the answer keeps `checks.truncated`. (proposal)
  - **`kind` narrows `changed` to the events an agent cares about** (D8). It's a comma-separated list of MOD-15's kinds, but `tracked` and `untracked`: `activity` is a new comment, review or thread reply (MOD-12), `review` the review decision. An unknown kind is `bad_request` (API-1). A too-old `since` (STA-9) still holds at once: what was missed can't be told apart. (proposal)
  - **`fail_fast` answers at the first failure** (D8), where `checks-done` alone waits for the last check; `gh pr checks --watch --fail-fast` does the same. It's a flag (API-1). The `sha` and `rerun_since` parts still have to hold. The outcome is `satisfied`, and the record's `checks.state` is still `pending`. (proposal)
- **API-11** **Outcomes:**

| `outcome` | Meaning | `reason` |
|---|---|---|
| `satisfied` | The condition holds | – |
| `ended` | It can no longer hold | `merged` (for `checks-done`, and for `changed` with `kind`), `closed`, `gone`, or `head_moved` |
| `timeout` | Neither, in time | – |

  - **The condition is checked before the ends.** An evaluation in which it holds answers `satisfied`, even when the PR is also merged, closed or gone; the record's `state` says so.
  - `closed` and `gone` end `merged` and `checks-done`. `merged` satisfies `until=merged`, and ends `checks-done` while its checks aren't done. A merge, a close or `gone` satisfies `changed` like any change. **With `kind`**, one of the three whose kind isn't named ends the wait instead, with that reason: the PR is over, and the wait would only run out its time.
  - **`head_moved`:** for `checks-done` with `sha`, if `head_sha` matched once and then changed, or still doesn't match on a refresh sent at least 60 s after the wait began, the wait ends: someone else pushed, or the SHA is wrong. The clock alone never ends it. (proposal)
  - **When a PR has both ended and moved its head**, the `reason` is its state's (`merged`, `closed` or `gone`): it says more.
  - For `checks-done`, `satisfied` covers passing and failing alike; the record's `checks.state` tells which.
- **API-12** **Timeout.** `timeout` is in seconds: default **100**, at most 3 600, and `0` means "evaluate once and answer": its first evaluation waits for the read-through or the arrival refresh that is sent (API-9), the one wait that can outlast its deadline. The default is under the 120 s at which three of the four clients stop waiting for a command, one of them by killing it (docs/03 §5). **The deadline runs from the request's arrival:** reading through (POL-16) and the arrival refresh (API-13) count against it. It counts the time asleep (POL-22). **Otherwise nothing waits past the deadline.** If it passes before the first evaluation, the condition is evaluated once on the record in memory, and the wait answers. **If it passes during a read-through,** the wait answers 200 with `outcome: timeout` and `record: null`; the read-through goes on, and tracks the PR if it succeeds. (proposal: the values)
- **API-13** **A waiter makes its PR hot** (POL-14), and on arrival forces a refresh unless POL-15's window holds, sent ahead of a poll in flight. So, in the `normal` quota state (POL-23), a `wait` sees a change within about `hot_interval` of GitHub showing it. Slowed, it's twice that; in reserve, `idle_interval`. Capped, rate-limited, in a backoff (POL-19) or in the `auth` state, no refresh may follow the arrival: the wait answers at once if the record already holds the condition or an end, and otherwise may only time out: `daemon.problem` (API-4) says why.
- **API-14** **Waiters end cleanly.** When the client closes the connection, the waiter is removed. At most 64 waiters are held; one more gets `limit`. At shutdown every open connection, a waiter's or another, is closed without an answer (OPS-7, CLI-10). (proposal: 64)

## Being told

A Claude Code session can ask to be told once instead of waiting: [08-notify](08-notify.md). Its conditions are API-10's and its outcomes API-11's.

## Changing the set

- **API-15** `POST /v1/track` with `{"pr": "owner/repo#12"}` fetches the PR (POL-16) and tracks it as `manual`; it answers with the record. `DELETE /v1/track/{owner}/{repo}/{number}` makes a `manual` PR `authored` if it's the user's own, open, and its owner is in `owners` (POL-5), and drops it otherwise, or makes it `asked` while it has a waiter or a subscription (API-9, NTF-4). On an `authored` or `asked` PR it changes nothing. Both are idempotent.
  - **`untrack` never reads through:** on a PR that isn't tracked it changes nothing and answers with `record: null`. `track` of a PR with a record changes only its `source` and doesn't call GitHub; of a loaded PR with no record (STA-8), it sets the saved `source` to `manual` first, then reads it through (POL-16) and answers with the record. If the read-through fails, it answers that error, and the source stays `manual`, saved (STA-7). A loaded `manual` PR with no record is dropped by `untrack`, or made `asked` while it has a waiter or a subscription: if it's the user's own and open, the next poll finds it again as `authored` (POL-4).
- **API-16** `POST /v1/refresh` starts a poll now and answers when it's done, with `outcome` and the points it cost (POL-15). It's done once every request of that poll (POL-7, POL-8, POL-12), and the first refresh of every PR it found for the first time (POL-4), has been applied or has failed. `outcome` is `polled` when every request answered; `shared`, at a cost of 0, inside POL-15's window or joined to a poll in flight; or the problem that stopped it, also when it stopped only some requests: `auth`, `github_unavailable` or `rate_limited`. The points count every request sent. **It never waits out a backoff or a reset** (POL-19, POL-23): it then answers at once with that problem, and the CLI exits as for the API-3 error of the same name (CLI-5).
- **API-17** `POST /v1/shutdown` answers, then stops the daemon as a `SIGTERM` would (OPS-7).

## Starting up

- **API-18** **Until the first poll has finished**, `GET /v1/prs` and `/v1/changes` wait for it, up to 10 s, and then answer `starting`. **The first poll has finished** once its finding and the first refresh of every PR it found (POL-4) or loaded (STA-8) have been applied or have failed. **Once it has finished, they answer with the records there are**, and name in `pending` each PR found or loaded whose first refresh hasn't been applied, but one STA-3 leaves out, then and at any time after, including one found by a later poll. Only a first poll none of whose requests was answered (POL-19) gives the problem's error instead (`auth`, `github_unavailable` or `rate_limited`, API-3), until a request is answered. Routes with a PR in their path don't wait: they read a PR with no record through, but `wait` (API-9), `untrack` (API-15) and `notify`'s cancel (NTF-3). `status` and `shutdown` always answer.
