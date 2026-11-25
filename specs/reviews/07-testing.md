# Hardening log: 07-testing.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 2/9/4 | 12 | 0 | 0 | 0 | 3 |
| 2 | 2026-11-25 | failure modes | 2/14/3 | 12 | 0 | 0 | 0 | 7 |
| 3 | 2026-11-25 | implementability | 3/11/2 | 12 | 0 | 0 | 0 | 4 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3 (facts, contracts, coherence), with 34 raw findings; 15 after merging duplicates. The `closed` row, `repo-gone` and the newer state file were each raised by all three. Dropped in verification: none as false; 2 out of scope (below). Merged in: follow-ups for this file logged in specs/reviews/04-api.md:233, 05-cli.md:76, :170, :237 and 06-operations.md:158. Another session edited 01, 05, 06 and TST-8's `no-ci` row during the pass (`checks.settle_s` became a fixed settle time); its changes were kept.

### H1-1 · high · Real-process tests would poll api.github.com with the owner's real gh token
- Location: specs/07-testing.md TST-1, TST-9; specs/02-polling.md POL-1; specs/06-operations.md OPS-16, OPS-2 `exit_after`
- Status: applied: TST-1 defines a test build as the `test-endpoint` feature (address from `CIRCITOR_TEST_GITHUB`, loopback only, `exit_after` down to 1 s; the release build has neither); TST-9 runs every real-process test on a test build with temporary XDG directories and `GH_CONFIG_DIR`, a fake `gh` through `gh_path`, and stops every daemon it started

### H1-2 · high · TST-10 set aside a newer-version state file, which STA-8 leaves alone
- Location: specs/07-testing.md TST-10; specs/03-state.md STA-8; specs/04-api.md API-5
- Status: applied: a corrupt file is set aside with `state_file_set_aside`; a newer one is left unchanged and never written, with `state_file_newer`

### H1-3 · medium · `closed` said every wait exits 4, but a close satisfies `changed`
- Location: specs/07-testing.md TST-8 `closed`; specs/04-api.md API-11
- Status: applied: `merged` and unfinished `checks-done` waits end with `closed`, exit 4; `changed` is satisfied, exit 0

### H1-4 · medium · `repo-gone` dropped POL-21's 5 minutes and MOD-11's conditions
- Location: specs/07-testing.md TST-8 `repo-gone`; specs/02-polling.md POL-21; specs/01-model.md MOD-11
- Status: applied: 5 minutes and at least three refreshes; three in under 5 minutes stay `stale`; a changed login or every PR `NOT_FOUND` never makes it gone

### H1-5 · medium · TST-9 replaced a daemon of "another version"; logged follow-ups never landed
- Location: specs/07-testing.md TST-5, TST-7, TST-9; specs/05-cli.md CLI-8 to CLI-10; specs/06-operations.md OPS-5 to OPS-9
- Status: applied: only an older daemon is replaced, a newer, foreground or unparsable one is kept, and after a `stop` a waiting CLI starts none; TST-9 adds the stop's deadline, the foreground takeover, `stop` on a hung daemon, an unwritable log, and codes 130 and 143; TST-7 adds `refresh`, `stop`, CLI-6's outcome lines and `more events:`; TST-5's fake token source gives a token, no login or a failure

### H1-6 · medium · TST-6's quota limits didn't hold as written, and four quota states had no test
- Location: specs/07-testing.md TST-6; specs/02-polling.md POL-14, POL-17, POL-23, POL-25
- Status: applied: the test world is stated (nothing changes, `mergeable` known); a hot PR adds at most 250 in the active tier and 350 in the idle; capped sends forced refreshes, reading through, `track` and their POL-12 requests in 30 s windows; `slowed`, `exhausted` and `secondary` are tested

