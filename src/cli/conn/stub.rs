//! A stub daemon for CLI tests (T24; T27's tests reuse it): a few lines of server on a Unix socket that
//! answers `testdata/api/` files per route, with a version and `mode` of the test's choice. It can
//! accept without answering, close without answering, and count its requests.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nix::fcntl::{Flock, FlockArg};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::AbortHandle;

use crate::clock::SettleProbe;

/// What the stub does with one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    /// Answers the route with its golden file.
    Answer,
    /// Reads the request and never answers.
    Stall,
    /// Reads the request and closes the connection without an answer.
    Close,
}

struct State {
    path: PathBuf,
    version: Mutex<String>,
    mode: Mutex<String>,
    script: Mutex<VecDeque<Act>>,
    otherwise: Mutex<Act>,
    requests: Mutex<Vec<String>>,
    /// Connections accepted and not yet answered, closed or stalled.
    handling: AtomicUsize,
    lock: Mutex<Option<Flock<File>>>,
    keep_lock: AtomicBool,
    accept: Mutex<Option<AbortHandle>>,
}

fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SettleProbe for State {
    fn settled(&self) -> bool {
        self.handling.load(Ordering::SeqCst) == 0
    }
}

/// The stub. Dropping it stops accepting.
pub(crate) struct Stub(Arc<State>);

impl Stub {
    /// Binds `path` and serves it. Must run inside a runtime.
    pub(crate) fn start(path: &Path, version: &str, mode: &str) -> Stub {
        let std = std::os::unix::net::UnixListener::bind(path).expect("the stub binds its socket");
        std.set_nonblocking(true).expect("a non-blocking socket");
        let listener = UnixListener::from_std(std).expect("a tokio listener");
        let state = Arc::new(State {
            path: path.to_path_buf(),
            version: Mutex::new(version.to_string()),
            mode: Mutex::new(mode.to_string()),
            script: Mutex::new(VecDeque::new()),
            otherwise: Mutex::new(Act::Answer),
            requests: Mutex::new(Vec::new()),
            handling: AtomicUsize::new(0),
            lock: Mutex::new(None),
            keep_lock: AtomicBool::new(false),
            accept: Mutex::new(None),
        });
        let st = state.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((s, _)) = listener.accept().await else {
                    return;
                };
                st.handling.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(serve(st.clone(), s));
            }
        });
        *guard(&state.accept) = Some(task.abort_handle());
        Stub(state)
    }

    /// Tells the test clock when the stub is still answering.
    pub(crate) fn probe(&self) -> Arc<dyn SettleProbe> {
        self.0.clone()
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0.path
    }

    /// What the next connections get, in order; after them, `otherwise`'s act.
    pub(crate) fn script(&self, acts: impl IntoIterator<Item = Act>) {
        guard(&self.0.script).extend(acts);
    }

    /// What every connection gets once the script is used up (`Answer` unless set).
    pub(crate) fn otherwise(&self, act: Act) {
        *guard(&self.0.otherwise) = act;
    }

    /// Each request as `METHOD /path?query`, in the order they arrived.
    pub(crate) fn requests(&self) -> Vec<String> {
        guard(&self.0.requests).clone()
    }

    /// How many requests started with `prefix` (`"POST /v1/shutdown"`).
    pub(crate) fn count(&self, prefix: &str) -> usize {
        guard(&self.0.requests)
            .iter()
            .filter(|r| r.starts_with(prefix))
            .count()
    }

    /// Takes the lock in `lock_file` and writes the holder's line, `<pid> <this socket>`.
    pub(crate) fn hold_lock(&self, lock_file: &Path, pid: u32) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_file)
            .expect("the lock file opens");
        let lock = Flock::lock(file, FlockArg::LockExclusiveNonblock)
            .map_err(|(_, e)| e)
            .expect("the stub takes the lock");
        let f: &File = &lock;
        f.set_len(0).unwrap();
        let mut line = format!("{pid} ").into_bytes();
        line.extend_from_slice(self.0.path.as_os_str().as_encoded_bytes());
        line.push(b'\n');
        (&mut &*f).write_all(&line).unwrap();
        *guard(&self.0.lock) = Some(lock);
    }

    /// A shutdown answers but leaves the lock held and the socket served.
    pub(crate) fn keep_lock_on_shutdown(&self) {
        self.0.keep_lock.store(true, Ordering::SeqCst);
    }

    pub(crate) fn release_lock(&self) {
        guard(&self.0.lock).take();
    }

    /// Stops accepting and removes the socket file: from now on the path is "no daemon".
    pub(crate) fn close(&self) {
        close(&self.0);
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        if let Some(h) = guard(&self.0.accept).take() {
            h.abort();
        }
    }
}

