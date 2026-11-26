//! The fake session socket (TST-16; T45): a Unix stream socket, mode 0600, in a directory of mode
//! 0700, that takes lines and never answers. It shares no code with `src/notify/`.
//!
//! It has no task of its own. Each time the test clock asks whether it's settled, and at the start of
//! [`FakeSession::lines`] and [`FakeSession::notices`], it accepts what is pending and reads every
//! connection to what it has without blocking, so a notice posted at an instant is stamped with that
//! instant (TST-4). It never writes a byte and never closes a connection before its peer has.
//!
//! ```ignore
//! let dir = ShortDir::new();
//! let clock = TestClock::new(start);
//! let session = FakeSession::bind(dir.path(), "s", clock.clone());
//! let mut c = std::os::unix::net::UnixStream::connect(session.path()).unwrap();
//! c.write_all(b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n").unwrap();
//! drop(c);
//! clock.settle().await;
//! assert_eq!(session.notices()[0].text, "hi");
//! ```
//!
//! What each mode looks like to a client connecting on Linux (only Linux is measured, docs/07):
//!
//! | Mode | `connect` | A short write |
//! |---|---|---|
//! | recording | succeeds | succeeds, and is recorded |
//! | `refuse` | `ConnectionRefused`; the path still exists | n/a |
//! | `vanish` | `NotFound`; the path is gone | n/a |
//! | `accept_and_never_read` | succeeds | succeeds (the kernel holds it); never recorded |
//! | `queue_full` | `WouldBlock` (`EAGAIN`) at once, from a non-blocking connect | n/a |
//!
//! A blocking `connect(2)` to a full backlog waits on Linux; a poster's is non-blocking, as tokio's is.

use std::io::{ErrorKind, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::{Clock, Mono, SettleProbe, TestClock, Time};

use super::MARKER_SESSION_TOKEN;

/// One line as it arrived.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Which connection, from 0.
    pub conn: usize,
    pub at: Time,
    pub mono: Mono,
    pub text: String,
}

/// One connection read as a notice (NTF-6): its auth line's token if it sent one, and the message's `content`.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub at: Time,
    pub mono: Mono,
    pub token: Option<String>,
    pub text: String,
}

/// What the listener is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Recording,
    Refusing,
    Vanished,
    NeverRead,
    QueueFull,
}

struct Conn {
    stream: UnixStream,
    number: usize,
    buf: Vec<u8>,
    eof: bool,
    /// Accepted under `accept_and_never_read`: kept open, never read.
    never_read: bool,
}

struct State {
    mode: Mode,
    listener: Option<UnixListener>,
    conns: Vec<Conn>,
    next_conn: usize,
    lines: Vec<Line>,
    /// The connections `queue_full` made to fill the backlog; held so it stays full.
    fillers: Vec<Connecting>,
    closed: bool,
}

struct Inner {
    path: PathBuf,
    clock: Arc<TestClock>,
    state: Mutex<State>,
}

pub struct FakeSession {
    inner: Arc<Inner>,
}

