---
id: T01
title: "Scaffold: the package, every dependency, the toolchain, make verify"
kind: infra
milestone: M1
size: S
status: done
depends_on: []
requirements: [OPS-11, TST-1, U12]
owns: [Cargo.toml, Cargo.lock, build.rs, rust-toolchain.toml, rustfmt.toml, Makefile, .gitignore, CLAUDE.md, src/lib.rs, src/main.rs, src/version.rs, tests/real/]
shares: []
---

# T01 · Scaffold: the package, every dependency, the toolchain, make verify

Follow-ups: T50

## Outcome
An empty `circitor` package builds, lints and tests with `make verify`: every dependency the plan needs is declared and locked, the `test-endpoint` feature exists, and the binary knows its version and commit (OPS-11). No later task edits `Cargo.toml`, `Cargo.lock` or the Makefile.

## Context
The first task: the repository holds only `specs/`, `docs/`, `plan/` and a root `CLAUDE.md`. T02 builds the module tree on this package, T03 runs its `make verify` in CI, and every later task compiles against the dependencies locked here. **Nobody can add a crate after you** (conventions, "Dependencies"), so the list below is the whole plan's.

## Read first
- `plan/conventions.md`: "Layout", "Toolchain and commands" (what `make verify` runs, in order), "Dependencies", "Tests" (what a test build is).
- `specs/06-operations.md#shipping`: OPS-11 (the version string), OPS-13 (the four targets).
- `specs/07-testing.md#rules`: TST-1, the `test-endpoint` feature and what the release build must not hold.
- `specs/00-overview.md#technology`: the library list the plan confirms (U12).
- `CLAUDE.md` at the root: you add one section to it.

