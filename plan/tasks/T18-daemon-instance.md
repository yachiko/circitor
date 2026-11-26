---
id: T18
title: "Daemon instance: the lock, the lock file, the stale socket, binding"
kind: code
milestone: M2
size: M
status: in-progress
depends_on: [T04]
requirements: [STA-11, STA-12, OPS-4:the-socket-s-mode-is-0600]
owns: [src/daemon/instance/]
shares: []
---

# T18 · Daemon instance: the lock, the lock file, the stale socket, binding

## Outcome
`daemon::instance` takes and tests the `flock` on `circitor.lock`, writes and reads its holder line, removes a socket file only when it's a socket that refuses a connection, binds the socket with mode 0600, and can tell whether the lock file and the socket are still its own.

## Context
T02 built `config::check_socket_path`, the only part of `config` this task uses; T04 gives the clock. T06 isn't needed: the tests make their own paths in a `testkit::ShortDir`. This task builds the primitives of the single instance, each a small function over real files. T28 calls them in STA-11's order and decides every exit; T24 uses `read_holder` and `is_held` to find and stop a daemon (CLI-7, CLI-8, CLI-9).

## Read first
- `plan/conventions.md`
- `plan/contracts/daemon.md`: "Declarations (`src/daemon/instance/`)" and "Semantics"; `plan/contracts/transport.md`: `Probe` and `SocketProbe` only; `plan/contracts/clock.md`: `sleep`
- `specs/03-state.md#one-daemon`: STA-11, STA-12
- `specs/06-operations.md#files`: OPS-4, the part "the socket's mode is 0600", and the 103 bytes
- `docs/04-platform.md` §4: socket path lengths

