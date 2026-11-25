---
id: T03
title: "CI: format, lints and every test on Linux and macOS, and the four release targets"
kind: infra
milestone: M4
size: S
status: blocked
depends_on: [T01]
requirements: [TST-14, OPS-13]
owns: [.github/workflows/]
shares: []
---

# T03 · CI: format, lints and every test on Linux and macOS, and the four release targets


> **Blocked (2026-11-25):** the owner chose no CI for now (plan decision P9). Don't start this task until the orchestrator unblocks it.
## Outcome
A GitHub Actions workflow runs `make verify` on a Linux and a macOS runner and builds the release binary for the four targets of OPS-13. It's written and checked for syntax here; its first real run is the owner's (T38).

## Context
T01 gives `make verify` and the pinned toolchain. This task writes the workflow that runs them on GitHub's runners. **It can't run here:** the repository has no remote yet. The owner pushes it and watches the first run (T38); the release itself is T39's.

## Read first
- `plan/conventions.md`: "Toolchain and commands" (what `make verify` runs), "Tests" (hermetic: no token, no real `gh`).
- `specs/07-testing.md#by-hand`: TST-14. `specs/07-testing.md#rules`: TST-1's sentence on CI.
- `specs/06-operations.md#shipping`: OPS-13, the four targets.
- `Makefile` and `rust-toolchain.toml` (T01): the commands and the toolchain the workflow relies on.

## Scope
- **One file, `.github/workflows/ci.yml`** (GitHub Actions: TST-14 leaves the service as a proposal; plan decision P9). It runs on a push to `main` and on every pull request, with `permissions: contents: read` at the top, and cancels a run in progress for the same branch.
- **Job `verify` (TST-14, whole but for the first run, which is T38's).** A matrix over a Linux and a macOS runner. Each checks out the repository and runs `make verify`: formatting, lints with warnings as errors, and every test, the real-process ones included, in a test build (TST-1: `cargo test --features test-endpoint` is inside `make verify`). `fail-fast: false`, so a macOS failure doesn't hide Linux's result.
- **Job `release-build` (OPS-13, whole: this is where the four targets are proven to build).** A matrix over `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, each on a runner of that architecture and system. Each runs `cargo build --release` **without the feature**, then `test -f target/release/circitor`. **Build only: it never runs the binary.** Between T02 and T27 every command panics at a stub, so a run step would turn the owner's early CI run red for no reason. Running the built binary on each system is T39's and T40's.
- **The toolchain comes from `rust-toolchain.toml`.** GitHub's runners carry rustup, which installs the pinned toolchain at the first `cargo` call. Add a step that prints `rustc --version` and `cargo --version`, so a run's log says what built it.
- **Actions:** `actions/checkout`, and `actions/cache` for `~/.cargo/registry`, `~/.cargo/git` and `target/`, keyed on the runner, the job and `Cargo.lock`. No other action.
- **A time limit on every job** (`timeout-minutes`: 30 for `verify`, 20 for `release-build`; your choice of value, in one place each), so a hung test can't burn runner minutes.

## Interfaces
- Consumes: `make verify`, `rust-toolchain.toml`, the `test-endpoint` feature (T01).
- Provides: the workflow T38 runs first, and the per-target build T39's release tool can be checked against.

## Out of scope
- Publishing, release archives, signing, the tap, any release tool: T39 (D5).
- The first run, creating the remote, fixing what macOS shows: T38.
- Any change to the `Makefile`, `Cargo.toml` or the tests: they're T01's and other tasks'. If the workflow needs one, report it.

## Acceptance
- **The file parses.** `actionlint .github/workflows/ci.yml` if `actionlint` is on the `PATH`; otherwise `python3 -c 'import yaml,sys; d=yaml.safe_load(open(".github/workflows/ci.yml")); print(sorted(d["jobs"]))'` prints `['release-build', 'verify']`. Say in your report which check you ran.
- **Every `run:` line is a command that passes in the worktree on Linux:** `make verify`; `cargo build --release`; `test -f target/release/circitor`; `rustc --version`; `cargo --version`. Run each and list the result. Nothing in a `run:` line is a pipeline or a script longer than one command.
- **Read against this list, line by line:**
  - `verify` has exactly two matrix entries, one Linux and one macOS, and its only build step is `make verify` (TST-14)
  - `release-build` has exactly the four targets of OPS-13, none of its commands names `test-endpoint`, and none runs `target/release/circitor`: `grep -n 'release/circitor' .github/workflows/ci.yml` shows only the `test -f` line
  - `permissions` is `contents: read` and nothing else; the words `secrets`, `GITHUB_TOKEN`, `GH_TOKEN` and `id-token` don't appear: `grep -nEi 'secrets|GITHUB_TOKEN|GH_TOKEN|id-token' .github/workflows/ci.yml` prints nothing
  - no step uploads, publishes, tags or pushes anything
- `make verify` passes (the workflow adds no code, so this only proves the worktree is whole).

## Traps
- **Never put a token in the environment.** The runners have a real `gh` and can mint a `GITHUB_TOKEN`; the tests must see neither (TST-1, TST-11). They use a fake `gh` script and the marker token, so the workflow passes nothing.
- **Don't install Rust with a third-party action or a `curl | sh` step.** `rust-toolchain.toml` is the one pin; a second one drifts.
- **Don't cross-compile where a native runner exists.** `cargo build --release --target …` on a foreign runner would stop in the same C dependency T01's `check-macos` meets, and a native build is what proves the target builds where it will run.
- **PyYAML reads the key `on` as the boolean `True`.** That's the checker's quirk, not an error in the file; don't quote `on` to please it.
- **macOS minutes cost ten times Linux's on a private repository.** Keep the macOS work to the two jobs above; no extra matrix axes (toolchains, features, debug and release).
- A pull request from a fork gets a read-only token and no secrets. This workflow needs neither, so don't add `pull_request_target`.

## Open questions
- **The runner labels can't be checked from the worktree.** Use `ubuntu-latest` and `macos-latest` for `verify`; for `release-build`, `macos-latest` (arm64), an Intel macOS label, `ubuntu-latest` and an arm64 Linux label. The Intel macOS and arm64 Linux labels in the file (`macos-15-intel` and `ubuntu-24.04-arm` are the writer's best knowledge) are **unverified**: list them as such in your report. T38's first run corrects them. If no hosted Intel macOS runner exists by then, building `x86_64-apple-darwin` with `--target` on the arm64 macOS runner is the fallback; that's T38's change, not yours.
- **The repository has no remote, and may be private.** Then macOS minutes are paid for, and arm64 Linux runners may not be offered. Build the workflow as written; the owner decides at T38 whether macOS runs on every pull request or on `main` only.
- **TST-14's CI service is a (proposal).** GitHub Actions is plan decision P9. Keep everything service-specific in this one file.
- **D5** doesn't touch this task: nothing here publishes.