### H1-7 · medium · Restart, start-up and several wait rules had no scenario
- Location: specs/07-testing.md TST-8; STA-5, STA-8, MOD-15, API-10, API-11, API-14, API-18
- Status: applied: new rows `restart`, `starting`, `head-moved`, `rerun`, `waiter-limit`

### H1-8 · medium · TST-13 merged a PR carrying a workflow, putting it on `main`
- Location: specs/07-testing.md TST-13 steps 2–3; docs/05-measurements.md rules
- Status: applied: the workflow PR in `yachiko/experiment` is closed unmerged (`wait --until merged` exits 4 with `ended: closed`); a second PR without a workflow is merged

### H1-9 · medium · TST-13 named the wrong docs/04 §8 items and never tried the Linux sandbox
- Location: specs/07-testing.md TST-13 steps 5–6; docs/04-platform.md §8; docs/03-clients.md §2, §6
- Status: applied: items 1, 2, 4 and 6 (a `wait` across a lid close, on macOS and Linux); the sandbox check on macOS and Linux with each setting; results go into docs/05 on the owner's say

### H1-10 · medium · TST-4's "timeouts of real sockets" exception contradicted TST-2
- Location: specs/07-testing.md TST-4, TST-2; specs/02-polling.md POL-26
- Status: applied: every timeout circitor sets (POL-2, OPS-5, CLI-7) runs on the clock trait; only TST-9 waits on the wall clock

### H1-11 · medium · The fake GitHub's contract left out what the specs read, and called a guess measured
- Location: specs/07-testing.md TST-3; docs/02 §2, §4, §6; MOD-6, MOD-11, POL-24, POL-25
- Status: applied: cost by GitHub's formula, checked against every row of the measured table; `Date` and `x-ratelimit-*` headers; a changeable `viewer.login`; a default world as docs/02 §4 records; a 200 without `data`; the rate-limit answers labelled assumed

### H1-12 · low · TST-13 steps 1 and 4 compared against references that can't match
- Location: specs/07-testing.md TST-13 steps 1, 4
- Status: applied: step 1 uses `--limit 200`, a minute after the last change, with `owners` empty and nothing tracked by hand; step 4 compares `remaining`'s fall in one reset window with the debug log's costs

