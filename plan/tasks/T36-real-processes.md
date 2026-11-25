---
id: T36
title: "M4: real processes: spawning, two CLIs, upgrades, signals, files and permissions"
kind: integration
milestone: M4
size: L
status: todo
depends_on: [T20, T27, T28]
requirements: [TST-9, OPS-17]
owns: [tests/real/]
shares: []
---

# T36 · M4: real processes: spawning, two CLIs, upgrades, signals, files and permissions

## Outcome
The built binary, in a test build, passes TST-9: the CLI starts a daemon that outlives it, two CLIs leave one daemon, a stale socket is replaced, stops leave no socket and a valid state file, an older daemon is replaced, the fake `gh`'s outcomes are told apart, modes and limits hold, and OPS-17's figures are met.

## Context
Everything before this ran in one process on a test clock. This task runs **the built binary as real processes, on real time**: the CLI spawning a detached daemon, signals, the lock and the socket on a real file system, a real `gh` child (a fake script). It's the M4 checkpoint, and the only suite that waits on the wall clock (TST-2). It runs on Linux here; its macOS run is CI's (T03, T38).

It builds on **the binary** (T28's `daemon::run::main`, T27's `cli::main`), **T20's `FakeGitHub::start_real`**, `ShortDir` and the markers (`testkit`, T02), and `daemon::instance`'s `read_holder` and `is_held` (T18, through T28). **It doesn't use T29's harness**, which runs everything in one process on the test clock. T25 (hot and forced refreshes, reading through) and T42 (`gone`) are usually merged by now, but they aren't dependencies: see Traps.

