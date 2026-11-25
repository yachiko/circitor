# Hardening log: 01-model.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 2/10/6 | 12 | 0 | 0 | 0 | 6 |
| 2 | 2026-11-25 | failure modes | 2/11/3 | 11 (+ H1-14) | 0 | 0 | 0 | 5 |
| 3 | 2026-11-25 | implementability | 1/7/9 | 12 | 0 | 0 | 0 | 5 |
| 4 | 2026-11-25 | planning findings | 0/4/5 | 9 | 0 | 0 | 0 | 0 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3; raw findings: 30; merged as duplicates: 12; dropped in verification: 0 whole findings, 2 sub-claims (1 misread: "up to 9 s" is supported by `a1.log` and `c1.log`; 1 weak evidence: the status that joins late in `c1.log` was posted by hand).

Three findings rest partly on GitHub's schema from memory, not on docs/02: `stale` as a check conclusion (H1-3), `CheckRun.databaseId` (H1-5), a review in state `PENDING` (H1-16).

### H1-1 · high · One check in the list settles it, so `passing` is believed while more checks are about to start
- Location: specs/01-model.md MOD-6, MOD-11; specs/07-testing.md TST-8
- Status: applied: `checks.settled` goes by time, whatever the list holds; MOD-11's `checks_pending` row covers `passing` and not settled, and `ready` needs settled checks; TST-8 gained `first-check-early`

### H1-2 · high · `checks.settled` turns true by the clock alone, so an old "no checks" read becomes `ready` between polls
- Location: specs/01-model.md MOD-6, MOD-14; specs/07-testing.md TST-8
- Status: applied: `settled` turns true only when a refresh is applied at least `checks.settle_s` after the head was first seen; `checks.settled` by itself is listed as not a change in MOD-14; TST-8 `no-ci` reworded

### H1-3 · medium · A check result the spec doesn't list counts as passing
- Location: specs/01-model.md MOD-5, MOD-2
- Status: applied: `passing` needs every result to be `success`, `skipped` or `neutral`, anything else fails; a new bullet under MOD-2's table says unlisted values pass through lowered and never fail decoding (proposal)

### H1-4 · medium · The fixed 60 s "settled at once" shortcut undercuts a longer `checks.settle_s`
- Location: specs/01-model.md MOD-6; specs/06-operations.md OPS-2
- Status: applied: the shortcut is `checks.settle_s` plus 40 s, 60 s by default

### H1-5 · medium · "A check never moves backwards" keys on an unmeasured ID, has no key for a commit status, and hides a re-run for good
- Location: specs/01-model.md MOD-4, MOD-3; specs/07-testing.md TST-8
- Status: applied: a step back is ignored for 30 s, then believed as a re-run; check runs match by `id`, commit statuses by `name` and always take the newest read; marked (proposal); TST-8 `commit-status` and `check-out-of-order` extended. A re-run with `databaseId` recorded isn't measured yet.

### H1-6 · medium · A kept `mergeable: yes` survives a move of the base branch, and MOD-8's two bullets disagree on a merge
- Location: specs/01-model.md MOD-8, MOD-9
- Status: applied: the kept value is for one head and base while the PR is open, and lapses when a refresh 60 s after the first `UNKNOWN` still reads `UNKNOWN`; on a merged PR both fields are `unknown`, with no `mergeable` kind; MOD-9 follows. What a push to the base does isn't measured.

### H1-7 · medium · TST-8 `mergeable-unknown` asserts no event after a push, and MOD-8 makes two
- Location: specs/07-testing.md TST-8; specs/01-model.md MOD-8
- Status: applied: the test gave way. MOD-8 says a push shows `unknown` and both steps are visible changes; the scenario tests silence on an unchanged head, the 60 s lapse, and the two changes after a push

### H1-8 · medium · Some visible changes have no event kind, and two event kinds aren't visible changes
- Location: specs/01-model.md MOD-14, MOD-15
- Status: applied: only a rise in `counts.comments` or `counts.reviews` is a change; first appearance and dropping take a revision; new kind `edited` for `title` and `base`; `activity` carries the new `counts`, the items (which may be none) and `more`; every visible change has a kind; a count's rise and its items are one event. The `more: true` wording was already in the row on disk and was kept.

