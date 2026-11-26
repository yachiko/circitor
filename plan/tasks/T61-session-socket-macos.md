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

- **A full queue on macOS is refused, not `EAGAIN`.** The second dry run compiled and failed three unit tests (`tst16_queue_full`, `tst16_restore`, `tst16_settles`): filling the backlog got `ECONNREFUSED` (os error 61), as NTF-6 says macOS does ("from source"). `FULL_QUEUE` names the platform's kind; `queue_full` fills until it, and `tst16_queue_full` expects it.
- **macOS keeps the first backlog.** The third dry run (`macos-26`) passed those tests but failed `tst16_queue_full`'s bound: filling took 128 connections, not under 10, because a second `listen` with 0 doesn't shrink the queue on macOS. `FILL_LIMIT` is 130 on macOS and 10 on Linux.
- **`scn_notify_session_gone_full_queue` expects the platform's log reason:** `session_gone` on macOS (a refused connection, NTF-6), `failed` on Linux. TST-8's row only says it's dropped the same way.

## Out of scope
- Anything outside `bind_listener`. The order and backlog rules of T45 and T57 stay as they are.

## Acceptance
- `cargo check --target aarch64-apple-darwin --all-targets --features test-endpoint` passes (checked once from Linux through zig, outside the repository's tooling).
- `cargo test --features test-endpoint --test scn_notify` and `make verify` pass on Linux.
- `release.yml`'s `verify (macos-15)` gets past the compile.

## Open questions
- **The flags are no longer set atomically on Linux.** A child forked between `socket()` and `fcntl` could inherit the listener. Only the tests' fake socket is affected, and nothing forks in that window today.

## Evidence
- **2026-11-26**, by the orchestrator, on the owner's OK. Failing dry runs: https://github.com/yachiko/circitor/actions/runs/45793244625 (compile), https://github.com/yachiko/circitor/actions/runs/45794230505 (the full queue), https://github.com/yachiko/circitor/actions/runs/45795604691 (the fill bound).
- **docs/05, 2026-11-26, "a full listen queue on macOS"** (the owner's OK): `ECONNREFUSED` at once, after 128 queued connections. docs/07 and NTF-6 now say verified.
