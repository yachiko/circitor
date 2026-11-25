# Hardening log: 02-polling.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 3/10/5 | 12 | 0 | 0 | 0 | 6 |
| 2 | 2026-11-25 | failure modes | 5/10/4 | 12 | 0 | 0 | 0 | 7 |
| 3 | 2026-11-25 | implementability | 1/8/8 | 9 | 0 | 0 | 0 | 8 |
| 4 | 2026-11-25 | planning findings | 1/3/7 | 11 | 0 | 0 | 0 | 0 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3, with 29 raw findings; 18 after merging 11 duplicates. Dropped in verification: 0 whole findings; 2 sub-claims trimmed (1 "consider adding", 1 belonging to 06-operations).

### H1-1 · high · A refused `isRequired` empties the check nodes, and the PR then reads "no checks"
- Location: specs/02-polling.md POL-9, POL-21; docs/02-github-api.md §4 (required checks)
- Status: applied: POL-9 says a refusal empties the check, the refresh isn't applied, and the PR is read again without the field; POL-21 applies a node only if no error's path points inside it; TST-3 gains "an error inside a PR's node"
- Still unmeasured: `isRequired` on the owner's own repositories. The owner chose not to measure in this pass.

### H1-2 · high · A count change is applied before its items are fetched, and a failed fetch loses them
- Location: specs/02-polling.md POL-12; specs/03-state.md STA-4; specs/01-model.md MOD-14, MOD-15
- Status: applied: POL-12 makes the count change and its items one change, and keeps the old counts when the ID request fails; MOD-15's `activity` row carries `more`

