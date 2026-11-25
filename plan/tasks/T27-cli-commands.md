---
id: T27
title: "CLI commands: run, each command, the wait's deadline and reconnect, exit codes"
kind: code
milestone: M3
size: L
status: todo
depends_on: [T10, T11, T24]
requirements: [CLI-4, CLI-5, CLI-6:without-timeout, CLI-6:one-deadline-for-the-whole-command, CLI-6:sigint-and-sigterm-end-it-at-once, CLI-7:every-request-has-a-limit, CLI-8, CLI-9:a-bad-request-or-unknown-route-from, CLI-10, CLI-2, U10]
owns: [src/cli/commands/]
shares: []
---

# T27 · CLI commands: run, each command, the wait's deadline and reconnect, exit codes

## Outcome
`cli::run` works: each command of CLI-1 parses, names its PR, reaches the daemon, sends its request, prints text or `--json` and exits with CLI-5's code; `wait` keeps one deadline for the whole command and repeats itself once when its connection drops (CLI-6, CLI-10). `notify` is handed to `cli::notify::run` (T48), with CLI-2's naming flow open for it to call.

## Context
This task joins three parts built apart: T11 parses and names, T24 connects, T10 renders. It adds the flow between them, the real `git`, the real `CliEnv` and `main`. T48 builds the `notify` command behind the stub this task calls, with this task's two naming functions. T29's harness calls `run` as a function; T36 runs the binary. **`run` reads nothing but its `CliEnv`**, and returns an `Output` instead of printing.

