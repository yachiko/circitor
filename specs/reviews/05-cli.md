# Hardening log: 05-cli.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 4/7/4 | 12 | 0 | 0 | 0 | 3 |
| 2 | 2026-11-25 | failure modes | 1/11/9 | 12 | 0 | 0 | 0 | 9 |
| 3 | 2026-11-25 | implementability | 2/9/3 | 12 | 0 | 0 | 0 | 2 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3; 29 raw findings; dropped in verification: 0 as wrong, 14 merged as duplicates (the reset, the exit codes, the `again:` line and the event lines were each found by all three).

Another session edited 04-api and 05-cli during this pass: it added `Circitor-Cursor` to API-9, rewrote CLI-10 to use it, and added `list --since`'s `untracked` lines to CLI-3. H1-3 was merged onto that.

### H1-1 · high · A too-old cursor printed as "nothing happened"
- Location: specs/05-cli.md CLI-3, CLI-6; specs/04-api.md API-8; specs/03-state.md STA-9
- Status: applied: CLI-3 prints a reset line and one `list` line per record before the events

### H1-2 · high · The bare PR form could pick a merged PR from the same branch, or fail on a PR just opened
- Location: specs/05-cli.md CLI-2; specs/03-state.md STA-2; specs/02-polling.md POL-4
- Status: applied: an open match first; with none, a poll (API-16) and a second look; then the most recent merged or closed match, said so; two matches of one kind are exit 2; a fork clone needs the full form (forks not measured)

### H1-3 · high · A `wait` could run past 120 s, and a reconnect could lose a change
- Location: specs/05-cli.md CLI-6, CLI-7, CLI-10; specs/04-api.md API-12; docs/03 §1, §5
- Status: applied with change: CLI-6 holds one deadline for the whole command, and each request passes the time left; CLI-10 reconnects with the time left and ends as a timeout with none. The lost change is covered by the other session's `Circitor-Cursor` (API-9), so the proposed "read the cursor first" bullet was dropped.

### H1-4 · high · Replacing a daemon of another version couldn't detect a foreground daemon, blocked a wait first, and let versions fight
- Location: specs/05-cli.md CLI-8, CLI-9; specs/04-api.md API-2, API-5
- Status: applied: only an older daemon is replaced; a `wait` checks `GET /v1/status` first; a newer or `foreground` daemon is used with a warning; one replacement per command; CLI-8's commands never replace. API-5 gains `mode`; API-2 freezes `status`'s `version` and `mode` and `shutdown`.

### H1-5 · medium · Four API errors had no exit code, and an interrupt was "unexpected"
- Location: specs/05-cli.md CLI-5, CLI-6; specs/04-api.md API-3
- Status: applied: code 9 for `starting`, `github_unavailable`, `rate_limited`; code 10 for `limit`; 130 and 143 for signals; each API error has one code; an unknown route is 1; code 5 marks its assumed fact (docs/02 §5); code 9 says `gh` doesn't help

### H1-6 · medium · The CLI never showed `daemon.problem`
- Location: specs/05-cli.md CLI-3; specs/04-api.md API-4; specs/02-polling.md POL-20
- Status: applied: a `problem:` line before the cursor, naming the problem and what to do; exit codes unchanged

### H1-7 · medium · The `again:` line had no `--since` and didn't end with `cursor:`
- Location: specs/05-cli.md CLI-6, CLI-3
- Status: applied: the example carries `--since` and ends with `cursor:`; the line repeats only the parameters given

### H1-8 · medium · Event lines lost `fetch_inline` and `more`, and two kinds of "review" read alike
- Location: specs/05-cli.md CLI-3; specs/01-model.md MOD-12, MOD-15; docs/02 §4 (a thread reply is a review, verified)
- Status: applied: one line per event kind and per item; `review none -> changes_requested` vs `new review …`; `fetch_inline` printed when `inline` is above 0; a line for `more: true`

### H1-9 · medium · `changes` with no PR couldn't mean every PR
- Location: specs/05-cli.md CLI-1, CLI-2; specs/04-api.md API-8
- Status: applied: the bare form is for `show` and `wait`; `changes` without `PR` covers every tracked PR

