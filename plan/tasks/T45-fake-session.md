---
id: T45
title: "Fake session socket: what a Claude Code session's inbox looks like from outside"
kind: test
milestone: M3
size: S
status: todo
depends_on: [T04]
requirements: [TST-16]
owns: [src/testkit/session/]
shares: []
---

# T45 · Fake session socket: what a Claude Code session's inbox looks like from outside

## Outcome
`testkit::session::FakeSession` binds a Unix socket in a short directory as a Claude Code session does, records every line posted to it with the test clock's time, never answers, and can be told to refuse, to vanish, to accept and never read, and to listen without accepting until its backlog is full, so a connect fails at once (TST-16).

## Context
`notify` posts notices into a session's own socket (NTF-6), and that socket never answers (docs/07), so only the receiving end can show what arrived. This task builds that end for tests. T02 declared the types in `src/testkit/session/`; T04 gives `TestClock` and `SettleProbe`. The harness (T29) binds one at every start and puts its path and a marker token in each command's environment; the notify scenarios (T49) and the golden files (T35) read what it recorded. **The notifier itself (T46) isn't needed here, and doesn't use this:** its own tests post to a `RecordingPoster` or to a listener they bind.

## Read first
- `plan/conventions.md` ("Tests", "Work in flight", "Platform rules")
- `plan/contracts/testkit.md`: "The fake session socket" (the declarations and their five bullets), `ShortDir`, `MARKER_SESSION_TOKEN`; `plan/contracts/clock.md`: `TestClock`, `SettleProbe`, `add_probe`, "Semantics"; `plan/contracts/notify.md`: `Poster::post`'s doc (the two lines a post writes) and "Don't assume"
- `specs/07-testing.md#test-doubles`: TST-16, and TST-4 ("Settled means…"); `#what-is-tested`: TST-8's `notify-session-gone` row (what the four modes are for)
- `specs/08-notify.md#posting`: NTF-6
- `docs/07-notify.md`, "The session socket": every row of the table. `docs/measurements/2026-11-25/post-to-session.py`: a post as a script makes it