fn close(state: &State) {
    if let Some(h) = guard(&state.accept).take() {
        h.abort();
    }
    let _ = std::fs::remove_file(&state.path);
}

/// Reads one request: its head, and its body by `Content-Length`. Returns `METHOD /path?query`.
async fn read_request(s: &mut UnixStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut b = [0u8; 4096];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = s.read(&mut b).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&b[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let length = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < end + length {
        let n = s.read(&mut b).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&b[..n]);
    }
    let first = head.lines().next()?;
    let mut parts = first.split(' ');
    Some(format!("{} {}", parts.next()?, parts.next()?))
}

fn golden(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/api")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("a golden file of testdata/api");
    serde_json::from_str(&text).expect("golden JSON")
}

/// The golden file of a route, and its HTTP status.
fn route(method: &str, target: &str) -> (u16, &'static str) {
    let path = target.split('?').next().unwrap_or(target);
    match (method, path) {
        ("GET", "/v1/status") => (200, "status.json"),
        ("POST", "/v1/refresh") => (200, "refresh.json"),
        ("POST", "/v1/track") => (200, "track.json"),
        ("DELETE", p) if p.starts_with("/v1/track/") => (200, "untrack-null.json"),
        ("GET", "/v1/prs") => (200, "list.json"),
        ("GET", "/v1/changes") => (200, "changes.json"),
        ("GET", p) if p.starts_with("/v1/prs/") && p.ends_with("/changes") => (200, "changes.json"),
        ("GET", p) if p.starts_with("/v1/prs/") && p.ends_with("/wait") => {
            (200, "wait-satisfied.json")
        }
        ("POST", p) if p.starts_with("/v1/prs/") && p.ends_with("/notify") => {
            (200, "notify-registered.json")
        }
        ("DELETE", p) if p.starts_with("/v1/prs/") && p.ends_with("/notify") => {
            (200, "notify-cancelled.json")
        }
        ("GET", p) if p.starts_with("/v1/prs/") => (200, "show.json"),
        _ => (404, "error-unknown_route.json"),
    }
}

fn answer(state: &State, method: &str, target: &str) -> (u16, String) {
    let version = guard(&state.version).clone();
    if method == "POST" && target == "/v1/shutdown" {
        return (200, serde_json::json!({ "version": version }).to_string());
    }
    let (status, name) = route(method, target);
    let mut body = golden(name);
    if let Some(obj) = body.as_object_mut() {
        obj.insert("version".into(), version.into());
        if name == "status.json" {
            obj.insert("mode".into(), guard(&state.mode).clone().into());
        }
    }
    (status, body.to_string())
}

async fn serve(state: Arc<State>, mut s: UnixStream) {
    let act = guard(&state.script)
        .pop_front()
        .unwrap_or(*guard(&state.otherwise));
    let Some(req) = read_request(&mut s).await else {
        state.handling.fetch_sub(1, Ordering::SeqCst);
        return;
    };
    guard(&state.requests).push(req.clone());
    match act {
        Act::Stall => {
            state.handling.fetch_sub(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
        Act::Close => {
            drop(s);
            state.handling.fetch_sub(1, Ordering::SeqCst);
        }
        Act::Answer => {
            let (method, target) = req.split_once(' ').unwrap_or((&req, ""));
            let (status, body) = answer(&state, method, target);
            if method == "POST"
                && target == "/v1/shutdown"
                && !state.keep_lock.load(Ordering::SeqCst)
            {
                // API-17 as the daemon does it: stop accepting, remove the socket, release the lock.
                close(&state);
                guard(&state.lock).take();
            }
            let reason = if status == 200 { "OK" } else { "Not Found" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(response.as_bytes()).await;
            let _ = s.shutdown().await;
            // Settled only once the client has read the answer and let the connection go, so the
            // test clock doesn't move while an answer is on its way.
            let mut b = [0u8; 256];
            while matches!(s.read(&mut b).await, Ok(n) if n > 0) {}
            state.handling.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
