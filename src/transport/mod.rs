//! HTTP/1.1 over the Unix socket, from the client's side (`plan/contracts/transport.md`; T17).
//!
//! One connection per request (`Connection: close`), no pool, no retry, and no timeout in `send` or
//! `body`: the callers own those (CLI-7, CLI-10). Only `probe` has a limit, OPS-4's, on the clock.

use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::task::AbortHandle;

use crate::api::{CURSOR_HEADER, Request, StatusLite};
use crate::clock::{SharedClock, timeout};
use crate::config::check_socket_path;

/// OPS-4: a socket answers when a complete status arrives within this.
const PROBE_LIMIT: Duration = Duration::from_secs(2);

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
    // OPS-4: a path over 103 bytes is refused before anything is opened.
    if check_socket_path(path).is_err() {
        return Err(ConnectError::TooLong);
    }
    match tokio::net::UnixStream::connect(path).await {
        Ok(stream) => Ok(Conn { stream }),
        Err(e) => Err(match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => ConnectError::NoDaemon,
            ErrorKind::PermissionDenied => ConnectError::Denied(e),
            _ => ConnectError::Other(e),
        }),
    }
}

/// Ends the connection's task when the `Head` goes: a CLI that gives up leaves nothing running.
struct TaskGuard(AbortHandle);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Conn {
    /// Sends `req` and returns once the status line and headers have arrived.
    pub async fn send(self, req: &Request) -> Result<Head, Dropped> {
        let io = TokioIo::new(self.stream);
        let (mut sender, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
            .await
            .map_err(|_| Dropped)?;
        let task = tokio::spawn(async move {
            // The outcome shows in the request's own result.
            let _ = connection.await;
        });
        let guard = TaskGuard(task.abort_handle());

        let body = req.body();
        let mut builder = http::Request::builder()
            .method(req.method())
            .uri(req.path_and_query())
            .header(http::header::HOST, "circitor")
            .header(http::header::CONNECTION, "close");
        if body.is_some() {
            builder = builder.header(http::header::CONTENT_TYPE, "application/json");
        }
        let request = builder
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|_| Dropped)?;

        // Never half-close here: the daemon reads a closed connection as a client that left (API-14).
        let response = sender.send_request(request).await.map_err(|_| Dropped)?;
        let (parts, body) = response.into_parts();
        let cursor = parts
            .headers
            .get(CURSOR_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        Ok(Head {
            status: parts.status.as_u16(),
            cursor,
            body,
            _guard: guard,
        })
    }
}

/// A response whose status line and headers have arrived.
pub struct Head {
    status: u16,
    cursor: Option<String>,
    body: Incoming,
    _guard: TaskGuard,
}

impl Head {
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The `circitor-cursor` header (API-9).
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// The whole body; `Dropped` if it's cut short.
    pub async fn body(self) -> Result<Vec<u8>, Dropped> {
        let collected = self.body.collect().await.map_err(|_| Dropped)?;
        Ok(collected.to_bytes().to_vec())
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

async fn ask(path: &Path) -> Probe {
    let conn = match connect(path).await {
        Ok(c) => c,
        Err(ConnectError::NoDaemon) => return Probe::NoDaemon,
        Err(ConnectError::Denied(e)) => return Probe::Denied(e.to_string()),
        Err(ConnectError::TooLong) => return Probe::NotAnswering("socket path too long".into()),
        Err(ConnectError::Other(e)) => return Probe::NotAnswering(e.to_string()),
    };
    let head = match conn.send(&Request::Status).await {
        Ok(h) => h,
        Err(e) => return Probe::NotAnswering(e.to_string()),
    };
    let status = head.status();
    let body = match head.body().await {
        Ok(b) => b,
        Err(e) => return Probe::NotAnswering(e.to_string()),
    };
    if !(200..300).contains(&status) {
        return Probe::NotAnswering(format!("status {status}"));
    }
    match serde_json::from_slice::<StatusLite>(&body) {
        Ok(s) => Probe::Answers(s),
        Err(_) => Probe::NotAnswering("the answer is not a status".into()),
    }
}

pub async fn probe(path: &Path, clock: &SharedClock) -> Probe {
    // OPS-4: a socket answers when a complete status arrives within 2 s of the clock's time.
    match timeout(clock, PROBE_LIMIT, ask(path)).await {
        Ok(p) => p,
        Err(_) => Probe::NotAnswering("timed out after 2 s".into()),
    }
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
        probe(path, &self.clock).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{TestClock, Time};
    use crate::model::PrName;
    use crate::testkit::ShortDir;
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{UnixListener, UnixStream};

    /// Serves each connection with `f`, in a task of its own.
    fn serve<F, Fut>(path: &Path, f: F) -> tokio::task::JoinHandle<()>
    where
        F: Fn(UnixStream) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((s, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(f(s));
            }
        })
    }