### H1-3 · high · The budget doesn't bound forced refreshes, and POL-15 mixes a one-PR refresh with a whole poll
- Location: specs/02-polling.md POL-15, POL-17; specs/04-api.md API-13, API-16
- Status: applied: POL-15 rewritten (`fresh` and a wait's arrival refresh one PR, `refresh` runs a whole poll at most once in 10 s, 30 s windows when capped or in reserve); API-16 says what it answers inside the window

### H1-4 · medium · One steady waiter sits exactly on the 80 % throttle line
- Location: specs/02-polling.md POL-13, POL-14, POL-17; specs/07-testing.md TST-6
- Status: applied: POL-14 lets a poll count as the hot refresh (one hot PR adds 240 an hour, 360 with the active tier) and batches hot PRs by 40; TST-6's 370 is now 250

### H1-5 · medium · "A poll costs 1 point" holds only for small sets, and ended PRs are refreshed on every poll
- Location: specs/02-polling.md POL-4, POL-10, POL-13; specs/03-state.md STA-2
- Status: applied: POL-10 refreshes a `merged`, `closed` or `gone` PR every `idle_interval` only; POL-4 points to it; POL-13 says when a poll costs more than 1 point

### H1-6 · medium · The quota states don't say which requests they stop, or what the API reports
- Location: specs/02-polling.md POL-17, POL-18; specs/04-api.md API-4
- Status: applied with change: the table of states went in as a new requirement, POL-23, so it has an ID; POL-13 says hot isn't a tier; POL-15's 30 s windows cite POL-23

### H1-7 · medium · The `auth` state contradicts the API, and a missing token has no polling rule
- Location: specs/02-polling.md POL-20; specs/04-api.md API-3, API-4; specs/06-operations.md OPS-5
- Status: applied: POL-20 answers tracked PRs from memory with `problem: auth`, gives the error only to requests that need GitHub, keeps waiters, and covers a missing token; TST-8 `token-rotated` and OPS-5 follow

### H1-8 · medium · POL-16 turns every failed read-through into `not_found`
- Location: specs/02-polling.md POL-16; specs/04-api.md API-3
- Status: applied: POL-16 maps each failure to `not_found`, `github_unavailable`, `auth` or `rate_limited`, and tries a read-through once during the backoff (proposal)

### H1-9 · medium · The baseline fetch of STA-5 has no request in the polling spec
- Location: specs/02-polling.md POL-4, POL-6, POL-12; specs/03-state.md STA-5
- Status: applied: POL-4's first refresh also fetches the last 20 IDs (2 points for 40 PRs); POL-12 counts an unseen ID as new only if it isn't older than the newest item seen of its kind; STA-5 cites POL-4

### H1-10 · medium · POL-11 fits a fresh PR only, and chases an `UNKNOWN` that never resolves
- Location: specs/02-polling.md POL-11; docs/02-github-api.md §4 (mergeability), §6 item 7
- Status: applied: open PRs only, one shared follow-up that never causes another, also after a PR's first refresh since the start; the 3–4 s is labelled as measured on new, small PRs

### H1-11 · medium · POL-9 changes the query's shape, so POL-7's cost and POL-21's error path are unmeasured
- Location: specs/02-polling.md POL-4, POL-7, POL-21; docs/02-github-api.md §2, §5; docs/05-measurements.md cost row 4
- Status: applied: POL-4 names the aliased `node(id:)` form; POL-7 and POL-21 say the aliased form's cost and error path aren't measured
- Still unmeasured: cost, node count, duration and error path of 40 aliased fields with `isRequired`. The owner chose not to measure in this pass.

### H1-12 · medium · The Errors section rests on responses labelled `assumed`, and no rule covers a 403
- Location: specs/02-polling.md POL-19, POL-20, POL-21; docs/02-github-api.md §5; specs/README.md conventions
- Status: applied: POL-19 to POL-21 carry their labels, and POL-24 is the catch-all (anything else is handled as POL-19; only `NOT_FOUND` counts towards `gone`)

### H1-13 · medium · A `gone` PR has no way back, and "three polls" isn't a fixed time
- Location: specs/02-polling.md POL-21; specs/03-state.md STA-2; specs/01-model.md MOD-15
- Status: applied in pass 2, as part of H2-12 (the way back is POL-10, STA-2 and H2-2)
- Note: a token that sees fewer repositories (a `GITHUB_TOKEN` in the spawning shell, a lapsed SSO authorisation) makes every private PR `gone` after three refreshes, 30 s for a hot PR if a hot refresh counts as a poll. Nothing in POL-21 says what happens when the node returns. STA-2 was changed during this pass, outside it, to let a gone PR that GitHub returns again be open again (MOD-11); POL-21 doesn't say so yet, nor whether a `gone` PR counts on hot refreshes. A good candidate for pass 2.

### H1-14 · low · POL-13 disagrees with API-5 and the glossary, and never says when the first poll runs
- Location: specs/02-polling.md POL-13; specs/04-api.md API-5, API-18; specs/00-overview.md glossary ("Tier")
- Status: parked
- Note: `status` counts no client activity (API-5) but POL-13 says "asked for anything"; the glossary still lists hot as a tier; no rule says the daemon polls once at start.

### H1-15 · low · "At once" and "straight after" clash with POL-2's one request at a time, and the single lane has no order
- Location: specs/02-polling.md POL-2, POL-4, POL-13, POL-16
- Status: parked
- Note: POL-4's "one request" also still reads against POL-7's batches.

### H1-16 · low · Values with no (proposal), and facts with no label
- Location: specs/02-polling.md POL-3 (10 minutes; token rotation is `documented`), POL-12 (last 20), POL-19 (5 s to 5 min), POL-20 (60 s)
- Status: parked

### H1-17 · low · POL-17's own count misses requests that return no `rateLimit` (applied in pass 4: H4-6)
- Location: specs/02-polling.md POL-17; docs/02-github-api.md §2 (a timed-out query deducts extra points, documented)
- Status: parked

### H1-18 · low · POL-18 puts a hard threshold on a counter that was seen to jump
- Location: specs/02-polling.md POL-18, POL-23; docs/02-github-api.md §2 (counters lag, observed)
- Status: parked
- Note: near the reserve, consecutive responses can flip the state, the warning and a log line. The `slowed` state at 80 % has the same lack of hysteresis.

## Out of scope (other files, not walked through)

- specs/05-cli.md CLI-5: no exit code for the API's `github_unavailable`, `rate_limited`, `starting` and `limit`.
- specs/04-api.md API-15: untrack makes a PR `authored` "if it's the user's own and open", ignoring `owners` (POL-5); nothing fetches the viewer's login.
- specs/03-state.md STA-3: doesn't say what happens to an `asked` PR when the 200 are full.
- specs/06-operations.md OPS-5: the first CLI's environment token decides the account for every session sharing the daemon.
- specs/06-operations.md OPS-1, OPS-2: the ranges allow sets that can't work (`hot_interval` above `interval`; `quota.max_per_hour` below 3 600 ÷ `interval`).
- specs/01-model.md MOD-12: `review_state` has no value for GitHub's `PENDING`.

## Pass 2: failure modes (2026-11-25)

Reviewers: 3, with 25 raw findings; 19 after merging 5 duplicates. Dropped in verification: 1 (out of scope: 01-model MOD-6). 12 walked through, 7 parked. Another session hardened 03-state at the same time and edited this file (POL-4, POL-7, POL-21) during the pass; the fixes below were applied on top of its text.

### H2-1 · high · One PR's error abandons the whole poll, on every poll
- Location: specs/02-polling.md POL-24, POL-21, POL-19
- Status: applied: POL-24 sends only whole-response failures to POL-19; a `null` node with another error is handled as POL-21, and the rest of the response is applied

### H2-2 · high · A reopened PR stays `closed` for up to `idle_interval`
- Location: specs/02-polling.md POL-10, POL-4; specs/07-testing.md TST-8 `reopened`
- Status: applied: a PR the open list shows is refreshed in that poll as an open one; TST-8 `reopened` says "at the next poll"

### H2-3 · high · A failed second page of checks can leave a false `passing`
- Location: specs/02-polling.md POL-8
- Status: applied: a PR's check pages are one read; a failed page leaves the record as it was, `stale`

### H2-4 · high · Without a token, a network or quota, the first poll never finishes, and `list` answers `starting` forever
- Location: specs/02-polling.md POL-19, POL-20, POL-4; specs/04-api.md API-18
- Status: applied: a failed or unsent first poll has finished (POL-19); API-18 answers the problem's error until a poll is applied; a found PR is tracked only once its first refresh is applied (POL-4)

### H2-5 · high · Rate-limit answers fall into POL-24's 5 s retry
- Location: specs/02-polling.md POL-18, POL-23, POL-24; specs/07-testing.md TST-3
- Status: applied: new POL-25 recognises an exhausted limit (`x-ratelimit-remaining: 0`, reset from `x-ratelimit-reset`) and a secondary limit (`Retry-After`, else 60 s doubling); POL-23 gains a `secondary` row; TST-3 gains the misbehaviour
- Note: the answers are documented in GitHub's GraphQL rate-limit page (references.md), not yet in docs/02 §2, which still labels them `assumed`. Left to the owner.

### H2-6 · medium · POL-19's backoff doesn't say which requests it holds back, or what ends it
- Location: specs/02-polling.md POL-19, POL-16
- Status: applied: the backoff holds every request but one read-through per step; any success ends it and starts a poll; a wake restarts it at 5 s; answered requests of a split poll are applied

### H2-7 · medium · A failed periodic token read can put a working daemon into `auth`
- Location: specs/02-polling.md POL-3, POL-20
- Status: applied: a failed periodic read keeps the token in hand; no token only at start leads to `auth`; in `auth` a request needing GitHub reads the token first, at most once in 5 s

### H2-8 · medium · A PR looked at once is refreshed at full cadence for an hour
- Location: specs/02-polling.md POL-10; specs/01-model.md MOD-16
- Status: applied: an `asked` PR that isn't hot is refreshed at the ended rate; MOD-16's stale interval follows

### H2-9 · medium · No duration names its clock, and a sleep is noticed only when a poll falls due
- Location: specs/02-polling.md POL-17, POL-19, POL-22, POL-25; specs/04-api.md API-12; specs/01-model.md MOD-16
- Status: applied: new POL-26 (wall clock for GitHub's windows, monotonic for the rest); POL-22 rewritten (wall vs monotonic gap over 30 s, checked on every tick and request); API-12's deadline counts the time asleep; MOD-16 cites a noticed sleep

### H2-10 · medium · A saved `authored` PR is never checked again against `owners` or the account
- Location: specs/02-polling.md POL-5; specs/03-state.md STA-6, STA-8
- Status: applied: an `authored` PR the filter no longer admits, on loading or missing from a complete finding while open, becomes `asked`

### H2-11 · medium · Found PRs are matched by name, so a renamed or transferred repository duplicates a PR
- Location: specs/02-polling.md POL-4, POL-6; specs/01-model.md MOD-1, MOD-14, MOD-15
- Status: applied: matched by node ID; POL-6 reads the owner and name; a new name is an `edited` change on `pr` (MOD-14, MOD-15)
- Still unmeasured: what a rename or transfer does to a node ID.

### H2-12 · medium · Losing access looks like deletion, and "three polls running" isn't a time
- Location: specs/02-polling.md POL-21, POL-24
- Status: applied: `gone` needs `NOT_FOUND` on every refresh for 5 minutes, at least three; a PR failing with any error for 5 minutes raises a warning naming the PR, the error's `type` and the token source. Closes H1-13.

### H2-13 · medium · A restarted daemon starts its spend count at zero
- Location: specs/02-polling.md POL-17; specs/03-state.md STA-6
- Status: parked
- Note: each start pays the baseline again (2 points for 40 PRs) and POL-17's count starts at 0, so `slowed` and `capped` never trigger across frequent restarts. The main driver is CLI-9 (out of scope below). Fix idea: keep points spent per minute over the last hour in state.json.

### H2-14 · medium · A query that times out at GitHub is resent unchanged, and pays each time
- Location: specs/02-polling.md POL-2, POL-7, POL-19; docs/02 §2 (a timed-out query deducts extra points, documented)
- Status: parked
- Note: if 40 aliased PRs with `isRequired` take over 10 s, every retry fails and costs extra. Fix idea: a 502 or 504 halves the next batch. Needs the duration measured (see H1-11).

### H2-15 · medium · A comment deleted and posted again between two refreshes is never reported
- Location: specs/02-polling.md POL-12
- Status: parked
- Note: sticky-comment bots in "recreate" mode leave the count unchanged. Fix idea: also fetch IDs when `updated_at` moved with no other visible change. Rests on unmeasured deletions (docs/02 §6 item 4).

### H2-16 · low · POL-14's "while its status is" doesn't say when the status is read
- Location: specs/02-polling.md POL-14
- Status: parked
- Note: suggested wording: "within 2 minutes of a client asking for it by name, for as long as its `status` is `checks_pending` or `unknown`".

### H2-17 · low · Redirects and bodies that aren't complete JSON aren't named
- Location: specs/02-polling.md POL-24; specs/06-operations.md OPS-16
- Status: parked
- Note: a captive portal's redirect; a 3xx and a truncated or HTML body should be POL-19, and redirects never followed.

### H2-18 · low · A PR tracked again in the same epoch is baselined again, and what arrived in the gap is lost
- Location: specs/02-polling.md POL-4; specs/03-state.md STA-1, STA-5
- Status: parked
- Note: an `asked` PR dropped after 60 min and read through again later baselines the review that arrived in between; the cursor is still valid, so no `reset`.

### H2-19 · low · Nothing stops a daemon whose socket or binary is gone from polling
- Location: specs/02-polling.md POL-13; specs/06-operations.md OPS-6, OPS-8
- Status: parked
- Note: after `brew uninstall`, or with the socket removed, a daemon with `exit_after = 0` or in the foreground polls on unreachably. The README doesn't list what an uninstall leaves.

## Out of scope, pass 2 (other files, not walked through)

- specs/01-model.md MOD-6: a head that returns to an earlier SHA (A → B → A after a force-push) may count as already settled, from "first saw this `head_sha`".
- specs/05-cli.md CLI-9: any version difference replaces the daemon, so two installed binaries (brew and a dev build) replace each other on every call.
- specs/03-state.md STA-6: a `gone` PR isn't saved, so a `manual` PR turned `gone` by a token problem is forgotten at a restart.
- docs/02-github-api.md §2: the documented exhausted and secondary-limit answers (GraphQL rate-limit page) aren't recorded; POL-25 cites references.md until they are.

## Pass 3: implementability, testability, simplification (2026-11-25)

Reviewers: 3 (implementer, tester, simplifier), with 26 raw findings; 20 after merging 5 duplicates and 1 out of scope. Dropped in verification: 3, all handled by other sessions during the pass: `gone` counted per request (MOD-11 and POL-21 now count in a poll only), `required` deferred to after D6 (POL-9 struck, MOD-3, D6), and POL-9's re-read timing (moot with POL-9 gone). Two items parked in other logs were merged in: 04-api H2-15 (into H3-9) and the ended-PR half of 01-model H2-15 (into H3-5).

### H3-1 · high · A PR's first refresh asks only for IDs, but POL-12 and STA-6 need each item's time
- Location: specs/02-polling.md POL-4, POL-12; specs/03-state.md STA-5, STA-6
- Status: applied: every first refresh (found, read through, loaded) asks POL-12's fields, times included; STA-5 says so

### H3-2 · medium · `reserve` overrides `capped`, and the quota rules are stated three times
- Location: specs/02-polling.md POL-17, POL-18, POL-23; specs/04-api.md API-5; specs/07-testing.md TST-6
- Status: applied: POL-23's rows put `capped` below `reserve`, so the stricter wins; POL-17 and POL-18 keep their triggers and point to POL-23; each warning follows its own condition (POL-23, API-5's clear column); TST-6's reserve line cites POL-23

### H3-3 · medium · Nothing leads out of the `auth` state
- Location: specs/02-polling.md POL-20
- Status: applied: only a 401 on the repeat enters `auth`; a read giving another token allows one try at once, the same token one try every 10 minutes; a success ends `auth`

### H3-4 · medium · "Tick" is never defined, yet five rules are checked "at each tick"
- Location: specs/02-polling.md POL-22, POL-26; specs/03-state.md STA-2; specs/06-operations.md OPS-17
- Status: applied: POL-26 defines a 10 s monotonic tick that checks the clocks and sends nothing; POL-22 and OPS-17 cite it

### H3-5 · medium · Cadences have no anchor, so the hot cost depends on timer phase
- Location: specs/02-polling.md POL-10, POL-13, POL-14; specs/07-testing.md TST-6
- Status: applied: every cadence runs from send times on the monotonic clock, due at "at least" its period; a hot PR is due `hot_interval` after its last refresh of any kind, so its refreshes fall between polls

### H3-6 · medium · POL-26 makes MOD-8's wall-clock 60 s monotonic
- Location: specs/02-polling.md POL-26; specs/01-model.md MOD-8
- Status: applied: MOD-8's 60 s is in the wall-clock list; the monotonic list is examples

### H3-7 · medium · POL-12's request reads as one per PR
- Location: specs/02-polling.md POL-12
- Status: applied: batched as POL-7, up to 40 PRs, each PR's part applied or failed on its own (proposal; 1 point by the formula, not measured)

### H3-8 · medium · An answer that spent the last point counts as "exhausted", and its data may be dropped
- Location: specs/02-polling.md POL-25
- Status: applied: only an answer without `data` is the exhausted answer; one with data is applied, and the daemon is `exhausted` after it

### H3-9 · medium · A merged or closed PR with a waiter: hot rate or ended rate?
- Location: specs/02-polling.md POL-10, POL-14; specs/01-model.md MOD-16; specs/04-api.md API-13
- Status: applied: POL-10 has one table, a PR's period (hot, then ended or `asked`, then the tier's); MOD-16 cites it. Closes 04-api H2-15.