### H1-13 · low · TST-8 says all timings come from the logs, but some are chosen
- Location: specs/07-testing.md TST-8 lead; `first-check-early` (+3 s, +8 s), `late-ci` (40 s)
- Status: parked. No log has an early commit status (c1.log's was posted at +40 s) or a head without checks for 40 s. Idea: "Timings marked (measured) come from the logs; the others are chosen to test a rule."

### H1-14 · low · TST-12 doesn't fix the fixtures' format or where untestable cases come from
- Location: specs/07-testing.md TST-12; docs/05-measurements.md:9 ("the testing spec decides their format"); docs/02 §4
- Status: parked. `blocked`, required checks and over 100 checks can't be recorded in `yachiko/experiment` (no branch protection); recording is a live measurement needing the owner's OK. Idea: one JSON response per file beside its query; read-only recordings from public repositories, or hand-written ones marked so.

### H1-15 · low · TST-5 gives commands a fixed time zone that CLI-13 doesn't list as an input
- Location: specs/07-testing.md TST-5; specs/05-cli.md CLI-13
- Status: parked. CLI-3 prints local times (`until 14:05 local`), so the time zone is an input; CLI-13 should name it.

### Out of scope
- specs/02-polling.md POL-13: doesn't say whether a held waiter counts as client activity for the tier. TST-6's idle-tier figure assumes it doesn't, as OPS-8's "no client request and no waiter" suggests.
- specs/02-polling.md POL-18: with GitHub's counters lagging (docs/02 §2: `remaining` read 4 989 then 4 998), `remaining` near `quota.reserve` can flip the `reserve` state and its warning on every poll. No hysteresis is specified.

## Pass 2: failure modes (2026-11-25)

Reviewers: 3 (time and PR state, infrastructure, lifecycle), with 30 raw findings; 19 after merging. Retention was raised by two reviewers, the test clock's advance by two, and growth by two. Dropped in verification: 1 withdrawn by its reviewer (covered by the new `settled-at-once` row); one clause cut from H2-11 ("a page shift makes no PR `asked`"), because POL-5 as written would demote it (out of scope, below). Another session edited 01 to 07 during the pass, adding the rows `renamed`, `unlisted-values`, `head-back`, `base-changed` and `gone-returns`; H2-12 was narrowed to what those leave out, and its changes were kept.

### H2-1 · high · On macOS, real-process tests used the owner's real state directory and socket
- Location: specs/07-testing.md TST-9; specs/06-operations.md OPS-3, OPS-4
- Status: applied: `HOME`, every XDG directory and `GH_CONFIG_DIR` in one temporary directory under `/tmp` (socket path under 103 bytes); `exit_after` a few seconds; a 30 s limit; daemons found through `circitor.lock` are stopped; OPS-4's long path and non-socket file are tested

### H2-2 · high · A restart with GitHub down, a crash, and files removed under a running daemon were untested
- Location: specs/07-testing.md TST-5, TST-8, TST-9; specs/03-state.md STA-7, STA-8, STA-11
- Status: applied: the harness can crash the daemon; rows `restart-offline`, `crash`, `files-removed`; TST-9 adds `SIGKILL` with a waiting CLI

### H2-3 · medium · TST-4 didn't say how an advance fires timers; quota tests could pass with polling broken
- Location: specs/07-testing.md TST-4, TST-6, `sleep`; POL-22, POL-26; docs/04 §8 item 6
- Status: applied: each timer fires at its own due time, in order, settling after each; a sleep moves the wall time alone or both clocks; the wall time can be set back; TST-6 has lower bounds (110 and 11 polls); `sleep` runs both ways; new row `clock-back`

### H2-4 · medium · Retention (U7) had no scenario
- Location: specs/07-testing.md TST-8; specs/03-state.md STA-1, STA-2, STA-8
- Status: applied: new row `retention`

### H2-5 · medium · A first run with no login or no network had no scenario; the real `gh` reader was never run
- Location: specs/07-testing.md TST-5, TST-8, TST-9; API-18, POL-3, POL-19, POL-20, OPS-5
- Status: applied: new row `first-run`; the fake token source can be slow; TST-9's fake `gh` gives every outcome, and the reader tells them apart and kills a hung `gh`

### H2-6 · medium · TST-10's kill test couldn't catch a missing sync; disk-full and unwritable writes untested
- Location: specs/07-testing.md TST-10; specs/03-state.md STA-7; specs/06-operations.md OPS-7
- Status: applied: writes go through a fakeable file-system interface; a power cut, a full disk, a denied write and a leftover temporary file are tested

### H2-7 · medium · The fake's "two requests at once" check tripped on its own hang
- Location: specs/07-testing.md TST-3; POL-2
- Status: applied: a request ends when answered or its connection closes; a hung answer watches its connection; the 1 s gap is checked

### H2-8 · medium · Quota tests couldn't play other tools' spending, missed limit forms, and didn't count `mergeable` follow-ups
- Location: specs/07-testing.md TST-3, TST-6, `mergeable-unknown`; POL-11, POL-25; docs/02 §2
- Status: applied: the test sets `remaining` and the reset time and can make them lag; no `x-ratelimit-reset`, 403 and 200 secondary forms, a jump past the reset; at most one follow-up per refresh, shared at start, none when capped or in reserve

### H2-9 · medium · The fake GitHub shared the daemon's wall clock
- Location: specs/07-testing.md TST-3, `settled-at-once`, `sleep`; MOD-6
- Status: applied: the fake has its own clock with a settable skew, and `Date` comes from it; `settled-at-once` with the local clock 5 minutes behind; `sleep` with a PR merged meanwhile

### H2-10 · medium · Network failures beyond a 5xx, and per-PR errors other than `NOT_FOUND`, had no scenario
- Location: specs/07-testing.md TST-3, `github-down`; POL-16, POL-21, POL-24, API-5, OPS-16
- Status: applied: non-JSON and cut bodies, a redirect (not followed) and a refused connection; a hang abandoned at 20 s; one read-through per backoff step; new row `pr-error`

### H2-11 · medium · Growth past one batch, one page and the 200 limit was never tested
- Location: specs/07-testing.md TST-3, TST-6, `many-checks`; POL-7, POL-8, STA-3
- Status: applied: the fake pages over the live world; new row `many-prs`; check-page edges and a failed second page; 200 PRs cost at most 600 points an hour

### H2-12 · medium · `track`/`untrack` idempotency and a transfer out of `owners` were untested
- Location: specs/07-testing.md TST-8 `renamed`; API-15, STA-1, STA-3, CLI-10, POL-5
- Status: applied: new row `track-untrack`; `renamed` adds a held wait, a read-through under the new name, and a transfer out of `owners`

### H2-13 · medium · Config changes reach the daemon only by a restart or replacement, neither tested with a changed config
- Location: specs/07-testing.md TST-8, TST-9; OPS-1, POL-5, STA-8, CLI-9, OPS-9
- Status: parked. Idea: a `config-restart` row (narrowed `owners` demotes loaded `authored` PRs to `asked`, a shorter `keep_done` drops entries on loading); TST-9: an older daemon kept when the CLI rejects the config, `--check-config`'s exit status, log rotation at 5 MB.

### H2-14 · medium · Version skew is tested only against today's code under another version string
- Location: specs/07-testing.md TST-5, TST-9, TST-10; API-2, STA-8
- Status: parked. Idea: `tests/compat/<version>/` holds each release's `status` response, `shutdown` exchange and `state.json`, never rewritten by TST-7's update flag; the current CLI and daemon must read each.

### H2-15 · medium · Fixtures and the fake GitHub drift apart; TST-12's "fails here first" can't hold
- Location: specs/07-testing.md TST-12, TST-3; MOD-2
- Status: parked (with H1-14). Idea: each fixture stores its query; a changed production query fails until re-recorded; the fake's answer to a fixture's query has the same keys and types; fixtures are re-recorded before each release under docs/05's rules.

### H2-16 · medium · The `rerun` row passes or fails by which re-run form the test scripts
- Location: specs/07-testing.md TST-3, `rerun`; MOD-4, API-10; docs/02 §4 (re-run not measured)
- Status: parked. Idea: the fake scripts a re-run as a new `databaseId` and as the same one returning to `queued`; run both, including one that finishes between two reads. The spec side is out of scope (below).

### H2-17 · low · Scenarios land exactly on thresholds and word the comparison differently from the spec; `late-ci` stops short of 60 s
- Location: specs/07-testing.md `no-ci`, `late-ci`, `mergeable-unknown`, `cursor-reset`, `head-moved`
- Status: parked. Idea: an answer delay in the fake; each threshold read 1 s before, at and after; `late-ci` with none for 70 s.

### H2-18 · low · Changes that change nothing visible are untested; the default world omits a review's late `updatedAt`
- Location: specs/07-testing.md TST-3, TST-8; MOD-14; docs/02 §4
- Status: parked. Idea: a review moves `updatedAt` up to 13 s after its count; a `no-change` row: only `updatedAt`, a falling count, `review_threads` or a check's `url` takes no revision.

### H2-19 · low · TST-13 never covers an upgrade, an uninstall, or test hooks in the release binary
- Location: specs/07-testing.md TST-13, TST-1; OPS-15, STA-12
- Status: parked. Idea: a step 7 with a real `brew upgrade` while a `wait` is held, a check that the tap's binary ignores `CIRCITOR_TEST_GITHUB`, and what `brew uninstall` leaves; TST-9 checks only STA-12's files are written.

### Out of scope
- specs/02-polling.md POL-5: finding pages by cursor over a list ordered by `UPDATED_AT`; a PR updated or closed between two pages can shift another out of both, and POL-5 then makes that user's own PR `asked`.
- specs/02-polling.md POL-11: a waited-on PR whose `mergeable` stays `UNKNOWN` gets a follow-up after every hot refresh, about 360 points an hour on top of POL-14's; only POL-17's budget bounds it.
- specs/02-polling.md POL-25: doesn't say whether a 200 with `data` and `x-ratelimit-remaining: 0` is applied, or what a reset time already past by a fast local clock means.
- specs/01-model.md MOD-4, specs/04-api.md API-10: a re-run that keeps its `databaseId` and finishes between two reads with the same result makes no `checks` event, so `--rerun-since` times out although CI ran.

## Pass 3: implementability (2026-11-25)

Reviewers: 3 (implementer, tester, simplifier), with 26 raw findings; 16 after merging. Dropped in verification: none as false; several cited line numbers were off as the files changed, but every quote held. Three findings (H3-2, H3-4, H3-5) fix contradictions introduced by pass 2's own edits. Another session added to TST-10 during the pass (STA-7's 5 s spacing, and `testdata/state/` with one file per released `version`), which covers part of H2-14.