## Scope
- **`FakeSession::bind(dir, name, clock)`** (TST-16). Creates `<dir>/<name>.sock` as a Unix stream listener, mode 0600 (docs/07), non-blocking, with `std::os::unix::net`; adds itself to `clock` as a `SettleProbe`; returns an `Arc`. Dropping it removes the file.
- **Recording.** It has no task of its own. Whenever the clock asks `settled()`, and at the start of `lines()` and `notices()`, it accepts every pending connection and reads each to its end, never blocking. Each complete line (up to a newline) becomes a `Line` with the connection's number, the clock's wall and monotonic time **at that read**, and the text without its newline. **It's settled once it has read every connection it accepted to its end** (TST-4), but those accepted under `accept_and_never_read`, which it never reads. It never writes and never closes a connection before its peer has.
- **`notices()`**: one `Notice` per connection, parsed with `serde_json` as plain values: an optional first line `{"type":"auth","token":…}`, then exactly one `{"type":"user","message":{"role":"user","content":…}}`. Anything else (a third line, another `type`, a key missing, a line that isn't JSON, a connection with no message line) panics, naming the connection and the line. `token()` is `MARKER_SESSION_TOKEN`.
- **The four modes** (TST-16), each until `restore()`:
  - `refuse()`: the listener is closed and its file left in place, so a connect is refused and a `stat` sees the same device and inode.
  - `vanish()`: the listener is closed and the file removed.
  - `accept_and_never_read()`: connections are accepted and kept open, nothing is read, nothing is recorded.
  - `queue_full()`: a new listener on the same path with a backlog of 0, which the fake fills with connections of its own and never accepts, so a further connect fails at once (`EAGAIN` on Linux).
  - `restore()`: a recording listener on the same path again. After `vanish` and `queue_full` it's a new socket file: say so in the method's doc.
- **Module docs** with one example test, and what each mode looks like to a connecting client on Linux (below).

## Interfaces
- **Consumes:** `clock::{TestClock, SettleProbe, Time, Mono}` (T04), `testkit::{ShortDir, MARKER_SESSION_TOKEN}` (T02), `serde_json`.
- **Provides:** `testkit::session::{FakeSession, Line, Notice}` as `plan/contracts/testkit.md` declares them, for T29, T35 and T49.

## Out of scope
- The poster, the subscriptions and the notices' words: T46. **Don't import anything from `src/notify/`:** the fake parses the lines by itself, so a wrong key in the poster fails a test instead of agreeing with it.
- Binding one in the harness, the environment variables, and TST-11's sweep of what was recorded: T29.
- The scenarios: T49. `src/testkit/mod.rs` is T02's: add files under `src/testkit/session/` only.

## Acceptance
Unit tests in `src/testkit/session/`, each in a `ShortDir` on a `TestClock`, posting with a plain `std::os::unix::net::UnixStream` as `post-to-session.py` does:
- `tst16_records_lines_with_their_time`: an auth line and a message line on one connection, closed; after `clock.settle()`, `lines()` has both with the connection's number and the clock's time; after an `advance` of 30 s and a second post, its lines carry the later time. `notices()` gives two, the first with `token` `Some(MARKER_SESSION_TOKEN)`.
- `tst16_without_an_auth_line`: one message line alone is a `Notice` with `token` `None`. A multi-line `content` (`\n` inside the JSON string) arrives intact as one line.
- `tst16_never_answers`: after a post, a read on the client's side gets no byte and no end of stream while the fake lives.
- `tst16_malformed_panics` (`#[should_panic]`, one test each): a plain-text line; a `type` of `message`; two message lines on one connection.
- `tst16_refuse`: a connect fails with `ConnectionRefused`; the path still exists, and its device and inode are the ones from before. `tst16_vanish`: a connect fails with `NotFound`; the path is gone.
- `tst16_accept_and_never_read`: a connect and a write of a 300-byte line both succeed at once; `lines()` stays empty.
- `tst16_queue_full`: a connect fails at once: on Linux with `WouldBlock` (`EAGAIN`), as NTF-6 cites (measured 2026-11-25). The test asserts that it failed without waiting, and prints the error it saw.
- `tst16_restore`: after each mode, `restore()` records again; after `refuse`, on the same inode.
- `tst16_settles`: with a client that has connected and written half a line, `settled()` is `false`; once the line ends and the client closes, `true`. In `accept_and_never_read` and `queue_full` it's always `true`.
- `tst16_mode_0600`: the socket file's mode.
- `cargo test --features test-endpoint --lib testkit::session::` and `make verify` pass.

## Traps
- **Your own shell has a real session socket.** `CLAUDE_CODE_MESSAGING_SOCKET` is set in the session that runs you. Never read it, and never post to anything but a socket your test bound: a line written there lands in a live session.
- **A short line never blocks a writer.** A peer that doesn't read, and one that doesn't even accept, both take a 300-byte line: the kernel holds it. That's why `accept_and_never_read` counts as posted (NTF-6), and why only a full backlog stops a connection.
- **`queue_full` isn't portable.** On Linux a connect to a full backlog gives `EAGAIN` at once (measured 2026-11-25, cited by NTF-6, not yet in docs/05). On macOS it refuses, `ECONNREFUSED`, from source and not measured. Either way it fails at once: no test may depend on a wait.
- **Everything runs on one current-thread runtime** (TST-4), and `settled()` is called from the clock: it must not block and must not await. Hence non-blocking `std` sockets, not tokio's.
- A socket path is at most 103 bytes on macOS: bind under the `ShortDir` you're given, with a short `name`.
- `make verify` lints without the feature too: the whole module sits under `cfg(any(test, feature = "test-endpoint"))`, as `src/testkit/mod.rs` declares it.

## Open questions
- **Only Linux is measured** (docs/07): the mode, that the socket never answers and never closes first, and the format. What a macOS session's socket does isn't known; the fake plays Linux.
- **TST-16's fourth mode** says what Linux does: a connect fails at once. NTF-6 ends the subscription for that, as for a refusal, so the scenario (T49) holds on either system. If your machine shows a third behaviour, report it.