## Scope
**STA-11's share here is the primitives.** The order at start, the exits by mode, the takeover and the minute's check are T28's.
- **`acquire`.** Opens the lock file read-write, creating it with mode 0600, **without truncating**, close-on-exec. Takes `flock(LOCK_EX | LOCK_NB)` (`nix::fcntl::Flock`, or `flock(2)` on the owned descriptor). Refused: tries again every 100 ms of the clock until `patience` has passed, then `Busy(read_holder(path))`. An error from opening is returned at once, never retried.
- **`LockGuard`.** Holds the descriptor for life; `Drop` releases. `write_holder` truncates and writes `<pid> <socket path>\n`. `still_own` compares the path's `stat` with the descriptor's `fstat`, device and inode.
- **`read_holder`.** Takes the first line only if its newline is there. The process ID is the text before the first space; the path is everything after it, spaces included. Anything else is `None`: a missing file, no newline, no space, a number that doesn't parse, an empty path.
- **`is_held`** (STA-11's test). Opens the file read-only, **never creating it**; a missing file is `Ok(false)`. Takes `LOCK_EX | LOCK_NB` on that descriptor and lets go at once: taken means not held.
- **`clear_socket`** (OPS-4, with the lock held). `lstat` first: nothing there is `Absent`; anything but a socket is `NotASocket`, left alone. A socket is probed: `NoDaemon` removes it (`RemovedStale`), `Answers` is `Live`, `NotAnswering` and `Denied` both leave it and give `NotAnswering`.
- **`bind` (OPS-4, the part "the socket's mode is 0600").** Checks the length with `config::check_socket_path`, then sets the umask to **0177**, binds a `tokio::net::UnixListener` and restores the umask, all under one process-wide mutex. It records the socket's device and inode. **The mode comes out as 0600 from the bind itself**, with no moment in which it's wider. daemon.md says why it's 0177 and not OPS-4's 077.
- **`Bound`.** `still_own` compares the path's `stat` with the recorded device and inode. `remove_if_own` unlinks the path only then.
- **STA-12's share:** this module creates the lock file and the socket and nothing else: no temporary file, no PID file.
- **The constants** `RETRY_EVERY`, `PATIENCE` and `TAKEOVER_PATIENCE` are declared in daemon.md: `acquire` tries every `RETRY_EVERY`; the other two are T28's arguments.

## Interfaces
- **Consumes:** `clock::{SharedClock, sleep}`; `transport::{SocketProbe, Probe}` (the trait only: tests pass a fake); `api::types::StatusLite`; `config::check_socket_path` (T02's, working; T06 may only reword its message).
- **Provides:** `daemon::instance::{Holder, Acquire, acquire, LockGuard, read_holder, is_held, SocketFile, clear_socket, Bound, bind}` and the three constants, as daemon.md declares them.

## Out of scope
- The start order, what a spawned or a foreground daemon does when the lock is busy, the minute's check, the exit statuses: T28.
- Creating the directories and their mode: T06's `ensure_private_dir`, called by T28 and T24.
- The probe itself: T17. Connecting by the lock file's path, `SIGTERM` to its process: T24.

## Acceptance
Unit tests in `src/daemon/instance/`, with real files in a `testkit::ShortDir`, on `TestClock`, with a fake `SocketProbe`:
- `sta11_lock_is_exclusive`: with one guard held, `acquire` with `PATIENCE` gives `Busy` once the clock has moved 6 s, having tried about every 100 ms; `is_held` is `true`. After the guard drops, `is_held` is `false` and `acquire` gives `Held`.
- `sta11_lock_freed_while_waiting`: the holder drops at 3 s; the waiting `acquire` returns `Held` at its next try.
- `sta11_open_error_not_retried`: a directory at the lock's path gives `Err` at once, with the clock unmoved.
- `sta11_not_truncated_before_the_lock`: with a holder's line in the file, a refused `acquire` leaves the line, and `Busy` carries it.
- `sta11_holder_line`: `write_holder(4711, "/tmp/x/Application Support/circitor/circitor.sock")` reads back as that `Holder`; a second `write_holder` with a shorter path leaves no tail of the first. `read_holder` is `None` for a missing file, an empty one, a line without its newline, `abc /p\n`, and `12\n`.
- `sta11_is_held_creates_nothing`: on a missing file it's `Ok(false)`, and the file is still missing.
- `sta11_still_own`: `true`; after the lock file is removed, or replaced by a new file, `false`. The same for `Bound`, and `remove_if_own` then leaves the other file.
- `ops4_clear_socket`: nothing gives `Absent`; a regular file gives `NotASocket` and is still there; a socket with a probe saying `NoDaemon` is removed; `Answers` gives `Live` with the status, the file kept; `NotAnswering` and `Denied` keep it and give `NotAnswering`.
- `ops4_mode_0600`: after `bind`, the socket's mode is exactly 0600, and the process's umask is what it was. Two binds from two threads both give 0600. A path of 104 bytes is refused before anything is created.
- `sta12_only_its_files`: after `acquire`, `write_holder`, `bind` and `remove_if_own`, the directory holds `circitor.lock` and nothing else; while bound, also `circitor.sock`.
- `cargo test --features test-endpoint --lib daemon::instance::` and `make verify` pass.

## Traps
- **`flock`, never `fcntl` record locks** (STA-11): record locks belong to the process and vanish when any descriptor of the file is closed.
- **A `flock` belongs to the open file description.** A second descriptor in the same process is refused too, which is what lets tests start two daemons in one process. A descriptor inherited by a child keeps the lock alive: keep close-on-exec, so `gh` never holds it.
- **On Linux, connecting to a regular file is refused**, like a dead socket (checked on the development machine, 2026-11-25). Without the `lstat`, `clear_socket` would delete a user's file. Use `symlink_metadata`: a symlink isn't a socket.
- The umask is the whole process's. Hold the mutex from setting it to restoring it, and restore it on the error path too.
- `tokio::net::UnixListener::bind` needs a runtime: `bind` is called from inside one.
- The 100 ms tries are `clock::sleep`, not a thread sleep.
- A socket path may hold spaces and non-UTF-8 bytes: write and read it as bytes. One with a newline can't be written: `write_holder` returns an error.

## Open questions
- **The umask.** OPS-4's text says 077, which gives mode 0700 on Linux (checked 2026-11-25; plan README, spec issues). The contract's 0177 is what's built, and 0600 is what's asserted. macOS isn't checked (docs/04 §8): if `make check-macos` or CI shows another mode there, report it.
- STA-11's 6 s, 15 s and 100 ms aren't marked (proposal); OPS-4 is. One constant each.
- What macOS answers on connecting to a non-socket isn't checked. The `lstat` makes it not matter.
