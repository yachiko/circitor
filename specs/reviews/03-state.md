# Hardening log: 03-state.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 3/4/5 | 12 | 0 | 0 | 0 | 0 |
| 2 | 2026-11-25 | failure modes | 4/10/4 | 17 | 1 | 0 | 0 | 0 |
| 3 | 2026-11-25 | implementability | 1/7/5 | 13 | 0 | 0 | 0 | 0 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3, with 27 raw findings; 13 after merging duplicates. Dropped in verification: 3 (all the same finding, the baseline fetch of STA-5 having no request in polling: POL-4 gained it during this pass, in 02-polling's own pass 1). 1 out of scope. 01-model and 02-polling were hardened at the same time, and both edited this file during the pass (STA-2, STA-5).

### H1-1 · high · A PR that merges or closes while no daemon runs is lost
- Location: specs/03-state.md STA-6, STA-8, STA-9; specs/06-operations.md OPS-8; docs/02 §3 (the open list drops a PR at once, verified)
- Status: applied: STA-6 saves every tracked PR but the `asked` ones, with `source`; STA-8 says one that ended meanwhile is seen ending at the first poll; OPS-8 now says a restart loses the event history, the epoch and the `asked` PRs

### H1-2 · high · Comments and reviews posted while no daemon runs are absorbed into the baseline
- Location: specs/03-state.md STA-5, STA-6; specs/02-polling.md POL-4; specs/01-model.md MOD-15; specs/04-api.md API-8
- Status: applied: STA-6 saves per PR the time of the newest comment and review seen; STA-5 makes IDs newer than those `activity` on loading (POL-12's rule); POL-4 and MOD-15 follow; API-8 gives a cursor from another epoch every event of the current one while the history holds them all

### H1-3 · high · A `wait` with a cursor from an earlier daemon times out instead of reporting the change
- Location: specs/03-state.md STA-9; specs/04-api.md API-9, API-10
- Status: applied: STA-9 compares revisions only within one epoch, and every route taking a cursor (API-6, API-8, API-9) resets on a too-old one; a too-old `since` makes `changed` hold at once; API-9's answer carries `reset: true` and API-8's events
- Note for pass 2: a CLI-10 reconnect of a wait that had no `since` still waits for the next change, and can miss one made during the restart.

### H1-4 · medium · STA-4 said a refresh "replaces" the record, against the rules that keep earlier values
- Location: specs/03-state.md STA-4; specs/01-model.md MOD-4, MOD-6, MOD-8, MOD-9; specs/02-polling.md POL-12, POL-21
- Status: applied: a refresh is merged into the stored record under those rules; `source`, the seen IDs and both clocks carry over

### H1-5 · medium · A PR loaded from the file had no record until refreshed, and a `gone` one never got one
- Location: specs/03-state.md STA-6, STA-8; specs/04-api.md API-18; specs/01-model.md MOD-2
- Status: applied: `gone` PRs aren't written; until its first refresh a loaded PR is left out of `list` and read through by single-PR routes, keeping its saved `source`

### H1-6 · medium · A version upgrade raced STA-11's lock check against the old daemon's shutdown
- Location: specs/03-state.md STA-11; specs/05-cli.md CLI-9; specs/06-operations.md OPS-7
- Status: applied: STA-11 retries the lock for up to 6 s before deciding; CLI-9 waits up to 5 s for the old socket to stop answering before starting its own

### H1-7 · medium · STA-3's limit disagreed with POL-7, cost the whole budget, and left its edge cases open
- Location: specs/03-state.md STA-3; specs/02-polling.md POL-7, POL-23
- Status: applied: 200 of every source and state; at the limit `track` is refused, a read-through isn't tracked and finding stops; nothing is pushed out; the cost near the limit (5 to 6 points a poll, `slowed` or `capped`) is stated; POL-7 finds up to STA-3's limit instead of 300

### H1-8 · low · `untrack` ignored `owners` and dropped `authored` PRs
- Location: specs/03-state.md STA-1; specs/04-api.md API-15
- Status: applied: API-15 checks `owners` and changes nothing on an `authored` or `asked` PR; STA-1 says `untrack` changes only a `manual` PR

### H1-9 · low · An `asked` PR could expire under its own waiter
- Location: specs/03-state.md STA-1; specs/04-api.md API-12
- Status: applied: the 60 minutes run from when the last request ended, and never while the PR has a waiter

### H1-10 · low · STA-12's list of written files was incomplete, and STA-11 didn't say which socket it checks
- Location: specs/03-state.md STA-11, STA-12; specs/06-operations.md OPS-4, OPS-9
- Status: applied: STA-12 lists the temporary file, the set-aside file and the old log, and the `CIRCITOR_SOCKET` exception; the lock file holds the holder's socket path, which STA-11 checks

### H1-11 · low · Reopening was stated as settled with no fact behind it
- Location: specs/03-state.md STA-2; docs/02 (no entry on reopening)
- Status: applied: STA-2's bullet is (proposal; reopening isn't measured). docs/02 §6 could list it as an open item; left to the owner.

### H1-12 · low · The epoch's alphabet wasn't given, and a failed write might never be retried
- Location: specs/03-state.md STA-7, STA-9
- Status: applied: the epoch is 8 random lower-case letters and digits; a failed write is retried at the next change or 60 s later

### Out of scope
- specs/04-api.md API-6 and specs/01-model.md MOD-14: a PR dropped after a cursor never shows in `list --since` (it has no record to return), and an `asked` PR becoming `manual` isn't a visible change. Only `changes` reports either.
- specs/05-cli.md CLI-5: the API's `limit` error (409) has no exit code of its own, so it falls to 1, "an unexpected error".

## Pass 2: failure modes (2026-11-25)

Reviewers: 3 (time and PR-state boundaries; infrastructure failure; lifecycle), with 29 raw findings; 18 after merging duplicates. Three were found by two or three reviewers each: the gone PR left out of the file, the reconnected `changed` wait, and the clocks. Dropped in verification: 1 whole finding (an `authored` PR never leaving after a change to `owners`: POL-5 gained that rule in 02-polling's pass 2, during this one) and 1 sub-claim (what an uninstall leaves: parked in 02-polling's log as H2-19). Another session hardened 01-model and 02-polling at the same time; STA-4's MOD-2 exception and POL-26 came from it.

### H2-1 · high · A `gone` PR isn't saved, so a `manual` PR that lost access is forgotten at the next restart
- Location: specs/03-state.md STA-6; specs/01-model.md MOD-11; specs/02-polling.md POL-10, POL-21
- Status: applied: STA-6 saves every tracked PR but `asked` ones, and `gone` with `merged` and `closed` inside `keep_done`; a loaded gone PR stays gone until GitHub returns it

### H2-2 · high · A loaded PR's saved entry has no defined home until its first refresh, so a write can drop it
- Location: specs/03-state.md STA-6, STA-8
- Status: applied: STA-8 keeps the saved entry until the first refresh, writes carry it unchanged, `keep_done` runs from the saved time, and `NOT_FOUND`s count as for the saved state and login

### H2-3 · high · A reconnected `changed` wait without `since` misses what happened during the restart
- Location: specs/03-state.md STA-9; specs/04-api.md API-9, API-10; specs/05-cli.md CLI-10
- Status: applied: API-9 sends the arrival cursor in a `Circitor-Cursor` header when the waiter is registered; CLI-10 reconnects with it as `since`; API-10 takes the arrival revision after the read-through and the forced refresh; STA-9 says a wait without `since` is covered. Closes pass 1's note on H1-3.

### H2-4 · high · A state file from another version is set aside, losing the `manual` set
- Location: specs/03-state.md STA-6, STA-8
- Status: applied: a binary reads every earlier `version` and ignores unknown keys; a newer file is left alone and not written, with a warning; one `.bad` file is kept; `version` rises only for a change an older binary would misread

### H2-5 · medium · STA-5 can't compare a loaded PR's items: the baseline has no times, and the edges disagree
- Location: specs/03-state.md STA-5, STA-6; specs/02-polling.md POL-4, POL-12
- Status: applied: STA-6 saves per kind the newest time and the IDs at that time; an item is new if later, or the same time with an unsaved ID; `more: true` with all 20 new; POL-4's loaded PR asks POL-12's fields

### H2-6 · medium · Retention drops a PR that has a waiter, and with `keep_done = 0` the merge can be dropped before its wait sees it
- Location: specs/03-state.md STA-2
- Status: applied: no drop while a waiter is held; the ending refresh is applied and its waiters evaluated before any drop

### H2-7 · medium · `keep_done` and the `asked` hour name no clock, and POL-26's catch-all makes them monotonic
- Location: specs/03-state.md STA-1, STA-2; specs/02-polling.md POL-26
- Status: applied: both go by the wall clock, a time ahead of now counts as now; a PR made `asked` by POL-5 starts its hour then; POL-26's wall-clock list names them

### H2-8 · medium · The file saves one login, but MOD-11's `gone` rule needs it per PR
- Location: specs/03-state.md STA-6; specs/01-model.md MOD-11
- Status: applied: the login is saved per PR

### H2-9 · medium · At STA-3's limit, a `wait` on an untracked PR is never refreshed, and a `track` of a tracked PR is refused
- Location: specs/03-state.md STA-3; specs/04-api.md API-9, API-15
- Status: applied: a `track` of a tracked PR is never refused; a `wait` on an untracked PR at the limit answers `limit` at once
- Not addressed: `asked` PRs can fill the 200 and keep a new PR of the user's own out (pass 1 chose not to push PRs out, H1-7).

### H2-10 · medium · A removed socket or lock file leaves an unreachable daemon, or two daemons
- Location: specs/03-state.md STA-11; specs/06-operations.md OPS-7
- Status: applied: the daemon checks every minute that its socket and lock file are its own (device and inode) and stops if not; at exit it removes the socket only if it's still its own (OPS-7 follows)

### H2-11 · medium · The start order isn't stated, so the state file or the lock file can be touched before the lock is held
- Location: specs/03-state.md STA-8, STA-11; specs/06-operations.md OPS-4
- Status: applied: the lock file is opened without truncating; the socket path, the stale socket and `state.json` come only once the lock is held

### H2-12 · medium · A restart during an outage turns a wait on a saved PR into an immediate error
- Location: specs/03-state.md STA-8; specs/02-polling.md POL-16, POL-19
- Status: applied: a `wait` naming a loaded PR is held as a waiter until its first refresh; other requests read through as before
- Note: the `list` half (loaded PRs missing from `list` while the first refresh fails) is signalled by `daemon.problem` in the outage cases, and was left as H1-5 decided.

### H2-13 · medium · The event history can roll past a long wait's cursor, and its answer then lacks events without `reset`
- Location: specs/03-state.md STA-9, STA-10; specs/04-api.md API-9
- Status: applied (2026-11-25, parked walk-through): too old is judged when the answer is built (STA-9); API-9's answer follows
- Note: too old is judged only on arrival; an hour-long wait on a busy PR, or 200 `tracked` events at start, can push its first events out of the 500. Fix idea: judge too old when the answer is built.

### H2-14 · medium · A refresh that answers after its PR was dropped can bring the PR back
- Location: specs/03-state.md STA-4; specs/04-api.md API-15
- Status: applied (2026-11-25, parked walk-through): STA-4 applies a refresh only to a PR that holds a place (STA-3) when it answers; one dropped meanwhile is discarded, with no event
- Note: an `untrack`, a `keep_done` drop or an `asked` expiry while a request is in flight (up to 20 s); STA-8 creates records from refreshes, so "no record" can't tell loaded from dropped. Fix idea: a refresh is applied only to a PR still tracked when it answers.

### H2-15 · low · STA-9 doesn't define its boundaries: the first revision, a cursor from the future, a malformed cursor
- Location: specs/03-state.md STA-9
- Status: applied (2026-11-25, parked walk-through): the first revision is 1, `<epoch>-0` before any; too old is another epoch, a `rev` above the counter, or an event after it no longer held; a malformed cursor is `bad_request`
- Note: fix idea: the first revision is 1, `<epoch>-0` before it; too old when an event after it is no longer held; a `rev` above the counter is too old; a malformed cursor is `bad_request`.

### H2-16 · low · Disk failures are silent, and an unusable state directory looks like a held lock
- Location: specs/03-state.md STA-7, STA-11; specs/06-operations.md OPS-9
- Status: applied (2026-11-25, parked walk-through): a failed write raises `state_file_unwritable` (API-5), cleared by one that succeeds, and OPS-9 logs it failing and working again; a lock file that can't be opened or created stops the daemon at once, naming the path and the error
- Note: a failing write raises no warning (API-5), and "logged and retried" every 60 s reads against OPS-9's "never each retry"; a lock file that can't be created waits 6 s, then fails with no reason the CLI can show.

### H2-17 · low · A `track` is answered before it is saved
- Location: specs/03-state.md STA-7; specs/04-api.md API-15
- Status: applied (2026-11-25, parked walk-through): a `track` or `untrack` that changes the file is written before it's answered; a failed write still answers
- Note: a crash in the 5 s after `track` or `untrack` loses it. Fix idea: write before answering those two.

### H2-18 · low · A second daemon's "already running" exit is silent, and a CLI with another socket path can't reach the holder
- Location: specs/03-state.md STA-11; specs/05-cli.md CLI-7; specs/06-operations.md OPS-4
- Status: rejected: handled since pass 2 by CLI-7, which tries the lock file's socket after its own, with a warning naming both, and waits for it after a spawned daemon exits 0
- Note: `CIRCITOR_SOCKET` set in one agent's environment and not another's makes every call of the second spawn a daemon that exits 0 silently, and the CLI exit 6. Fix idea: log the holder's socket; CLI-7 compares its path with the lock file's.

### Out of scope

- specs/01-model.md MOD-16: "A `gone` record is never stale: there is nothing newer to fetch", while POL-10 refreshes gone PRs and MOD-11 lets them return.
- specs/01-model.md MOD-11: a loaded open PR that turns `gone` before any refresh has no fields for "keeps the other fields it last had".
- specs/05-cli.md CLI-9: any version difference replaces the daemon, so a dev build and the installed one replace each other on every call (also in 02-polling's log). H2-4 makes it lose nothing, but each swap starts a new epoch.
- specs/04-api.md API-18: a first poll that succeeds with no PRs at all could read as "finished without applying anything".
- specs/04-api.md API-11: a waiter whose PR is dropped by `untrack` has no outcome (also in 01-model's log).

## Pass 3: implementability (2026-11-25)

Reviewers: 3 (implementer; tester; simplifier), with 26 raw findings; 15 after merging duplicates. The 200 seen IDs were found by all three, the file names by all three, and the limit and the read-through at the limit by two each. Dropped in verification: 2 (a file-store seam for STA-7: TST-10 already gives one; STA-3's cut-off order and cost sentence: chosen in H1-7). 1 out of scope. Other sessions were editing 07-testing during this pass.

### H3-1 · high · STA-7 doesn't promise that a change made within 5 s of the last write is ever written
- Location: specs/03-state.md STA-7; specs/07-testing.md TST-10
- Status: applied: a change within 5 s of the last write is written 5 s after it; TST-10 tests it

### H3-2 · medium · STA-11 and OPS-6 give a foreground daemon different exit statuses when it can't get the lock
- Location: specs/03-state.md STA-11; specs/06-operations.md OPS-6
- Status: applied: STA-11's exit rule is for a spawned daemon; a foreground one follows OPS-6 and never exits 0 for want of the lock

### H3-3 · medium · The lock's kind isn't named, and the lock-file line breaks on a macOS path
- Location: specs/03-state.md STA-11; specs/05-cli.md CLI-8, CLI-9
- Status: applied: `flock` with `LOCK_EX`, tested with `LOCK_NB`; the socket path is the rest of the line, spaces included; the minute check compares `stat` with `fstat`

### H3-4 · medium · `state.json` has no layout, and one bad entry sets aside the whole file
- Location: specs/03-state.md STA-6, STA-8; specs/07-testing.md TST-10
- Status: applied: STA-6 gives version 1's layout; only a file that isn't JSON or has no integer `version` is set aside; an entry that doesn't decode is skipped and logged; TST-10 keeps one fixture per released version

### H3-5 · medium · STA-5's 200 seen IDs never decide anything, and a kind with no item has no time to compare
- Location: specs/03-state.md STA-4, STA-5; specs/02-polling.md POL-12
- Status: applied: one rule in STA-5, the newest `at` and the IDs at it per kind, `null` with none seen (then every item is new); the loaded rule and POL-12 point to it; the 200 are gone

### H3-6 · medium · STA-3 doesn't say when a place is taken, so a race passes 200, and a left-out PR sits in `pending` forever
- Location: specs/03-state.md STA-3; specs/04-api.md API-5 (`pr_limit`), API-18
- Status: applied: a place is taken when a PR is found, loaded or read through, and given back if its first refresh fails; a found PR is sent for its first refresh only while a place is free; a left-out PR isn't `pending`; `pr_limit` clears when a poll leaves nothing out; finding stops paging once full

### H3-7 · medium · At the limit, a read-through answers with a record that has no valid `source` or `rev`, at a request per call
- Location: specs/03-state.md STA-3
- Status: applied: at the limit any request that would read through an untracked PR answers `limit` and sends nothing

### H3-8 · medium · The `keep_done` clock's start isn't defined, nor what a loaded PR's first refresh does to it
- Location: specs/03-state.md STA-2, STA-6, STA-8
- Status: applied: the clock counts from the send time of the refresh that first showed the PR ended; a loaded PR's first refresh is the same by a poll or a read-through and keeps a confirmed ended state's saved time; a drop on loading makes no event

### H3-9 · low · STA-1's "How it leaves" column misses two ways out, and the waiter guard is stated three times
- Location: specs/03-state.md STA-1, STA-2
- Status: applied: the column names POL-5's and API-15's demotions; one STA-1 bullet says no PR is dropped while it has a waiter

### H3-10 · low · The temporary file has no name, and the set-aside name can't replace an earlier one
- Location: specs/03-state.md STA-7, STA-8, STA-12
- Status: applied: `state.json.tmp`, the directory synced after the rename; `state.json.bad`, the time in the warning and the log

### H3-11 · low · STA-2 restates POL-26's clock rule and spells out the clock case by case
- Location: specs/03-state.md STA-2; specs/02-polling.md POL-26
- Status: applied: one rule (cleared on `open`, started again on a change to another ended state); the wall-clock rule cites POL-26

### H3-12 · low · The order of `untracked` events at one tick is unspecified
- Location: specs/03-state.md STA-2
- Status: applied: PRs dropped at one tick take revisions in the order of `pr`

### H3-13 · low · STA-1 doesn't say which requests renew the `asked` hour
- Location: specs/03-state.md STA-1
- Status: applied (2026-11-25, parked walk-through): STA-1 defines a request naming a PR (in its path or body, API-7 to API-9, API-15; ended when answered or its connection closes); POL-14 cites it
- Note: "the last request that named it" could include `list` or `changes` answers that hold it; CLI-2's bare-PR resolution reads `list`; "ended" isn't defined for a dropped connection. Fix idea: a route with the PR in its path or body (API-7 to API-9, API-15), ended when its answer is sent or its connection closes.

### Out of scope

- specs/07-testing.md TST-2, TST-5: randomness is seeded, and nothing says each daemon start in the harness gets its own seed, so two starts can make the same epoch and `cursor-reset` passes or fails on event counts. Fix idea: the harness gives each start its own seed, drawn from the test's.
- specs/02-polling.md POL-12: a review with no `submittedAt` (the viewer's own pending review) has no `at` for STA-5's rule; docs/02 doesn't record whether `reviews(last:)` returns it.
