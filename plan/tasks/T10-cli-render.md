---
id: T10
title: "CLI text output: every line of list, show, changes, wait, status, refresh and stop, with golden files"
kind: code
milestone: M3
size: L
status: done
depends_on: [T02, T41]
requirements: [CLI-3, CLI-6:the-outcome-line-says-what-the-wait-found, CLI-6:with-sha, CLI-6:a-wait-answered-before-its, CLI-6:a-truncated-failing-adds, CLI-6:on-a-timeout-it-prints, CLI-6:a-wait-that-ends-without, CLI-9:a-bad-request-or-unknown-route-from, CLI-5, TST-7, NTF-11:when-the-answer-is-already-known, NTF-11:otherwise-it-prints, NTF-11:a-registered-answer-with-reset, NTF-11:when-already]
owns: [src/cli/render/, tests/golden/render/]
shares: []
---

# T10 · CLI text output: every line of list, show, changes, wait, status, refresh and stop, with golden files

## Outcome
`cli::render` turns each API answer into CLI-3's compact text and each wait's end into CLI-6's outcome, `again:` and `cursor:` lines, and maps answers and errors to CLI-5's exit codes. Pure functions, fixed line by line by golden files. **The outcome line, the condition's flags and the durations are public on their own:** the notices a session is sent are built from them (NTF-8).

## Context
T02 gives the API's types and the model's types; T41 gives the golden answers in `testdata/api/`. Nothing else is needed: no clock, no socket, no daemon. T27 calls every function here and decides what goes to stdout, to stderr and into the exit status; T48 does the same for `notify`, with `already` and `registered`; T46's `notify::text` calls `outcome_line`, `flags`, `duration` and `stale_mark`. T35 fixes the same text end to end. **This text is the CLI's contract with agents** (CLI-3).

