---
id: T53
title: "Follow-up to T01: nix's socket feature, for the fake session socket (D13)"
kind: infra
milestone: M3
size: S
status: done
depends_on: [T01]
requirements: [TST-16]
owns: [Cargo.toml, Cargo.lock]
shares: []
---

# T53 · Follow-up to T01: nix's socket feature, for the fake session socket (D13)

## Outcome
`nix` is declared with its `socket` feature, so the test kit can make a Unix socket step by step (`socket`, `bind`, `listen` with a backlog of 0). No other crate or feature changes, but `memoffset`, which that feature uses.

## Context
T45 found that with `std` alone `FakeSession` can't listen with a backlog of 0: it filled `SOMAXCONN` (4 097 connections and descriptors on Linux) instead, which fails under a common descriptor limit of 1 024 and macOS's 256; and `refuse` then `restore` gave a new inode, against TST-16. **D13** (the orchestrator, under the owner's rule of 2026-11-26): nix's `socket` feature, so `queue_full` has a true backlog of 0, filled by a few connections. **`restore` after `refuse` stays a new socket file:** a socket can't stop listening and a bound path can't be bound twice, and nothing needs the inode back (a refused connect already ends a subscription as `session_gone`, NTF-6).

## Scope
- `Cargo.toml`: `nix`'s features gain `socket`. `Cargo.lock` gains `memoffset` 0.9.1.

## Evidence
- **2026-11-26**, on main, by the orchestrator. `make verify` passes with the feature, the macOS check included.
