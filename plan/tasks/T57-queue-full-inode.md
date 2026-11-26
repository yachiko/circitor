---
id: T57
title: "Follow-up to T45: queue_full keeps the socket's inode (D16)"
kind: test
milestone: M5
size: S
status: done
depends_on: [T45, T49]
requirements: [TST-16, NTF-6]
owns: [src/testkit/session/]
shares: [tests/scn_notify.rs]
---

# T57 · Follow-up to T45: queue_full keeps the socket's inode (D16)

## Outcome
`FakeSession::queue_full` makes the same socket, same device and inode, refuse to take a connection (a connect fails at once with `EAGAIN` on Linux), so the notifier's `look` still sees the session it registered and its post fails as `reason=failed` (NTF-6), as TST-8 `notify-session-gone` asks. `restore` from it records again on the same socket.

## Context
T49's `scn_notify_session_gone_full_queue` is ignored: T45's `queue_full` binds a new listener at the path, a new socket file, so the notifier's `look` before the post sees another `SocketId` and ends the subscription as `session_gone` (NTF-4) instead of trying the post. **D16** (the orchestrator, under the owner's rule of 2026-11-26): keep the one listening socket and lower its backlog. On Linux `listen(2)` on a socket already listening sets its backlog again; with the backlog at 0 and one connection of the fake's own pending, a further connect fails at once.

## Read first
- `plan/conventions.md`; `plan/contracts/testkit.md` (the fake session socket).
- `specs/07-testing.md`: TST-16; `specs/08-notify.md`: NTF-4, NTF-6; TST-8 `notify-session-gone`.
- `src/testkit/session/mod.rs` as T45 left it (nix's `socket` feature is declared, T53), and T49's `scn_notify_session_gone_full_queue`.

## Scope
- `queue_full`: stop accepting, call `listen` again on the recording socket with `Backlog` 0, and fill its queue with the fake's own connections until one gets `EAGAIN`. **No new socket file.** `restore` from `queue_full`: `listen` again with the old backlog, drop the fill connections, accept and record as before. Update the method docs.
- **`shares`:** remove the `#[ignore]` on `scn_notify_session_gone_full_queue` in `tests/scn_notify.rs`; change nothing else there.

## Acceptance
- `tst16_queue_full` (T45's, updated): the socket's device and inode are unchanged after `queue_full` and after `restore`; a connect fails at once; fewer than 10 descriptors are used; after `restore` a post is recorded.
- `scn_notify_session_gone_full_queue` passes un-ignored, with one `notice not posted … reason=failed` and the subscription ended.
- `cargo test --features test-endpoint --lib testkit::session::` 5 times, `--test scn_notify` and `make verify` pass. If `listen` again doesn't lower the backlog on Linux as said, report what it does: don't change the scenario.

## Open questions
- macOS isn't measured: there a full queue may refuse instead (TST-16).

## Evidence
- **2026-11-26**, branch `t57-queue-full-inode`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/testkit/session/mod.rs`: `queue_full` listens again on the same socket with backlog 0 (`relisten`) and fills it; `restore` and `accept_and_never_read` go through `unfill` and keep the socket. On Linux `listen` again does lower the backlog. `tst16_queue_full` checks device and inode unchanged after `queue_full` and `restore`. **`shares` `tests/scn_notify.rs`:** T49's `scn_notify_session_gone_full_queue` asserted no notice at all, forgetting the registration's confirmation; on the orchestrator's instruction only that assertion changed (one notice), with a comment, and the ignore was removed: it passes, with one `notice not posted … reason=failed`. `--test scn_notify`: 36 passed, 0 ignored, twice. `make verify` passes on main after the merge, first try. macOS's full queue isn't measured.
