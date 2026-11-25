---
id: T12
title: "Token source: running gh auth token"
kind: code
milestone: M1
size: M
status: todo
depends_on: [T04]
requirements: [OPS-5:a-gh-path-with-a, OPS-5:a-read-has-three-outcomes, OPS-5:the-token-is-held-in-memory-only, U11, U13]
owns: [src/token/gh/]
shares: []
---

# T12 · Token source: running gh auth token

## Outcome
`token::gh::GhTokenSource` runs `gh auth token --hostname github.com` within 10 s of the clock, finds `gh` as OPS-5 says at every read, tells a token, no login and a failure apart, and never lets the token or `gh`'s output out of a `Token`.

## Context
T02 declared the `TokenSource` trait, `Token`, `TokenRead` and `GhFailure` in `src/token/`, with a fake for tests, and left `src/token/gh/` as a stub. This task builds the real one on T04's clock. T13's `Auth` decides when to read and what each outcome does to the daemon; T28 constructs your type for a real daemon; T36 runs it in real processes against a fake `gh`.

## Read first
- `plan/conventions.md`: "The token", "Time", "Work in flight", "Nothing blocks the runtime", "Tests" (never the machine's `gh`).
- `plan/contracts/github.md`: "Declarations (`src/token/`)" and the Semantics lines on `TokenRead` and `Token`. `plan/contracts/clock.md`: `clock::timeout`, and who holds an `Activity` guard.
- `specs/06-operations.md#the-token`: OPS-5, all of it, to know which bullets are yours.
- `specs/07-testing.md#what-is-tested`: TST-9's sentence on the fake `gh` script, and TST-11.
- `docs/02-github-api.md` §1: what `gh auth token` prints, with the variables set and with no login. `docs/04-platform.md` §2: a service's minimal `PATH`.
- `src/token/mod.rs`, `src/token/gh/` and `src/testkit/mod.rs` (`MARKER_TOKEN`, `ShortDir`) as T02 left them.

## Scope
- **`GhTokenSource`** implements `TokenSource`; `GhTokenSource::new` is declared in `plan/contracts/github.md` ("The real ones' constructors").
- **Finding `gh` (OPS-5, part `a-gh-path-with-a`).** At every read, not once:
  - a `gh_path` with a `/` is the only one tried
  - a name is looked for in the `PATH` of the `Env` given at construction, entry by entry (an empty entry is skipped), then in `/opt/homebrew/bin`, `/usr/local/bin` and `/home/linuxbrew/.linuxbrew/bin`, in that order; the first regular file with an execute bit wins. Never the directory of circitor's own binary.
  - The three directories are one constant (proposal: Homebrew's layout isn't in docs/04), and a crate-private constructor lets a test put its own list in their place.
  - Nothing found, or a spawn that fails, is `Failed(CantRun)`. `gh_path()` returns the path last run, or the configured value before any run.
- **Running it (U11, U13).** `gh auth token --hostname github.com`, those four arguments and no others: gh's login for github.com, one account, no authentication of circitor's own. The child gets **the `Env`'s variables and nothing else**, a null stdin, piped stdout and stderr, and is killed when its handle is dropped.
- **The 10 s limit** is `clock::timeout` around the whole read, the lookup included, so its timer is set at the read's first poll; the value is one constant. When it passes: kill the child, wait for it to be reaped, then answer `Failed(Timeout)`.
- **Three outcomes, told apart (OPS-5, part `a-read-has-three-outcomes`).**
  - `Token`: exit 0, and stdout, once one trailing newline is cut, is one non-empty line. Stderr is ignored.
  - `NoLogin`: a non-zero exit whose stderr contains `no oauth token found`. Stderr is searched for that and dropped.
  - `Failed`: everything else. `Exit(code)` for another non-zero exit; `Killed` when a signal ended it; `Timeout`; `CantRun`; `NoOutput` for exit 0 with anything but one non-empty line (nothing, only white space, two lines, bytes that aren't UTF-8). (interpretation: the spec names only the empty case)
  - What the daemon does with each outcome (keeping or dropping the token in hand, the `auth` state, its message) is T13's.
- **One read at a time.** Concurrent `read()` calls share one run of `gh` and get its outcome. The run belongs to its callers: **when every caller's future is dropped, the run is dropped and the child killed** (OPS-7's "a running `gh` is killed" depends on it), and the next `read()` starts a new one.
- **`kind()`**, fixed at construction from the same `Env`: a non-empty `GH_TOKEN`, else a non-empty `GITHUB_TOKEN`, else gh's login (docs/02 §1: an empty one is ignored).
- **Memory only (OPS-5, part `the-token-is-held-in-memory-only`: the reading side).** Stdout goes into a `Token` and nowhere else; neither stream's bytes reach a log, an error, a `GhFailure`, a `Debug` output or a panic message. This type has no logger: T13 logs a failed read from the `GhFailure` alone.
- **Work in flight.** Hold an `Activity` guard while `gh` runs, as the clock contract says: real I/O that ends without the clock moving. A `gh` that never exits is the test's to excuse.

## Interfaces
- Consumes: `token::{TokenSource, Token, TokenRead, GhFailure, TokenSourceKind}`, `config::Env`, `clock::{SharedClock, timeout, Activity}`.
- Provides: `impl TokenSource for GhTokenSource`, and `GhTokenSource::new(gh_path: &str, env: Env, clock: SharedClock, activity: Activity) -> Arc<GhTokenSource>` exactly as github.md declares it. T02 stubbed it and T28 calls it: fill the stub, don't change its signature.

## Out of scope
- When the token is read, the 10-minute and 401 reads, the `auth` state and its message: T13 (POL-3, POL-20, and OPS-5's parts `the-token-is-read-after` and `a-read-that-ends-without`).
- Starting a daemon without `GH_TOKEN` and `GITHUB_TOKEN`: T24. Showing the token source and the path in `status`: T21 and T22.
- `token::fake::FakeTokenSource` and `Token` itself: T02. The `Authorization` header: T07.
- The same outcomes in real processes, and the marker sweep: T36, T29 (TST-9, TST-11).
- Whether `gh auth token` works from a LaunchAgent: the owner's check, T40.

## Acceptance
Unit tests in `src/token/gh/`, each with fake `gh` shell scripts written into a `ShortDir`, on `TestClock`. The `Env` is built by hand, with a `PATH` that holds `/usr/bin:/bin` for the scripts' own commands:
- `ops5_a_token`: a script that warns on stderr and prints `MARKER_TOKEN` and a newline gives `Token`, whose `expose()` is the marker. `u13_asks_for_github_com_only`: the script's recorded arguments are exactly `auth token --hostname github.com`.
- `ops5_no_login`: exit 1 with `no oauth token found for github.com` on stderr and nothing on stdout gives `NoLogin` (docs/05, 2026-11-25).
- `ops5_a_failure`: exit 3 gives `Failed(Exit(3))`; a script that kills itself with `SIGKILL` gives `Failed(Killed)`; exit 0 with no output, with only a newline, and with two lines each give `Failed(NoOutput)`.
- `ops5_ten_seconds`: a script that never exits, its run held with `activity.excuse()`: the read is still pending after `advance(9.999 s)`; after one more millisecond the awaited read is `Failed(Timeout)`, and the script's process (its PID written to a file) no longer exists.
- `ops5_a_path_with_a_slash_is_the_only_one_tried`: `gh_path` names a missing file while a working `gh` is in `PATH`: `Failed(CantRun)`, and `gh_path()` is the configured path. The same for a file without an execute bit.
- `ops5_a_name_is_looked_up_in_path_then_the_fixed_places`: found in the second `PATH` entry; found in the test's stand-in for the fixed list when `PATH` has none; `CantRun` when nowhere.
- `ops5_looked_up_again_at_each_read`: after the first read the script is removed and another put in a later `PATH` entry: the second read runs that one, and `gh_path()` names it.
- `ops5_one_run_for_concurrent_reads`: three reads at once give three tokens, and the script's run counter is 1. `ops5_a_dropped_read_is_not_kept`: after the only caller is dropped mid-run, a new read makes the counter 2.
- `ops5_kind`: `GH_TOKEN` set; `GH_TOKEN` empty with `GITHUB_TOKEN` set; neither. `ops5_the_child_gets_the_given_environment`: a script that prints a non-empty `GH_TOKEN`, else `GITHUB_TOKEN`, else the marker (TST-9's script) returns a second marker put in `Env`; and a script that dumps its environment shows no `CARGO_MANIFEST_DIR`, which the test process has.
- `ops5_nothing_of_gh_gets_out`: with scripts that print the marker on stdout and stderr **before** failing in each way above (TST-11), `format!("{:?}")` of every `TokenRead` holds no marker; and of a `Token`, only `Token(…)`.
- `cargo test --features test-endpoint --lib token::gh::` and `make verify` pass.

## Traps
- **Never run the machine's `gh`:** every test sets `gh_path` or `PATH` to its own script. A test that passes only where `gh` is installed is wrong.
- **A hung `gh` keeps the test clock unsettled:** you hold a guard while it runs, and `advance` settles first. The test that hangs takes `activity.excuse()` for as long as the script hangs (clock.md: "a fake `gh` that never exits is excused by the test"), and awaits the read itself after the last `advance`: the kill and the reap take real time the clock no longer waits for.
- **A script that hangs must `exec` its sleep.** Killing a shell that started `sleep` as a child leaves the `sleep` behind.
- **`tokio::process`, never `std::process::Command`.** Read stdout and stderr together with the wait: a child that fills one pipe while you read the other never exits.
- **A spawned task that owns the run outlives its callers.** Share the future among the callers instead (`futures`' `Shared`, held weakly by the source), so dropping them drops the child.
- **"Text file busy":** a script written and run at once can fail to start when another test thread forks in between. Write scripts through one helper that closes the file before returning and retries a spawn that fails that way. In the test helper only: production code reports `CantRun`.
- **`String::from_utf8_lossy` plus a format string is how a token leaks.** Convert once, strictly; on failure answer `NoOutput` and drop the bytes.
- Stderr's phrase is matched as gh writes it, case and all (docs/02 §1, gh 2.101.0). Don't match on the exit status alone: gh also exits 1 for other failures.
- `Env::get` returns `None` for an empty variable: use it for `kind()`, and pass the variables on as they are.
- Process and file lookups differ between Linux and macOS only in paths: no `cfg(target_os)`.

## Open questions
- **OPS-5 is a (proposal)** where it names Homebrew's directories, and whether `gh auth token` works from a service on macOS, or what it answers while the Keychain asks, isn't checked (docs/04 §8 items 2 and 4). Build as written; TST-13 step 6 checks it on the owner's Mac.
- **What counts as `NoOutput`** beyond empty output is this brief's reading: list it under Open items.
- D5 and D6 don't touch this task.