## Read first
- `plan/conventions.md`
- `plan/contracts/cli.md` (all of it: `run`, `main`, `CliEnv`, `Git`, `Interrupt`, the three parts' signatures, "Semantics", "Don't assume"); `plan/contracts/api.md`: `Request`, `Envelope`, the body types, `ErrorBody`, `CURSOR_HEADER`; `plan/contracts/transport.md`: `Head`, `Dropped`; `plan/contracts/config.md`: `resolve`, `load`, `Env`; `plan/contracts/daemon.md`: `run::main`; `plan/contracts/clock.md`: `timeout`, `Mono`, `TestClock`
- `specs/05-cli.md` (all of it; CLI-2, CLI-4, CLI-5, CLI-6, CLI-8 and CLI-10 are yours in part)
- `specs/04-api.md#waiting`: API-9 (the head, then the body), API-12; `#changing-the-set`: API-16 (a refresh's outcomes); `#reading`: API-8 (`limit`); `#basics`: API-3, API-5 (`github.login`)
- `specs/06-operations.md#configuration`: OPS-1 (`--check-config`); `#shipping`: OPS-11
- `specs/07-testing.md#rules`: TST-1 (`CIRCITOR_TEST_CLI_LIMIT`); TST-8's `bare-pr` row
- `src/cli/conn/stub.rs` (T24's stub daemon), `src/cli/render/`, `src/cli/naming/`: the code you call

## Scope
- **`run`.** `args::parse`; an `Err` is its line on stderr and exit 2. `version` prints `circitor <env.version>` (OPS-11), `help` its text, `guide` `GUIDE`: none resolves a path or opens a socket (CLI-8). Every other command resolves `Paths` with `config::resolve`; a `PathError` is its line and exit 6. The whole command is raced against `env.interrupt` (`CLI-6:sigint-and-sigterm-end-it-at-once`): 130 or 143, nothing printed.
- **`notify`.** `Command::Notify` goes straight to `cli::notify::run(pr, what, json, no_spawn, env)` and its `Output` is returned as it is: no path is resolved here first (T48 exits 11 before anything else when there's no session). The call is all of `notify` in this task; until T48 is merged it's T02's stub.
- **Each command (CLI-1, U10).** `list`, `show`, `changes`, `track`, `untrack` and `refresh` go through `conn::ensure` and `Link::request`; `status` through `conn::existing` (none running: `not running` on stderr, exit 6); `stop` through `conn::stop` and `render::stop`, exit 0; `track` and `untrack` hand `render::tracked` the PR they named (CLI-8: these two never start or replace a daemon; T24 builds how). `changes` hands `render::changes` its PR, or `None` without one, for `more events:` (CLI-3, D9). `changes` and `wait` send `limit=50` (CLI-3), one constant. `changes` without a PR covers them all: it never takes the branch's PR. `daemon --check-config` is `config::load`: exit 0, or its message and exit 1 (OPS-1). `daemon` without it never reaches `run`: answer exit 1.
- **Answers (CLI-5, shared with T10).** A 200 decodes into its `Envelope` and goes through its `render` function to stdout, exit 0. Any other status decodes into `ErrorBody`: `render::api_error` on stderr, `render::error_code`, both with `link.older_unreplaced()` (CLI-9, `CLI-9:a-bad-request-or-unknown-route-from`: your share is passing the flag on; the line and the code 1 are T10's). A body that doesn't decode is exit 1 with one line. A `ConnError` is its lines and its code. `warn` goes to stderr first. **Your share of CLI-5:** putting a code on every `Output`. T10's four tables and `ConnError.code` decide it; you add only 1 (unexpected), 2 (usage, naming), 130 and 143.
- **`refresh`'s outcome (API-16).** `auth`, `github_unavailable` or `rate_limited` in a 200 exits as the API error of that name: `render::refresh_code`. The text is `render::refresh`, on stdout.
- **`--json` (CLI-4).** stdout is `Reply.body` as it came, one line, for a 200 and for an error alike; an error still prints its stderr line and sets its code. Decode only to find the code. Never re-serialise.
- **Naming a PR (CLI-2, your share: the flow; the forms and the choice are T11's).** **Two public functions, as `plan/contracts/cli.md` declares them, used by `show`, `wait` and T48's `notify` alike:** `name_offline` (everything before a request: `Full`, `Number`, and `Ok(None)` for `Bare`) and `find_bare` (the flow against a daemon). `Full` is used as it is. `Number` takes the repository from `env.git.origin_url` and `naming::repo_of_origin`; none is exit 2, saying to use `owner/repo#12`, before any request. `Bare` (`show`, `wait`, `notify`) also needs `push_branch`: `Detached` and `NoRepo` are exit 2. Then: `ensure`; `List { all: true }`; `Status`, for `github.login`; `match_branch` without ended ones. No open match: `Refresh`, then the list again, **whatever the refresh answered**, then `match_branch` with ended ones. `Ended` is used, with one stderr line saying which PR and that it has ended. `Several` is exit 2, listing them. `None`: after a problem outcome, that problem's code (`render::refresh_code`) and line (an `ApiError` built from the answer's `daemon` block); after `shared`, exit 2 with CLI-2's hint to name it by number; after `polled`, exit 2 saying which form to use.
- **`RealGit`.** `git -C <cwd>` through `tokio::process`: `remote get-url origin`; the branch is the upstream's branch name when the upstream is on `origin`, else the branch's own name; a detached HEAD is `Detached`; any failure of `git` is `NoRepo` or `None`.
- **`wait`: the deadline** (`CLI-6:without-timeout`, `CLI-6:one-deadline-for-the-whole-command`). The deadline is the command's start plus `--timeout`, or plus `cli::DEFAULT_WAIT_TIMEOUT`. `WaitAsk` carries the command's `Ask` (`--fail-fast` and `--kind` in it) and `timeout_given` and `json` as the command had them, so `again:` repeats them. `Request::Wait` gets the same `Ask`. Naming, `ensure` (its `budget` is what's left) and every request count against it. Each wait request sends `timeout` as the whole seconds left. Under 1 s left, nothing is sent: `NoAnswer::NoTimeLeft`, exit 3. `--timeout 0` always sends `timeout=0`, and its `budget` is what's left of 60 s.
- **`wait`: the limit** (`CLI-7:every-request-has-a-limit`, your share: a wait; `Link::open` has no limit): `clock::timeout` around the head and the body, ending at the deadline plus 10 s, or at 60 s from the start for `--timeout 0`. Past it: `NoAnswer::NotAnswering`, exit 6.
- **`wait`: the answer.** A head that isn't 200 is an API error, as above. From a 200 keep **the first** `circitor-cursor` of the command, never a later one. The body is `render::wait` and `render::wait_code`.
- **`wait`: a drop** (CLI-10, your share: the wait; `Link::request`'s retry is T24's). One repeat for the whole command. Dropped after the cursor arrived: `conn::reconnect` with the time left, then the same request with `since` set to the `--since` given or that first cursor, and `timeout` to the seconds left. Dropped before it: `conn::ensure` again, which may start a daemon. `reconnect` failing, or a second drop: `NoAnswer::DaemonStopped`, exit 6. A `ConnError` from `ensure` while under 1 s is left is `NoTimeLeft`, not exit 6.
- **The three endings** print `render::wait_no_answer` on stdout, with `link.socket()` (`paths.socket` if no `Link` was made), and exit by `render::no_answer_code`; with `--json`, on stderr.
- **`CliEnv::real` and `main`.** `real` builds the system clock, the local offset, the process's `Env` and directory, `version::version()`, `conn::RealSpawner`, `RealGit`, `Interrupt::never()` and 90 s; in a test build `CIRCITOR_TEST_CLI_LIMIT` replaces the 90 s (TST-1). It installs nothing. `main` parses first: `daemon` without `--check-config` goes straight to `daemon::run::main(spawned)`. Otherwise it builds a multi-thread runtime, wires `SIGINT` and `SIGTERM` to an `Interrupt`, runs, writes stdout and stderr, returns the code.

## Interfaces
- **Consumes:** `args::parse`, `naming::*`, `guide::GUIDE`, `render::*`, `conn::{ensure, existing, reconnect, stop, Link, Reply, ConnError, RealSpawner}`, `transport::{Head, Dropped}`, `config::{resolve, load}`, `api::types`, `clock::timeout`, `daemon::run::main`, `version::version`.
- **Provides:** `cli::commands::run` and `cli::commands::main` behind `cli::run` and `cli::main`, `CliEnv::real`, `cli::commands::RealGit`, and `cli::commands::{name_offline, find_bare}`, as `plan/contracts/cli.md` declares them.

## Out of scope
- How a connection is made, a daemon started, replaced or stopped; the retry of a request that isn't a wait: T24.
- Any wording T10 renders, the guide, the argument checks: T10, T11. Don't format a record or an event here.
- **The `notify` command**: its session, its request, its lines, exit code 11: T48. Here it's one call.
- The daemon: T28. `daemon::run::main` may still be T02's stub when you build: call it, don't test it.
- The harness and the commands against a real daemon: T29. End-to-end golden files: T35. Real signals and the real spawner: T36.

## Acceptance
Unit tests in `src/cli/commands/`, on `TestClock`, a `ShortDir`, `FakeSpawner` and `FakeGit` (T43's, through T24) and T24's stub daemon answering `testdata/api/` files. `tz` is UTC.
- `cli1_each_command`: for each of the eight commands with an answer that are this task's (`notify` is T48's), stdout equals its `render` function on the same file, exit 0; the request's path and query are what `Request` builds. `wait … --fail-fast` and `wait … --until changed --kind activity` send `fail_fast=1` and `kind=activity`.
- `cli2_name_offline`: `Some` for `o/r#12` and for a URL, with no call to `git`; `12` uses `origin`; nothing is `Ok(None)`; `12` outside a repository is `Err` with exit 2.
- `cli4_json_is_the_body`: `list --json` prints the stub's bytes on one line; with a `bad_request` answer too, exit 2.
- `cli5_error_codes`: each `error-<code>.json` gives 2, 5, 1, 7, 9, 9, 9 and 10, with one stderr line and an empty stdout; an undecodable 200 gives 1; `refresh` answering `rate_limited` in a 200 gives 9.
- `cli8_no_daemon`: `status` exits 6 and `stop` exits 0 with `not running`, with zero spawns; `version`, `help` and `guide` work where the socket's directory doesn't exist.
- `cli2_number_uses_origin`; `cli2_outside_a_repository`: exit 2, zero requests, zero spawns; `cli2_detached_head`: exit 2.
- `cli2_bare_flow`, the cases of TST-8 `bare-pr`: found at once (no `refresh` sent); found after one `refresh`; after `shared`, the second list is still read; a merged and a closed match give the later `closed_at`, with the note; two open matches exit 2 with both names; no match after `polled` exits 2, after `shared` exits 2 with the hint, after `rate_limited` exits 9 and after `auth` 7.
- `cli6_default_timeout`: no `--timeout` sends `timeout=100`. `cli6_one_deadline`: `--timeout 30` with 3 s spent before the request sends `timeout=27`. `cli6_under_1s_is_not_sent`: exit 3, `timeout after 30s: <pr> (no answer from the daemon)`, an `again:` with `--timeout 30`, no wait request.
- `cli6_timeout_0`: sends `timeout=0`; a stub that never answers ends at 60 s, exit 6, `daemon not answering on <path>`. `cli7_wait_limit`: a default wait never answered ends at 110 s, exit 6, not before.
- `cli6_wait_codes`: `wait-satisfied.json` and files of your own give 0, 3, 4 and 8; with `--fail-fast`, a `satisfied` answer whose checks are `pending` with one failed gives 8.
- `cli10_drop_after_the_cursor`: the second request carries `since=<first cursor>` and the seconds left; its answer is printed. With no daemon back: `daemon stopped`, `again:` and `cursor:` with the first cursor, exit 6 after 8 s, zero spawns. `cli10_drop_before_the_cursor`: `ensure` runs again; a second drop is exit 6.
- `cli6_interrupt`: `SIGINT` during a held wait is 130, `SIGTERM` 143, at once, with empty output.
- `ops1_check_config`: no file is 0; `testdata/config/` bad files are 1 with the message.
- `cargo test --features test-endpoint --lib cli::commands::` and `make verify` pass.

## Traps
- **The clock stands still unless the test moves it.** Spawn `run`, advance to the moment under test, then await it. Never put an answer and a limit inside one advance.
- The deadline is on `mono()`. Durations never come from the wall clock (CLI-13).
- **A later cursor loses events.** After a repeat, `again:` still carries the first one.
- **No `Local::now`.** T04's source-guard test fails on it outside `src/clock/`, as on `Utc::now`. `CliEnv::real` gets the local offset without reading the time itself: `chrono::Local.offset_from_utc_datetime` on the clock's wall time.
- `main` must not build a runtime before it hands `daemon` over: `daemon::run::main` builds its own. Write output with `write_all` and ignore a closed pipe; `println!` panics on one.
- A newer daemon may answer with keys or codes this CLI doesn't know: unknown keys are ignored, an unknown code is exit 1.
- `Link::request` already retries and limits. Don't wrap it in another retry.
- `status` asks for the login only inside the bare flow, and only after `list`, which waits for the first poll (API-18).

## Open questions
- CLI-2's short forms, CLI-5's code 8 and CLI-7's limits are (proposal): build them as written.
- **Not said by the specs, built as Scope says, and reported to the planner:** the exit code of a `PathError`; `--json` with an error or an ending without an answer; which stream the three endings use; the wording of CLI-2's own lines (T35's golden files fix them).
- docs/03 §6 item 2 (a killed `wait` leaves nothing in the daemon) is unverified: nothing here rests on it.