## Read first
- `plan/conventions.md`
- `plan/contracts/cli.md`: the block "`src/cli/render/` (T10)" (the signatures you build) and "Semantics"; `plan/contracts/api.md`: the envelope, the body types, the list of golden files; `plan/contracts/model.md`: `Record`, `Checks`, `Event`, `EventKind`, `Item`, `short_sha`
- `specs/05-cli.md#output`: CLI-3 (all 17 parts) and CLI-5; `specs/05-cli.md#waiting`: CLI-6; `specs/05-cli.md#the-daemon`: CLI-9's last bullet (the older-daemon hint)
- `specs/01-model.md`: MOD-12 (`fetch`, `fetch_inline`), MOD-15 (what each kind carries), MOD-16 and MOD-2's `error` (the stale mark)
- `specs/04-api.md#basics`: API-3, API-4 (`problem`, `message`, `until`), API-5 (the status keys, in order); `specs/04-api.md#waiting`: API-10 (`fail_fast`, `kind`), API-11
- `specs/08-notify.md`: NTF-8 (how a notice uses the outcome line and the stale mark) and NTF-11's parts "when the answer is already known", "otherwise it prints", "a registered answer with reset" and "when `already:` comes with `reset: true`"
- `specs/07-testing.md#what-is-tested`: TST-7
- `plan/contracts/testkit.md`: `golden::check`
- `testdata/api/*.json` (T41's): your inputs

## Scope
- **The record line** (CLI-3's first example): one private function; `list`, `show`, a reset's records and `tracked` all use it. Fields are separated by two spaces, as the example shows. `checks 3/5` is `completed/total`. The stale mark (`CLI-3:a-stale-record-is-marked`) goes last on the line; its age is `ctx.now` minus `fetched_at`. **`stale_mark(record, now)`** is that mark as a public function, with its leading space, "" when `stale` is false: the record line calls it, and so does `notify::text::expired` (T46, NTF-8).
- **Durations** (`CLI-3:durations-are-the-largest-unit-and-the-next`): interpreted as the largest unit and the next, **a zero second unit left out**: `45s`, `4m` (240 s), `4m30s`, `1h3m`, `1h`, `2d5h`. Units are `d`, `h`, `m`, `s`; 0 is `0s`. It's the public `render::duration`: a subscription's life is printed with it (`1d`, NTF-7).
- **Titles** (`CLI-3:a-title-is-in-double-quotes`): control characters (`char::is_control`) as `\u{..}` in lower-case hex, at least two digits (`\u{0a}`). Never cut.
- **`show`** (`CLI-3:show-adds-one-line-per`): the record line; one line per failed or unfinished check with its URL; passed checks as a number; `mergeable`, `merge_state`, the counts; the `mergeable unknown: …` hint with the PR's full name.
- **Events** (`CLI-3:changes-and-the-end-of`, `CLI-3:a-new-comment-or-review`, `CLI-3:more`): one line per kind of an event, in `kinds`' order, and one per item. The twelve examples are exact. `untracked` prints like `merged`, without its source. A review with `inline` 0 has no `, N inline` and no second command. An item with no author prints `ghost`. `more: true` prints `more new items than shown` with `gh api repos/{o}/{r}/issues/{n}/comments` and `gh api repos/{o}/{r}/pulls/{n}/reviews`. **An `activity` kind with no items and no `more` prints no line** (CLI-3 gives none).
- **The tail** (`CLI-3:after-the-events`): pending, untracked, the outcome, `more events:`, `again:`, `problem:`, `cursor:`. `CLI-3:list-since-adds-a-line`, `CLI-3:list-and-changes-add-a`, `CLI-3:the-last-line-of-list`, `CLI-3:changes-and-wait-print-at-most-50-events` (you print what the answer holds, and the `more events:` line on `more_events`; T27 sends `limit=50`), `CLI-3:a-reset-is-never-silent`, `CLI-3:a-daemon-problem-is-never-silent` (the relative time from `ctx.now`, the local time from `ctx.tz`; without `until`, `problem: <code>, <message>`). The `problem:` line also ends `show`, `tracked`, `refresh` and `status`, which have no `cursor:` line.
- **`list`** (`CLI-3:list-without-since-prints-at-most-50-prs`): `with_since` false prints 50 records at most (`LIST_MAX`), then one line with the number left out; true prints all.
- **`wait`** (CLI-6, your six parts): the outcome line (`CLI-6:the-outcome-line-says-what-the-wait-found`), from `outcome`, `reason`, the record and `ask.cond.until`; `satisfied` for `checks-done` reads `passed`, `failed` or `no checks reported` by `checks.state`, **and `checks failing (1 failed, 3 of 5 done) on <sha>` while `checks.state` is still `pending`**: that's `--fail-fast`'s answer (API-10, D8), told by the record alone. The stale mark and `CLI-6:a-truncated-failing-adds` go last. The timeout line (`CLI-6:on-a-timeout-it-prints`), with the head pair of `CLI-6:with-sha` when `ask.cond.sha` is given and `head_sha` doesn't start with it (without case), and `no record yet` for `CLI-6:a-wait-answered-before-its`. A SHA prints as its first 7 characters. `<n>` is `ask.timeout`.
- **`outcome_line`** is that line as a public function, **without the stale mark and without a newline**, with the truncated mark: `wait` adds the stale mark to it, `already` puts `already: ` before it, and `notify::text::outcome` (T46) puts it in a notice (NTF-8). One function, three users.
- **`flags`**: a condition as the flags that ask for it: `--until <cond>`, then `--sha`, `--rerun-since`, `--fail-fast` and `--kind`, each only if it was given and as it was (CLI-6). Never `--since`.
- **`again:`**: `circitor wait <pr in full> `, then `flags(&ask.cond)`, then `--timeout <n>` only when `ask.timeout_given`, then `--json` only when `ask.json`, then `--since <the answer's cursor>` (CLI-6: "repeats the other parameters given"). Only a timeout and the three endings below print it.
- **`already`** (NTF-11's parts "when the answer is already known" and "when `already:` comes with `reset: true`", your share: the text; T48 calls it): what `wait` prints for `e.body.as_wait()`, in NTF-11's order: a reset and its records, the events, `already: ` and the outcome line, `replaced an earlier notify` when `replaced` (an answer already known still replaces, NTF-2), `more events:`, `again:`, `problem:`, `cursor:`. Never a timeout. **With `reset: true` the `again:` line is `again: circitor notify <pr> ` with `flags(cond)`, ` --for <life>` when given, and ` --since <the envelope's cursor>`**: nothing since the reset is known to match.
- **`registered`** (NTF-11's parts "otherwise it prints" and "a registered answer with reset"; T48 calls it with its first line): a reset and its records, then the events, as `wait` prints them; then the `first` line it's given; the record line; `a confirmation was posted to this session; if none arrives, notices are held or refused here: run circitor notify <pr> --cancel, then use circitor wait`, word for word; `replaced an earlier notify` when `replaced`; `more events:`; `problem:`; `cursor:` (NTF-11's order, which follows CLI-3's).
- **`wait_no_answer`** (`CLI-6:a-wait-that-ends-without`): the three first lines exactly as CLI-6 writes them, then `again:` and `cursor:` with the cursor given. With `cursor` `None` (no `--since`, and the daemon never sent one): `again:` without `--since`, and no `cursor:` line.
- **`refresh`, `stop`, `tracked`** (`CLI-3:refresh-prints-refresh`): `1 point`, `2 points`. A problem outcome prints `refresh: <outcome>, <n> points`. `tracked` prints the record line, or the `pr` it's given and `not tracked` for `record: null`.
- **`status`**: `version`, then one `key: value` line per key of `StatusBody`, in its order (API-5; `subscriptions` after `waiters`). `github`, `tracked`, `paths` and `warnings` are one line each; `null` prints `none`; times print as RFC 3339. A last line `cli_version: <v>` when `ctx.cli_version` differs from the answer's `version`.
- **`api_error`** (CLI-5, `CLI-5:errors-go-to-stderr-as-one-line`, `CLI-5:on-code-9`): one line, `<code>: <message>`. For the three codes that exit 9 it ends with when to try again: `until` as in the `problem:` line, or "try again in a minute". `older_daemon` adds CLI-9's hint (`CLI-9:a-bad-request-or-unknown-route-from`, your share: the line and the code 1; T24 gives the flag, T27 passes it on): the daemon is older than this CLI, and `circitor stop` or a service restart fixes it.
- **Exit codes** (CLI-5, `CLI-5:each-api-error`, `CLI-5:code-8-exists-so-circitor`): `wait_code`, `error_code`, `refresh_code` and `no_answer_code` are tables. `satisfied` is 8 only for `checks-done` with `checks.state` `failing`, or still `pending` with `checks.failed` above 0 (`--fail-fast`: one has failed and others may still run); `timeout` 3; `ended` 4. `bad_request` 2, `not_found` 5, `unknown_route` 1, `auth` 7, `starting`, `github_unavailable` and `rate_limited` 9, `limit` 10, `no_session` 11; with `older_daemon`, `bad_request` and `unknown_route` are 1. `refresh_code` is 0 for `polled` and `shared`, and otherwise `error_code` of the error of that name (API-16): 7, 9, 9. `no_answer_code` is 3, 6, 6, in `NoAnswer`'s order. **Every table is yours.** T27's share of CLI-5 is calling them, and codes 1, 2, 130 and 143; exit 6 with its lines is T24's `ConnError`; exit 11 without a session, its line, and that `notify` never exits 3, 4 or 8 are T48's.
- **TST-7's share:** the text, as golden files in `tests/golden/render/`. The end-to-end files, `--json` and each exit code through a running daemon are T35's; the notices' own words T46's; `notify`'s own lines T48's.
- `CLI-3:the-exact-text-is-fixed-by-golden-files`: where CLI-3 gives no wording (the "more PRs" line, `show`'s extra lines, `status`' nested values), choose it, keep it compact, and list each choice in your report.

## Interfaces
- **Consumes:** `api::types` (`Envelope`, the body types, `NotifyBody` and its `as_wait`, `Ask`, `ApiError`, `ErrorCode`, `Outcome`, `Reason`, `Until`, `DaemonBlock`), `poller::PollOutcome`, `model` (`Record`, `Event`, `EventKind`, `Item`, `PrName`, `short_sha`), `clock::Time`, `cli::exit`, `testkit::golden::check` (tests).
- **Provides:** every function and type under "`src/cli/render/` (T10)" in `plan/contracts/cli.md`, with those signatures.

## Out of scope
- Which stream a text goes to, `--json`, calling these functions: T27. The `notify` command, its `registered:` lines and exit code 11: T48. A notice's words around the outcome line: T46.
- The lines of a connection that fails (`ConnError.stderr`): T24. Usage errors and the guide: T11.
- End-to-end golden files, the `<tmp>` and `<os error>` replacements: T35. Codes 130 and 143: T36.
- The types: T02's. `testdata/api/`: T41's. A golden answer that contradicts 04-api is an open item, not an edit.

## Acceptance
Unit tests in `src/cli/render/`. Each text is compared with `tests/golden/render/<name>.txt` through `testkit::golden::check`, which rewrites the file under `CIRCITOR_UPDATE_GOLDEN=1`; don't write a helper of your own. Inputs are `testdata/api/*.json` and your own under `tests/golden/render/in/`. `ctx` is fixed: `now` 2026-11-24T22:00:00Z, `tz` UTC, and once +01:00 for the local time.
- `cli3_list_line`: the record line reads exactly `yachiko/circitor#12  checks_pending  1a2b3c4  checks 3/5, 0 failed  review none  "Add the poller"`.
- `cli3_event_lines`: one input with every kind gives CLI-3's twelve example lines byte for byte, from the example's values (heads `1a2b3c4` and `9f8e7d6`, check `build`, review 604123 by `octocat` with 3 inline, comment 918273), plus `closed`, `reopened`, `gone` and `untracked`.
- `cli3_durations`: 0, 45, 59, 60, 61, 240, 270, 3 599, 3 600, 3 780, 86 399, 86 400 and 190 800 s give `0s`, `45s`, `59s`, `1m`, `1m1s`, `4m`, `4m30s`, `59m59s`, `1h`, `1h3m`, `23h59m`, `1d`, `2d5h`.
- `cli3_title_escapes`: `a "b" \ c`, a tab and an ESC come out escaped; a 300-character title isn't cut.
- `cli3_more_events_names_the_pr`: `more_events: true` prints `more events: circitor changes yachiko/circitor#12 --since <cursor>` from `wait` and from `changes` with a PR, and `more events: circitor changes --since <cursor>` from `changes` with `None`.
- `cli3_tail_order`: a `changes` answer with pending, `more_events`, a problem and a cursor prints them in CLI-3's order; `list --since` adds `untracked <pr>`; the last line is `cursor: <cursor>`.
- `cli3_list_50`: 51 records without `--since` print 50 and "1 more"; with it, 51.
- `cli3_reset`: `reset: cursor too old, current state follows` comes first, then one record line each, then the events.
- `cli3_stale`: `(stale, 4m old)` and `(stale, 4m old, error FORBIDDEN)`.
- `cli3_problem_line`: `rate_limited` with `until` 12 minutes ahead and `tz` +01:00 starts `problem: rate_limited for 12m (until 23:12 local), `; `auth` has no time.
- `cli6_outcome_lines`: each of `checks passed (5/5) on 1a2b3c4`, `checks failed (1 of 5) on 1a2b3c4`, `checks failing (1 failed, 3 of 5 done) on 1a2b3c4`, `no checks reported on 1a2b3c4`, `merged`, `changed`, `ended: closed`, `ended: gone`, `ended: merged`, `ended: head moved to 9f8e7d6, expected 1a2b3c4`; one with the stale mark; a truncated `failing` with `(truncated: unread checks may still run)`.
- `cli6_outcome_line_alone`: `outcome_line` gives each of those without a newline; for a stale record it has no stale mark while `wait`'s line has; a truncated one keeps its mark; `Timeout` gives an empty string.
- `cli6_flags`: `--until merged` alone; `--until checks-done --sha 1A2b3c4 --rerun-since k3x9q2mz-400 --fail-fast` with each as given; `--until changed --kind activity,merged`; never `--since`, though the `Ask` holds one.
- `ntf11_already`: `notify-already.json` prints its event, `already: checks failed (1 of 1) on <sha7>` and `cursor: <cursor>`, with no `again:` line. With `reset: true` set in the test: the reset line and its record first, and `again: circitor notify <pr> --until checks-done --since <cursor>` before `cursor:` (and before a `problem:` line, with `daemon.problem` set); with `life` 3600, `--for 3600` before `--since`. With `replaced: true`: `replaced an earlier notify` straight after the `already:` line, before `more events:` and `again:`.
- `ntf11_registered`: `notify-registered.json` with a `first` line given prints NTF-11's four lines byte for byte for NTF-11's values; with `replaced: true`, `replaced an earlier notify` before `cursor:`; with `reset: true` or events, those first; with `daemon.problem` set, its `problem:` line before `cursor:`.
- `cli3_stale_mark_alone`: ` (stale, 4m old)`, ` (stale, 4m old, error FORBIDDEN)`, and "" for a record that isn't stale.
- `cli6_timeout_block`: the three lines of CLI-6's example, exactly, for `--until checks-done --sha 1a2b3c4` and cursor `k3x9q2mz-418`; with `timeout_given`, `--timeout 300` is added, and so is a given `--timeout 100`; without it, none; with `json`, `--json`; `--rerun-since`, `--fail-fast` and `--kind` are repeated as given.
- `cli6_timeout_other_head`: `timeout after 100s: yachiko/circitor#12 head 9f8e7d6, not 1a2b3c4 (stale, 2m old)`. `cli6_no_record`: `timeout after 100s: yachiko/circitor#12 no record yet`.
- `cli6_no_answer`: `timeout after 100s: yachiko/circitor#12 (no answer from the daemon)`, `daemon stopped`, `daemon not answering on /tmp/c/circitor.sock`, each with `again:` and `cursor:`; the `None` cursor case.
- `cli3_show`, `cli3_status` (`status.json`, `status-problem.json`, a differing CLI version), `cli3_refresh_stop_tracked`: golden files; `untrack-null.json` prints the PR's name and `not tracked`.
- `cli5_error_lines`: one golden line per `error-<code>.json`, `no_session` among them (code 11), and the two older-daemon lines. `cli5_wait_code` (a `satisfied` `checks-done` whose record is `pending` with one failed is 8, with none failed 0), `cli5_error_code`, `cli5_refresh_code` (`polled` 0, `shared` 0, `auth` 7, `github_unavailable` 9, `rate_limited` 9) and `cli5_no_answer_code` (3, 6, 6): every row, by table.
- `cargo test --features test-endpoint --lib cli::render::` and `make verify` pass.

## Traps
- **Pure means pure:** no clock, no environment, no file, no terminal check. Time is `ctx.now`; nothing is ever coloured (CLI-12).
- The spec's `checks failing` example ends in `https://github.com/…`: the ellipsis is the spec's. Print the whole URL, and use a whole one in the input.
- `more` is about one event's items; `more_events` is about the answer (API-8). They print different lines.
- `Mergeable::Other`, `Review::Other`, `merge_state` and a check's `result` hold values circitor doesn't know: print them as they are.
- A name prints through `PrName`'s `Display`. Every line ends in `\n`; no trailing spaces, no blank lines.
- After a golden rewrite, read each file against the spec. A file that only mirrors the code proves nothing.
- `ask.cond.sha` may be 7 to 40 characters and in any case: compare without case, print 7.
- **`outcome_line` takes no `fail_fast`:** a `satisfied` `checks-done` whose checks are still `pending` can only be `--fail-fast`'s. Don't add the flag to its signature.

## Open questions
- CLI-3's layout, the 50 and CLI-5's code 8 are (proposal): build them as written, the numbers as constants.
- **`more events:` names the PR after an answer about one PR** (CLI-3, D9): `wait`, `already` and `registered` take it from their PR, `changes` from its `pr` argument; `changes` over every PR (`None`) prints it without one.
- Not said by the specs, built as Scope says: an `activity` with no items, `refresh` with a problem outcome, `again:` without a cursor, and that `status` prints no `tier` line (it isn't a key of API-5).

## Evidence
- **2026-11-26**, branch `t10-cli-render`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/cli/render/` (`mod.rs`, new `parts.rs` and `tests.rs`) and 33 golden files in `tests/golden/render/`. `duration`, `stale_mark` and `LIST_MAX` moved to `parts.rs` and are re-exported from `cli::render` with the contract's signatures. Every test the Acceptance names exists.
- **Golden files read against the spec by the orchestrator:** CLI-3's twelve event lines (`changes-every-kind.txt`, with the whole URL for the spec's `…`), CLI-3's record line, CLI-6's timeout block and NTF-11's four registered lines are byte for byte the spec's examples.
- `make verify` passes in the worktree after rebasing and on main after the merge (150 lib tests, 9 golden), the macOS check included.
- Wording chosen where CLI-3 gives none (fixed by the golden files): `N more PR(s) (all are in circitor list --json)`; `show`'s `check <result|running|queued>: <name> <url>`, `passed checks: N`, `mergeable X, merge_state Y`, `counts: comments N, reviews N, review_threads N`; `checks passing (5/5)` and `checks none` events; `status`'s nested values on one line each (`last_poll`, `github`, `tracked`, `paths`, `warnings`); exit-9 errors end `(try again in 12m, until 23:12 local)` or `(try again in a minute)`; the older-daemon hint `(the daemon is older than this CLI: run circitor stop, or restart its service, to replace it)`; `expected <sha>` lower-cased; the truncated mark on `checks failed`/`checks failing` only; `checks done` for a `checks-done` with no record; `pending` lines before `untracked`.