/// A non-blocking listener at `path`, mode 0600, made with `socket`, `bind` and `listen` so the
/// backlog is ours: `None` is the system's maximum, `Some(0)` is `queue_full`'s.
fn bind_listener(path: &Path, backlog: Option<i32>) -> UnixListener {
    use nix::sys::socket::{
        AddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind, listen, socket,
    };
    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        None,
    )
    .expect("a socket");
    let addr = UnixAddr::new(path).expect("a socket address");
    bind(fd.as_raw_fd(), &addr).expect("bind the fake session socket");
    let backlog = match backlog {
        Some(n) => Backlog::new(n).expect("a backlog"),
        None => Backlog::MAXCONN,
    };
    listen(&fd, backlog).expect("listen on the fake session socket");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .expect("mode 0600 on the fake session socket");
    UnixListener::from(fd)
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Accepts what's pending and reads every readable connection to what it has now.
    fn pump(&self, s: &mut State) {
        if s.closed {
            return;
        }
        if let (Mode::Recording | Mode::NeverRead, Some(listener)) = (s.mode, s.listener.as_ref()) {
            let never_read = s.mode == Mode::NeverRead;
            let mut accepted = Vec::new();
            loop {
                match listener.accept() {
                    Ok((stream, _)) => accepted.push(stream),
                    Err(e)
                        if matches!(
                            e.kind(),
                            ErrorKind::Interrupted | ErrorKind::ConnectionAborted
                        ) => {}
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => panic!("the fake session socket's accept failed: {e}"),
                }
            }
            for stream in accepted {
                stream
                    .set_nonblocking(true)
                    .expect("a non-blocking connection");
                s.conns.push(Conn {
                    stream,
                    number: s.next_conn,
                    buf: Vec::new(),
                    eof: false,
                    never_read,
                });
                s.next_conn += 1;
            }
        }
        let (at, mono) = (self.clock.wall(), self.clock.mono());
        let State { conns, lines, .. } = s;
        for conn in conns.iter_mut().filter(|c| !c.never_read && !c.eof) {
            let mut chunk = [0u8; 4096];
            loop {
                match (&conn.stream).read(&mut chunk) {
                    Ok(0) => {
                        conn.eof = true;
                        break;
                    }
                    Ok(n) => conn.buf.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    // A reset ends the connection as an end of stream does.
                    Err(_) => {
                        conn.eof = true;
                        break;
                    }
                }
            }
            while let Some(i) = conn.buf.iter().position(|b| *b == b'\n') {
                let rest = conn.buf.split_off(i + 1);
                let mut line = std::mem::replace(&mut conn.buf, rest);
                line.pop();
                lines.push(Line {
                    conn: conn.number,
                    at,
                    mono,
                    text: String::from_utf8_lossy(&line).into_owned(),
                });
            }
        }
    }

    fn is_settled(&self) -> bool {
        let mut s = self.lock();
        self.pump(&mut s);
        s.closed || s.conns.iter().all(|c| c.never_read || c.eof)
    }
}

impl SettleProbe for Inner {
    fn settled(&self) -> bool {
        self.is_settled()
    }
}

impl SettleProbe for FakeSession {
    fn settled(&self) -> bool {
        self.inner.is_settled()
    }
}

impl FakeSession {
    /// Binds `<dir>/<name>.sock` and adds itself to the clock as a `SettleProbe`. `dir` is a short one (`ShortDir`).
    pub fn bind(dir: &Path, name: &str, clock: Arc<TestClock>) -> Arc<FakeSession> {
        let path = dir.join(format!("{name}.sock"));
        let listener = bind_listener(&path, None);
        let inner = Arc::new(Inner {
            path,
            clock: clock.clone(),
            state: Mutex::new(State {
                mode: Mode::Recording,
                listener: Some(listener),
                conns: Vec::new(),
                next_conn: 0,
                lines: Vec::new(),
                fillers: Vec::new(),
                closed: false,
            }),
        });
        clock.add_probe(inner.clone());
        Arc::new(FakeSession { inner })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// `MARKER_SESSION_TOKEN`.
    pub fn token(&self) -> &'static str {
        MARKER_SESSION_TOKEN
    }

    /// Every line posted so far, in the order read, each with the clock's time when it was read. Reads what's pending first.
    pub fn lines(&self) -> Vec<Line> {
        let mut s = self.inner.lock();
        self.inner.pump(&mut s);
        s.lines.clone()
    }

    /// The same, one per connection. Panics, naming the line, on anything that isn't NTF-6's shape: an
    /// optional `{"type":"auth","token":…}` line, then exactly one `{"type":"user","message":{"role":"user","content":…}}` line.
    pub fn notices(&self) -> Vec<Notice> {
        let lines = self.lines();
        let numbers: Vec<usize> = {
            let s = self.inner.lock();
            s.conns
                .iter()
                .filter(|c| !c.never_read)
                .map(|c| c.number)
                .collect()
        };
        numbers
            .into_iter()
            .map(|n| {
                let mine: Vec<&Line> = lines.iter().filter(|l| l.conn == n).collect();
                notice_of(n, &mine)
            })
            .collect()
    }

    /// The listener is closed and its file left, so a connect is refused and a `stat` still sees the same socket.
    pub fn refuse(&self) {
        let mut s = self.inner.lock();
        self.inner.pump(&mut s);
        s.listener = None;
        s.fillers.clear();
        s.mode = Mode::Refusing;
    }

