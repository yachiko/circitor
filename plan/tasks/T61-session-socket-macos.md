---
id: T61
title: "Follow-up to T45 and T57: the fake session socket compiles on macOS"
kind: test
milestone: M5
size: S
status: done
depends_on: [T45, T57]
requirements: [TST-16]
owns: [src/testkit/session/]
shares: []
---

# T61 · Follow-up to T45 and T57: the fake session socket compiles on macOS

## Outcome
The test build (TST-16's fake session socket, with `test-endpoint`) compiles and runs on macOS as on Linux.

## Context
**The first release dry run (T38) failed on `macos-15` at compile time**, on 2026-11-26. `bind_listener` in `src/testkit/session/mod.rs` passed `SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC` to `socket()`. **Both flags are Linux-only**, and nix doesn't define them for macOS. The old `check-macos` (T50) built without `test-endpoint`, so it never compiled this code. Production code was unaffected.

## Read first
- `plan/conventions.md`, "Platform rules": differences are values, not `cfg`.
- `src/testkit/session/mod.rs`: `bind_listener`.

## Scope
- `socket()` takes no flags. Close-on-exec and non-blocking are then set with `fcntl`, on every platform, with no `cfg`.

## Out of scope
- Anything outside `bind_listener`. The order and backlog rules of T45 and T57 stay as they are.

## Acceptance
- `cargo check --target aarch64-apple-darwin --all-targets --features test-endpoint` passes (checked once from Linux through zig, outside the repository's tooling).
- `cargo test --features test-endpoint --test scn_notify` and `make verify` pass on Linux.
- `release.yml`'s `verify (macos-15)` gets past the compile.

## Open questions
- **The flags are no longer set atomically on Linux.** A child forked between `socket()` and `fcntl` could inherit the listener. Only the tests' fake socket is affected, and nothing forks in that window today.

## Evidence
- **2026-11-26**, by the orchestrator, on the owner's OK. Failing dry run: https://github.com/yachiko/circitor/actions/runs/45793244625.
