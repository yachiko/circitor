---
id: T48
title: "CLI: the notify command"
kind: code
milestone: M3
size: M
status: done
depends_on: [T27, T46]
requirements: [NTF-11, U14, CLI-5:notify-exits-0-once-it-s, TST-7]
owns: [src/cli/notify/, tests/golden/notify_cmd/]
shares: []
---

# T48 · CLI: the notify command

## Outcome
`circitor notify [PR] --until …` registers a subscription for the Claude Code session it runs in and returns at once, and `circitor notify [PR] --cancel` drops that session's subscriptions on the PR (NTF-11, U14): with the session's socket from the environment, exit code 11 where there's none, `already:` when the answer is known, and an exit code that says only that the command worked.

## Context
T11 parses `notify` into `Command::Notify`; T27's `commands::run` hands that command to `cli::notify::run`, which T02 left as a stub, and exposes CLI-2's naming flow (`name_offline`, `find_bare`); T24 connects; T10 renders `already` and the error lines; T46's `notify::text::watching` gives the words the confirmation uses. This task fills `src/cli/notify/`: the flow and the command's own few lines. The daemon's side (T47, T46) isn't needed: the tests answer from T24's stub daemon. T29's harness runs the command against the real one, with a fake session socket in its environment.

## Read first
- `plan/conventions.md` ("Tests": never a real session's socket)
- `plan/contracts/cli.md`: `run`, `CliEnv`, `NotifyWhat`, the blocks "`src/cli/notify/` (T48)", "`src/cli/commands/` (T27)", "`src/cli/render/`" (`already`, `registered`, `flags`, `api_error`, `error_code`) and "`src/cli/conn/`" (`ensure`, `existing`, `reconnect`), and in "Semantics" the two `notify` bullets; `plan/contracts/api.md`: `Request::Notify`, `NotifyCancel`, `NotifyBody`, `CancelBody`, `Ask`, `ErrorCode::NoSession`; `plan/contracts/notify.md`: `text::watching`; `plan/contracts/config.md`: `Env::get`
- `specs/08-notify.md#the-command`: NTF-11, every part; NTF-7 (what the third line tells the agent); NTF-2 (the body's keys); NTF-10 (`--no-spawn` waits 8 s)
- `specs/05-cli.md`: CLI-1, CLI-2, CLI-4, CLI-5 (code 11, and the part "`notify` exits 0"), CLI-7, CLI-8 (`notify --cancel` never starts a daemon), CLI-10
- `specs/00-overview.md`: U14. `docs/07-notify.md`: "Where", "The token", "A subagent's shell"
- In `testdata/api/` (T41's, there once T41 is merged): `notify-registered.json`, `notify-already.json`, `notify-cancelled.json`, `error-no_session.json`; `src/cli/conn/stub.rs` (T24's stub daemon); `src/cli/commands/` (T27's flow, which you call)

## Scope
- **The session (NTF-11's part "the session is the one in the environment").** `env.env.get(SOCKET_VAR)` and `get(TOKEN_VAR)`: nothing else is read, and never the process's environment (CLI-13).
- **No socket in the environment: exit 11, before anything else** (NTF-11, D9): no path resolved, no daemon reached or started, no PR named. One stderr line. Registering: NTF-11's line, `no session here: use circitor wait <pr> ` with `render::flags(cond)`, then ` --since <since>` if given: the condition's parameters as given, **never `--timeout` and never `--for`** (`--for` has no counterpart, API-12). `<pr>` is the PR argument as it was typed, if there was one. `--cancel`: a line that names no `wait`.
- **Registering.** `config::resolve`; `commands::name_offline`; `conn::ensure` (it starts the daemon when needed, CLI-7); for no argument `commands::find_bare`; then `Request::Notify` with the `Ask`, `life` only when `--for` was given, the socket, the token or none, and `limit` 50 (CLI-3's cut, as `wait` sends it), through `Link::request`: it's safe to repeat (CLI-10).
  - **`registered: false`** (NTF-11's part "when the answer is already known"): `render::already` on stdout, with `--for` if given, so that with `reset: true` its `again:` line is `again: circitor notify …` (T10), and with `replaced: true` it holds `replaced an earlier notify` (NTF-2: an answer already known still replaces). Exit 0, also when a check failed.
  - **`registered: true`** (the part "otherwise it prints"): `render::registered(e, first, pr, ctx)` with `first` = `registered: ` and `notify::text::watching(pr, cond, life)`. T10's function prints the rest in NTF-11's order: a reset and events first, the PR's one line, the line that says to cancel and then use `wait`, `replaced an earlier notify`, `more events:`, `problem:`, `cursor:`. The first line is NTF-7's one rule for the condition's words, every parameter included (`watching`, T46). The life printed is `--for`, or `notify::DEFAULT_LIFE` without it: never worked out from `expires_at`. Exit 0.
  - **`--no-spawn` with no daemon** (NTF-10, CLI-7): `conn::existing` first; with none, `conn::reconnect` for up to 8 s, which starts none; then exit 6 as `ensure` would. This is the command a `stopped` notice gives: after a replacement or a takeover the new daemon answers within those 8 s.
  - **`no_session`:** exit 11 (`render::error_code`), the same one stderr line, now with the PR in full (NTF-11: "without the first, or when the daemon can't reach that socket").
  - Any other API error: `render::api_error` and `render::error_code`, as every command (CLI-5). A `ConnError`: its lines and its code.
- **`--cancel`** (the part "`--cancel` prints"; CLI-8). `conn::existing`, never `ensure` and never `reconnect`: with no daemon it prints `cancelled: 0` and exits 0, and nothing is started. The PR is named as above, but a bare one with no daemon is `cancelled: 0` too: there's nothing to ask. Otherwise `Request::NotifyCancel`, and `cancelled: <n>`.
- **`--json` (CLI-4).** stdout is the reply's body as it came, for a 200 and for an error alike; the exit code and the stderr line stay. Exit 11 without a socket prints nothing on stdout.
- **The exit code (NTF-11's last part; CLI-5's part "`notify` exits 0").** 0 for registered, already known and cancelled. Never 3, 4 or 8: `render::wait_code` isn't called here. 11 for no session; an API error's own code otherwise.
- **U14's share:** this command is how a session asks. That a notice then arrives is T46's and T47's, and T49 proves it end to end.
- **TST-7's share:** the command's own lines as golden files in `tests/golden/notify_cmd/`. The same lines against a real daemon are T35's.

## Interfaces
- **Consumes:** `cli::{CliEnv, Output, NotifyWhat, exit}`; `commands::{name_offline, find_bare}` (T27); `conn::{ensure, existing, reconnect, Link, Reply, ConnError}` (T24); `render::{already, registered, flags, api_error, error_code, RenderCtx}` (T10); `notify::text::watching`, `notify::DEFAULT_LIFE` (T46); `api::types::{Request, Ask, Envelope, NotifyBody, CancelBody, ErrorBody, ErrorCode}`; `config::resolve`.
- **Provides:** `cli::notify::{run, SOCKET_VAR, TOKEN_VAR}` as `plan/contracts/cli.md` declares them. Nothing else is public.

## Out of scope
- Parsing `notify`'s flags and their usage errors, and the guide's paragraph on `notify`: T11. The dispatch from `commands::run`, CLI-2's flow itself: T27.
- `render::already`, `render::registered`, the outcome line, the API error lines and every code but 11: T10. The words of `watching`: T46. Your golden files fix the command's whole output as the user sees it.
- The routes, the confirmation and every notice: T47, T46. The fake session socket and the harness: T45, T29. The scenarios: T49.

## Acceptance
Unit tests in `src/cli/notify/`, on `TestClock`, a `ShortDir`, `FakeSpawner` and `FakeGit`, and T24's stub daemon answering `testdata/api/` files; the `Env` is built in the test. Texts go through `testkit::golden::check` on `tests/golden/notify_cmd/<name>.txt`.
- `ntf11_registered`: with `notify-registered.json`: NTF-11's four lines, the third exactly as NTF-11 writes it with the PR in full, the first reading `registered: <pr> until checks-done on 1a2b3c4, for 1d` for the file's values, exit 0; the request is `POST /v1/prs/…/notify` whose body holds `until`, `sha`, `socket`, `token` and `limit` 50, and no `for`.
- `ntf11_for_is_sent_only_when_given`: `--for 3600` puts `for: 3600` in the body and `for 1h` in the line.
- `ntf11_replaced_line`: `replaced: true` adds `replaced an earlier notify` after the confirmation line and before `cursor:`; with `notify-already.json` and `replaced: true`, it comes straight after the `already:` line (NTF-11's order, in both cases).
- `ntf11_registered_extra_lines`: an answer with `reset: true` prints the reset line and its records first; one with events prints them first; one with `daemon.problem` set prints its `problem:` line before `cursor:`; the second line is the PR's `list` line, `merged` for a merged record.
- `ntf11_already_with_reset`: `notify-already.json` with `reset: true`: the output ends, before `cursor:`, with `again: circitor notify <pr> --until checks-done --since <the new cursor>`, and with `--for 3600` given, ` --for 3600` before `--since`.
- `ntf10_no_spawn_waits`: `--no-spawn` with no daemon: zero spawns; a stub daemon that starts answering 5 s on gets the registration, exit 0; with none, exit 6 once the clock has moved 8 s, and not before.
- `ntf11_already`: with `notify-already.json` (a failed `checks-done`): stdout is `render::already`'s text, with `already: checks failed (…)`, and **exit 0**.
- `ntf11_no_socket_is_11`: an `Env` without `SOCKET_VAR`: exit 11, one stderr line reading `no session here: use circitor wait 12 --until checks-done --sha 1a2b3c4` for `notify 12 --until checks-done --sha 1a2b3c4`; for `notify 12 --until changed --kind review --since c1 --for 3600`, `… --until changed --kind review --since c1`, with no `--for` and no `--timeout`; an empty stdout, zero requests, zero spawns, and it works where the socket's directory doesn't exist. An empty `SOCKET_VAR` is the same. `--cancel` without it: exit 11, a line with no `wait` in it.
- `ntf11_no_session_is_11`: with `error-no_session.json`: exit 11, one line, `no session here: use circitor wait <the full PR> --until checks-done --sha 1a2b3c4`.
- `ntf11_token_is_optional`: without `TOKEN_VAR` the body's `token` is `null`.
- `ntf11_cancel`: `cancelled: 1` from `notify-cancelled.json`, exit 0, a `DELETE` with `socket` percent-encoded; with no daemon, `cancelled: 0`, exit 0 at once (the clock hasn't moved), zero spawns, also with `--no-spawn`.
- `ntf11_api_errors_keep_their_codes`: `error-limit.json` gives 10, `error-bad_request.json` 2, `error-not_found.json` 5, each with `render::api_error`'s line.
- `ntf11_json`: `--json` prints the stub's bytes on one line for a 200 and for `no_session`, with exit 0 and 11.
- `ntf11_bare_uses_cli2`: with no argument in a `FakeGit` repository, the PR is the branch's, through `find_bare`.
- `ntf11_starts_the_daemon`: registering with no daemon spawns one (`FakeSpawner::spawns()` is 1).
- `cargo test --features test-endpoint --lib cli::notify::` and `make verify` pass.

## Traps
- **Your own shell has `CLAUDE_CODE_MESSAGING_SOCKET` set**, and `CliEnv::real()` reads the process's environment. No test here builds a real `CliEnv` or runs the binary: every `Env` is hand-built, and its socket path is a name in a `ShortDir` that nothing listens on (the stub daemon never connects to it).
- **Exit 0 proves nothing about the checks** (NTF-11): don't "help" by returning 8 for a failed already-known answer.
- **The token goes into the request's body and nowhere else:** not into an error line, not into stderr, not into `--json`'s output, which is the daemon's answer and never holds it.
- `find_bare` needs a `Link`: for registering that's `ensure`'s; for `--cancel` there may be none, and then nothing is asked.
- `Link::request` already retries and limits. Don't wrap it.
- The clock stands still unless the test moves it: spawn `run`, advance, then await it, for the cases that start a daemon.

## Open questions
- **NTF-11's exit codes are (proposal).** Built as written.
- **NTF-11 fixes the registering line** (D9): the PR as typed without the variable, in full after `no_session`, and nothing else printed. `--cancel`'s exit-11 line is this task's, fixed by its golden files: list it in your report.
- **A subagent's shell has its parent's socket** (docs/07): nothing to build; the notice goes to the main conversation.
- **macOS isn't measured** (docs/07): the two variables' names are what Linux showed.

## Evidence
- **2026-11-26**, branch `t48-cli-notify`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/cli/notify/` (`mod.rs`, new `stub.rs` and `tests.rs`, test-only) and 13 golden files in `tests/golden/notify_cmd/`, read by the executor against NTF-11. **Checked by the orchestrator:** no `CliEnv::real`, `Env::from_process` or `std::env::var` in the module; the exit-11 lines carry the condition's flags and `--since`, never `--for` or `--timeout`, and the PR as typed; `registered.txt` is NTF-11's block. `cargo test --features test-endpoint --lib cli::notify::`: 15 passed. `make verify` passes on main after the merge, the macOS check included.
- Choices: its own stub daemon (records request bodies and routes `notify`); `--cancel --json` with no daemon prints `{"cancelled":0}`; `--cancel`'s no-socket line is `no session here: there is no session's notify to cancel`; `name_offline` runs before `conn::existing`, so a bad PR argument is exit 2 even with no daemon.

Follow-ups: T63 (D18: a subagent's notify, in words).