### H1-10 · medium · `list` stopped at 50 PRs but handed out a cursor past all of them
- Location: specs/05-cli.md CLI-3; specs/01-model.md MOD-13
- Status: applied: the cap applies only without `--since`

### H1-11 · medium · A socket blocked by a sandbox was taken for "no daemon"
- Location: specs/05-cli.md CLI-7; docs/03 §2; specs/06-operations.md OPS-14
- Status: applied: only a missing socket file or a refused connection spawns; any other error is exit 6 naming `sandbox.network.allowUnixSockets` (proposal; the errors aren't measured)

### H1-12 · low · The spawn wait (5 s) was shorter than the lock retry (6 s)
- Location: specs/05-cli.md CLI-7; specs/03-state.md STA-11; specs/06-operations.md OPS-7
- Status: applied: the CLI waits up to 8 s, within the command's one deadline

### H1-13 · low · The spawned daemon's stderr points at a log file OPS-9 rotates
- Location: specs/05-cli.md CLI-7; specs/06-operations.md OPS-6, OPS-9
- Status: parked. One idea: stdin and stdout to `/dev/null`, stderr to the log until the daemon opens its own log and takes stderr over; or rotate by copy-and-truncate.

### H1-14 · low · `--json` has no size bound; Claude Code cuts output at 30 000 characters
- Location: specs/05-cli.md CLI-4; docs/03 §2; docs/02 §4 (up to 187 checks, observed)
- Status: parked. Fits the targeted "agent UX and output size" lens.

### H1-15 · low · `--json` prints the private API's body, but nothing says it isn't stable
- Location: specs/05-cli.md CLI-4, line 5; specs/04-api.md (log D5)
- Status: parked. Related to specs/reviews/01-model.md H2-12.

### Out of scope
- specs/07-testing.md TST-9: "a daemon of another version is replaced" now reads "an older one" (H1-4); TST-7 should cover codes 9, 10, 130 and 143.
- specs/04-api.md API-9, API-11: in the `auth` or `exhausted` state a `wait` can only end in `timeout` (also in the 01-model log); the CLI now prints the problem line (H1-6), but the exit code is still 3.

## Pass 2: failure modes (2026-11-25)

Reviewers: 3; 30 raw findings; dropped in verification: 0 as wrong, 7 merged as duplicates (the reconnect after a stop, the bare form's poll, the outcome lines, the problem line and the spawn failure were each found twice or more), 4 handled by another session during the pass (a re-run answering `checks-done` with the old result: API-10's `since`; `checks-done` on an auto-merged PR: API-11; a non-`NOT_FOUND` read-through error with no code: API-3's `github_unavailable`; `circitor daemon` beside a spawned daemon: OPS-6's takeover).

Another session edited 04-api, 05-cli and 06-operations during this pass: CLI-6's `record: null` line, CLI-9's `bad_request` from an older daemon, CLI-3's `pending` line, CLI-11's `--since` rule, API-4's `until`. H2-8 was merged onto its API-4: `until` plays the part of the proposed `retry_at`, and `message` was added beside it.

### H2-1 · high · A wait's outcome line can report the old head's status as the answer
- Location: specs/05-cli.md CLI-6; specs/04-api.md API-10, API-13
- Status: applied: CLI-6 lists every outcome line with the head it's about, `no checks reported` included; with `--sha` and another head, the timeout line names both heads

### H2-2 · medium · A waiting CLI undoes `circitor stop` and can push out a service's daemon
- Location: specs/05-cli.md CLI-10; specs/06-operations.md OPS-7; specs/03-state.md STA-11
- Status: applied: a reconnecting wait never starts a daemon; it waits up to 8 s for one, else exit 6 `daemon stopped` with `again:`. OPS-7 changed to match.

### H2-3 · medium · Replacing an older daemon throws away an answer that held new items
- Location: specs/05-cli.md CLI-9; specs/03-state.md STA-5, STA-6
- Status: applied: any command with `--since` checks the version first, as a `wait` does

### H2-4 · medium · The bare PR form fails on a poll that is `shared` or refused, and its tie rule contradicts itself
- Location: specs/05-cli.md CLI-2; specs/04-api.md API-16; specs/02-polling.md POL-15
- Status: applied: the poll's answer doesn't end the search; an ended match (`merged`, `closed`, `gone`) by latest `closed_at`; two open matches are exit 2; exit 2 means GitHub was read; a problem exits with its own code; API-16's "done" includes the first refresh of new PRs

### H2-5 · medium · Nothing limits how long the CLI waits for an answer
- Location: specs/05-cli.md CLI-6, CLI-7
- Status: applied: CLI-7 gives every request a CLI-side limit (wait: deadline + 10 s; refresh, track, show --fresh: 60 s; the rest: 20 s; proposal)

### H2-6 · medium · A wait that ends without a body has no defined output, and the obvious cursor skips events
- Location: specs/05-cli.md CLI-6, CLI-10; specs/04-api.md API-9
- Status: applied: CLI-6 defines the line; `again:` and `cursor:` use the `--since` given, or the first `Circitor-Cursor`, never a later one

### H2-7 · medium · Starting the daemon fails slowly and blindly, and an upgrade can replace a working daemon with one that won't start
- Location: specs/05-cli.md CLI-7, CLI-9; specs/06-operations.md OPS-1, OPS-3
- Status: applied: the CLI creates the state directory and opens the log, watches the child's exit, and runs `--check-config` before replacing

### H2-8 · medium · The `problem:` and exit-9 lines need data the API doesn't send
- Location: specs/05-cli.md CLI-3, CLI-5; specs/04-api.md API-3, API-4
- Status: applied with change: merged onto the other session's `until`; API-4 adds `message`, and `github_unavailable` and `rate_limited` errors carry `until`; CLI-3 prints the code and the message with a relative time; code 9 says when to try again, or in a minute

### H2-9 · medium · `changes` text output has no size bound, and its `cursor:` line comes last
- Location: specs/05-cli.md CLI-3; specs/04-api.md API-8; docs/03 §2
- Status: applied: API-8 and API-9 take `limit` and answer `more_events: true` (named apart from MOD-15's `more`); the CLI prints at most 50 events and a `more events:` line (proposal: 50)

### H2-10 · medium · An agent looping `show --fresh` on `mergeable unknown` can spend the whole budget
- Location: specs/05-cli.md CLI-3, CLI-11; specs/02-polling.md POL-15, POL-23
- Status: applied: `show` adds a `wait --until changed` hint on `unknown`; the guide says never to loop `show --fresh`

### H2-11 · medium · Only `wait` survives a dropped connection
- Location: specs/05-cli.md CLI-10; specs/06-operations.md OPS-7, OPS-8
- Status: applied: any other command, and a wait dropped before its `Circitor-Cursor`, is tried once more as a new command

### H2-12 · low · Every connect error but two is blamed on the sandbox, and `stop` can say "not running" while a daemon runs
- Location: specs/05-cli.md CLI-7, CLI-8; specs/06-operations.md OPS-4
- Status: applied: the path's length is checked first; only a denied permission names the sandbox; `status` and `stop` read connect errors as CLI-7 does

### H2-13 · low · Short `again:` waits never reach `head_moved`
- Location: specs/04-api.md API-11
- Status: parked. `head_moved`'s 60 s run per wait, so repeated 30 s waits with a wrong SHA never end it. H2-1's timeout line now names both heads. One idea: count the 60 s from the first waiter on that PR with that `sha`.

### H2-14 · medium · A `github_unavailable` for a lasting refusal (SSO, a scope) says "try again"
- Location: specs/05-cli.md CLI-5 code 9; specs/04-api.md API-3
- Status: parked. The other session's API-3 now maps non-`NOT_FOUND` errors to `github_unavailable`, so exit 9 tells the agent to retry something that won't change. The responses aren't measured (docs/02 §6).

### H2-15 · low · After a repository rename or transfer, the short forms name the old repository
- Location: specs/05-cli.md CLI-2; specs/01-model.md MOD-1
- Status: parked. `origin` keeps the old name; the bare form never matches and forces a poll each call. GraphQL's handling of a renamed repository isn't measured.

### H2-16 · low · A truncated checks list can read `checks_failing … 0 failed`, with nothing to show
- Location: specs/05-cli.md CLI-3; specs/01-model.md MOD-7
- Status: parked. One idea: `checks 300 of 400 read, 0 failed among them (truncated)`, and `show` adds the PR's checks page.

### H2-17 · low · `--json` doesn't say where an error goes
- Location: specs/05-cli.md CLI-4, CLI-5
- Status: parked. One idea: on failure stdout is empty and the line goes to stderr.

### H2-18 · low · API-5's warnings never reach normal output
- Location: specs/05-cli.md CLI-3; specs/04-api.md API-4, API-5; specs/03-state.md STA-8
- Status: parked. A set-aside or newer `state.json` empties the manual set unnoticed. One idea: a `warnings: 2, see circitor status` line.

### H2-19 · low · A socket removed by hand gives exit 6 for up to a minute, with no hint
- Location: specs/05-cli.md CLI-7; specs/03-state.md STA-11
- Status: parked

### H2-20 · low · `untrack` on an authored or asked PR does nothing, silently
- Location: specs/05-cli.md CLI-1; specs/04-api.md API-15
- Status: parked. The output of `track` and `untrack` isn't specified at all.

### H2-21 · low · Config edits take effect only after a restart, and the CLI never says so
- Location: specs/05-cli.md CLI-1, CLI-8; specs/06-operations.md OPS-1
- Status: parked

### Out of scope
- specs/07-testing.md TST-9: "a waiting CLI reconnects" should now say it starts no daemon after a `stop` (H2-2); TST-7 should cover the new outcome lines (H2-1), the no-body wait (H2-6) and `more events:` (H2-9).
- specs/02-polling.md POL-15: one PR can be force-refreshed every 2 s, up to 1 800 points an hour from one client (H2-10 only fixes the guide).

## Pass 3: implementability (2026-11-25)

Reviewers: 3; 26 raw findings; dropped in verification: 0 as wrong, 12 merged as duplicates (the `again:`/`--since` bug, the no-answer endings, the version format, `stop`'s completion and the text output were each found two or three times). Two overlap findings parked in other logs and were folded in: the fork-PR branch match (01-model H1-18) and the version string (06-operations H1-18).

Another session edited 04-api, 05-cli, 06-operations during this pass. It split the re-run guard as `rerun_since` / `--rerun-since` before H3-1 was applied, added CLI-6's under-1-s rule and CLI-8's `SIGTERM` for a daemon that doesn't answer. Those were kept and the fixes below merged onto them.

### H3-1 · high · The `again:` line turns on `checks-done`'s re-run guard, so a repeated wait can never end
- Location: specs/05-cli.md CLI-1, CLI-6, CLI-11; specs/04-api.md API-10; specs/01-model.md MOD-14
- Status: applied with change: the other session's split, named `rerun_since` / `--rerun-since` rather than the proposed `after`; `since` is only where events start

### H3-2 · high · `circitor show #12` runs as `circitor show`: a shell reads `#12` as a comment
- Location: specs/05-cli.md CLI-2
- Status: applied: `#12` dropped; CLI-2 says why

### H3-3 · medium · A dropped wait should run its own `again:` line; the no-answer endings had no exit codes
- Location: specs/05-cli.md CLI-6, CLI-10; specs/03-state.md STA-9
- Status: applied: CLI-10 runs `again:` (no spawn once `Circitor-Cursor` arrived), so a replaced wait of any condition gets `reset: true`; CLI-6 lists three endings with codes 3, 6, 6; `again:` names the PR in full

### H3-4 · medium · "The seconds left" had no rounding rule, and the CLI couldn't know the default timeout
- Location: specs/05-cli.md CLI-6; specs/04-api.md API-9, API-12
- Status: applied: 100 s default in CLI-6; rounded down; `--timeout 0` limit 60 s (the under-1-s rule was the other session's)

### H3-5 · medium · Check the daemon's version before every command, and pin down the version string
- Location: specs/05-cli.md CLI-9; specs/06-operations.md OPS-11; specs/04-api.md API-2
- Status: applied: every command reads `GET /v1/status` first, no "repeats the request"; OPS-11 gives `MAJOR.MINOR.PATCH+<commit>`; build metadata ignored; an unparseable version is never replaced. Closes 06-operations H1-18.

### H3-6 · medium · `stop` and a replacement didn't say when the old daemon counts as gone
- Location: specs/05-cli.md CLI-8, CLI-9; specs/06-operations.md OPS-7
- Status: applied: both wait up to 8 s for the lock to be released; `stop` then falls through to the `SIGTERM` rule, a replacement keeps the old daemon with a warning

### H3-7 · medium · The per-command limits misclassified `show` of an untracked PR
- Location: specs/05-cli.md CLI-7
- Status: applied: a `wait`, its deadline + 10 s (`--timeout 0`, 60 s); any other request 90 s

### H3-8 · medium · "Connecting" was defined three ways, so `stop` could miss a daemon on another socket path
- Location: specs/05-cli.md CLI-7, CLI-8, CLI-10; specs/06-operations.md OPS-4
- Status: applied: one Connecting rule in CLI-7 (own path, then the lock file's, warning); CLI-8, CLI-10 and OPS-4 cite it

### H3-9 · medium · The test harness couldn't drive the CLI's clock, working directory, spawner or version
- Location: specs/05-cli.md CLI-7, new CLI-13; specs/07-testing.md TST-5, TST-8, TST-9
- Status: applied: CLI-13 makes the inputs injectable and names the clock for each duration; TST-5 gains the CLI's inputs, a git repository and another daemon version; TST-8 `bare-pr`; three TST-9 checks; CLI-7's log lines bounded to 20 the new daemon wrote

### H3-10 · medium · The bare form compared the local branch name, had no rule for a detached HEAD, and could pick a contributor's fork PR
- Location: specs/05-cli.md CLI-2; specs/01-model.md MOD-2
- Status: applied: upstream branch name; detached HEAD exit 2; only PRs whose `author` is the login in use; exit 2 scoped to after the poll. Closes the CLI half of 01-model H1-18.

### H3-11 · medium · Most of the text output was left to the code
- Location: specs/05-cli.md CLI-1, CLI-3; specs/07-testing.md TST-7
- Status: applied: one line per event kind (the `review` line now prints only the new value, as MOD-15 carries); durations; title quoting; the order after the events; `refresh`, `stop` and `status` lines; the `--json` command list. The layout stays (proposal).

### H3-12 · low · The same rules were stated twice
- Location: specs/05-cli.md CLI-6, CLI-7, CLI-8, CLI-9; specs/06-operations.md OPS-7
- Status: applied: folded into one place each; OPS-7 cites CLI-10 without the value

### H3-13 · low · `list`'s 50-PR cap guards nothing and hides PRs
- Location: specs/05-cli.md CLI-3
- Status: parked. `list --since` and a reset already print up to 200 lines; PRs past 50 have no text-mode command. One idea: drop the cap, or cut titles instead.

### H3-14 · low · The guide and its two shipped copies have no test
- Location: specs/05-cli.md CLI-11, CLI-12; specs/06-operations.md OPS-12
- Status: parked. One idea: a TST that `circitor guide` equals both copies (the skill after its front matter), is under 60 lines, names every CLI-5 code; CLI-12 checked with stdin closed and stdout a pipe.

### Out of scope
- specs/04-api.md API-9 and API-12: `timeout=0` "waits for a refresh that is sent", while API-12 says "Nothing waits past the deadline". CLI-6 now gives `--timeout 0` a 60 s limit; the API should say which rule wins.
- specs/07-testing.md TST-7 still lists only `list`, `show`, `changes`, `wait` and `status` for golden files; `refresh` and `stop` now have fixed text (H3-11).
