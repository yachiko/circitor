---
id: T49
title: "Scenarios: notify"
kind: test
milestone: M5
size: L
status: done
depends_on: [T29, T42, T47, T48]
requirements: [TST-8, NTF-5, NTF-9, NTF-10]
owns: [tests/scn_notify.rs]
shares: []
---

# T49 · Scenarios: notify

## Outcome
Ten of TST-8's scenarios run on the harness and pass: `notify-outcome`, `notify-already`, `notify-ends`, `notify-session-gone`, `notify-stop`, `notify-keeps-active`, `notify-keeps-ended`, `notify-problem`, `notify-no-session`, `notify-register`; and so do the subscription parts of two more, `sleep` and `clock-back` (T33 plays the rest of those two).

## Context
T29's harness runs the whole daemon in-process on the test clock, against the fake GitHub, with the CLI's commands as functions; it binds a fake session socket (T45) and puts its path and a marker token in every command's environment, and the daemon posts to it through the real poster (T46). The routes (T47) and the command (T48) are merged, and so is T42, which raises `pr_failing` (`notify-problem`). This task plays the ten TST-8 rows about **notify**, and the subscription sentences of `sleep` and `clock-back`, on it: the first time a subscription is registered through the CLI, evaluated by polls and told to a socket. T30 to T33 play the other rows. Its failures are how bugs in T46, T47, T48, T28, T15, T09 and T21 are found.

