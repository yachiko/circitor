---
id: T65
title: "Follow-up to T24, T27, T28 and T46: who started the daemon, and what a stop means (D19)"
kind: code
milestone: M5
size: M
status: todo
depends_on: [T24, T27, T28, T29, T46, T49, T62, T63]
requirements: [CLI-7:the-cli-says-who-starts-it, CLI-8:stop-stops-the-daemon-that-runs, CLI-3:refresh-prints-refresh, CLI-11, OPS-7, OPS-9:logged, NTF-8:ask-again-repeats-the-parameters-given, NTF-10, TST-8, TST-18, TST-7]
owns: [src/cli/commands/, src/cli/conn/, src/cli/render/, src/cli/guide/, skills/, AGENTS.section.md, README.md, src/notify/text.rs, src/notify/text/, tests/golden/notify/, tests/golden/render/, tests/golden/cli/, tests/golden_cli.rs, tests/scn_notify.rs, tests/spawner.rs, src/daemon/, src/testkit/harness/]
shares: []
---

# T65 · Follow-up to T24, T27, T28 and T46: who started the daemon, and what a stop means (D19)

## Outcome
- A spawned daemon's `start` line names the CLI that started it: `spawner_pid=`, `spawner_cmd=`, `spawner_cwd=`, `spawner_session=` (OPS-9).
- `stop` says the next command starts a daemon again (CLI-8), and the guide and README tell timed commands to pass `--no-spawn` (CLI-11).
- The `stopped` notice's command carries `--since` only for `until changed` (NTF-8).
- Nothing else changes: D1's auto-start stays as it is.

## Context
The owner's field test (2026-11-26, `notify-findings.md` issue 2): a mod polling `show` every 20 s restarted the daemon 4 s after `circitor stop`, and the log couldn't say who did it. The `stopped` notice's re-ask then registered with that daemon after a `reset: cursor too old`, because its `--since` was from the stopped daemon's epoch. D19 keeps the behaviour and fixes the words and the log.

## Read first
- `plan/conventions.md`; `specs/decisions.md`: D19.
- `specs/05-cli.md`: CLI-7 ("The CLI says who starts it"), CLI-8 ("`stop` stops the daemon that runs"), CLI-3's `stop` output, CLI-11.
- `specs/06-operations.md`: OPS-7, OPS-9's "Logged". `specs/08-notify.md`: NTF-8's `Ask again`, NTF-10. `specs/07-testing.md`: TST-8 `notify-stop`, TST-18.
- `plan/contracts/cli.md`: "`CIRCITOR_SPAWNER`". `plan/contracts/daemon.md`: `Deps::spawner`.
- Code: `src/cli/conn/mod.rs` (`start`), `src/cli/conn/spawn.rs` (`daemon_command`, `WITHHELD`), `src/cli/commands/mod.rs` (`stop`), `src/daemon/mod.rs` (`Deps`, `main`), `src/daemon/run/start.rs` (the `start` line, around line 360), `src/notify/text.rs` (`stopped`), `src/testkit/harness/mod.rs` (how the harness builds `Deps` from the `FakeSpawner` closure's `Env`).

## Scope
- **The spawner (CLI-7):** `conn::start` puts `CIRCITOR_SPAWNER` in the `Env` it hands to `Spawner::spawn`, as `contracts/cli.md` says. Bring the command's name to `conn`, and name the choice in your report: the orchestrator writes it into the contract (executors don't edit `plan/`). `RealSpawner` passes the variable on.
- **The daemon (OPS-9):** `daemon::main` reads `CIRCITOR_SPAWNER` into `Deps::spawner` in spawned mode only, and removes it from its own environment. The `start` line ends with `spawner_pid=… spawner_cmd=… spawner_cwd=… spawner_session=…`: each field only when known, and a control character in a value written as `?`. A foreground daemon and a spawned one without the variable log none. The harness fills `Deps::spawner` from the `Env` its `FakeSpawner` closure receives.
- **`stop` (CLI-8, CLI-3):** after it stops a daemon, it prints `stopped; the next circitor command without --no-spawn starts it again`. `not running` is unchanged.
- **The `stopped` notice (NTF-8):** `text::stopped` writes `--since` only when `until` is `changed`. `--rerun-since` and the other parameters stay as given. `expired`'s `Ask again` is unchanged.
- **Words (CLI-11, OPS-7):** one bullet in the guide (`guide.md`, `skills/circitor/SKILL.md`, `AGENTS.section.md`): a command run on a timer (a poller, a hook, a mod) passes `--no-spawn` or sets `CIRCITOR_NO_SPAWN=1`, or it starts the daemon again after a `stop`. The README's "Stopping" section and its `circitor stop` line in "Using it" say the same.
- **Tests:**
  - `tst18_start_line_names_the_spawner` in `tests/spawner.rs`, through the harness. It sets `CLAUDE_CODE_SESSION_ID` and a working directory whose name holds a line break, then checks the four fields with `?` for the break, all on one line. `tst18_foreground_start_names_no_spawner` checks the other case. TST-11's sweep runs over that log.
  - A unit test of `daemon_command` showing `CIRCITOR_SPAWNER` is passed on, and the session's socket and token are still withheld.
  - Golden files (TST-7): `tests/golden/cli/stop_stopped.out`; `notice_stopped.notice` and `tests/golden/notify/stopped*` for `merged`, `checks-done` (no `--since`) and `changed` (with it).
  - TST-8 `notify-stop` in `tests/scn_notify.rs`: the notice's command has no `--since` for `checks-done`, and the re-ask registers with no `reset`.

## Out of scope
- A stop that sticks (`stop --hold`, a marker): D19 left it out.
- The 8.9 s re-ask: no change (D19's note).
- `status` showing the spawner: not specified.
- Recording `CLAUDE_CODE_SESSION_ID` in docs/07: T64's.

## Acceptance
- `cargo test --features test-endpoint --test spawner --test golden_cli --test scn_notify` passes. So do the unit tests in `src/cli/conn/`, `src/cli/commands/`, `src/notify/text/` and `src/daemon/run/`.
- `grep -rn 'CIRCITOR_SPAWNER' src` shows it set only in `conn` and read only in `daemon`.
- `make verify` passes.

## Open questions
- CLI-7's use of `CLAUDE_CODE_SESSION_ID` is a (proposal) until T64 records the variable in docs/07. Build it as written: a variable that isn't set just leaves `session=` out.
