# Hardening log: 04-api.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 3/10/2 | 12 | 0 | 0 | 0 | 2 |
| 2 | 2026-11-25 | failure modes | 2/8/9 | 12 | 0 | 0 | 0 | 7 |
| 3 | 2026-11-25 | implementability | 1/9/6 | 12 | 0 | 0 | 0 | 4 |
| 4 | 2026-11-25 | planning findings | 0/3/6 | 9 | 0 | 0 | 0 | 0 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3, with 30 findings; 15 after merging duplicates (the cold start, the truncated checks and the wait's deadline were raised by all three). Dropped in verification: 1 already handled (the exit codes for API-3's errors, added to CLI-5 by another session during this pass), 3 side notes (2 out of scope, 1 minor). Another session (05-cli's hardening) edited 04-api and 05-cli during the pass; its changes were kept and merged with these (see H1-6, H1-7).

### H1-1 · high · The first `list` after a cold start can be empty
- Location: specs/04-api.md API-18; specs/02-polling.md POL-4; specs/03-state.md STA-8
- Status: applied: API-18 defines the first poll as finished once its finding and the first refresh of every found or loaded PR have been applied or have failed; an unanswered request gives the problem's error

### H1-2 · high · `list --since` hides the PR that just merged or closed, and dropped PRs
- Location: specs/04-api.md API-6; specs/05-cli.md CLI-3
- Status: applied: `since` returns changed records whatever their `state`, and names PRs dropped after the cursor in `untracked`; CLI-3 prints a line `untracked <pr>` for each

### H1-3 · high · A wait on a PR that isn't tracked, or is dropped, never ends but by timeout
- Location: specs/04-api.md API-9, API-15; specs/03-state.md STA-2, STA-3
- Status: applied: a PR with a waiter stays tracked (API-9); `untrack` makes it `asked` (API-15); `keep_done` never drops it while it has a waiter (STA-2); at STA-3's limit a `wait` on an untracked PR gets `limit`

### H1-4 · medium · `closed` both ends and satisfies a `changed` wait
- Location: specs/04-api.md API-10, API-11
- Status: applied: `closed` and `gone` end `merged` and `checks-done`; a merge, a close or `gone` satisfies `changed`

### H1-5 · medium · `head_moved` can fire from the clock alone, and API-13 overstates latency
- Location: specs/04-api.md API-11, API-13; specs/02-polling.md POL-19, POL-23
- Status: applied: `head_moved` needs a refresh sent at least 60 s after the wait began, never the clock alone; API-13 gives the latency in each quota state and says `daemon.problem` tells why a wait can only time out

### H1-6 · medium · Nothing keeps a whole `wait` under 120 s
- Location: specs/04-api.md API-12; specs/05-cli.md CLI-6, CLI-10
- Status: applied with change: API-12's deadline runs from the request's arrival, with the read-through and the arrival refresh counted. The CLI half (one deadline from the CLI's start, the time left on reconnect) was written by another session during this pass, so CLI-6 and CLI-10 weren't edited here.

### H1-7 · medium · The wait didn't say whether it checks before or after its arrival refresh
- Location: specs/04-api.md API-9, API-10, API-13
- Status: applied with change: API-9's first evaluation comes after the read-through or the arrival refresh, and `timeout=0` waits for it. `changed`'s revision at arrival is taken after that refresh, as another session wrote in API-10 during this pass, not before it as first proposed (the owner chose to keep it).

### H1-8 · medium · "Tracked" meant both a PR with a record and a loaded PR without one
- Location: specs/04-api.md API-3, API-7, API-9; specs/03-state.md STA-8
- Status: applied: API-3 says "a PR with a record" is answered from memory and `github_unavailable` is for a PR with no record; API-7 and API-9 read through "a PR with no record"

### H1-9 · medium · API-3's `auth`, `rate_limited` and `not_found` rows misstated their cause
- Location: specs/04-api.md API-3, API-5; specs/02-polling.md POL-20, POL-23, POL-25; docs/02 §1, §5
- Status: applied: `auth` names the token source and its fix (POL-20); `rate_limited` covers a secondary limit and is a (proposal) on an assumed response; `not_found` labels the hidden-repository case assumed; API-5 shows the token source

### H1-10 · medium · Error bodies had no `version`, an unknown route read as "no such PR", and `wait` wasn't always 200
- Location: specs/04-api.md API-2, API-3, API-9; specs/05-cli.md CLI-5, CLI-9
- Status: applied: errors carry `version`; new code `unknown_route` (404), which CLI-5's code 1 already expects; `wait` answers 200 once the waiter is registered, and as API-3 when refused before that

### H1-11 · medium · `refresh` had no defined outcomes, and could block through a backoff
- Location: specs/04-api.md API-16; specs/02-polling.md POL-19, POL-23
- Status: applied: `outcome` is `polled`, `shared` or the problem that stopped it; `refresh` never waits out a backoff or a reset, and the CLI exits as for the API-3 error of the same name

### H1-12 · medium · The exit codes for API-3's errors
- Location: specs/05-cli.md CLI-5
- Status: dropped in verification: CLI-5 gained codes 9 and 10 and "each API error has one code" from another session during this pass

### H1-13 · medium · A truncated `failing` satisfies `checks-done` while checks still run
- Location: specs/04-api.md API-10; specs/01-model.md MOD-7; specs/05-cli.md CLI-6
- Status: applied: API-10 says a truncated `failing` may be early and still holds `checks-done`, with `checks.truncated` kept; CLI-6's outcome line flags it. The reviewers' fix (wait for the rollup) doesn't work: the rollup reads `FAILURE` while checks still run (docs/02 §4, verified). A real fix needs a measurement, e.g. of per-state check counts.

### H1-14 · low · `untrack` with an empty `owners` drops the user's own PR
- Location: specs/04-api.md API-15; specs/02-polling.md POL-5
- Status: parked. Read literally, "its owner is in `owners`" is false for every owner when `owners` is empty, so the PR is dropped, found again and re-baselined, losing what arrived in between. Fix: "passes POL-5's owner filter".

### H1-15 · low · Sorting `list` by `updated_at` hides check activity
- Location: specs/04-api.md API-6; docs/02 §4 (checks don't move `updatedAt`, verified)
- Status: parked. Sorting by `rev` would put the last visible change first, checks included.

### Out of scope
- specs/05-cli.md CLI-2: a bare `changes` resolves to the current branch's PR, so API-8's all-PR mode can't be reached from the CLI.
- specs/04-api.md API-18 still says "Routes naming one PR … read through"; since H1-8 only a PR with no record is read through. Wording only.

## Pass 2: failure modes (2026-11-25)

Reviewers: 3 (time and PR-state boundaries; infrastructure failure; lifecycle), with 26 raw findings; 22 after merging duplicates. The held arrival refresh, the wait on a loaded PR, and `stop` undone by waiters were each raised by two or three reviewers. Dropped in verification: 3. One misread: a killed client's waiter leaking until its timeout. hyper 1.11.1 notices EOF while a response is pending unless half-close is on, which it isn't by default (`src/proto/h1/conn.rs`, `mid_message_detect_eof`). One handled by another session during this pass: `stop` undone by waiters reconnecting (CLI-10 now never starts a daemon; OPS-7 "a stop stays a stop"). One out of scope: a required check that never reports (01-model MOD-3, open decision D6). Another session hardened 05-cli at the same time; its API-4 `message` and API-16 changes were kept.

### H2-1 · high · A wait whose arrival refresh is held is never evaluated, so it times out on a record that already holds
- Location: specs/04-api.md API-9, API-12, API-13; specs/02-polling.md POL-15, POL-19
- Status: applied: an arrival refresh that isn't sent (POL-15's window, POL-19's backoff, `auth`, or POL-23 sending none) counts as failed at once; nothing waits past the deadline, which evaluates the record in memory; API-13 answers at once when memory holds the condition or an end

### H2-2 · high · `checks-done` returns the previous run's result when CI runs again on the same head
- Location: specs/04-api.md API-10; specs/01-model.md MOD-6; specs/05-cli.md CLI-11
- Status: applied: `checks-done` takes `since`, and with it needs a `checks` event after `since`; a too-old `since` drops that part (proposal; a re-run isn't measured). CLI-11's guide gives the rule.

### H2-3 · medium · API-9 reads through a loaded PR that STA-8 holds, and a held wait has no record to answer with
- Location: specs/04-api.md API-3, API-9; specs/03-state.md STA-8; specs/05-cli.md CLI-6
- Status: applied: a loaded PR's wait is held with no read-through and no arrival refresh; answered before its first refresh it has `record: null`; API-3's note says "except by `wait`"; CLI-6 prints `<pr> no record yet`

### H2-4 · medium · A refresh showing both "checks done" and `merged` ends a `checks-done` wait, so the exit code depends on timing
- Location: specs/04-api.md API-11
- Status: applied: the condition is checked before the ends; `merged` ends `checks-done` only while its checks aren't done

### H2-5 · medium · `daemon.problem` has no time and no precedence, so CLI-3's "rate_limited until 14:05" can't be built
- Location: specs/04-api.md API-4; specs/05-cli.md CLI-3
- Status: applied: API-4 gains `until` and an order (`auth`, `rate_limited`, `github_unavailable`, `quota_low`); `last_poll_at` is the last poll applied

### H2-6 · medium · After a first poll that partly failed, `list` errors although most records are in memory
- Location: specs/04-api.md API-18; specs/02-polling.md POL-19; specs/05-cli.md CLI-3
- Status: applied: once the first poll has finished, `list` and `changes` answer with the records there are and name the rest in `pending`; only a first poll none of whose requests was answered gives the error. CLI-3 prints `pending <pr>`.

### H2-7 · medium · A read-through answered with an error other than `NOT_FOUND` has no API-3 code
- Location: specs/04-api.md API-3; specs/02-polling.md POL-16
- Status: applied: it's `github_unavailable`, the message naming the error's `type` and the token source, never GitHub's text

### H2-8 · medium · A newer CLI's new parameter is either "wrong usage" or silently ignored by an old foreground daemon
- Location: specs/04-api.md API-1, API-3; specs/05-cli.md CLI-9
- Status: applied: unknown parameters, keys and values are `bad_request`, never ignored; CLI-9 reads `bad_request` or `unknown_route` from an older daemon it doesn't replace as "restart it", exit 1

### H2-9 · medium · A read-through, `fresh` or arrival refresh queues behind a whole poll, with no bound
- Location: specs/04-api.md API-7, API-13; specs/02-polling.md POL-2, POL-15
- Status: applied: POL-15 sends a forced refresh, a read-through and a `track` next, between a poll's requests; API-13 says so

### H2-10 · medium · A PR whose own refresh keeps failing looks only `stale`; the reason is only in `status`
- Location: specs/01-model.md MOD-2, MOD-14; specs/02-polling.md POL-21, POL-24; specs/05-cli.md CLI-3
- Status: applied: the record carries `error: {type, since}`, not a visible change; CLI-3's stale mark names the type

### H2-11 · low · `untrack` of a PR that isn't tracked reads it through and tracks it
- Location: specs/04-api.md API-15, API-18
- Status: applied: `untrack` never reads through and answers `record: null`; `track` of a PR with a record doesn't call GitHub, and of a loaded one sets the saved `source`

### H2-12 · low · `list --since` names a PR that was dropped and then tracked again as `untracked`
- Location: specs/04-api.md API-6; specs/05-cli.md CLI-3
- Status: applied: only PRs dropped after the cursor and not tracked now

### H2-13 · low · Requests other than waiters have no defined answer at shutdown
- Location: specs/04-api.md API-14, API-17; specs/06-operations.md OPS-7
- Status: parked. A `track` or `refresh` in progress when the daemon stops (a replacement, an idle exit) loses its connection, and the CLI can't tell whether it took effect. Fix: answer from memory or with a new `stopping` (503) code, and the CLI repeats once; every route is idempotent.

### H2-14 · low · `changed --since` is satisfied by the wait's own read-through
- Location: specs/04-api.md API-10; specs/01-model.md MOD-14, MOD-15
- Status: parked. A wait on an untracked PR with a cursor from `list` reads it through, which takes a revision with a lone `tracked` event, so the wait is satisfied at once though nothing changed on GitHub. Fix: a `tracked` event with no other kind doesn't hold `changed`.

### H2-15 · low · A waiter on a merged, closed or gone PR isn't refreshed at the hot rate
- Location: specs/04-api.md API-13; specs/02-polling.md POL-10, POL-14
- Status: parked. POL-10 refreshes an ended PR only every `idle_interval` "whatever the tier", which contradicts API-13's "a waiter makes its PR hot". Fix: POL-10 adds "unless it's hot". Closed in 02-polling pass 3 (H3-9): POL-10's period table puts hot first.

### H2-16 · low · API-11's 60 s and API-13's 5 s have no clock
- Location: specs/04-api.md API-11, API-13; specs/02-polling.md POL-26
- Status: parked. POL-26's list of monotonic durations doesn't name them, and the 5 s age is read from `fetched_at`, a wall time; after the clock is set back, no arrival refresh happens. Fix: both on the monotonic clock, from the refresh's send.

### H2-17 · low · A rename or transfer during a wait loop breaks the `again:` command
- Location: specs/04-api.md API-9; specs/05-cli.md CLI-6; specs/01-model.md MOD-1
- Status: parked. Nothing says a waiter is bound to the node ID, and the `again:` line repeats the old name. Fix: waiters are bound to the node ID; `again:` names the answer's `pr`.

### H2-18 · low · Nothing tells the user that a running daemon ignores an edited `config.toml`
- Location: specs/04-api.md API-5; specs/06-operations.md OPS-1
- Status: parked. Fix: a `config_changed` warning, checked with STA-11's minute check.

### H2-19 · low · "Only the user can open it" rests on the socket file's mode, which `CIRCITOR_SOCKET` can place anywhere
- Location: specs/04-api.md API-1; specs/06-operations.md OPS-4
- Status: parked, for the security lens. Fix: check each connection's peer user ID (`SO_PEERCRED`, `getpeereid`), or refuse a `CIRCITOR_SOCKET` in a directory others can write to.

### Out of scope
- specs/01-model.md MOD-3: a required check that never reports (`EXPECTED`, or left out) keeps `checks-done` pending forever or passes it while `merge_state` is `blocked`. Rests on D6.
- specs/01-model.md MOD-4: a re-run that keeps its check run's `id` is hidden for 30 s; H2-2's `since` wait then waits that long.
- specs/02-polling.md POL-22: a wall-clock step while awake (NTP, a manual change) reads as a sleep and ends every wait as a timeout. Comparing a clock that counts suspend with one that doesn't would avoid it; it needs a platform fact in docs/04 first.
- specs/03-state.md STA-11: a foreground daemon restarted by its service manager can find a spawned one holding the lock (any CLI-7 call in between) and exit 0, leaving the spawned one in its place.
- specs/05-cli.md CLI-7: a daemon that crashes on some response is started again by every CLI call, and each new one re-baselines and polls at once. Nothing limits it.

## Pass 3: implementability, testability, simplification (2026-11-25)

Reviewers: 3 (implementer, tester, simplifier), with 25 raw findings; 17 after merging duplicates. The `timeout=0` contradiction was raised four times, `since`'s two jobs three times, and the response keys, `changes?pr=` and the `list` tie-break twice each. Dropped in verification: 1 out of scope (new TST-8 scenarios, below). Another session edited API-3's `auth` row, API-5 and API-14 during the pass; its changes were kept.

### H3-1 · high · `since` does two jobs, so the `again:` line can turn `checks-done` into a wait that never holds
- Location: specs/04-api.md API-10; specs/05-cli.md CLI-1, CLI-6, CLI-11; specs/01-model.md MOD-14, MOD-15
- Status: applied: the re-run rule moves to a new `rerun_since` (`--rerun-since`); `since` only sets where the answer's events start, for every condition; `sha` and `rerun_since` are `checks-done`'s only, `bad_request` with another condition; `again:` repeats `--rerun-since` as given; CLI-11's guide names it

### H3-2 · medium · `timeout=0` both waits for a refresh and answers at once, and a deadline that passes during a read-through has no answer
- Location: specs/04-api.md API-9, API-12; specs/05-cli.md CLI-6
- Status: applied: `0` waits for its first evaluation, the one wait that can outlast its deadline (H1-7's meaning kept); a deadline that passes during a read-through answers 200 `timeout` with `record: null`, and the read-through goes on; `daemon.problem` says why only when a problem holds; the CLI sends no request with under 1 s left, unless the user gave `--timeout 0`

### H3-3 · medium · No route's response body names its keys
- Location: specs/04-api.md API-3, API-4, API-5, new API-19
- Status: applied: new API-19 table of each route's keys (lists never left out, flags `false`); API-5 names its keys; API-3's error gains `until`; API-4's parentheses are explanations, not values (proposal: the names). The event's own shape stays with 01-model H1-17.

### H3-4 · medium · API-5's warnings have no codes and no rule for clearing
- Location: specs/04-api.md API-5; raised by POL-7, POL-18, POL-21, POL-23, STA-3, STA-8
- Status: applied: a table of six codes, each with what raises and what clears it (proposal: the codes)

### H3-5 · medium · `refresh` can count as done before the poll's other requests answer, and a partly failed poll has no `outcome`
- Location: specs/04-api.md API-16; specs/02-polling.md POL-7, POL-19
- Status: applied: done once every request of the poll (POL-7, POL-8, POL-12) and the new PRs' first refreshes have answered or failed; `polled` needs every request answered, otherwise the problem; the points count every request sent

### H3-6 · medium · `changes?pr=` has no value format, and it's unclear whether it waits or reads through
- Location: specs/04-api.md API-8, API-18, API-19
- Status: applied: the one-PR form is `GET /v1/prs/{owner}/{repo}/{number}/changes`; routes with a PR in their path don't wait and read a PR with no record through, but `wait` and `untrack`. Also settles pass 1's out-of-scope note on API-18's wording.

### H3-7 · medium · The steps of a wait's arrival have no order, and "since the wait began" has two anchors
- Location: specs/04-api.md API-9, API-10, API-14
- Status: applied: API-9 lists the arrival in order (parameters, limits, read-through, registration and headers, arrival refresh, first evaluation); `Circitor-Cursor` is the cursor at registration; without `since` the events are those after it. `changed`'s base stays after the refresh (H1-7).

### H3-8 · medium · A cursor from a one-PR answer silently skips other PRs' events
- Location: specs/04-api.md API-8, API-9; specs/05-cli.md CLI-11; specs/01-model.md MOD-13
- Status: applied: a one-PR answer's cursor covers that PR only, and a wait's events are its PR's; the guide says so

### H3-9 · medium · The accepted values of the parameters aren't listed, although API-1 rejects any other
- Location: specs/04-api.md API-1, API-8, API-10, API-12
- Status: applied: flags take `1` or `0`; `limit` is 1 to 500; `sha` is 7 to 40 hex characters, compared without case; a value out of range is `bad_request`, never clamped

### H3-10 · medium · Two thresholds gate one PR's forced refresh, and API-13 cites POL-15 for a 5 s it doesn't hold
- Location: specs/04-api.md API-13; specs/02-polling.md POL-15
- Status: applied: one rule in POL-15, a forced refresh of one PR only when its last refresh of any kind was sent more than 5 s ago, on the monotonic clock (30 s capped or in reserve); API-13 cites it. Settles the 5 s half of parked H2-16; a `show --fresh` loop now costs at most 720 points an hour.

### H3-11 · low · `list`'s order has no tie-break
- Location: specs/04-api.md API-6
- Status: applied: ties broken by `pr`; `untracked` and `pending` sorted by `pr`

### H3-12 · low · `pending` isn't scoped: only the first poll, or any PR found and not yet refreshed
- Location: specs/04-api.md API-18; specs/02-polling.md POL-4
- Status: applied: at any time after the first poll, including a PR found by a later poll

### H3-13 · low · `untrack` reimplements POL-5's owner filter
- Location: specs/04-api.md API-15; specs/03-state.md STA-1
- Status: parked. Overlaps parked H1-14. Simpler: `untrack` makes a `manual` PR `asked`, and the next poll's finding makes the user's own one `authored` again (POL-5); one branch, no owner check in the API. Lost: an untracked PR that isn't the user's own stays as `asked` for up to 60 minutes.

### H3-14 · low · `head_moved`'s 60 s branch costs a per-waiter clock and can say something false
- Location: specs/04-api.md API-11; specs/05-cli.md CLI-6
- Status: parked. Fix: `head_moved` only when the head matched once and then changed; a SHA that never matches ends as `timeout`, whose line names both heads. Settles parked H2-16's 60 s half and 05-cli H2-13.

### H3-15 · low · `list --since` is a second "what changed" route that no user requirement needs
- Location: specs/04-api.md API-6; specs/05-cli.md CLI-1, CLI-3; specs/03-state.md STA-9
- Status: parked; a product choice. Removing it drops the `untracked` filtering, a reset path and CLI-3's exception to the 50-line cap; `changes --since` and `show` give the same.

### H3-16 · low · `daemon.tier` and `last_poll_at` on every response have no reader
- Location: specs/04-api.md API-4, API-5
- Status: parked. CLI-3 reads only `problem`, `message` and `until`; `status` carries the tier and the last poll itself. Fix: `daemon` is `{problem, message, until}`.

### Out of scope
- specs/07-testing.md TST-8: no scenario for `head_moved`, `checks-done --rerun-since` on a re-run, API-18's `starting` and `pending`, or API-14's 65th waiter and a client closing. Each can be scripted on the fake GitHub and the test clock.

## Pass 4: planning findings (2026-11-25)

No reviewers: the findings came from building the implementation plan (`plan/README.md`, "Spec issues found while planning") and from its brief writers, and were verified against the spec text. The `gh_path` key, the `log_unwritable` warning and the `auth` message for a refused gh token were applied from 06-operations' and 02-polling's pass 4 (their H4-3, H4-2 and H4-4).

### H4-1 · medium · Only `list` and `show` say whether a record carries `checks.items`
- Location: specs/04-api.md API-6, API-7, API-19
- Status: applied: every single `record` carries them; a list of `records` only in `list` with `checks=1`

### H4-2 · medium · `track` of a loaded PR with no record
- Location: specs/04-api.md API-15; specs/03-state.md STA-8
- Status: applied: the saved source becomes `manual` first, then a read-through; a failure answers its error and the source stays saved

### H4-3 · medium · `untrack` of a loaded `manual` PR can't apply its own rule
- Location: specs/04-api.md API-15
- Status: applied: it's dropped (`asked` with a waiter); the next poll finds it again if it's the user's own

### H4-4 · low · `paths.state`: directory or file
- Location: specs/04-api.md API-5
- Status: applied: `state.json`'s path

### H4-5 · low · `last_poll.outcome`'s values
- Location: specs/04-api.md API-5, API-16
- Status: applied: API-16's words without `shared`

### H4-6 · low · A too-old `since` on `list` and `state`
- Location: specs/04-api.md API-6
- Status: applied: all the records `state` asks for

### H4-7 · low · A known path with another method
- Location: specs/04-api.md API-3
- Status: applied: `unknown_route`, by path or by method

### H4-8 · low · `Circitor-Cursor` on a 200 sent before the waiter is registered
- Location: specs/04-api.md API-9, API-12
- Status: applied: every 200 answer to a `wait` carries it

### H4-9 · low · Ended and head moved at once
- Location: specs/04-api.md API-11
- Status: applied: the state's reason wins