### H3-10 · low · POL-12's `more: true` has no observable condition, and disagrees with STA-5 (applied in pass 4: H4-2)
- Location: specs/02-polling.md POL-12; specs/03-state.md STA-5
- Status: parked
- Note: the daemon can't see "more than 20 new"; STA-5's "all 20 fetched of a kind are new" is observable. Use it in both.

### H3-11 · low · POL-12's time rule makes STA-5's 200-ID memory redundant
- Location: specs/02-polling.md POL-12; specs/03-state.md STA-5, STA-6
- Status: parked
- Note: only IDs at the newest time decide anything. One rule in POL-12 (newest `at` and the IDs at it, as STA-6 saves), STA-5's 200 struck. Also settles 01-model's out-of-scope "newer than" vs "isn't older than".

### H3-12 · low · Rules stated in two places
- Location: specs/02-polling.md POL-4 (vs MOD-1), POL-7 (vs STA-3), POL-13 (vs OPS-8), POL-19 (vs API-18)
- Status: parked
- Note: each pair is one edit from disagreeing; keep one source and point to it.

### H3-13 · low · Which failed requests start the backoff, and POL-6's fields, are left to the code
- Location: specs/02-polling.md POL-19, POL-6
- Status: parked
- Note: POL-19 names polls only; a failing hot refresh could hammer every 10 s. POL-6 omits `contexts.totalCount`, `pageInfo`, `rateLimit` and the `Date` header; `frag.graphql` is a cost sample, not the query.

