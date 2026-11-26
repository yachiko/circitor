# Conventions

Every executor reads this file in full, then its task, then what the task's "Read first" lists. Everything here binds every task.

## Authority

- **`specs/`** decides behaviour. **The facts docs** (`docs/02-github-api.md`, `docs/03-clients.md`, `docs/04-platform.md`) decide what GitHub, the clients and the platforms do; where a spec's copy of a fact differs, the facts doc wins. **`plan/contracts/`** decides interfaces. **The task** decides scope.
- A conflict, a gap or a rule that can't be built as written is an **open item in your report**. Don't resolve it by choosing, and never by changing a test, a contract or a spec.
- **Never edit `specs/`, `docs/` or `plan/`.** Write only inside your task's `owns` paths, and its `shares` paths exactly as the task describes.
- `plan.py show ID` prints one requirement: `python3 -I .claude/skills/build-plan/scripts/plan.py show POL-12`.

## Layout

One package, `circitor`: a library (`src/lib.rs`) and one binary (`src/main.rs`). The module tree is fixed by T02; **every parent `mod.rs` is T02's and is final**. A task owns leaf directories and adds files and submodules inside them only.

| Path | Holds | Spec |
|---|---|---|
| `src/version.rs`, `build.rs` | The version and commit (OPS-11) | 06 |
| `src/clock/` | `Clock`, `Time`, `Mono`, the system and test clocks, `Activity`, `SleepWatch` | 07, 02 |
| `src/log/`, `src/status/` | `Logger`; the `Warnings` registry | 06, 04 |
| `src/model/` | The record, checks, status, events, items; the merge of a read into a record | 01 |
| `src/github/` | The `GitHub` trait and its types; `client/` the real client; `fake.rs` the scripted fake | 02 |
| `src/token/` | The `TokenSource` trait, `Token`; `gh/` runs `gh auth token` | 06 |
| `src/config/` | `Config`, loading and checking it; `Env`, `Platform`, `Paths` | 06 |
| `src/state/` | `store/` the tracked set, revisions, events, retention; `file/` `state.json` | 03 |
| `src/poller/` | `limits/` quota and backoff; `cadence/` tiers, periods, windows; `auth/` the token's life; `engine/` the loop | 02 |
| `src/api/` | `types.rs` the JSON both sides share; `condition.rs` a condition, checked and evaluated; `server/` routes; `wait/` waiters; `notify/` the two notify routes | 04, 08 |
| `src/transport/` | The HTTP client over the Unix socket, and the socket probe | 04, 06 |
| `src/cli/` | `args/`, `naming/`, `guide/`, `render/`, `conn/` (connect, spawn, replace), `commands/`, `notify/` (the `notify` command) | 05, 08 |
| `src/daemon/` | `instance/` the lock, the socket, the directories; `run/` start, stop, wiring | 06, 03 |
| `src/notify/` | Subscriptions, the loop that evaluates them, the poster, `text.rs` the notices' words | 08 |
| `src/testkit/` | Only with `test-endpoint` or under `cfg(test)`: `github/` and `server/` the fake GitHub; `session/` the fake session socket; `harness/` | 07 |
| `tests/` | Integration tests, one file or directory per task; `tests/golden/`; `tests/cost/` | 07 |
| `testdata/` | `github/` (schema, recorded and synthetic responses), `api/`, `state/`, `config/` | 07 |

## Toolchain and commands

- Rust **1.98.1**, pinned in `rust-toolchain.toml`; edition 2024.
- **`make verify`** must pass before you report. It runs, in order: `cargo fmt --all --check`; `cargo clippy --all-targets --features test-endpoint -- -D warnings`; `cargo clippy --lib --bins -- -D warnings` (the release configuration, without the feature); and `cargo test --features test-endpoint`. **It checks only the machine it runs on:** macOS is verified by `release.yml` at a release tag (P9).
- One module's unit tests: `cargo test --features test-endpoint --lib model::`. One integration file: `cargo test --features test-endpoint --test scn_checks`.
- **A test build** is one with the `test-endpoint` feature (TST-1). Every file under `tests/` starts with `#![cfg(feature = "test-endpoint")]`; `tests/real/` is also declared with `required-features` in `Cargo.toml`.
- Each worktree has its own `target/`. Don't set `CARGO_TARGET_DIR`.

## Code rules from the specs