### H3-1 · high · The test build lacked the hooks TST-9 needs, and nothing made the tests use one
- Location: specs/07-testing.md TST-1, TST-5, TST-9; CLI-7 to CLI-9, OPS-7, OPS-11
- Status: applied: `CIRCITOR_TEST_VERSION`, `CIRCITOR_TEST_CLI_LIMIT`, `CIRCITOR_TEST_STOP_DELAY`; `CIRCITOR_TEST_GITHUB` is a full loopback URL, and a test build without it stops; `required-features` and `cargo test --features test-endpoint`; a hung daemon is one sent `SIGSTOP`, with the CLI's limit at 2 s; the kept-daemon cases (newer, foreground, unparsable) move in-process to TST-5

### H3-2 · high · TST-4's "settles" couldn't finish while the fake hangs, and couldn't be observed
- Location: specs/07-testing.md TST-4, TST-3, `github-down`
- Status: applied: settled is a count of work in flight that a test build keeps, with scripted hangs and held waiters aside, and every accepted connection answered, closed or held; timers due at one instant fire in the order set

### H3-3 · high · The token tests couldn't fail
- Location: specs/07-testing.md TST-9, TST-11; OPS-5, OPS-9; docs/02 §1
- Status: applied: the fake `gh` prints `GH_TOKEN`, else `GITHUB_TOKEN`, else the marker, as gh does; `GH_TOKEN` is set to a second marker the fake GitHub must never receive; every test uses a marker token and searches for it at its end; the fake `gh` prints it on stderr and before failing; titles, error messages and the 401 body carry markers that never reach the log or the state file