    /// The file is removed too.
    pub fn vanish(&self) {
        let mut s = self.inner.lock();
        self.inner.pump(&mut s);
        s.listener = None;
        s.fillers.clear();
        let _ = std::fs::remove_file(&self.inner.path);
        s.mode = Mode::Vanished;
    }

    /// Connections are accepted and nothing is ever read from them.
    pub fn accept_and_never_read(&self) {
        let mut s = self.inner.lock();
        self.inner.pump(&mut s);
        if s.listener.is_none() || s.mode == Mode::QueueFull {
            self.reopen(&mut s, None);
        }
        s.mode = Mode::NeverRead;
    }

    /// It listens without accepting until its backlog is full, so a connect fails at once.
    ///
    /// A new socket file on the same path, listening with a backlog of 0. The fake fills the backlog with
    /// connections of its own (one or two on Linux), from a non-blocking connect, until one gets `EAGAIN`,
    /// so this needs the tokio runtime (every test has it).
    pub fn queue_full(&self) {
        let mut s = self.inner.lock();
        self.inner.pump(&mut s);
        self.reopen(&mut s, Some(0));
        s.mode = Mode::QueueFull;
        for _ in 0..1_000_000 {
            match connect_now(&self.inner.path) {
                Ok(stream) => s.fillers.push(stream),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(e) => panic!("filling the fake session socket's backlog failed: {e}"),
            }
        }
        panic!("the fake session socket's backlog never filled");
    }

    /// Back to recording, on the same path. After `vanish` and `queue_full` it's a new socket file: another
    /// inode. After `refuse` too, in this build: std can't listen again on a closed listener's file, so the
    /// stale file is replaced.
    pub fn restore(&self) {
        let mut s = self.inner.lock();
        self.inner.pump(&mut s);
        if s.mode != Mode::Recording {
            self.reopen(&mut s, None);
        }
        s.mode = Mode::Recording;
    }

    /// A new listener on the path, over whatever file is there.
    fn reopen(&self, s: &mut State, backlog: Option<i32>) {
        s.listener = None;
        s.fillers.clear();
        let _ = std::fs::remove_file(&self.inner.path);
        s.listener = Some(bind_listener(&self.inner.path, backlog));
    }
}

impl Drop for FakeSession {
    fn drop(&mut self) {
        let mut s = self.inner.lock();
        s.closed = true;
        s.listener = None;
        s.fillers.clear();
        s.conns.clear();
        let _ = std::fs::remove_file(&self.inner.path);
    }
}

/// A connect in flight, kept so the connection stays open.
type Connecting = std::pin::Pin<
    Box<dyn std::future::Future<Output = std::io::Result<tokio::net::UnixStream>> + Send>,
>;

/// A non-blocking connect, polled once: what a tokio client does. `WouldBlock` is `EAGAIN`, a full backlog.
/// tokio's `connect` makes the `connect(2)` call at once and then waits for the socket to be writable, so
/// a future that is still pending has connected; it's handed back to hold the connection open.
/// Needs the tokio runtime's context.
fn connect_now(path: &Path) -> std::io::Result<Connecting> {
    use std::task::{Context, Poll, Waker};
    let path = path.to_path_buf();
    let mut fut: Connecting = Box::pin(async move { tokio::net::UnixStream::connect(path).await });
    match fut.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Ok(stream)) => Ok(Box::pin(std::future::ready(Ok(stream)))),
        Poll::Ready(Err(e)) => Err(e),
        Poll::Pending => Ok(fut),
    }
}