### H3-14 · low · POL-25's secondary-limit match names no text
- Location: specs/02-polling.md POL-25; specs/07-testing.md TST-3
- Status: parked
- Note: the code and the fake would share one guess. Name the substring and its source, or treat a 403 with `Retry-After` as one; record a fixture once docs/02 §6 item 6 is measured.

### H3-15 · low · POL-17's "second opinion" describes no behaviour
- Location: specs/02-polling.md POL-17
- Status: parked
- Note: say GitHub's counters never change circitor's count, and are used only for POL-18 and `status`.

### H3-16 · low · POL-11's 3 s follow-up could be struck, or kept for hot PRs only
- Location: specs/02-polling.md POL-11
- Status: parked
- Note: a waiter's PR is hot already, so the gain is at most `hot_interval` − 3 s; the cost is a third per-PR timer and a 60 s window unlike POL-14's 2 minutes.

### H3-17 · low · The read-through exception to the backoff is stated twice, and is asymmetric (in part: pass 4, H4-10)
- Location: specs/02-polling.md POL-16, POL-19
- Status: parked
- Note: an untracked PR probes GitHub during a backoff, a tracked one's `fresh` doesn't. Dropping it loses at most one backoff step for read-throughs.

## Out of scope, pass 3 (other files, not walked through)