### H3-4 · medium · The fake GitHub's clock followed the laptop's, so `clock-back` rewound GitHub
- Location: specs/07-testing.md TST-3, `clock-back`; POL-26, MOD-6
- Status: applied: GitHub's clock only moves forward, with advances and sleeps; setting the local wall time back leaves it; the 1 s gap is measured on the test clock's monotonic time

### H3-5 · medium · The fake's one-at-a-time and 1 s checks spanned daemons
- Location: specs/07-testing.md TST-3; `crash`, `restart`, TST-9; POL-2
- Status: applied: the checks are per daemon, starting afresh at each start, and per process in TST-9

### H3-6 · medium · The harness set no mode, config or log sink, and its temporary directories broke on macOS
- Location: specs/07-testing.md TST-5; OPS-2, OPS-4, OPS-8, OPS-9
- Status: applied: by default the daemon is `spawned` with OPS-2's defaults but `exit_after` 0, logs to a buffer the test reads, and is given its state directory and socket path under `/tmp` directly

### H3-7 · medium · Golden files held values that change between runs and operating systems
- Location: specs/07-testing.md TST-5, TST-7; STA-9, API-5, CLI-7
- Status: applied: a fixed seed; the test clock starts at 2026-11-24T22:00:00Z; `<tmp>` and `<os error>` replace paths and OS error text; the update flag is `CIRCITOR_UPDATE_GOLDEN=1`