fn notice_of(conn: usize, lines: &[&Line]) -> Notice {
    let bad = |l: &Line, why: &str| -> ! {
        panic!("connection {conn}: {why}: {:?}", l.text);
    };
    let mut token = None;
    let mut message: Option<(&Line, String)> = None;
    for l in lines {
        let v: serde_json::Value = serde_json::from_str(&l.text)
            .unwrap_or_else(|e| bad(l, &format!("a line that isn't JSON ({e})")));
        let Some(obj) = v.as_object() else {
            bad(l, "a line that isn't an object");
        };
        match obj.get("type").and_then(|t| t.as_str()) {
            Some("auth") => {
                if token.is_some() || message.is_some() {
                    bad(l, "an auth line out of place");
                }
                if obj.len() != 2 {
                    bad(l, "an auth line with keys besides type and token");
                }
                match obj.get("token").and_then(|t| t.as_str()) {
                    Some(t) => token = Some(t.to_string()),
                    None => bad(l, "an auth line without a string token"),
                }
            }
            Some("user") => {
                if message.is_some() {
                    bad(l, "a second message line");
                }
                if obj.len() != 2 {
                    bad(l, "a message line with keys besides type and message");
                }
                let Some(m) = obj.get("message").and_then(|m| m.as_object()) else {
                    bad(l, "a message line without a message object");
                };
                if m.get("role").and_then(|r| r.as_str()) != Some("user") {
                    bad(l, "a message whose role isn't user");
                }
                if m.len() != 2 {
                    bad(l, "a message with keys besides role and content");
                }
                match m.get("content").and_then(|c| c.as_str()) {
                    Some(c) => message = Some((l, c.to_string())),
                    None => bad(l, "a message without a string content"),
                }
            }
            _ => bad(l, "a line whose type is neither auth nor user"),
        }
    }
    match message {
        Some((l, text)) => Notice {
            at: l.at,
            mono: l.mono,
            token,
            text,
        },
        None => panic!("connection {conn} has no message line"),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    use std::time::Duration;

    use super::*;
    use crate::testkit::ShortDir;

    const MSG: &str = r#"{"type":"user","message":{"role":"user","content":"hello"}}"#;

    fn start() -> Time {
        Time::parse("2026-11-24T22:00:00Z").expect("a time")
    }

    fn setup() -> (ShortDir, Arc<TestClock>, Arc<FakeSession>) {
        let dir = ShortDir::new();
        let clock = TestClock::new(start());
        let session = FakeSession::bind(dir.path(), "s", clock.clone());
        (dir, clock, session)
    }

    fn post(session: &FakeSession, text: &str) {
        let mut c = UnixStream::connect(session.path()).expect("connect");
        c.write_all(text.as_bytes()).expect("write");
        c.write_all(b"\n").expect("write");
    }

    fn auth() -> String {
        format!(r#"{{"type":"auth","token":"{MARKER_SESSION_TOKEN}"}}"#)
    }

    // TST-16
    #[tokio::test]
    async fn tst16_records_lines_with_their_time() {
        let (_dir, clock, session) = setup();
        {
            let mut c = UnixStream::connect(session.path()).unwrap();
            c.write_all(format!("{}\n{MSG}\n", auth()).as_bytes())
                .unwrap();
        }
        clock.settle().await;
        let t0 = clock.wall();
        let lines = session.lines();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.conn == 0 && l.at == t0));
        assert_eq!(lines[1].text, MSG);
        clock.advance(Duration::from_secs(30)).await;
        post(&session, MSG);
        clock.settle().await;
        let lines = session.lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2].conn, 1);
        assert_eq!(lines[2].at, t0.plus(Duration::from_secs(30)));
        assert_eq!(lines[2].mono, clock.mono());
        let notices = session.notices();
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].token.as_deref(), Some(MARKER_SESSION_TOKEN));
        assert_eq!(notices[0].text, "hello");
        assert_eq!(notices[1].token, None);
        assert_eq!(notices[1].at, t0.plus(Duration::from_secs(30)));
    }

    // TST-16
    #[tokio::test]
    async fn tst16_without_an_auth_line() {
        let (_dir, clock, session) = setup();
        post(&session, MSG);
        post(
            &session,
            r#"{"type":"user","message":{"role":"user","content":"a\nb\n\nc"}}"#,
        );
        clock.settle().await;
        let lines = session.lines();
        assert_eq!(lines.len(), 2);
        let notices = session.notices();
        assert_eq!(notices[0].token, None);
        assert_eq!(notices[1].text, "a\nb\n\nc");
    }

    // TST-16
    #[tokio::test]
    async fn tst16_never_answers() {
        let (_dir, _clock, session) = setup();
        let mut c = UnixStream::connect(session.path()).unwrap();
        c.write_all(format!("{MSG}\n").as_bytes()).unwrap();
        // The client holds its end open, so the fake isn't settled: look without settling.
        assert_eq!(session.lines().len(), 1);
        c.set_nonblocking(true).unwrap();
        let mut buf = [0u8; 16];
        let err = c.read(&mut buf).expect_err("no byte and no end of stream");
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
    }

    fn malformed(text: &str) {
        let (_dir, _clock, session) = setup();
        post(&session, text);
        session.notices();
    }

    // TST-16
    #[test]
    #[should_panic(expected = "isn't JSON")]
    fn tst16_malformed_panics_plain_text() {
        malformed("hello there");
    }

    // TST-16
    #[test]
    #[should_panic(expected = "neither auth nor user")]
    fn tst16_malformed_panics_wrong_type() {
        malformed(r#"{"type":"message","message":{"role":"user","content":"x"}}"#);
    }

    // TST-16
    #[test]
    #[should_panic(expected = "a second message line")]
    fn tst16_malformed_panics_two_messages() {
        let (_dir, _clock, session) = setup();
        let mut c = UnixStream::connect(session.path()).unwrap();
        c.write_all(format!("{MSG}\n{MSG}\n").as_bytes()).unwrap();
        session.notices();
    }

    // TST-16
    #[tokio::test]
    async fn tst16_refuse() {
        let (_dir, clock, session) = setup();
        let before = std::fs::metadata(session.path()).unwrap();
        session.refuse();
        let err = UnixStream::connect(session.path()).expect_err("refused");
        assert_eq!(err.kind(), ErrorKind::ConnectionRefused);
        let after = std::fs::metadata(session.path()).unwrap();
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        clock.settle().await;
    }

    // TST-16
    #[tokio::test]
    async fn tst16_vanish() {
        let (_dir, _clock, session) = setup();
        session.vanish();
        let err = UnixStream::connect(session.path()).expect_err("gone");
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(!session.path().exists());
    }

    // TST-16
    #[tokio::test]
    async fn tst16_accept_and_never_read() {
        let (_dir, clock, session) = setup();
        session.accept_and_never_read();
        let mut c = UnixStream::connect(session.path()).expect("a connect succeeds");
        c.write_all(&[b'x'; 299]).unwrap();
        c.write_all(b"\n").unwrap();
        clock.settle().await;
        assert!(session.lines().is_empty());
        assert!(session.notices().is_empty());
    }

    // TST-16
    #[tokio::test]
    async fn tst16_queue_full() {
        let (_dir, clock, session) = setup();
        session.queue_full();
        let err = connect_now(session.path())
            .err()
            .expect("a full queue fails at once");
        println!("queue_full: a connect saw {:?}: {err}", err.kind());
        assert_eq!(err.kind(), ErrorKind::WouldBlock);
        let used = session.inner.lock().fillers.len();
        assert!(used < 10, "{used} descriptors used to fill the backlog");
        clock.settle().await;
        assert!(session.lines().is_empty());
    }

    // TST-16
    #[tokio::test]
    async fn tst16_restore() {
        let (_dir, clock, session) = setup();
        for mode in 0..4 {
            match mode {
                0 => session.refuse(),
                1 => session.vanish(),
                2 => session.accept_and_never_read(),
                _ => session.queue_full(),
            }
            session.restore();
            post(&session, MSG);
            clock.settle().await;
            assert_eq!(session.notices().len(), mode + 1, "after mode {mode}");
        }
    }

    // TST-16
    #[tokio::test]
    async fn tst16_settles() {
        let (_dir, _clock, session) = setup();
        assert!(session.settled());
        let mut c = UnixStream::connect(session.path()).unwrap();
        c.write_all(b"{\"type\":\"user\"").unwrap();
        assert!(!session.settled());
        c.write_all(b",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n")
            .unwrap();
        assert!(!session.settled(), "the client hasn't closed");
        drop(c);
        assert!(session.settled());
        assert_eq!(session.notices().len(), 1);

        session.accept_and_never_read();
        let mut c = UnixStream::connect(session.path()).unwrap();
        c.write_all(b"half").unwrap();
        assert!(session.settled());
        session.queue_full();
        assert!(session.settled());
        drop(c);
    }

    // TST-16
    #[tokio::test]
    async fn tst16_mode_0600() {
        let (_dir, _clock, session) = setup();
        let mode = std::fs::metadata(session.path()).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600);
        session.restore();
        session.vanish();
        session.restore();
        let mode = std::fs::metadata(session.path()).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn dropping_removes_the_file() {
        let (_dir, _clock, session) = setup();
        let path = session.path().to_path_buf();
        assert!(path.exists());
        drop(session);
        assert!(!path.exists());
    }
}
