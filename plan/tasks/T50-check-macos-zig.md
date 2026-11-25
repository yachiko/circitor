---
id: T50
title: "Follow-up to T01: check-macos from Linux through cargo-zigbuild, last step of make verify"
kind: infra
milestone: M1
size: S
status: done
depends_on: [T01]
requirements: [OPS-13]
owns: [Makefile, rust-toolchain.toml]
shares: []
---

# T50 · Follow-up to T01: check-macos from Linux through cargo-zigbuild

## Outcome
`make check-macos` builds circitor for `aarch64-apple-darwin` from Linux, and it's the last step of `make verify`. The crypto provider stays `aws-lc-rs`: P8 doesn't change.

## Context
T01 found that `check-macos` stops in `aws-lc-sys` from Linux: the host's `gcc` gets macOS flags. The owner asked whether another library would do (2026-11-26). What was tried, from Linux, with the darwin target installed:

- **aws-lc-rs with clang:** stops; aws-lc wants macOS's libc headers and `CoreServices.h`.
- **ring, with gcc or clang:** stops the same way; ring wants `TargetConditionals.h`. Swapping to ring doesn't help.
- **rustls-graviola** (pure Rust, no C): passes, and TLS works. It's pre-1.0 and needs AVX2 and ADX on x86-64, AES and SHA2 on arm64.
- **aws-lc-rs through cargo-zigbuild:** passes. zig ships macOS's libc headers. **The owner chose this one**: it changes only the tools, not the shipped crypto.

## Read first
- `plan/tasks/T01-scaffold.md`: "check-macos: try it, then decide" and its Evidence.
- `plan/conventions.md`, "Toolchain and commands".

## Scope
- **`rust-toolchain.toml`**: `targets = ["aarch64-apple-darwin"]`.
- **`Makefile`**: `verify` ends with `make check-macos`. On a Mac, `check-macos` runs `cargo check` for the target; elsewhere, `cargo zigbuild` for it, after saying which tool is missing if `zig` or `cargo-zigbuild` isn't on the `PATH`.
- **The root README's "Building" section** names the two tools and how to install them. It was written on main with this task. T19 owns the README and its brief says to keep the section.

## Out of scope
- `x86_64-apple-darwin` from Linux, and any CI: T03 builds every OPS-13 target natively.
- The crypto provider and `Cargo.toml` (P8).

## Acceptance
- `make verify` passes on Linux with zig and cargo-zigbuild installed, its last step the macOS build.
- Without `zig` on the `PATH`, `make check-macos` names it and fails.

## Traps
- **`cargo zigbuild` has no `check`:** it builds and links, which is a stronger check than T01's.
- **aws-lc turns off CPU jitter entropy** when it can't find `CoreServices.h`, as from Linux. It still uses the system's entropy. That matters only if a release binary were built this way; T03 and T39 build macOS natively.
- **rustc warns that `xcrun` is missing.** It's harmless: no SDK is needed for this build.

## Evidence
- **2026-11-26**, on main, by the owner's session (not a swarm executor).
- With zig 0.16.0 (Arch's package) and cargo-zigbuild 0.24.1: `make verify` passes in 52 s from cold, the macOS step about 50 s of it; a rebuild after touching `src/lib.rs` takes about 1.3 s.
- With zig 0.17.1: `cargo zigbuild --target aarch64-apple-darwin --lib --bins` passes and gives a `Mach-O 64-bit arm64 executable`.
- Without `zig` on the `PATH`, `make check-macos` prints `check-macos needs zig (README, Building)` and fails.
- `x86_64-apple-darwin` (OPS-13's other macOS target) isn't checked here; T03 builds it natively.
- **2026-11-26, after T02 merged: linking dropped.** T02's GitHub client brings in reqwest, and on macOS `rustls-platform-verifier` links the `Security` and `CoreFoundation` frameworks. zig doesn't ship them, and Apple's SDK can't be shipped, so `cargo zigbuild` stopped at the link: `unable to find system framework "Security"`. `check-macos` now runs `cargo check` for the target with `CC_aarch64_apple_darwin="cargo-zigbuild zig cc -- -target aarch64-macos-none"`: zig compiles aws-lc's C, and nothing is linked. The Traps' "builds and links" no longer holds. `make verify` on main (commit 50add46) passes with zig 0.17.1: 36 tests, and the macOS check in 67 s from cold.