## Read first
- `plan/conventions.md`: Tests (hermetic; `tests/real/` and its 30 s limit), Platform rules, Report.
- `plan/contracts/testkit.md` (`FakeGitHub::start_real`, what `requests()` holds, `ShortDir`, the markers, `MARKER_TOKEN_2`), `plan/contracts/log.md` (the event words), `plan/contracts/daemon.md` (`main`, the lock file's line, `instance::read_holder`, `is_held`), `plan/contracts/cli.md` (the real spawner, exit codes), `plan/contracts/config.md` (the paths per platform, the environment variables), `plan/contracts/transport.md` (the probe).
- `tests/real/main.rs` (T01's placeholder), `Cargo.toml`'s `[[test]] name = "real"`, and the module doc of `src/testkit/server/`.
- `specs/07-testing.md#what-is-tested`: TST-9 with its sub-bullet, TST-11; `#rules`: TST-1, TST-2.
- `specs/05-cli.md#the-daemon`: CLI-7 to CLI-10; CLI-5, CLI-6. `specs/03-state.md#one-daemon`: STA-11, STA-12; STA-7.
- `specs/06-operations.md`: OPS-1, OPS-3 to OPS-9, OPS-17.
- `docs/02-github-api.md` §1 (what `gh auth token` prints and how it fails). `docs/04-platform.md` §4 (socket path lengths, the directories).

## Scope
`tests/real/`: `main.rs` (replacing T01's placeholder) and modules of your choosing. Every test of TST-9's list, and OPS-17's figures.

- **The fixture every test uses** (TST-9's sub-bullet), one `ShortDir` each:
  - the binary: `env!("CARGO_BIN_EXE_circitor")`, a test build (TST-1)
  - a cleared environment, then only: `HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_RUNTIME_DIR`, `GH_CONFIG_DIR` (all inside the directory), a plain `PATH`, `CIRCITOR_TEST_GITHUB` (the fake's URL), and what the test adds
  - `config.toml` with `gh_path` set to a **fake `gh` script** and `exit_after` of a few seconds (longer where a test needs the daemon to stay)
  - the fake `gh`: a `/bin/sh` script that prints a non-empty `GH_TOKEN`, else `GITHUB_TOKEN`, else `MARKER_TOKEN`, as gh does (docs/02 §1); it also prints the marker on stderr (TST-11). Modes, chosen by a file beside it: exit 1 with `no oauth token found for github.com`; exit 3; never exit (recording its PID)
  - the fake GitHub in the test process, on real time: **`FakeGitHub::start_real(SystemClock::new())`**. Its POL-2 rule runs on real time and its per-daemon rules are held **per connection**: it can't tell processes apart otherwise (plan decision P10), so a replaced daemon and its successor don't trip each other
  - **a 30 s limit**, and a guard that, also on failure, stops every daemon named in the directory's `circitor.lock` (TST-9)
  - **TST-11's sweep at the end:** `circitor.log` and its old copy, `state.json`, and every stdout and stderr the test captured hold no `MARKER_TOKEN`; the log and the state file hold neither GitHub marker.
- **The tests** (TST-9, each with the requirement it proves):
  1. `list` with no daemon exits 0 and **the daemon lives on** (CLI-7): the PID in the lock file is alive, and the lock is held.
  2. **The CLI's stdout, read to its end, closes when the CLI exits** while the daemon lives on (CLI-7): a reader of the pipe sees EOF within 1 s of the exit.
  3. **Two CLIs at once leave one daemon** (CLI-7, STA-11): both exit 0; one lock holder; one more `list` connects to it.
  4. **A stale socket file is replaced** (OPS-4): `SIGKILL` the daemon, the socket file stays; the next `list` exits 0 on a new daemon.
  5. **`stop` and `SIGTERM` each leave no socket and a valid state file** (OPS-7, CLI-8): the path is gone, `state.json` parses with `version: 1`, the lock is free. A foreground daemon the test started exits 0 on `SIGTERM`.
  6. **The idle exit** (OPS-8): a spawned daemon with `exit_after = 2` is gone within 10 s of its last request; a foreground one isn't.
  7. **An older daemon is replaced and a waiting CLI reconnects** (CLI-9, CLI-10): daemon and waiter with `CIRCITOR_TEST_VERSION=0.1.0`; a `list` with `0.2.0` replaces it; the waiter starts none, reaches the new one, and exits 0 when the PR merges in the fake's world.
  8. **After a `stop` a waiting CLI starts none** (CLI-10, OPS-7): it ends `daemon stopped`, exit 6, and no lock is held.
  9. **A daemon killed with `SIGKILL` while a CLI waits** (CLI-10): the same ending.
  10. **A spawned daemon uses gh's login** (OPS-5, CLI-7): the CLI runs with `GH_TOKEN` set to `MARKER_TOKEN_2`; every `Authorization` value in the fake's `requests()` is `MARKER_TOKEN`.
  11. **A bad config exits 6 within 1 s** (CLI-7, OPS-1): an unknown key; stderr holds the OPS-1 line naming it.
  12. **The stop's 5 s deadline** (OPS-7): with `CIRCITOR_TEST_STOP_DELAY=30`, which holds the stop after the socket is removed and inside the deadline: the socket is gone at once, the process 5 s to 7 s after `stop` was sent, and the log holds a `stop deadline` line.
  13. **A foreground daemon takes over a spawned one** (OPS-6): within 15 s `status` reads `mode: foreground` and the spawned process is gone; a second foreground daemon exits non-zero, naming the holder's socket.
  14. **`stop` on a daemon sent `SIGSTOP`**, with `CIRCITOR_TEST_CLI_LIMIT=2` (CLI-8): no answer in 2 s, `SIGTERM`, `SIGKILL` 5 s later; the process is dead, `stop` exits 0.
  15. **A log that can't be written** (OPS-9): a foreground daemon whose stderr is a pipe with its reading end closed keeps serving, and `status --json` holds `log_unwritable` (plan decision P10).
  16. **The token reader tells the fake `gh`'s outcomes apart** (OPS-5): no login gives exit 7 naming `gh auth login`; exit 3 gives exit 7 naming `gh auth token`, the status and `gh_path`; a `gh` that never exits is killed after 10 s (its recorded PID is dead) and the message names the timeout.
  17. **A socket path over 103 bytes is refused** by the CLI and by the daemon, naming `CIRCITOR_SOCKET`; **a regular file at the socket path** stops the daemon, naming the path (OPS-4).
  18. **Modes** (OPS-3, OPS-4, STA-7): the state directory and, on Linux, the runtime directory are 0700; an existing 0755 state directory becomes 0700; the socket and `state.json` are 0600 (the daemon binds under a umask of 0177; OPS-4's "077" is in the plan's spec issues: assert the mode, not the umask). And STA-12: the state directory holds nothing but the files it lists.
  19. **Exit codes 130 and 143** (CLI-5, CLI-6): a held `wait` sent `SIGINT` exits 130, sent `SIGTERM` 143, each within 1 s.
- **OPS-17**, with 200 tracked PRs on the fake GitHub:
  - the daemon's resident memory is **under 30 MB** (`VmRSS` in `/proc/<pid>/status` on Linux, `ps -o rss= -p` on macOS), read after two polls
  - a `list` answered from memory **returns within 50 ms**: the median of 11 runs of the CLI process, start to exit
  - a cold start to the first `list` answer is within 5 s
  - on Linux, between two polls the daemon's CPU time (`utime + stime` in `/proc/<pid>/stat`) grows by at most 2 clock ticks over 5 s.

Interpretation: OPS-17's figures are measured on the test build, which is unoptimised. If one fails there and passes on `cargo build --release`, report both numbers; don't loosen the figure.

## Interfaces
- **Consumes:** the binary (T27, T28); `testkit::server::FakeGitHub::start_real` (T20), its `world`, `requests()` (the `Authorization` values among them) and `check()`; `testkit::ShortDir`, `MARKER_TOKEN`, `MARKER_TOKEN_2`, the GitHub markers; `daemon::instance::read_holder` and `is_held` to find and check a daemon.
- **Provides:** `tests/real/`, which CI runs on Linux and macOS (TST-14).

## Out of scope
- The same behaviours in-process on the test clock: T24 (`conn` against a stub), T28 (`m2_serve`), T29 (`m3_cli`), T33 (`crash`, `files-removed`). Here only what a real process, signal or file system adds.
- The CLI's text as golden files (TST-7): T35. This suite matches on a line's key words.
- Running it on macOS, and fixing what only macOS shows: T38, and follow-up tasks.
- A real `gh`, a real token, launchd, Homebrew, a lid close: T40, by hand.
- `Cargo.toml`: T01 declared the `real` test. Report a missing declaration; don't edit it.

## Acceptance
- `tests/real/main.rs` starts with `#![cfg(feature = "test-endpoint")]`. Tests 1 to 19 and OPS-17's four each exist, named with their requirement (`tst9_two_clis_one_daemon`, `ops17_resident_memory_under_30_mb`), with `// TST-9:` and the proved ID above.
- Each test fails by itself after 30 s. After any test, passed or failed, the process its `circitor.lock` named is dead (the guard asserts it), and `pgrep -fa 'circitor daemon'` shows the same processes after the suite as before it.
- The suite passes five times in a row: `for i in 1 2 3 4 5; do cargo test --features test-endpoint --test real || break; done`.
- The whole suite runs in under three minutes with the default test threads.
- No test reads the machine's `gh`, its login, `~/.config` or the real `HOME`: `grep -n 'env_clear' tests/real` finds the one place every child is started.
- Platform branches are run-time (`cfg!(target_os = "macos")` or `config::Platform`), and the report lists what couldn't be run on Linux.
- `cargo test --features test-endpoint --test real` and `make verify` pass. An ignored test is listed under Open items.

## Traps
- **A test that fails because production code is wrong is kept:** mark it `#[ignore = "T24: <what's wrong>"]` with the owning task's ID, and report it. Don't add retries or longer waits until it passes.
- **A spawned daemon isn't the test's child.** It's in its own session (CLI-7): find it by the PID in `circitor.lock`, never by waiting on a child. Its exit status can't be read; a foreground daemon the test starts itself gives one.
- **Find log lines by their event word** (`plan/contracts/log.md`: `stop cause=…`, `stop deadline`, `start`), never by a sentence.
- **Never sleep for a fixed time and hope.** Poll a condition every 20 ms (the socket answers, the lock is free, a PID is gone) up to a limit, and fail with what was seen.
- **Clear the environment.** An inherited `GH_TOKEN`, `XDG_RUNTIME_DIR`, `CIRCITOR_SOCKET` or `CIRCITOR_LOG` from the developer's shell changes what's tested, and a real token would reach the fake.
- **macOS has no runtime directory, a long `$TMPDIR`, no `/proc` and no `/dev/full`** (docs/04 §4). Use `ShortDir` under `/tmp`, `ps` for memory, a closed pipe for the unwritable log. Rust ignores `SIGPIPE`, so the write fails with an error, which is what OPS-9 needs.
- **`SIGSTOP`ped processes take `SIGTERM` only when continued:** test 14 ends by `SIGKILL`. The guard must `SIGKILL` too, or a stopped daemon outlives the suite.
- **No test leans on T25 or T42**, which aren't dependencies. No test needs a read-through (POL-16) or `gone` (MOD-11): every PR a test names is the fake login's own, found by the first poll, and named only after a `list` shows it. Tests 7, 8, 9 and 19 hold waits; only test 7 needs a refresh to end its wait, so its config sets `interval = 10`: a poll sees the merge in time with or without T25's hot refreshes. A test added later that needs a read-through or `gone` says so here.
- **POL-2 holds on real time:** the first poll and a found PR's first refresh are a second apart. A 200-PR world takes about 8 s before `list` is answered from memory: OPS-17's tests need most of their 30 s.
- **Tests run in parallel:** each has its own directory, socket and fake GitHub. Nothing is shared but the binary.
- **The fake `gh` must be executable and start with `#!/bin/sh`**; write it, set mode 0755, and close the file before running it (a file still open for writing fails with "text file busy").
- The test process's own runtime may be multi-threaded here: nothing settles on a test clock.

## Open questions
- **TST-3 says the fake's checks "in TST-9 hold per process"; the fake holds them per connection** (plan decision P10: nothing in a request names its process). If a test with two daemons trips POL-2's rule falsely, report it against T20; don't switch the rule off. Never use `FakeGitHub::start` with a test clock driven from a timer here: the 1 s rule would be measured in steps.
- **OPS-17 is (proposal)**, and measured here on a debug build on shared runners: the 50 ms and the 30 MB may be noisy. Report the measured numbers either way.
- **CLI-7's and CLI-8's limits, OPS-7's 5 s and STA-11's 6 s are (proposal).** Test them as written, with a second of slack above each and none below.
- **macOS results are unknown until T38.** Nothing in docs/04 was checked on a Mac.
- D5 and D6 don't touch this task.
