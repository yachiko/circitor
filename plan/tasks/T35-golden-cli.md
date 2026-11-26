---
id: T35
title: "Golden output end to end: every command's text and JSON, every exit code"
kind: test
milestone: M5
size: L
status: done
depends_on: [T29, T42, T47, T48]
requirements: [TST-7]
owns: [tests/golden_cli.rs, tests/golden/cli/]
shares: []
---

# T35 · Golden output end to end: every command's text and JSON, every exit code

## Outcome
Every text output, outcome line, error line and `--json` body the CLI prints against a real daemon, and every notice the daemon posts to a session, equals a golden file, and each exit code of CLI-5 but 130 and 143 has a test (TST-7).

## Context
The CLI's text and exit codes are circitor's contract with agents (CLI-3, CLI-5). T10 fixed every line's format as pure functions over hand-written API answers (`tests/golden/render/`); T46 did the same for the notices' words (`tests/golden/notify/`) and T48 for `notify`'s own lines (`tests/golden/notify_cmd/`). This task fixes **what the real CLI prints against the real daemon**: the same lines, produced end to end on T29's harness from a scripted GitHub, so a field the daemon fills differently from T10's inputs shows as a diff. It also gives each exit code a test.

## Read first
- `plan/conventions.md`: Tests (golden files, `CIRCITOR_UPDATE_GOLDEN=1`), Requirement tracing, Report.
- `plan/contracts/testkit.md` (the harness, `cli` and its variants, `api_raw`, `session()` and the fake session socket's `notices()`, `golden::check`, `TEST_VERSION`), `plan/contracts/notify.md` (`text`: which notices there are), `plan/contracts/cli.md` (`Output`, `exit`, the render functions' names, `FakeSpawner`), `plan/contracts/api.md` (the bodies, the golden files of `testdata/api/`), `plan/contracts/clock.md` (`advance`).
- `tests/golden/render/` and T10's test beside it: the lines already fixed, and how its golden helper compares and rewrites. `tests/m3_cli.rs`: each command once on the harness.
- `specs/07-testing.md#what-is-tested`: TST-7. `specs/05-cli.md#output`: CLI-3, CLI-4, CLI-5. `#waiting`: CLI-6. `#the-daemon`: CLI-7 to CLI-10 (the lines of exit code 6). `specs/08-notify.md`: NTF-7, NTF-8 (the notices), NTF-11 (the command's lines).
- `specs/04-api.md`: API-1, API-3, API-12, API-19 (the keys every body holds).
- `docs/03-clients.md` §2: the 30 000 characters behind CLI-3's 50 events.

## Scope
`tests/golden_cli.rs` and the files under `tests/golden/cli/`.

- **TST-7 is shared with T10, T46 and T48.** T10's share: every line's format, each duration unit, title escaping, from golden API inputs; T46's the notices' words; T48's `notify`'s lines from golden answers. **This task's share:** each command's output, each outcome line of CLI-6, each notice as a session gets it, `more events:`, each error line, `--json` bodies and each exit code, **produced by the real CLI, daemon and fake GitHub**. One case per kind of line; don't repeat T10's format variants.
- **The golden helper** of this file: runs a command through the harness, replaces the temporary root by `<tmp>` and an OS error's text by `<os error>` (TST-7 names these two and no other), then hands stdout and stderr to `testkit::golden::check` with `tests/golden/cli/<case>.out` and `<case>.err`; that function compares, or writes with `CIRCITOR_UPDATE_GOLDEN=1`. The exit code is asserted in the test, not in a file.
- **Nothing else may vary.** The clock starts at 2026-11-24T22:00:00Z, the seed is fixed (so the cursor's epoch is), the time zone is UTC, and the daemon's and the CLI's versions are the harness's `testkit::TEST_VERSION` (`1.2.3+test`), never the build's commit. Each restart has its own seed, so a case that restarts has two fixed epochs.
- **Text** (CLI-3, CLI-6), one case each:
  - `list`: several PRs in API-6's order with different `status` words; `list --all`; `list --since` with an `untracked` and a `pending` line; 51 PRs (the "how many more" line); a stale record; a `problem:` line.
  - `show`: a PR with a failed and a running check (their URLs), passed ones as a number; `mergeable unknown` with its hint; `(stale, …, error FORBIDDEN)`.
  - `changes`: a run that makes each event kind the fake can cause (`status`, `head`, `checks`, `merged`, `closed`, `reopened`, `gone`, `draft`, `mergeable`, `review`, a new comment, a new review with inline comments, `edited`, `tracked`, `untracked`); a reset (`reset: cursor too old, current state follows`); over 50 events (`more events: circitor changes --since <cursor>`).
  - `wait`, every outcome line: `checks passed (n/n) on <sha>`, `checks failed (1 of n) on <sha>`, `no checks reported on <sha>`, `merged`, `changed`, `ended: closed`, `ended: gone`, `ended: merged`, `ended: head moved to …, expected …`; a timeout with `again:` and `cursor:`; a timeout with `--sha` while the head isn't it; `<pr> no record yet`; a truncated `failing`; and the three endings without an answer (`timeout after <n>s: <pr> (no answer from the daemon)`, `daemon stopped`, `daemon not answering on <path>`).
  - `wait` with the narrower conditions (D8): `--fail-fast` answering `checks failing (1 failed, 1 of 2 done) on <sha>`, and its timeout's `again:` line repeating `--fail-fast`; `--kind activity` ended by a merge (`ended: merged`), and its `again:` repeating `--kind activity`.
  - `notify` (NTF-11): the registered block (`registered: … for 1d`, the confirmation line, `cursor:`); with `--for 3600` (`for 1h`); registered again (`replaced an earlier notify`); `--until changed --kind review --since <c> --for 7200` (NTF-7's second example's words in `registered:`); `already:` with a failed check, and the same after a registration it replaces (`replaced an earlier notify` straight after `already:`, NTF-11's order); `--cancel` (`cancelled: 1`, `cancelled: 0`).
  - `status`: healthy; with a problem, an `until` and two warnings; with the CLI's version differing; with `subscriptions: 2`. `refresh`: `polled` and `shared`. `stop`: `stopped` and `not running`. `track`, `untrack`.
- **Notices** (NTF-7, NTF-8), from `h.session().notices()`, each text compared with `tests/golden/cli/<case>.notice` through `testkit::golden::check`: the confirmation; an outcome for passed checks, for failed ones, for `--fail-fast`'s `checks failing`, for `merged`, and for `ended: closed`; `expired`; `stopped` (ending `--no-spawn`); and the `History too old` form, after over 500 events. **Each must equal the file T46 wrote for the same case** in `tests/golden/notify/`, but for the values. Assert beside each that its auth line carried the session's token.
- **Error lines** (CLI-5, API-3), stderr: one per API code the real daemon can give (`bad_request`, `not_found`, `auth`, `starting`, `github_unavailable` and `rate_limited` each with their `until`, `limit`, and `no_session` with the `wait` it names), and the CLI's own: `notify` with no session socket in its environment; a usage error, a PR that can't be named, no daemon with `--no-spawn`, a spawned daemon that exits 1 with an OPS-1 message, one that never binds.
- **Exit codes** (CLI-5): one test per code 0 to 11, named `cli5_exit_<n>_<what>`; where a code has several causes, one test per cause the CLI-5 table names: for 4 also `merged` under `--kind`, for 8 also `--fail-fast`, for 10 also a 65th subscription, for 11 both no socket in the environment and `no_session`. And `cli5_notify_exits_0_whatever_the_checks_did`: `notify` on failed checks exits 0. **Code 1 and `unknown_route`** can't come from the real daemon: serve `testdata/api/error-unknown_route.json` and a `foreground` status with an older version from a stub on a socket in the test, and run the CLI against it. 130 and 143 are T36's (TST-9).
- **`--json`** (CLI-4, API-19): for `list`, `show`, `wait`, `notify` (registered, already known and cancelled), `changes`, `track`, `untrack`, `refresh` and `status`, stdout is one line, equal to a golden `.json`, and equal byte for byte to the body the daemon sent. One case per route with nothing to report: every key of API-19's row is there, lists `[]`, flags `false`, `record` `null` where it may be.
- **`bad_request`** (API-1, API-12): the CLI can't send these, so use `h.api_raw`: `GET /v1/prs?bogus=1` and `GET /v1/prs/octo/app/1/wait?until=merged&timeout=3601` each answer 400, `bad_request`, the message naming the parameter. And `wait --timeout 3601` through the CLI exits 2 without a request.

## Interfaces
- **Consumes:** `testkit::harness::Harness` (`start(opts)`, `cli`, `cli_bg`, `api_raw`, `session()`, the version, working-directory, environment and spawner variants, `no_daemon`, `paths()`, `stop`, `restart`, `finish`), `testkit::session::FakeSession` (`notices()`), `testkit::golden::check`, `testkit::TEST_VERSION`, `testkit::server::FakeGitHub` (`world`, faults, `set_rate`), `cli::Output`, `cli::exit`.
- **Provides:** `tests/golden/cli/`: the CLI's output as an interface. A changed file is a contract change (`plan/contracts/cli.md`).

## Out of scope
- The render functions and `tests/golden/render/`: T10. `tests/golden/notify/` and `tests/golden/notify_cmd/`: T46, T48. A line that's wrong is T10's, T46's or T48's (format) or T22's, T23's, T27's, T47's (content): report it, don't edit their files.
- The behaviour behind each line (when a wait ends, what's stale, when a notice is posted): T30 to T33, T49. Here the worlds are the smallest that make the line.
- Exit codes 130 and 143, the real spawner, a daemon that doesn't answer for real: T36.
- `circitor guide`, `help` and `version` text: T11, T27.

## Acceptance
- `tests/golden_cli.rs` starts with `#![cfg(feature = "test-endpoint")]`; every test carries `// TST-7:` and the CLI or API ID of what it prints, and ends with `h.finish()`.
- `tests/golden/cli/` holds one `.out` (and `.err` where stderr isn't empty) per case of the Scope, one `.json` per `--json` case, and one `.notice` per notice. **Each file was read line by line against CLI-3 and CLI-6** after it was written; the report says so and names any line that looked wrong.
- Every outcome line of CLI-6, `more events:`, `reset:`, `again:`, `problem:` and `cursor:` appears in at least one golden file: `grep -rl 'more events:' tests/golden/cli` and the like find them.
- Codes 0 to 11 each have a passing test; `grep -c 'fn cli5_exit_' tests/golden_cli.rs` is at least 12.
- Two runs in a row produce no diff: `CIRCITOR_UPDATE_GOLDEN=1 cargo test --features test-endpoint --test golden_cli`, then `git status --short tests/golden/cli` shows only what the first run added.
- No golden file holds a path under `/tmp`, `MARKER_TOKEN`, `MARKER_SESSION_TOKEN` or a commit hash. No `.notice` file holds `MARKER-TITLE`, `MARKER-CHECK` or `marker-login`: the fake marks every check name and login it makes up (TST-11), so leave them to it.
- `cargo test --features test-endpoint --test golden_cli` and `make verify` pass.

## Traps
- **Never edit a golden file by hand**, and never accept one unread: `CIRCITOR_UPDATE_GOLDEN=1` writes whatever the code prints, bugs included.
- **A line that differs from T10's golden for the same case is a finding**, not a new golden: the daemon filled a field T10's input didn't expect, or the render is wrong. Mark the test `#[ignore = "T10: …"]` or with the task that fills the field, and report it.
- **`--json` prints the daemon's bytes** (`plan/contracts/cli.md`): compare with the body, don't parse and re-serialise before comparing.
- **Relative times move with the clock.** `(stale, 4m old)` and `rate_limited for 12m (until 22:17 local)` are fixed only if each case advances by exact amounts. Never advance "until something happens".
- **Titles carry the fake's `MARKER-TITLE`**: it's in the goldens, and that's fine. The token marker must never be, and neither must the session's; a title in a `.notice` file is a finding against T46 (NTF-8).
- **A `no_session` line holds a path and the system's error:** both are replaced, by `<tmp>` and `<os error>`, as TST-7 names them.
- **`notify`'s life prints `1d`**, CLI-3's form. An `expired` notice needs `--for 60` and an advance of a minute and a tick: `expired after 1m`.
- **An awaited `cli()` hangs when the command waits on the clock:** every `wait`, every command that spawns (the 8 s for a spawner that never binds), a `list` before the first poll has finished. Use `cli_bg`, advance, then await (testkit.md).
- **`list` prints at most 50 PRs without `--since`, every record with it** (CLI-3). `changes` and `wait` send `limit=50`.
- The stub daemon for code 1 needs a socket path under the harness's short directory (103 bytes, OPS-4), and must answer `GET /v1/status` too: the CLI reads it first (CLI-9).

## Open questions
- **The layout, the 50-event cut and code 8 are (proposal)** (CLI-3, CLI-5). The goldens fix today's.
- **`testdata/api/` is T41's** (it was T02's when `plan/contracts/api.md` was written): the stub for code 1 reads `error-unknown_route.json` from there and changes nothing in it.
- **A denied socket (CLI-7's sandbox line)** can't be staged portably: a test run as root isn't denied. It stays with T24's unit tests; say so in the report.
- D5 and D6 don't touch this task.

## Evidence
- **2026-11-26**, branch `t35-golden-cli`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `tests/golden_cli.rs` and 118 files in `tests/golden/cli/` (`.out`, `.err`, `.json`, `.notice`), written twice from empty with identical results and read by the executor against CLI-3 and CLI-6; the orchestrator checked none holds a token marker, a `gho_` value or a real `/tmp` path. 59 tests, none ignored; every exit code 0 to 11 from its own cause; `--json` equal byte for byte to `api_raw`'s body; notices checked against T46's by shape. `make verify` passes on main after the merge, first try.
- **Findings:** `error_starting.err` reads "…try again in a few seconds (try again in a minute)", two hints: follow-up **T59** (T22's message drops its own). Kept as spec'd: `wait_truncated.out`'s "checks failed (0 of 400)" (MOD-7: the failed check is unread); `notify_already.out` printing events since the arrival cursor (API-9).
- Choices: `already:` after a replacement staged with a held `changed --kind activity`; a stub daemon inside the test file for code 1; the `--json` files hold the fake's 40-character SHAs; a denied socket isn't staged (T24's unit tests).