- specs/07-testing.md: no acceptance check for POL-3 (token re-read), POL-7 (batches of 40, finding beyond 200), POL-15 (forced-refresh windows and order), POL-24 (one PR's error, rest applied), POL-26 (clock set back), POL-5's demotion to `asked`, POL-10's `asked` rate, POL-2's 1 s gap, and now POL-20's way out of `auth` and POL-26's tick. Another session was hardening 07-testing during this pass.

## Pass 4: planning findings (2026-11-25)

No reviewers: the findings came from building the implementation plan (`plan/README.md`, "Spec issues found while planning") and from its brief writers and reviewers, and were verified against the spec text.

### H4-1 · high · POL-13's "at least its period ago" makes every poll two requests for good
- Location: specs/02-polling.md POL-13, POL-4, POL-2; specs/07-testing.md TST-6
- Status: applied: every request of a poll counts as sent when the poll's first request was; MOD-2's and MOD-6's send times stay each request's own

### H4-2 · medium · `more: true` has no observable condition, and disagrees with STA-5
- Location: specs/02-polling.md POL-12; specs/03-state.md STA-5
- Status: applied: all 20 read are new and the count rose by more than 20; for a loaded PR, all 20 new. STA-5 points to POL-12. Closes H3-10.

### H4-3 · medium · POL-19 doesn't say which records go stale
- Location: specs/02-polling.md POL-19; specs/01-model.md MOD-16
- Status: applied: the PRs of the failed request and of the poll's requests not yet sent

### H4-4 · medium · POL-20: a refused gh-login token, and `problem` during the one try
- Location: specs/02-polling.md POL-20; specs/04-api.md API-3
- Status: applied: API-3's `auth` message names `gh auth login` when GitHub refuses gh's token twice; `auth` stays the problem until a success

### H4-5 · low · `slowed` doubles `interval` even when idle
- Location: specs/02-polling.md POL-23
- Status: applied: the tier's interval doubled

### H4-6 · low · An answer without `rateLimit` isn't counted
- Location: specs/02-polling.md POL-17
- Status: applied: it counts 1 point. Closes H1-17.

### H4-7 · low · Secondary limits: what counts towards the doubling, and which clock
- Location: specs/02-polling.md POL-25, POL-26
- Status: applied: every secondary limit in a row counts; a success ends the row; the wait is on the monotonic clock

### H4-8 · low · POL-11's "3 s later" than what
- Location: specs/02-polling.md POL-11
- Status: applied: 3 s after the answer that read `UNKNOWN`; one follow-up request per answer

### H4-9 · low · POL-21's 5 minutes: which clock, which errors
- Location: specs/02-polling.md POL-21, POL-26
- Status: applied: the monotonic clock; only the PR's own errors count

### H4-10 · low · A failed read-through during the backoff
- Location: specs/02-polling.md POL-19, POL-16
- Status: applied: its failure doesn't move the backoff; its success ends it. Part of H3-17.

### H4-11 · low · POL-15's window edges
- Location: specs/02-polling.md POL-15
- Status: applied: a whole poll is forced only when the last forced one was sent more than 10 s ago

