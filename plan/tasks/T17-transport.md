---
id: T17
title: "Transport: the HTTP client over the Unix socket, and the socket probe"
kind: code
milestone: M2
size: S
status: todo
depends_on: [T04, T43]
requirements: [OPS-4:a-socket-answers-when-get]
owns: [src/transport/]
shares: []
---

# T17 · Transport: the HTTP client over the Unix socket, and the socket probe

## Outcome
`transport::connect`, `Conn::send` and `Head::body` carry one request over the Unix socket and tell no daemon, a denied socket and a dropped connection apart; `probe` says whether a socket answers within OPS-4's 2 s.

## Context
T02 declared the types and built `api::types::Request`, which spells every path, and `config::check_socket_path`. This task builds the client side only. The CLI (T24, T27) sends every request through it, the daemon (T28) probes a lock's holder and asks a spawned one to shut down, and `daemon::instance` (T18) sees it through the `SocketProbe` trait. The server side is T28's.

## Read first
- `plan/conventions.md`
- `plan/contracts/transport.md`: the whole file; `plan/contracts/api.md`: `Request`, `StatusLite`, `CURSOR_HEADER`; `plan/contracts/clock.md`: `timeout`; `plan/contracts/config.md`: `check_socket_path`
- `specs/06-operations.md#files`: OPS-4, the part "a socket answers" and the 103 bytes
- `specs/04-api.md#waiting`: API-9, "the arrival cursor is sent at once"
- `specs/05-cli.md`: CLI-7 ("only a missing socket file or a refused connection"), CLI-10 (a drop before a full answer): what the callers do with this task's errors
- `docs/04-platform.md` §4: the socket path's length

## Scope
- **`connect`.** Checks the length with `config::check_socket_path` first (`TooLong`, nothing is opened), then connects a `tokio::net::UnixStream`. `ENOENT` and `ECONNREFUSED` are `NoDaemon`; `EACCES` and `EPERM` are `Denied`; anything else `Other`.
- **`Conn::send`.** One HTTP/1.1 request on a `hyper` client connection (`hyper_util::rt::TokioIo`): the method, `path_and_query()` and `body()` of the `Request`, `Host: circitor`, `Connection: close`, and `Content-Type: application/json` with a body. It returns the `Head` as soon as the status line and headers have arrived: a `wait`'s body comes later (API-9).
- **`Head`.** `status()`, `cursor()` (the `circitor-cursor` header) and `body()`, which reads to the end. Any end before a complete response is `Dropped`: EOF before the headers, a reset, a body shorter than its length, a chunked body without its last chunk. A complete response with an error status isn't.
- **No timeout in `send` or `body`, no retry, no pool:** the callers own both (CLI-7, CLI-10).
- **`probe` (OPS-4, the part "a socket answers"; this task's whole requirement).** `connect`, `GET /v1/status`, and the whole body, inside `clock::timeout` of 2 s, in one named constant. `Answers` when a complete response arrives in time and decodes as `StatusLite`. `NoDaemon` for `ConnectError::NoDaemon`. `Denied` for `ConnectError::Denied`, with the system's error as text. Everything else is `NotAnswering` with a short text of this task's or the system's error: a timeout, `Other`, `TooLong`, `Dropped`, and a complete response that isn't a status.
- **`RealProbe`** implements `SocketProbe` by calling `probe` with its clock.

## Interfaces
- **Consumes:** `api::types::{Request, StatusLite, CURSOR_HEADER}`; `clock::{SharedClock, timeout}`; `config::check_socket_path`.
- **Provides:** `transport::{ConnectError, Dropped, Conn, connect, Head, Probe, probe, SocketProbe, RealProbe}`, as transport.md declares them.

## Out of scope
- The order of socket paths, starting a daemon, the request limits, the one retry: T24. A `wait`'s deadline and reconnect: T27.
- Serving, and closing connections at a stop: T28. Removing a stale socket: T18.
- Paths and parameter names: T02's `Request`. Don't spell a route here.

## Acceptance
Unit tests in `src/transport/`, each with a few lines of server on a socket in a `testkit::ShortDir`, on `TestClock`:
- `answers`: a 200 with a JSON body gives the status and the body; a 404 with an error body is a `Head`, not `Dropped`.
- `head_before_body`: the server sends the headers with `circitor-cursor: abc12345-7` and a chunked body it holds back. `send` returns, `cursor()` is `abc12345-7`, and `body()` completes only once the server sends the rest.
- `dropped`: the server closes before the headers (`send` fails); in the middle of a chunked body, and before a `Content-Length` is met (`body()` fails).
- `no_daemon`: a missing file, and a socket file nobody listens on, are `NoDaemon`. `denied`: a socket with mode 0000 is `Denied` (skip when the tests run as root). `too_long`: a path of 104 bytes is `TooLong`, and one of 103 isn't.
- `request_as_sent`: the server sees `POST /v1/track`, `Host: circitor`, `Connection: close` and the body `{"pr":"o/r#12"}`.
- `ops4_probe_answers`: a server answering `/v1/status` with `version` and `mode` gives `Answers` with both.
- `ops4_probe_not_answering`: a server that accepts and never answers, and one that sends headers and stalls, give `NotAnswering` when the clock has moved 2 s, and not at 1.9 s. A 200 whose body isn't a status gives `NotAnswering`.
- `ops4_probe_no_daemon`: a missing file and a refused connection give `NoDaemon`, at once. `ops4_probe_denied`: a socket with mode 0000 gives `Probe::Denied` (skip as root).
- `cargo test --features test-endpoint --lib transport::` and `make verify` pass.

## Traps
- **A refused connection isn't proof of a dead daemon's socket.** On Linux, connecting to a regular file is refused too. T18 checks the file's type before it believes `NoDaemon`; don't add that check here.
- **`probe`'s 2 s are the clock's.** `tokio::time::timeout` would make the tests wait, and break TST-4.
- The connection task `hyper` needs must end when the `Head` is dropped: a CLI that gives up must not leave it running.
- **Never half-close the write side** after sending: the daemon reads a closed connection as a client that went away (API-14).
- Header names compare without case.
- The test servers do real I/O that the test clock doesn't count: await the client's future, or join it with the clock's advance. Don't sleep.

## Open questions
- None open on the seam: `check_socket_path` is T02's and real. Don't copy the 103 here.
- OPS-4's 2 s is part of a (proposal) requirement: built as written, one constant.
- What a sandbox answers on a denied socket isn't measured (docs/03 §6): `EACCES` and `EPERM` are the reading built.
