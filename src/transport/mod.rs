#![allow(unused_variables, dead_code)] // stub: T17 removes this
//! HTTP/1.1 over the Unix socket, from the client's side (`plan/contracts/transport.md`; T17).

use std::path::Path;

use crate::api::{Request, StatusLite};
use crate::clock::SharedClock;

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// The file is missing, or the connection was refused.
    #[error("no daemon")]
    NoDaemon,
    #[error("permission denied")]
    Denied(#[source] std::io::Error),
    /// Over 103 bytes (OPS-4), checked before connecting.
    #[error("socket path too long")]
    TooLong,
    #[error("{0}")]
    Other(#[source] std::io::Error),
}

/// The connection dropped before a full answer (CLI-10).
#[derive(Debug, thiserror::Error)]
#[error("connection dropped")]
pub struct Dropped;

/// One connection, one request.
pub struct Conn {
    stream: tokio::net::UnixStream,
}

pub async fn connect(path: &Path) -> Result<Conn, ConnectError> {
    unimplemented!("T17")
}

impl Conn {
    /// Sends `req` and returns once the status line and headers have arrived.
    pub async fn send(self, req: &Request) -> Result<Head, Dropped> {
        unimplemented!("T17")
    }
}

/// A response whose status line and headers have arrived.
pub struct Head {
    status: u16,
}

impl Head {
    pub fn status(&self) -> u16 {
        unimplemented!("T17")
    }

    /// The `circitor-cursor` header (API-9).
    pub fn cursor(&self) -> Option<&str> {
        unimplemented!("T17")
    }

    /// The whole body; `Dropped` if it's cut short.
    pub async fn body(self) -> Result<Vec<u8>, Dropped> {
        unimplemented!("T17")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    /// `GET /v1/status` gave a complete response within 2 s (OPS-4); one that isn't a status is `NotAnswering`.
    Answers(StatusLite),
    /// A missing file or a refused connection.
    NoDaemon,
    /// `EACCES` or `EPERM` on connect (CLI-7 names the sandbox settings); for OPS-4 it's "doesn't answer".
    Denied(String),
    /// Anything else, a timeout included; the text is the system's error.
    NotAnswering(String),
}

pub async fn probe(path: &Path, clock: &SharedClock) -> Probe {
    unimplemented!("T17")
}

/// What `daemon::instance` asks before it removes a socket file (OPS-4). `transport::probe` implements it.
#[async_trait::async_trait]
pub trait SocketProbe: Send + Sync {
    async fn probe(&self, path: &Path) -> Probe;
}

pub struct RealProbe {
    pub clock: SharedClock,
}

#[async_trait::async_trait]
impl SocketProbe for RealProbe {
    async fn probe(&self, path: &Path) -> Probe {
        unimplemented!("T17")
    }
}