### H3-8 · medium · Per-PR cadence and POL-15's forced-refresh windows were never checked
- Location: specs/07-testing.md TST-3, TST-6; POL-10, POL-14, POL-15, API-13
- Status: applied: the fake records the node IDs each request asked for; TST-6 gives a hot PR a lower bound (230 refreshes in the active hour), checks what makes a PR hot, and the ended and `asked` rate; new row `forced`

### H3-9 · medium · The order of `status` words and a truncated list's state were never checked
- Location: specs/07-testing.md new TST-15, `many-checks`; MOD-5, MOD-7, MOD-11
- Status: applied: TST-15 tests the derivations as tables over single records, with one case per row and per adjacent pair of `status` rows; `many-checks` adds the truncated states by rollup

### H3-10 · medium · TST-8's rows restated the rules they test, and had drifted
- Location: specs/07-testing.md TST-8, TST-6; API-15, POL-13, POL-14
- Status: applied (the light fix): where a row and its requirement differ, the requirement wins, and a row's values are today's defaults; TST-6's figures scale with the config; `track-untrack` now follows API-15; `push-no-checks-yet` folded into `late-ci`. Not done: rewriting every row to state only its world.

### H3-11 · medium · Outputs with fixed values or thresholds that no test read
- Location: specs/07-testing.md TST-6, TST-7, TST-9; OPS-3, OPS-4, STA-7, OPS-17, API-1, API-4, API-5, API-12, API-19
- Status: applied: file modes and OPS-17's memory and latency in TST-9; each state's `daemon.problem` and the quota warnings in TST-6; `--json` keys and `bad_request` in TST-7

### H3-12 · medium · The spec didn't say how the fake understands queries
- Location: specs/07-testing.md TST-3; docs/02 §2
- Status: applied: the fake checks each query against GitHub's public schema in `testdata/github/schema.graphql`, answers only the fields asked, and costs from the connections; the cost test's queries are in `tests/cost/`

### H3-13 · medium · Wait deadline and arrival edge cases have no row
- Location: specs/07-testing.md TST-8; API-9, API-12, CLI-6
- Status: parked. Idea: a `wait-deadline` row: a hung read-through answers `timeout` with `record: null` at the deadline; a spawn that took 7 s of 30 sends `timeout=23`; under 1 s left sends nothing and exits 3; an arrival inside POL-15's window or the backoff is evaluated at once.

### H3-14 · medium · The current-branch form of `show`/`wait` (CLI-2) can't be set up by the harness
- Location: specs/07-testing.md TST-5, `bare-pr`; CLI-2
- Status: parked. Idea: the harness's repository can have an upstream of another name, a detached HEAD, or an `origin` not on github.com; `bare-pr` adds another author's PR on the same branch (never matches), the upstream's name matching, and a `rate_limited` poll with no match exiting 9.

### H3-15 · low · `first-check-early` can't fail, and TST-13 step 4 has no tolerance
- Location: specs/07-testing.md `first-check-early`, TST-13 step 4; MOD-6
- Status: parked. Idea: `checks-done` doesn't hold before a refresh sent 20 s after the head was first read, then only once the workflow's checks finish; step 4 agrees within 10 points.

### H3-16 · low · TST-10's power-cut model costs more than a check of the write order
- Location: specs/07-testing.md TST-10; STA-7, STA-8
- Status: parked. Idea: the fake file system records calls; a test checks write, sync, rename in that order, and that stopping after any step leaves the old or the new file.
