# Seam: the transport between the CLI and the daemon

**Purpose.** HTTP/1.1 over the Unix socket (API-1, OPS-4), seen from the client: connecting, telling "no daemon" from "a daemon that doesn't answer" (CLI-7, OPS-4), sending one request, and reading a `wait`'s head before its body (API-9).

**Producer:** T02 declares; T17 builds. **Consumers:** T24 and T27 (the CLI), T28 (the daemon probes the lock's holder, and asks a spawned one to shut down, STA-11, OPS-6), T18 (through the `SocketProbe` trait only), T29.

## Declarations (`src/transport/`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("no daemon")] NoDaemon,                              // the file is missing, or the connection was refused
    #[error("permission denied")] Denied(#[source] std::io::Error),
    #[error("socket path too long")] TooLong,                    // over 103 bytes (OPS-4), checked before connecting
    #[error("{0}")] Other(#[source] std::io::Error),
}
/// The connection dropped before a full answer (CLI-10).
#[derive(Debug, thiserror::Error)] #[error("connection dropped")] pub struct Dropped;

pub struct Conn { /* one connection, one request */ }
pub async fn connect(path: &Path) -> Result<Conn, ConnectError>;
impl Conn {
    /// Sends `req` and returns once the status line and headers have arrived.
    pub async fn send(self, req: &Request) -> Result<Head, Dropped>;
}
pub struct Head { /* … */ }
impl Head {
    pub fn status(&self) -> u16;
    pub fn cursor(&self) -> Option<&str>;                        // the `circitor-cursor` header (API-9)
    pub async fn body(self) -> Result<Vec<u8>, Dropped>;         // the whole body; `Dropped` if it's cut short
}

#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    Answers(StatusLite),         // `GET /v1/status` gave a complete response within 2 s (OPS-4); one that isn't a status is `NotAnswering`
    NoDaemon,                    // a missing file or a refused connection
    Denied(String),              // `EACCES` or `EPERM` on connect (CLI-7 names the sandbox settings); for OPS-4 it's "doesn't answer"
    NotAnswering(String),        // anything else, a timeout included; the text is the system's error
}
pub async fn probe(path: &Path, clock: &SharedClock) -> Probe;

/// What `daemon::instance` asks before it removes a socket file (OPS-4). `transport::probe` implements it.
#[async_trait::async_trait]
pub trait SocketProbe: Send + Sync { async fn probe(&self, path: &Path) -> Probe; }
pub struct RealProbe { pub clock: SharedClock }
```

## Semantics

- **One connection per request.** `Connection: close`; no pool, no keep-alive, no retry. Retrying is the CLI's (CLI-10).
- **No timeout inside `send` or `body`.** The caller wraps them in `clock::timeout` with CLI-7's limits. `probe` is the exception: its 2 s are OPS-4's, on the clock.
- **`Dropped`** is any end of the connection before a complete response: EOF, a reset, a body shorter than its length or chunking says. A complete response with an error status is not `Dropped`.
- **`Denied`** is `EACCES` or `EPERM` on connect: the CLI names the sandbox settings (CLI-7).
- **A daemon that accepts and never answers** makes `send` wait: the caller's limit ends it.
- **The server side is not here.** The daemon serves `api::router` on a `tokio::net::UnixListener` it bound itself (T18 binds, T28 serves and closes connections at a stop).

## The fake

None. T17 tests against a few lines of server on a socket in a short `/tmp` directory; T24 and T27 do the same for the CLI (a stub daemon that answers golden files from `testdata/api/`).

## Don't assume

- That the path fits: `connect` checks the length first and returns `TooLong`.
- That a stale socket file means no daemon: a refused connection does; a timeout doesn't (OPS-4).