    /// Reads until the end of the request head.
    async fn read_head(s: &mut UnixStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut b = [0u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s.read(&mut b).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&b[..n]);
        }
        buf
    }

    fn sock(dir: &ShortDir) -> std::path::PathBuf {
        dir.path().join("s")
    }

    fn clock() -> SharedClock {
        TestClock::new(Time::from_unix_millis(0))
    }

    fn pr() -> PrName {
        PrName::parse("o/r#12").unwrap()
    }

    const STATUS: &str = "{\"version\":\"1.2.3\",\"mode\":\"running\"}";

    fn ok_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    #[tokio::test]
    async fn answers() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        let _s = serve(&p, |mut s| async move {
            let head = read_head(&mut s).await;
            let resp = if head.starts_with(b"GET /v1/status") {
                ok_response("{\"a\":1}")
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"error\":1}"
                    .to_string()
            };
            s.write_all(resp.as_bytes()).await.unwrap();
        });
        let head = connect(&p)
            .await
            .unwrap()
            .send(&Request::Status)
            .await
            .unwrap();
        assert_eq!(head.status(), 200);
        assert_eq!(head.body().await.unwrap(), b"{\"a\":1}");
        let head = connect(&p)
            .await
            .unwrap()
            .send(&Request::Refresh)
            .await
            .unwrap();
        assert_eq!(head.status(), 404);
        assert_eq!(head.body().await.unwrap(), b"{\"error\":1}");
    }

    // API-9: the head arrives before the body.
    #[tokio::test]
    async fn head_before_body() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let rx = std::sync::Arc::new(std::sync::Mutex::new(Some(rx)));
        let _s = serve(&p, move |mut s| {
            let rx = rx.clone();
            async move {
                read_head(&mut s).await;
                s.write_all(
                    b"HTTP/1.1 200 OK\r\ncircitor-cursor: abc12345-7\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .await
                .unwrap();
                let rx = rx.lock().unwrap().take().unwrap();
                let _ = rx.await;
                s.write_all(b"2\r\n{}\r\n0\r\n\r\n").await.unwrap();
            }
        });
        let head = connect(&p)
            .await
            .unwrap()
            .send(&Request::Status)
            .await
            .unwrap();
        assert_eq!(head.cursor(), Some("abc12345-7"));
        let mut body = std::pin::pin!(head.body());
        // The body is held back until the server sends the rest.
        let early = futures::poll!(&mut body);
        assert!(early.is_pending());
        tokio::task::yield_now().await;
        assert!(futures::poll!(&mut body).is_pending());
        tx.send(()).unwrap();
        assert_eq!(body.await.unwrap(), b"{}");
    }

    // CLI-10: an end before a complete answer is `Dropped`.
    #[tokio::test]
    async fn dropped() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        // Before the headers.
        let s = serve(&p, |mut s| async move {
            read_head(&mut s).await;
        });
        let r = connect(&p).await.unwrap().send(&Request::Status).await;
        assert!(r.is_err());
        s.abort();
        let _ = s.await;
        std::fs::remove_file(&p).unwrap();

        // In the middle of a chunked body.
        let s = serve(&p, |mut s| async move {
            read_head(&mut s).await;
            s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nabcde\r\n")
                .await
                .unwrap();
        });
        let head = connect(&p)
            .await
            .unwrap()
            .send(&Request::Status)
            .await
            .unwrap();
        assert!(head.body().await.is_err());
        s.abort();
        let _ = s.await;
        std::fs::remove_file(&p).unwrap();

        // Before a Content-Length is met.
        let _s = serve(&p, |mut s| async move {
            read_head(&mut s).await;
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc")
                .await
                .unwrap();
        });
        let head = connect(&p)
            .await
            .unwrap()
            .send(&Request::Status)
            .await
            .unwrap();
        assert!(head.body().await.is_err());
    }

    /// A socket nobody may connect to; `None` when the tests run as root, which connects anyway.
    fn mode_zero_socket(p: &Path) -> Option<tokio::task::JoinHandle<()>> {
        let s = serve(p, |_s| async move {});
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::os::unix::net::UnixStream::connect(p).is_ok() {
            return None;
        }
        Some(s)
    }

    #[tokio::test]
    async fn no_daemon() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        assert!(matches!(connect(&p).await, Err(ConnectError::NoDaemon)));
        // A socket file nobody listens on.
        drop(std::os::unix::net::UnixListener::bind(&p).unwrap());
        assert!(p.exists());
        assert!(matches!(connect(&p).await, Err(ConnectError::NoDaemon)));
    }

    #[tokio::test]
    async fn denied() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        let Some(_s) = mode_zero_socket(&p) else {
            return;
        };
        assert!(matches!(connect(&p).await, Err(ConnectError::Denied(_))));
    }

    // OPS-4: 103 bytes fit, 104 don't.
    #[tokio::test]
    async fn ops4_too_long() {
        let dir = ShortDir::new();
        let base = dir.path().to_str().unwrap().len() + 1;
        let long = dir.path().join("x".repeat(104 - base));
        assert_eq!(long.as_os_str().len(), 104);
        assert!(matches!(connect(&long).await, Err(ConnectError::TooLong)));
        let ok = dir.path().join("x".repeat(103 - base));
        assert_eq!(ok.as_os_str().len(), 103);
        assert!(matches!(connect(&ok).await, Err(ConnectError::NoDaemon)));
    }

    #[tokio::test]
    async fn request_as_sent() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let _s = serve(&p, move |mut s| {
            let tx = tx.clone();
            async move {
                let mut buf = Vec::new();
                let mut b = [0u8; 1024];
                while !String::from_utf8_lossy(&buf).ends_with("{\"pr\":\"o/r#12\"}") {
                    let n = s.read(&mut b).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&b[..n]);
                }
                tx.send(String::from_utf8_lossy(&buf).to_string()).unwrap();
                s.write_all(ok_response("{}").as_bytes()).await.unwrap();
            }
        });
        let head = connect(&p)
            .await
            .unwrap()
            .send(&Request::Track { pr: pr() })
            .await
            .unwrap();
        head.body().await.unwrap();
        let seen = rx.recv().await.unwrap();
        let lower = seen.to_ascii_lowercase();
        assert!(seen.starts_with("POST /v1/track HTTP/1.1\r\n"), "{seen}");
        assert!(lower.contains("\r\nhost: circitor\r\n"), "{seen}");
        assert!(lower.contains("\r\nconnection: close\r\n"), "{seen}");
        assert!(
            lower.contains("\r\ncontent-type: application/json\r\n"),
            "{seen}"
        );
        assert!(seen.ends_with("\r\n\r\n{\"pr\":\"o/r#12\"}"), "{seen}");
    }

    // OPS-4: a socket answers when `GET /v1/status` gives a status.
    #[tokio::test]
    async fn ops4_probe_answers() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        let _s = serve(&p, |mut s| async move {
            read_head(&mut s).await;
            s.write_all(ok_response(STATUS).as_bytes()).await.unwrap();
        });
        let got = probe(&p, &clock()).await;
        assert_eq!(
            got,
            Probe::Answers(StatusLite {
                version: "1.2.3".into(),
                mode: "running".into()
            })
        );
        let real = RealProbe { clock: clock() };
        assert!(matches!(real.probe(&p).await, Probe::Answers(_)));
    }

    // OPS-4: no answer within 2 s is "not answering", and not before.
    #[tokio::test]
    async fn ops4_probe_not_answering() {
        for case in 0..2 {
            let dir = ShortDir::new();
            let p = sock(&dir);
            let _s = serve(&p, move |mut s| async move {
                read_head(&mut s).await;
                if case == 1 {
                    s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 40\r\n\r\n{")
                        .await
                        .unwrap();
                }
                std::future::pending::<()>().await;
            });
            let clock = TestClock::new(Time::from_unix_millis(0));
            let shared: SharedClock = clock.clone();
            let p2 = p.clone();
            let task = tokio::spawn(async move { probe(&p2, &shared).await });
            clock.advance(Duration::from_millis(1900)).await;
            assert!(!task.is_finished(), "case {case} ended at 1.9 s");
            clock.advance(Duration::from_millis(100)).await;
            let got = task.await.unwrap();
            assert!(matches!(got, Probe::NotAnswering(_)), "{got:?}");
        }
        // A 200 whose body isn't a status.
        let dir = ShortDir::new();
        let p = sock(&dir);
        let _s = serve(&p, |mut s| async move {
            read_head(&mut s).await;
            s.write_all(ok_response("{\"hello\":1}").as_bytes())
                .await
                .unwrap();
        });
        assert!(matches!(probe(&p, &clock()).await, Probe::NotAnswering(_)));
    }

    // OPS-4: a missing file and a refused connection are "no daemon", at once.
    #[tokio::test]
    async fn ops4_probe_no_daemon() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        assert_eq!(probe(&p, &clock()).await, Probe::NoDaemon);
        drop(std::os::unix::net::UnixListener::bind(&p).unwrap());
        assert_eq!(probe(&p, &clock()).await, Probe::NoDaemon);
    }

    #[tokio::test]
    async fn ops4_probe_denied() {
        let dir = ShortDir::new();
        let p = sock(&dir);
        let Some(_s) = mode_zero_socket(&p) else {
            return;
        };
        assert!(matches!(probe(&p, &clock()).await, Probe::Denied(_)));
    }
}