## Read first
- `plan/conventions.md`: Tests, Requirement tracing, Report.
- `plan/contracts/testkit.md` (the harness, `session()`, `new_session`, the fake session socket and its four modes, the sweep), `plan/contracts/notify.md` (the semantics: the loop, the tick, a notice that isn't posted, `stop`), `plan/contracts/clock.md` (`advance`, `sleep_wall_only`, what "settled" means), `plan/contracts/api.md` (`NotifyBody`, `StatusBody`), `plan/contracts/cli.md` (`Output`, exit codes), `plan/contracts/log.md` (the notify row's event words).
- The module docs of `src/testkit/harness/`, `src/testkit/session/` and `src/testkit/server/`: the real names of the controls. `tests/m3_cli.rs` shows a test driving the harness.
- `specs/07-testing.md#what-is-tested`: TST-8, the ten `notify-*` rows and the last sentences of `sleep` and `clock-back`; TST-11. `#test-doubles`: TST-16, TST-4 ("settled" counts notices being posted).
- `specs/08-notify.md`: all of it. `specs/02-polling.md#cadence`: POL-10's table, POL-13, POL-14, and POL-26's tick; `#errors`: POL-20 (`auth`, a try with a new token), POL-21 (`pr_failing`), POL-22. `specs/03-state.md`: STA-1, STA-2, STA-11 (the minute's check). `specs/06-operations.md`: OPS-7, OPS-8.
- `specs/05-cli.md`: CLI-7 (`--no-spawn`), CLI-9.
- `specs/05-cli.md`: CLI-5 (code 11), CLI-6 (the outcome lines a notice holds).

## Scope
**TST-8's share here: these ten rows and two rows' subscription parts**, in `tests/scn_notify.rs`. The row is the spec; each entry below adds only the world's concrete values and what to assert. **Where a row and the requirement it cites differ, the requirement wins** (TST-8): then assert the requirement and report the row.

How every scenario is written:
- **One `#[tokio::test] async fn scn_<name>()` per row**, hyphens as underscores, with `// TST-8: <name>` and the row's IDs (`// NTF-4:`) above it. A row with several worlds may be several functions named `scn_<name>_<part>`, so an `#[ignore]` costs one part. Each starts its own harness and **ends with `h.finish()`**.
- **Script the world and set the config before the daemon's first poll**, through `Harness::start(opts)`.
- **Look at what the session got** through `h.session().notices()`: each has its text, the token of its auth line and the clock's time. **Look at the daemon** through the CLI: `status --json` (`subscriptions`, `daemon.tier`, `tracked`), `Output.code`, `h.log()` by event word (`subscription registered`, `subscription ended … end=…`, `notice not posted … reason=…`), `h.github().requests()` for send times.
- **Advance and look; never sleep.** `notify` returns at once, but its registration reads through and refreshes: run it under `cli_bg`, advance, then await it.
- **A notice's text is asserted whole only where an entry quotes it.** The exact words are T46's and T35's golden files.
- Keep the spec's values as named constants (`TICK = 10 s`, `PROBLEM_AFTER = 10 min`, `LIFE_MIN = 60 s`, `SUBSCRIPTIONS_MAX = 64`, `PER_SOCKET_MAX = 32`, `CATCH_UP = 60 s`, `NO_SPAWN_WAIT = 8 s`, `KEEP_AFTER_END = 60 min`, `STOP_DEADLINE = 5 s`): all are (proposal).

Names: PRs `octo/app#N`; leave check names and logins to the fake, which marks them (TST-11), as it marks titles. "Settled PR": `updatedAt` two minutes old at the first read. SHA `A` starts `dce9dbe`.

The scenarios:

- **`notify-outcome`** (NTF-5, NTF-6, NTF-7, NTF-8). World: a settled PR at `A` whose check is `IN_PROGRESS` and passes 95 s on. Steps: `notify --until checks-done --sha <A>`. Assert: exit 0; stdout's first line starts `registered: octo/app#1 until checks-done on dce9dbe, for 1d`; **when the command has returned, `notices()` already holds one**: its `token` is `MARKER_SESSION_TOKEN` and its text is `circitor: watching octo/app#1 until checks-done on dce9dbe, for 1d. If you can read this, notices reach this session.`; `status --json` reads `subscriptions: 1`. After the checks pass and the next poll: exactly one more notice, reading `circitor: octo/app#1 checks passed (1/1) on dce9dbe. Details: circitor changes octo/app#1 --since <c>`, with `<c>` the cursor the command printed; `subscriptions: 0`; and 10 minutes on, no third notice. **Never hot:** between the registration's own refresh and the outcome, every gap between two requests naming the PR's node is at least `interval` less 1 s, and no request without a finding names it after that first refresh.
- **`notify-already`** (NTF-2, NTF-11). World: a settled PR whose one check failed. `notify --until checks-done`: stdout holds `already: checks failed (1 of 1) on <sha7>` and a `cursor:` line; **exit 0**; `notices()` is empty; `subscriptions: 0`. The same with `--json`: `registered: false`, `outcome: "satisfied"`.
- **`notify-ends`** (NTF-3, NTF-4, NTF-9). One world each.
  - *Closed:* `notify --until merged` on an open PR, then the PR is closed: one notice after the confirmation, holding `ended: closed`.
  - *Expired:* `notify --until merged --for 60`: no second notice at 59 s; by the first tick at or after 60 s, one holding `expired after 1m; it's open` with no `(stale`, and `Ask again: circitor notify octo/app#1 --until merged --for 60 --since <c>`, `<c>` the cursor the command printed. `subscriptions: 0`.
  - *Cancelled:* `notify --until merged`, then `notify --cancel`: `cancelled: 1`, exit 0; again, `cancelled: 0`; the PR then merges: no notice but the confirmation.
  - *Registered twice:* on a PR whose check is still running, `notify --until checks-done` and the same again: a second confirmation, stdout holds `replaced an earlier notify`, `subscriptions: 1`, and no notice for the first's end. Then `notify --until checks-done --fail-fast`: held side by side, `subscriptions: 2`. Then `notify --cancel`: `cancelled: 2`.
  - *Counted:* three subscriptions on two PRs read `subscriptions: 3`; `waiters` stays 0.
- **`notify-session-gone`** (NTF-4, NTF-6). One world each, a settled open PR, `notify --until merged`, the tier kept by the subscription alone.
  - *Vanished:* `h.session().vanish()`: within one tick `subscriptions: 0`, the log has `subscription ended` with `end=session_gone`, and nothing more was posted. The PR then merges: nothing is posted.
  - *Refusing:* `refuse()`: the tick drops nothing (the file is the same socket). The PR merges: no notice arrives, the log has one `notice not posted` with `reason=session_gone`, `subscriptions: 0` at once, and 10 minutes on there was no other post or line.
  - *Taken over:* `vanish()`, then `restore()` at once (a new socket at the same path, another inode): at the next tick the subscription ends as `session_gone` and nothing is posted to the new one.
  - *Full queue:* `queue_full()`, then the merge: the connect fails at once (`EAGAIN` on Linux): one `notice not posted` line, with `reason=failed`, at the instant of the merge's poll, `subscriptions: 0` at once, and 10 minutes on no second line and no post: **never tried again** (NTF-6).
  - *Accepting, never reading:* `accept_and_never_read()`, then the merge: it counts as posted: no `notice not posted` line, `subscriptions: 0`, no second post.
  - *No poll is delayed:* in each part the polls' send times in `h.github().requests()` keep their `interval`.
- **`notify-stop`** (NTF-9, NTF-10, OPS-7, CLI-7). World: three settled open PRs, a `notify --until merged` on each, and a background `wait --until merged` on the first. `h.stop()`. Assert: three notices after the confirmations, each holding `the daemon is stopping` and ending `--no-spawn`; the wait's command ended with the connection dropped (CLI-10's `daemon stopped`, exit 6) **after** the notices' times; the stop took under 5 s of the clock and `exited()` is `Some(0)`; the state file holds no socket's path and no `MARKER_SESSION_TOKEN` (the sweep), and loads.
  - *Crash:* the same three, `h.crash()`: no notice beyond the confirmations. `h.restart()`: `subscriptions: 0`, and a merge posts nothing.
  - *Asking again after a stop:* after `h.stop()`, run the notice's `Ask again` command under `cli_bg`, with no daemon: zero spawns; it exits 6 once the clock has moved 8 s, and not at 7 s.
  - *Asking again after a replacement:* the same command, and `h.restart()` 3 s into its wait (standing in for CLI-9's new daemon): it registers with the new daemon (exit 0, `registered:`, a new confirmation), whose cursor is from the new epoch.
  - *A stop the daemon chose:* with one subscription, remove the daemon's socket file; at its next minute's check the daemon stops (`exited()` is `Some(0)`), and the notice ends with `--since <c>`, without `--no-spawn` (NTF-10, STA-11).
- **`notify-keeps-active`** (NTF-5, OPS-8, POL-10, POL-13). Config: `exit_after = 300`, `idle_after` its default 600 s. World: `octo/app#1` open, its check running, merged 20 minutes on.
  - *Held:* `notify --until merged`, then no client: `status --json` reads `daemon.tier: active` throughout the 20 minutes (it counts no request itself), polls keep `interval`, and `h.exited()` is `None` past 300 s.
  - *Ended, the last request long ago:* the merge's poll posts the outcome and ends the subscription; the last client request (the registration) is 20 minutes old, so the tier reads `idle` at the next status, not 600 s after the end (POL-13: once it ends, the tier follows the last client request). The daemon exits 300 s after the end, and not at 299 s (`exited()` is `Some(0)`, `stop cause=idle`).
  - *Ended, a request just before:* the same, with a `list` (state `all`) at 15 minutes: the tier reads `idle` 600 s after that `list`, not at the end; the daemon exits 300 s after the later of the `list` and the end.
  - *An `asked` PR:* `notify else/lib#2 --until merged` reads it through as `asked`. While the subscription is open its refreshes come every `interval`, not every `idle_interval` (POL-10's row), and it's still tracked after 60 minutes; none of them comes at `hot_interval`.
  Use `status` to look: any other command is a client request and would move what's measured.
- **`notify-keeps-ended`** (NTF-4, NTF-5, STA-1, STA-2). Config: `keep_done` its default 86 400 s, `exit_after` unset. World: `octo/app#1` merged 24 hours before the start. `notify octo/app#1 --until changed --for 7200`: a merged PR with nothing new neither satisfies nor ends it, and it's read through. Assert: past the registration's own 60 minutes, `octo/app#1` is still tracked (`status --json`'s `tracked`), though past its `keep_done`; **the tier reads `idle` from 600 s after the registration** (a subscription on an ended PR doesn't keep it), and polls go `idle_interval` apart. At 2 hours the subscription expires (`expired` notice). The PR is still tracked at the tick 59 minutes after that, and dropped (an `untracked` event, in `changes` fetched afterwards) at the first tick at or after 60 minutes (STA-1: the end counts as a request's end).
- **`notify-problem`** (NTF-2, NTF-4, NTF-8, POL-20, POL-21). One world each, the PR tracked by a poll first.
  - *Registering in `auth`:* the fake GitHub answers 401 to everything from `t`; once `status --json` reads `daemon.problem.code: auth`, `notify --until checks-done` on running checks exits **7** (`auth`), `subscriptions: 0`, no post. On a PR whose one check failed and settled before `t`: exit 0, `already:`.
  - *`auth` while held:* `notify --until merged`, then 401s from `t` (the repeat's 401 at `t` puts the daemon in `auth`). At `t` + 5 minutes the token source gives a new token, whose one try also gets a 401. No notice at the tick before `t` + 10 minutes; at the first tick at or after it, one notice: `circitor: can't read octo/app#1 from GitHub (auth); notify is cancelled. Details: circitor status`; `subscriptions: 0`; the log has `subscription ended … end=problem why=auth`.
  - *`pr_failing`:* the PR's node answers an error that isn't `NOT_FOUND` on every refresh from `t`. Once `status --json` shows the `pr_failing` warning for it (POL-21's 5 minutes), the next tick posts one notice holding `(pr_failing)`; the subscription on another PR stays. A second registration on the failing PR while the warning is raised posts its confirmation, then `problem` at the first tick.
- **`notify-register`** (NTF-1, NTF-2, NTF-3, NTF-11). One world each.
  - *A `since` from before a restart:* take `cursor:` from a `changes`, `h.restart()`, then `notify --until checks-done --since <that cursor>` on running checks: stdout prints the reset line and its records first, then `registered:` (exit 0); when the checks pass, the notice's `--since` is the cursor the command printed, not the one sent.
  - *Held side by side:* `notify --until checks-done --sha dce9dbe` and `--sha <A in full>`: two confirmations, `subscriptions: 2`, no `replaced an earlier notify`. `--sha DCE9DBE` then replaces the first: `replaced an earlier notify`, still 2.
  - *A replacement at 32:* 32 held for the harness's session on 32 PRs; the first registration again: exit 0 with `replaced an earlier notify`, `subscriptions: 32`.
  - *Renamed:* two sessions (`session()` and `new_session("b")`) each register `notify octo/app#1 --until merged`. The fake renames the repository to `octo/app2`, and a poll sees it. `notify octo/app#1 --cancel` from the first session: `cancelled: 1`. The PR merges: the second session gets one notice holding `octo/app2#1 merged`, and the first gets none.
- **`notify-no-session`** (NTF-2, NTF-11, CLI-5). The command with `CLAUDE_CODE_MESSAGING_SOCKET` taken out of its environment: exit **11**, one stderr line holding `circitor wait octo/app#1 --until merged`, no request reached the daemon (`subscriptions: 0`, no new log line). With the variable naming a regular file in the harness's directory: exit 11, `no_session`. With a relative path: exit 2, `bad_request`. *A refused confirmation:* `notify --until merged` (registered), then `h.session().refuse()` and the same command again: exit **11**, `subscriptions: 0`: nothing is kept, and the subscription it was replacing is gone too (NTF-2). *The 33rd:* 33 settled open PRs; `notify --until merged` on each of 32 from the harness's session (`subscriptions: 32`), then on the 33rd: exit **10**, `limit`; the same from `new_session("b")`'s environment: exit 0. *The 65th:* a world of 66 settled PRs, the last one merged; `notify --until merged` on each of the first 64, 32 from each of two sessions (`subscriptions: 64`), then on the 65th from a third: exit **10**, `limit`; on the merged one, whose answer is already known: exit 0 with `already:`. Registering one of the 64 again is a replacement: exit 0.

- **`sleep`, its subscription sentence** (NTF-4, NTF-8, POL-22). Name the functions `scn_sleep_notify_*`, with `// TST-8: sleep`. World: a settled open PR scheduled to merge 30 minutes on; `notify --until merged --for 3600`. At 10 minutes `sleep_wall_only(2 h)`. *Wake poll answered:* no `expired` notice; the notice after the wake poll holds `merged`. *Wake poll failing* (GitHub answering 5xx from the wake on): nothing at the wake; one notice 60 s later holding `expired after 1h; it's open (stale, `.
- **`clock-back`, its subscription sentence** (NTF-4, POL-26). `scn_clock_back_notify`, with `// TST-8: clock-back`. `notify --until merged --for 600` on a settled open PR; at 60 s `set_wall_back(1 h)`: the `expired` notice comes at the first tick at or after 600 s of the monotonic clock, not an hour later. A step forward is the `sleep` row's (TST-8, D9): not played here.

## Interfaces
- **Consumes:** `testkit::harness::Harness` (`start(opts)` with config, `session()`, `new_session`, the environment variant, `cli`, `cli_bg`, `stop`, `crash`, `restart`, `exited()`, `log()`, `state_file()`, `github()`, `token()` (the `FakeTokenSource`, for `notify-problem`'s new token), `finish`), `testkit::session::FakeSession` (`notices()`, `lines()`, the four modes with `queue_full`, `restore`), `testkit::server::FakeGitHub`'s world controls (a rename, a node's error, 401s), `testkit::{MARKER_SESSION_TOKEN, MARKER_CHECK, MARKER_LOGIN}`, `testkit::server::FakeGitHub` (`world`, `requests()`), `clock::TestClock` (`advance`, `sleep_wall_only`), the JSON of `plan/contracts/api.md`.
- **Provides:** nothing.

## Out of scope
- The other TST-8 rows: T30 to T33. `fail-fast` is T30's and `changed-kind` T31's: they're about `wait`.
- The notifier's rules as unit tests (the 2 s limit, the 1 s at a stop, replacing, the memory of events): T46. The routes' strictness: T47. The command's lines one by one: T48. Here each is seen once, end to end.
- The exact text of every notice and of the command (TST-7): T46, T48, T35.
- What a real Claude Code session does with a notice (held, refused, shown): nothing automated can see it. T40's seventh check.
- The harness and the fakes: T29, T45, T20. A missing control is reported, not added.
- Fixing what a scenario finds in `src/`: the task named in the `#[ignore]`.

## Acceptance
- `tests/scn_notify.rs` starts with `#![cfg(feature = "test-endpoint")]` and holds every scenario above as `scn_<name>` or `scn_<name>_<part>`, each asserting everything its entry lists, with the entry's values.
- `cargo test --features test-endpoint --test scn_notify -- --list` shows at least one test for each of the ten names, and `scn_sleep_notify_*` and `scn_clock_back_notify`; `grep -c 'TST-8' tests/scn_notify.rs` is at least 12.
- No `sleep`, no `tokio::time`, no `std::thread` in the file. Every test ends with `finish()`: TST-11's sweep ran, for the GitHub token, for **the session's token** (NTF-9) and for a title, a check's name and a login in every posted notice (NTF-8), and so did the fake GitHub's rules (POL-1, POL-2).
- The whole file runs in under a minute of real time.
- `cargo test --features test-endpoint --test scn_notify` and `make verify` pass. An ignored test is listed under Open items with its reason.

## Traps
- **Never the real session socket.** The shell that runs you has `CLAUDE_CODE_MESSAGING_SOCKET` set. Every command here goes through the harness, whose `Env` names its own fake; no test reads the process's environment or runs the binary.
- **A scenario that fails because production code is wrong is kept:** mark it `#[ignore = "T46: <what's wrong>"]` with the owning task's ID, and report it. Don't weaken the assertion, change the world to fit, or touch `src/`.
- **Looking changes what you measure.** `list`, `show` and `changes` are client requests: they keep the tier active and push the idle exit away, and `show` makes a `checks_pending` PR hot for two minutes. Where an entry is about the tier, the idle exit or the cadence, look with `status` and the request log only.
- **`keep_done` and STA-1's 60 minutes go by the wall clock; `for`, `exit_after` and `idle_after` by the monotonic one, `for` with the sleep noticed added** (POL-26, NTF-4). A plain `advance` moves both; `sleep_wall_only` and `set_wall_back` tell them apart.
- **A subscription's end keeps its PR 60 minutes** (STA-1): a cancelled or ended subscription's PR isn't dropped at the next tick.
- **Every condition ends when its PR does, but `changed`.** To keep a subscription open on an ended PR, as `notify-keeps-ended` needs, register `--until changed` on a PR that has already merged. That subscription doesn't keep the tier active.
- **Registering and cancelling are client requests** (POL-13): a `notify --cancel` moves the tier and the idle exit as a `list` would. Where the end of a subscription is what's measured, end it by its PR or its `for`, never by a cancel.
- **`auth` starts at the repeat's 401** (POL-20), not at the first: take `t` from the second request's time in `h.github().requests()`.
- **`refuse` leaves the socket's file**, so the tick sees nothing wrong: only a post finds out. `vanish` is the one the tick sees.
- **`queue_full` fails a connect at once on Linux**, it doesn't hang (`plan/contracts/testkit.md`). Assert one failed post and nothing after it, not a 2 s wait.
- **The confirmation is posted before the command's answer**, but the command's future must be awaited before `notices()` is read: the test clock doesn't count a command's own socket I/O.
- An awaited `cli()` hangs when the command waits on the clock: a registration that reads through or refreshes waits out POL-2's 1 s gap. Use `cli_bg`, advance, then await.

## Open questions
- **Everything that posts is a (proposal)**, and the fake plays what docs/07 measured on Linux: a session that holds or refuses a notice looks, from here, like one that got it.
- **A killed session may leave its socket file** (NTF-4): not played here; `refuse` is the nearest.
- **NTF-5's cost** (the active tier, about 120 points an hour) is T34's kind of figure; no row asks for it. `notify-keeps-active` only shows the cadence.
- **`pr_failing`'s notice** is NTF-8's sentence (D9): `notify-problem` asserts it whole, `circitor: can't read octo/app#1 from GitHub (pr_failing); notify is cancelled. Details: circitor status`.
- D5 and D6 don't touch this task.

Follow-ups: T57, T60, T63, T65

## Evidence
- **2026-11-26**, branch `t49-scenarios-notify`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `tests/scn_notify.rs` only (no real environment read; every command through the harness's own fake session): 36 tests covering the ten notify rows and the subscription parts of `sleep` and `clock-back`, each ending in `h.finish()`. `cargo test --features test-endpoint --test scn_notify`: 35 passed, **1 ignored**: `scn_notify_session_gone_full_queue` (T45's `queue_full` makes a new socket file, so the post is never tried and the end is `session_gone`, not `reason=failed`): decided **D16**, follow-up **T57**. `make verify` passes on main after the merge.
- **Readings kept by the orchestrator** (TST-8: the requirement wins): the expired notice reads the record's `status` (NTF-8: `expired after 1m; it's ready`), the row's "it's open" being a slip; `notify-keeps-active` (ended request before) asserts `active` at the end and the end plus 250 s, and the idle exit at the end plus 300 s, as the row's numbers can't all be read in one run.
- Choices: each PR its own head SHA; `keeps-ended`'s 24 h-old merge made by merging then advancing 24 h; bare `notify` rows run with the PR in full.