- **Read-only towards GitHub (POL-1).** No code builds a mutation. Every byte sent to GitHub goes through `github::GitHub::send`; nothing else opens a connection to anything but the daemon's Unix socket, **with one exception: `notify::Poster::post`, which writes one notice to the session socket a subscription named** (OPS-16, NTF-6). It never reads from it.
- **The token (OPS-5).** It lives in a `token::Token` and nowhere else: never in a `String` that outlives the call, a log line, an error, a file, an API response or a panic message. `Token` has no `Display`, and its `Debug` prints a fixed word. **A session's token (NTF-9) is kept the same way**, in a `notify::SessionToken`, for as long as its subscription lasts.
- **Time (TST-2, TST-4, POL-26).** All time comes from the injected `clock::Clock`: no `Instant::now()`, `SystemTime::now()`, `tokio::time::sleep`, `tokio::time::timeout` or `chrono::Utc::now()` outside `src/clock/`. Wall time is `Time`, monotonic time is `Mono`; never convert one to the other. POL-26 says which clock each duration goes by.
- **Work in flight (TST-4).** A loop takes `Activity::begin()` as the first thing after it wakes (before its next `.await`) and drops the guard only when it parks again. A held waiter isn't work in flight. Without this the test clock can't tell when the daemon has settled.
- **Logging (OPS-9).** Only through `log::Logger`; no `println!`, `eprintln!` or logging crate. A line holds names, numbers, states, costs and error `type`s that circitor's own code chose or GitHub's enums name: **never a title, a message from GitHub, a body, `gh`'s output, the token, a session's token or a notice's text**. `plan/contracts/log.md` says which task logs which event.
- **Unknown input.** From GitHub: never refuse an unknown field or enum value (MOD-2). From a client: an unknown parameter, key or value is `bad_request` (API-1). From the config file: an unknown key stops the daemon (OPS-1). From the state file: unknown keys are ignored, the `version` decides (STA-8).
- **JSON names** are `snake_case`; times are RFC 3339 UTC in whole seconds; durations are whole seconds (specs/README).
- **Files (STA-12).** The daemon writes nothing but the files STA-12 lists, and only through `state::file` (the state file), `daemon::instance` (the lock, the socket, the directories) and the log sink.
- **Errors.** Typed errors (`thiserror`) at every seam; no `anyhow`. No `unwrap()` or `expect()` outside tests, except on an invariant the line above explains. A panic is a crash (exit 1, OPS-7).
- **`unsafe`** is denied crate-wide. The one allowed use is the spawner's `pre_exec` (T24), with a comment.

## Platform rules

- **Written on Linux, shipped to macOS first.** Platform differences are values, not `cfg`: code takes a `config::Platform` and both branches compile and are tested on Linux. Where only `cfg(target_os = "macos")` will do, keep it to one small function and say in your report what couldn't be compiled.
- Paths come from `config::Paths`. Nothing reads `HOME` or an XDG variable itself.
- **A Unix socket path is at most 103 bytes** (OPS-4). That's the daemon's own socket: a session's socket path is Claude Code's, and has no rule (NTF-2). Tests make their directories under `/tmp` with a short name, never under `std::env::temp_dir()` (macOS's is long).
- **Nothing blocks the runtime:** no `std::thread::sleep`, no `std::process::Command`, no blocking file I/O on a hot path (`tokio::process`, `tokio::fs` or `spawn_blocking`). **No lock is held across an `.await`**: the store's lock is a `std::sync::Mutex`, taken and released inside one function.
- A dropped future is a cancelled request. Whatever must be undone when a client goes away is undone in `Drop`.

## Requirement tracing

- A `// STA-6:` comment at the code that implements a rule, and above each test that proves it. The ID goes in the test's name too: `sta6_no_title_in_the_file`.
- `grep -rn 'STA-6' src tests` must find both.

## Tests

- **Hermetic (TST-1).** Loopback and the Unix socket only. Never `api.github.com`, never the machine's `gh` or its login, never a real token: use `github::fake::ScriptedGitHub`, the fake GitHub server, `token::fake::FakeTokenSource` and a fake `gh` script. The test token is the marker `testkit::MARKER_TOKEN`. **Never a real Claude Code session's socket either:** your shell has `CLAUDE_CODE_MESSAGING_SOCKET` set, and a test that posts to it puts text into the session that runs you. Tests post to `notify::fake::RecordingPoster`, to `testkit::session::FakeSession`, or to a listener they bind in a `testkit::ShortDir`; nothing reads that variable but through a `config::Env` the test built.
- **Deterministic (TST-2).** The test clock, a fixed seed, no wall-clock sleeps. Only `tests/real/` (real processes) waits on real time, with a 30 s limit per test.
- **Runtime.** Tests on the test clock use `#[tokio::test]` (the current-thread runtime): settling depends on it. Never `start_paused` or `tokio::time::pause`.
- **Golden files** live in `tests/golden/` and `testdata/`. They're rewritten only by running the tests with `CIRCITOR_UPDATE_GOLDEN=1`, then read by you line by line against the spec. Never edit one by hand to make a test pass.
- **A test that fails because another task's code is wrong:** keep the test, mark it `#[ignore = "T21: <what's wrong>"]`, and list it under Open items. Don't bend the test, and don't fix code outside your `owns`.
- Unit tests sit beside the code (`#[cfg(test)] mod tests`); a module's testdata sits in `testdata/<area>/`.

## Dependencies

Only the crates T01 declared in `Cargo.toml`. A new crate, feature or version is a plan decision: report it, don't add it. `Cargo.toml` and `Cargo.lock` are T01's.

## Git

Work in your worktree. No commits, no pushes, no pull requests: the orchestrator commits.

## Report

End with this, filled in, and nothing else:

```markdown
**Task:** Tnn · done | partly done | blocked
**Changed paths:** one per line
**Commands run:** each command, with its result (pass, or the failure in one line)
**Requirements:** each ID from the frontmatter → where it's implemented (path and function) and the test that proves it, or "not done: why"
**Deviations:** what was built differently from the brief or the contracts, and why; or "none"
**Contract changes needed:** the change, and which tasks it affects; or "none"
**Open items:** checks not run, spec gaps, ignored tests, questions for the orchestrator; or "none"
```