## Scope
- **`Cargo.toml`.** One package, `circitor`, version `0.1.0` (plan decision: the spec is draft 0.1), edition 2024, `rust-version = "1.98"`. A library (`src/lib.rs`) and one binary, `circitor` (`src/main.rs`): **Rust, one binary** is U12's share here; Homebrew is T39's.
- **Dependencies**, each at its latest compatible version, pinned by committing `Cargo.lock`:
  - `tokio` (`rt-multi-thread`, `macros`, `net`, `time`, `process`, `signal`, `sync`, `io-util`, `fs`)
  - `axum` 0.8; `hyper` 1 (`client`, `server`, `http1`); `hyper-util` (`tokio`, `server`, `server-auto`, `server-graceful`, `service`, `http1`); `http-body-util`; `http`; `bytes`; `tower` (`util`: tests call `ServiceExt::oneshot`)
  - `reqwest` with **no default features**, plus its rustls backend and `json`. **rustls's crypto provider:** this machine has no `cmake` and no `nasm`. If the default provider (`aws-lc-rs`) needs either to build, use the `ring` provider instead (through reqwest's or rustls's feature for it). Either rustls provider is fine; another TLS backend (`native-tls`, OpenSSL) isn't. **The first Linux build passes without installing anything.**
  - `serde` (`derive`), `serde_json`, `toml`, `clap` (`derive`), `chrono` (`serde`, `clock`; no other default feature needed), `semver`
  - `async-trait`, `futures`, `thiserror`, `fastrand`
  - `nix` (`fs`, `process`, `signal`, `user`)
  - optional, switched on by the feature: `apollo-compiler` (the latest **stable 1.x**, never a 2.0 pre-release) and `tempfile`. The same two again under `[dev-dependencies]`, so `cfg(test)` code compiles without the feature.
  
  Feature names move between versions (reqwest's rustls feature was `rustls-tls` in 0.12). **Check each name against the version Cargo resolves**; what must hold is the use named above, not the spelling.
- **The feature (TST-1's share here).** `[features] default = []` and `test-endpoint = ["dep:apollo-compiler", "dep:tempfile"]`. It exists, is off by default, and the release build holds none of it. What the feature switches on in code is T07's (the endpoint), T06's (`exit_after`), T27's and T28's (the other variables); yours is `CIRCITOR_TEST_VERSION`, below.
- **`[[test]]`**: `name = "real"`, `path = "tests/real/main.rs"`, `required-features = ["test-endpoint"]`. Create that file with one line, `#![cfg(feature = "test-endpoint")]`; T36 fills it.
- **`[lints]`**: rust `unsafe_code = "deny"`; clippy `all = { level = "warn", priority = -1 }`. `make verify` and CI turn warnings into errors with `-D warnings`.
- **`build.rs` and `src/version.rs` (OPS-11, whole).** `build.rs` runs `git rev-parse --short HEAD` and passes the result to the compiler as `CIRCITOR_COMMIT`; outside a repository, without `git`, or with no commit yet, the value is `unknown`. **It never fails the build.** `src/version.rs`:
  - `pub fn version() -> String`: `MAJOR.MINOR.PATCH+<commit>`, from `CARGO_PKG_VERSION` and `CIRCITOR_COMMIT`.
  - `pub fn version_from(test_version: Option<&str>) -> String`: `Some(v)` gives `v` as it is (a test wants an unparsable one too, CLI-9), `None` the built version.
  - Only under `cfg(feature = "test-endpoint")`, `version()` reads `CIRCITOR_TEST_VERSION` and passes it to `version_from` (TST-1). The release build never reads it.
- **`src/lib.rs`**: `pub mod version;` and nothing else. **`src/main.rs`**: a `main` that prints `circitor::version::version()` and a newline, whatever its arguments. T02 replaces both.
- **`rust-toolchain.toml`**: `channel = "1.98.1"`, `components = ["rustfmt", "clippy"]`, and `targets` as "check-macos" below decides.
- **`rustfmt.toml`**: `edition = "2024"`; any other option must be a stable one. Every later task formats with this file, so it never changes after you.
- **`Makefile`**: `verify` runs exactly the commands of conventions, "Toolchain and commands", in that order, stopping at the first failure. `check-macos` runs `cargo check --target aarch64-apple-darwin --lib --bins`.
- **check-macos: try it, then decide.** Add `aarch64-apple-darwin` to `rust-toolchain.toml`'s `targets` and run it. It probably stops in a C dependency of rustls (`ring` or `aws-lc-sys` wants a macOS C toolchain). If it passes, make it the last step of `verify`. If it can't pass from Linux, **leave it out of `verify`**, keep the `check-macos` make target for running by hand, take the darwin entry out of `rust-toolchain.toml` again, and put a comment in the Makefile naming the crate that stopped it. **Don't swap the TLS backend or the provider to make it pass:** the provider is the one the Linux build above settled.
- **`.gitignore`**: `target/`.
- **`CLAUDE.md`**: add a short "Building" section that points at `plan/conventions.md` and `make verify`. Change nothing else in the file.

## Interfaces
- Consumes: nothing.
- Provides: `circitor::version::version()` and `version_from` (T27 prints it, T28 sends it as API-2's `version`); the `test-endpoint` feature; `make verify` and `make check-macos` (T03 runs them); the locked dependency set.

## Out of scope
- The module tree, `src/main.rs` in its final form, `cli::main`: T02.
- The CI workflow: T03. The release tool and the tap: T39 (D5).
- `circitor version` as a command: T11 and T27. `version` in API answers: T22.
- The other test-build variables of TST-1: T06, T07, T27, T28. The tests in `tests/real/`: T36.

## Acceptance
- Unit tests in `src/version.rs`:
  - `ops11_version_is_semver_with_commit`: `version()` parses with `semver::Version::parse`; its major, minor and patch equal `CARGO_PKG_VERSION`; its build metadata isn't empty (OPS-11).
  - `tst1_test_version_replaces_it`: `version_from(Some("9.9.9"))` is `9.9.9`, `version_from(Some("not a version"))` is that string, `version_from(None)` equals the built version (TST-1).
- Commands, all passing in the worktree:
  - `make verify`
  - `cargo build --release && ./target/release/circitor` prints one line, `0.1.0+<commit>`, where `<commit>` is the short hash of `HEAD` or `unknown`
  - `cargo tree -e normal | grep -c apollo-compiler` prints `0` (the release build holds none of the feature's crates, TST-1), and `cargo tree -e normal --features test-endpoint | grep -c apollo-compiler` prints at least `1`
  - `strings target/release/circitor | grep -c CIRCITOR_TEST_VERSION` prints `0`
  - `cargo test --features test-endpoint --test real` runs and passes with no tests in it
  - `git status --short` shows `Cargo.lock` as a new file and nothing under `target/`
- Your report says which rustls provider is in the build (`aws-lc-rs` or `ring`, as `cargo tree -e normal | grep -E 'aws-lc|ring'` shows) and, for `ring`, the default's build error in one line; and whether `check-macos` is in `verify`, and if not, which crate stopped it and its error in one line.

## Traps
- **A worktree's `.git` is a file, not a directory.** Don't write `cargo:rerun-if-changed=.git/HEAD`. Ask git where `HEAD` is (`git rev-parse --git-path HEAD`) and emit that path only when the command works; always emit `cargo:rerun-if-changed=build.rs`.
- **`unsafe_code` is `deny`, not `forbid`:** T24 allows it on one function, which `forbid` would refuse.
- **`rust-toolchain.toml` selects the toolchain `1.98.1`, which rustup keeps apart from `stable`**, though both are 1.98.1 on this machine. The darwin targets are installed for `stable` only. Listing a target in the file makes rustup download it; nothing else installs it.
- **Declare `apollo-compiler` and `tempfile` twice** (optional and dev). With only the optional one, `cargo test` without the feature can't compile `cfg(test)` code that uses them.
- **Keep automatic test discovery on.** Later tasks add `tests/*.rs` files without touching `Cargo.toml`; only `real` is declared, for its `required-features`.
- **No `[profile]` tuning and no extra crates or features "for later".** In particular no `panic = "abort"` (T28's panic hook is how a crash exits 1, OPS-7) and no `preserve_order` on `serde_json`. Each is a plan decision: report the wish, don't add it.
- The token rules of `CLAUDE.md` already bind `build.rs`: it runs `git`, never `gh`.

## Open questions
- **The library list is a (proposal)** in 00-overview; this task's list is the plan's confirmation of it. If a crate can't be resolved or a named use needs a feature that doesn't exist, report it; don't substitute another crate.
- **D5** (formula or cask, the release tool) doesn't touch this task: no release configuration goes into `Cargo.toml` here.
- **Whether anything macOS-only compiles from Linux** is decided by your `check-macos` attempt. If it's left out, the first macOS compile is CI's (T03, T38).

## Evidence
- **2026-11-26**, branch `t01-scaffold` (commit 53eb611), executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- In the worktree and again on main: `make verify` passes (fmt, both clippy runs, 2 unit tests: `ops11_version_is_semver_with_commit`, `tst1_test_version_replaces_it`).
- `cargo build --release && ./target/release/circitor` prints `0.1.0+977b852`.
- `cargo tree -e normal | grep -c apollo-compiler` is `0`; with `--features test-endpoint` it's `1`. `strings target/release/circitor | grep -c CIRCITOR_TEST_VERSION` is `0`.
- `cargo test --features test-endpoint --test real`: passes, 0 tests.
- **rustls provider: `aws-lc-rs` 1.19.0** (`aws-lc-sys` 0.47.0); it builds on Linux without cmake or nasm. Resolved: reqwest 0.13.5 (features `rustls`, `json`), apollo-compiler 1.33.1.
- **`check-macos` is not in `verify`:** `aws-lc-sys` stops with `cc: error: unrecognized command-line option '-mmacosx-version-min=11.0'`. The make target stays for running by hand; the darwin target isn't in `rust-toolchain.toml`. Per P9, macOS is first compiled on the owner's Mac or in CI.