### H1-9 · medium · `gone` has no field behind it, although `status` is derived and other files treat it as a third end state
- Location: specs/01-model.md MOD-2, MOD-11; specs/02-polling.md POL-21; specs/03-state.md STA-2
- Status: applied: `gone` is a value of `state`, circitor's own; only an open PR becomes gone; a gone record keeps its other fields; one that GitHub returns again is open again (STA-2); POL-21 gained "if it was open"

### H1-10 · medium · `behind` is called observed where docs/02 says never seen, and requirements on unmeasured facts lack labels or (proposal)
- Location: specs/01-model.md MOD-2, MOD-3, MOD-9, MOD-10, MOD-11, MOD-12; specs/decisions.md D6
- Status: applied: MOD-9 lists `merge_state` values by how well they're known, with `draft` added as never seen; (proposal, D6) on the `blocked` row and on `required`; (proposal) on the `EXPECTED` mapping; `draft` note labelled observed; MOD-10 reworded to what docs/02 says; the bots sentence no longer claims `author` tells them apart; D6's text corrected

### H1-11 · medium · `stale` is undefined while polling is slowed or stopped for quota, and for a PR that's gone or not found
- Location: specs/01-model.md MOD-16
- Status: applied with change: as proposed (age against the configured interval of the PR's tier, `NOT_FOUND` a failed attempt until `gone`, a gone record never stale), plus `idle_interval` for a merged or closed PR, to match POL-10 as it now reads

### H1-12 · low · A truncated checks list can read `failing` while unread checks still run, against MOD-5
- Location: specs/01-model.md MOD-5, MOD-7
- Status: applied: MOD-7 states the order of its clauses, says `completed` and `failed` count read items only, and that a truncated `failing` can be early; MOD-5 names the exception

### H1-13 · low · Naming and types: the `mergeable` mapping to `yes` and `no` is never stated and the README's lowering example contradicts it; `author` is typed string but can be `null`; `created_at` and `updated_at` are typed nullable; `fetch_inline` isn't in the item table
- Location: specs/01-model.md MOD-2, MOD-12; specs/README.md conventions
- Status: parked

### H1-14 · medium · `checks.settle_s` of 20 s rests on three samples of GitHub Actions in a quiet repository; a CI that first reports later reads `none`, settled, `ready`
- Location: specs/01-model.md MOD-6
- Status: applied in pass 2: MOD-6 keeps a head whose previous head had checks unsettled while it has none, until three times `checks.settle_s` (60 s by default); TST-8 gained `late-ci`

### H1-15 · low · The checks come from `commits(last: 1)` and the SHA from `headRefOid`; nothing ties the two together
- Location: specs/01-model.md MOD-2, MOD-3
- Status: parked. Whether they can differ in one response isn't measured; asking the commit's `oid` and comparing would guard it.

### H1-16 · low · A review in state `PENDING` (the viewer's own, not yet submitted) may count in `reviews.totalCount`, with no `submittedAt` (applied in pass 4: H4-5)
- Location: specs/01-model.md MOD-12
- Status: parked. Needs a measurement; `dismissed` is in the spec's list and not in docs/02.

### H1-17 · low · The event's JSON shape is undefined (`kinds` as names or as objects), "short SHA" has no length, the `checks` object has no table, and `counts`, `item`, `status` and `review` each name two things (applied in pass 4: H4-2)
- Location: specs/01-model.md MOD-5 to MOD-7, MOD-15
- Status: parked (implementability: pass 3)

### H1-18 · low · 00-overview sends the reader to 01-model for auto-merge and the merge queue, which the model doesn't mention; CLI-2's branch match has no head repository for a PR from a fork
- Location: specs/00-overview.md non-goals; specs/05-cli.md CLI-2; specs/01-model.md MOD-2
- Status: parked

### Out of scope

- docs/02 §4 has no line for the check conclusions beyond `SUCCESS`, `FAILURE`, `SKIPPED` and `CANCELLED`, for `CheckRun.databaseId`, for the review states `DISMISSED` and `PENDING`, or for `author: null` on a deleted account; docs/references.md has no source for GitHub's enums. The spec names all of them. The facts docs are the owner's to change.

## Pass 2: failure modes (2026-11-25)

Reviewers: 3 (time and PR-state boundaries; infrastructure failure; lifecycle); raw findings: 22; merged as duplicates: 6 (five pairs that two reviewers found independently, and one gone-PR detail folded into H2-1); dropped in verification: 0 whole findings, 2 sub-claims (GitHub serving an old head after a newer one isn't measured; "one extra point per review" was wrong, since every push moves `updatedAt`). H1-14, parked in pass 1, was walked through too.

Another session hardened 02-polling and 03-state during this pass and edited this file: STA-8's loaded PRs (MOD-15), `asked` in MOD-16, a sleep making records stale (MOD-16), and `pr` in MOD-14 and in `edited`. POL-8 gained "a PR's check pages are one read" during the pass, which covers half of H2-8.

### H2-1 · high · Lost access, or another login, turns PRs `gone`: waits end falsely, manual PRs are forgotten, and a PR that returns is called open
- Location: specs/01-model.md MOD-11, MOD-16; specs/02-polling.md POL-4, POL-21; specs/03-state.md STA-2, STA-6
- Status: applied: `gone` counts only under the same login, and not when every tracked PR in a response answered `NOT_FOUND`; a returning gone PR takes GitHub's state with that state's kind; POL-4 asks `viewer { login }`; POL-21 cites MOD-11's conditions; STA-2 restarts or clears the clock to match; STA-6 saves the login

### H2-2 · high · A kept `mergeable: yes` survives a sleep or a long gap, so an old value reads `ready`
- Location: specs/01-model.md MOD-8; specs/07-testing.md TST-8 `mergeable-unknown`
- Status: applied: kept for at most 60 s after the last read that gave it, on the wall clock; TST-8 follows

### H2-3 · medium · A head that returns to an earlier SHA counts as settled at once
- Location: specs/01-model.md MOD-6, MOD-4
- Status: applied: settling counts from when the head last changed to this SHA; any change of head, also back, resets `settled` and the checks list

### H2-4 · medium · A review's `at` is "`createdAt` or `submittedAt`", and `createdAt` drops a review that took a while to write
- Location: specs/01-model.md MOD-12; specs/02-polling.md POL-12; specs/03-state.md STA-5
- Status: applied: a comment's `createdAt`, a review's `submittedAt`; POL-12 and STA-5 compare `at`. That a pending review's `createdAt` is its start isn't measured.

### H2-5 · medium · An answer's cursor and records needn't come from one view, so a change can be skipped for good
- Location: specs/01-model.md MOD-13
- Status: applied: a revision and its record and event are stored in one step; every answer reads one view, and its cursor is the highest revision in it

### H2-6 · medium · The "settled at once" shortcut compares GitHub's `updated_at` with the machine's clock
- Location: specs/01-model.md MOD-6
- Status: applied: the age is measured against the response's `Date` header, `checks.settle_s` plus 5 s (25 s by default); no header, no shortcut; (proposal: the header isn't recorded in docs/02)

### H2-7 · medium · Reopening a PR starts new check runs on the same head, but only a head change reset `settled`
- Location: specs/01-model.md MOD-6; specs/07-testing.md TST-8 `reopened`
- Status: applied: a reopening starts `settled` at false (proposal); TST-8 `reopened` reads `checks_pending` until settled. That Actions re-runs on `reopened` is from GitHub's docs, not in docs/references.md.

### H2-8 · medium · A checks list that changes between page reads is believed complete
- Location: specs/01-model.md MOD-7; specs/02-polling.md POL-8
- Status: applied: `truncated` is true whenever fewer items were read than GitHub's count. The failed-page half was already covered by POL-8's new text.

### H2-9 · medium · A refresh held for its comment IDs can be applied after a newer one and roll the head back
- Location: specs/01-model.md MOD-2; specs/03-state.md STA-4; specs/02-polling.md POL-11, POL-12, POL-14
- Status: applied: `fetched_at` is when the request was sent; a refresh sent before the one last applied isn't applied; STA-4 lists MOD-2 among its exceptions

### H2-10 · medium · Identity is the name, so a renamed or transferred repository splits one PR into two records
- Location: specs/01-model.md MOD-1, MOD-2, MOD-14, MOD-15
- Status: applied: keyed by node ID; `pr` read from `repository.nameWithOwner` and `number` at every refresh; a found or read-through PR whose node ID is tracked is that record; (proposal: the node ID across a rename isn't measured). `pr` in MOD-14 and `edited` was added by the other session at the same time.

### H2-11 · medium · A PR that ended while no daemon ran got no `merged` or `closed` event after a restart
- Location: specs/01-model.md MOD-15; specs/03-state.md STA-5, STA-8
- Status: applied: a loaded PR's `tracked` event also carries `merged` or `closed` if it was saved open and has ended, and `activity` for what STA-5 makes new

### H2-12 · medium · Nothing says how a CLI reads a record or event from a daemon of another version
- Location: specs/01-model.md MOD-2, MOD-11, MOD-15; specs/05-cli.md CLI-4, CLI-9 (a foreground daemon is never replaced)
- Status: parked. One idea: records and events change only by adding within `/v1/`; a CLI ignores unknown fields and treats an unknown `checks.state` as not done.

### H2-13 · medium · A required check that only the rollup knows about (`EXPECTED`) is invisible to `checks.state`
- Location: specs/01-model.md MOD-5, MOD-3; decisions D6
- Status: parked. One idea: `pending` also when the rollup says `PENDING` or `EXPECTED`; the rollup can only hold the state back. Whether an expected check appears as an item needs D6's public repository.

### H2-14 · low · One node that doesn't decode can sink a whole response (applied in pass 4: H4-3)
- Location: specs/01-model.md MOD-2; specs/02-polling.md POL-21, POL-24
- Status: parked. POL-24 now keeps a PR's error with that PR; a node that fails decoding (an unexpected `null`, a renamed field) isn't said to be handled the same way.

### H2-15 · low · `stale`'s "twice the interval" sits on real refresh periods, so it flickers when slowed and for ended PRs in the idle tier
- Location: specs/01-model.md MOD-16; specs/02-polling.md POL-10, POL-23
- Status: parked. Slowed polling refreshes exactly at the stale threshold; POL-10's strict "older than `idle_interval`" makes an ended PR's period 600 s in the idle tier. One idea: a 10 s margin, or "stale while slowed" on purpose. The ended-PR half is closed in 02-polling pass 3 (H3-5: "at least", from send times); the slowed half stays parked.

### H2-16 · low · A deletion and a new comment between two refreshes keep the count level, and the new comment is missed
- Location: specs/01-model.md MOD-12, MOD-14; specs/02-polling.md POL-12
- Status: parked. Deletions are a non-goal and unmeasured (docs/02 §6 item 4).

### Out of scope

- specs/02-polling.md POL-24: a secondary limit (403 or 429 with `Retry-After`) goes to POL-19's 5 s backoff and ignores `Retry-After`.
- specs/02-polling.md POL-12 vs POL-19: whether a timed-out what's-new request applies the held refresh or abandons the poll.
- specs/04-api.md API-9, API-11: in the `auth` or `exhausted` state a `wait` can only end in `timeout`; nothing tells the agent why.
- specs/04-api.md API-11: no outcome for a waiter whose PR is dropped by `untrack`.
- specs/05-cli.md CLI-9, CLI-10: an old CLI still waiting after `brew upgrade` reconnects and may replace the new daemon; the Cellar path it would spawn may be gone.
- specs/03-state.md STA-8: a newer binary reading an older state file `version`, and a downgrade setting the file aside, losing the `manual` set.
- specs/03-state.md STA-1: after a change to `owners` and a restart, PRs found under the old owners stay tracked.
- specs/03-state.md STA-5 vs specs/02-polling.md POL-12: "newer than" against "isn't older than", so an item from the same second as the last one seen is lost across a restart.
- specs/03-state.md STA-6: "A gone PR can't be fetched back" now reads against MOD-11's returning gone PR.

## Pass 3: implementability, testability and simplification (2026-11-25)

Reviewers: 3 (implementer, tester, simplifier); raw findings: 25; merged as duplicates: 8 (the `gone` guard was found by all three, the settle clock and the step-back window by two each); dropped in verification: 1 whole finding (MOD-8's wall clock against POL-26: the other session had already added MOD-8's 60 s to POL-26's wall-clock list), 2 sub-claims (hyper adding a `Date` header by default is from memory, though TST-3 still didn't name its clock; the held refresh's items have no rule to test yet, parked as H3-17). Verification strengthened H3-5: docs/02 §4 says a check never moves `updatedAt`, so a new PR first polled at 30 s took the settled-at-once shortcut.

Another session edited 02-polling, 03-state, 05-cli, 06-operations, 07-testing and this file during the pass, among them MOD-16's interval (now "twice the PR's period, POL-10"), TST-3's `Date` and `viewer.login`, and new TST-8 rows. Every edit here was made against the file as it then stood.

### H3-1 · high · Reopening doesn't restart the settle clock, and the clock's start is undefined
- Location: specs/01-model.md MOD-6; specs/07-testing.md TST-3, TST-8
- Status: applied: MOD-6 defines the **settle start** (the send time of the refresh that first read the record, also a loaded one, first showed this head, or first showed the PR open again); `settled` needs an applied refresh sent the settle time after it; the start's own refresh leaves it false but for the shortcut; the shortcut covers loaded and read-through records; TST-3's `Date` is from the test clock and can be left out; TST-8 gained `settled-at-once`

### H3-2 · medium · A lone tracked PR never becomes `gone`, and hot refreshes have no login to compare
- Location: specs/01-model.md MOD-11; specs/02-polling.md POL-21; specs/07-testing.md TST-8 `repo-gone`
- Status: applied: only a poll's `NOT_FOUND` counts, under the login its finding gave; the every-PR guard needs two or more PRs in the poll; a hot or forced refresh's `NOT_FOUND` neither counts nor breaks the run. With one tracked PR, lost access now makes it gone after 5 minutes (H2-1's guard given up for that case, on the owner's OK). `repo-gone` gained the lone PR.

### H3-3 · medium · Check changes are counted per item, not by the summary
- Location: specs/01-model.md MOD-14, MOD-15
- Status: applied: a checks change is `checks.state`, `total`, `completed`, `failed`, `truncated`, `failed_items` or which items are in the list; a move between `queued` and `in_progress` isn't a change; the `checks` kind is present whenever one of these changed. Also covers a truncated list whose rollup moves with no item changing.

### H3-4 · medium · MOD-4's 30 s step-back window is undefined ("ignored", clock, bound, reset), and it hides a re-run
- Location: specs/01-model.md MOD-4; specs/03-state.md STA-4; specs/07-testing.md TST-8 `check-out-of-order`
- Status: applied: the window is struck and each read believed; a step back from `completed` reads `pending`. Narrows H1-5's fix. STA-4 no longer lists MOD-4; the scenario now proves no event and `pending`.

### H3-5 · medium · A new PR whose CI reports late reads `ready`, and the late-CI rule needs a memory of the previous head
- Location: specs/01-model.md MOD-6; specs/07-testing.md TST-8 `no-ci`, `late-ci`
- Status: applied: the settle time is 20 s for a list with checks and 60 s for an empty one, the shortcut that list's time plus 5 s; the previous-head bullet is gone. Replaces H1-14's mechanism. A repository with no CI reads `checks_pending` for about 60 s after a push.

### H3-6 · medium · "A newer read wins" has no ordering key, and `fetched_at` is a wall time
- Location: specs/01-model.md MOD-2
- Status: applied: refreshes are ordered by a number given at sending, never by `fetched_at`

### H3-7 · medium · `required` is read by no rule, yet forces every refresh into an unmeasured aliased query
- Location: specs/01-model.md MOD-3; specs/02-polling.md POL-4, POL-7, POL-9, POL-21; specs/decisions.md D6; specs/07-testing.md TST-3
- Status: applied: `required` isn't read in v1 (a note under MOD-3's table); POL-9 struck; POL-4 refreshes by `nodes(ids:)` with one shared fragment, the measured form; the "aliased form isn't measured" clauses are gone from POL-7 and POL-21; D6 and TST-3 follow

### H3-8 · medium · Five behaviours the fake GitHub could check have no scenario
- Location: specs/07-testing.md TST-8
- Status: applied: rows `renamed` (MOD-1), `unlisted-values` (MOD-2, MOD-5), `head-back` (MOD-6), `base-changed` (MOD-8), `gone-returns` (MOD-11, STA-2)

### H3-9 · low · "One poll's changes make one event", but hot, forced and follow-up refreshes aren't polls
- Location: specs/01-model.md MOD-15
- Status: applied: one applied refresh makes at most one event per PR (a count's rise with its items, and a PR's check pages, are one refresh); PRs in one response take revisions in the order of `pr`

### H3-10 · low · `error` has no stated casing, `since` has no rule, and which failures set it is unclear
- Location: specs/01-model.md MOD-2; specs/05-cli.md CLI-3
- Status: applied: `type` as GitHub writes it (an exception to the lowering, matching CLI-3's `error FORBIDDEN`); `since` is the first of the current run of this PR's errors; whole-request failures and a failed check page leave it; a gone record keeps it

### H3-11 · low · An unlisted check `phase` is both "passed through" and "counted as `queued`"
- Location: specs/01-model.md MOD-2, MOD-3
- Status: applied: MOD-2's pass-through excludes `phase`

### H3-12 · low · MOD-14 leaves several fields in neither list
- Location: specs/01-model.md MOD-14
- Status: applied: `url`, `author`, `head`, `created_at`, `merged_at`, `closed_at` and `source` alone aren't a change; `track` on a tracked PR takes no revision

### H3-13 · low · MOD-13 doesn't say which number the first revision takes, or the cursor of a view with no revision
- Location: specs/01-model.md MOD-13; specs/03-state.md STA-9
- Status: parked. One idea: an empty view's cursor is `<epoch>-0`, and the first change takes 1.

### H3-14 · low · MOD-7 doesn't say which page's `totalCount` counts, or how a check read on two pages is counted
- Location: specs/01-model.md MOD-7; specs/02-polling.md POL-8
- Status: parked. One idea: de-duplicate by `id` (a status by `name`), the later page winning; GitHub's count is the largest `totalCount` read.

### H3-15 · low · MOD-15 restates STA-5's rule for a loaded PR's first event
- Location: specs/01-model.md MOD-15; specs/03-state.md STA-5
- Status: parked. The MOD-16 half of this finding was settled by the other session (MOD-16 now cites POL-10's period).

### H3-16 · low · `counts.review_threads` is carried but used by nothing, and needs a MOD-14 exception
- Location: specs/01-model.md MOD-2, MOD-14
- Status: parked. It counts resolved threads too, so an agent may misread it as open ones.

### H3-17 · low · MOD-2's "a newer read wins" doesn't say what happens to the items a dropped held refresh fetched (POL-12)
- Location: specs/01-model.md MOD-2; specs/02-polling.md POL-12, POL-15
- Status: parked. A test (`held-refresh`) needs the rule first.

### Not walked

- The simplifier's alternative to H3-2's fix: moving the whole `gone` trigger into POL-21, one source of truth. H3-2 kept the split.
- The simplifier's alternative for MOD-8: striking the kept `mergeable` value (H1-6, H2-2) until docs/05 records a flicker on the same head. Not raised once the clock half was settled.

### Out of scope

- specs/02-polling.md POL-26: "a wall time ahead of now counts as now" may keep MOD-8's kept `yes` for as long as the clock is set back, depending on whether the stored time or the age is clamped.

## Pass 4: planning findings (2026-11-25)

No reviewers: the findings came from building the implementation plan (`plan/README.md`, "Spec issues found while planning") and from its brief writers, and were verified against the spec text.

### H4-1 · medium · "The order of `pr`" isn't defined
- Location: specs/01-model.md MOD-1, MOD-15; specs/03-state.md STA-2; specs/04-api.md API-6
- Status: applied: owner, then repository, without case, then the number as a number

### H4-2 · medium · An event's JSON shape is half-defined
- Location: specs/01-model.md MOD-15; specs/04-api.md API-19
- Status: applied: `kinds` is a list of objects tagged by `kind`, in the table's order; `draft` carries `draft`; a short SHA is 7 characters. Closes H1-17.

### H4-3 · medium · An unlisted PR `state` or an undecodable node has no outcome
- Location: specs/01-model.md MOD-2; specs/02-polling.md POL-21
- Status: applied: that PR's error, with `UNLISTED_STATE`, `UNDECODABLE` or `UNTYPED`; the rest of the response is applied. Closes H2-14.

### H4-4 · medium · An unlisted `mergeable` or `review` can read `ready`
- Location: specs/01-model.md MOD-11
- Status: applied: for `status` only, they count as `unknown` and `review_required`

### H4-5 · low · A review without `submittedAt`
- Location: specs/01-model.md MOD-12; specs/03-state.md STA-5
- Status: applied: not an item until submitted. Closes H1-16.

### H4-6 · low · The order of items inside an `activity`
- Location: specs/01-model.md MOD-15
- Status: applied: oldest `at` first, comments before reviews, then `id`

### H4-7 · low · A rename that changes only the case
- Location: specs/01-model.md MOD-1, MOD-14
- Status: applied: the new spelling, without a revision

### H4-8 · low · Whether `checks.settled` stays true
- Location: specs/01-model.md MOD-6
- Status: applied: until the next settle start

### H4-9 · low · A check run with no `databaseId`
- Location: specs/01-model.md MOD-4
- Status: applied: matched by `name`

