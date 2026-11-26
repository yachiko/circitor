//! A stub daemon for `notify`'s tests: it answers `testdata/api/` files per route like T24's, and keeps
//! each request's body too, which the tests of the body's keys need. Each route has a queue of answers,
//! then its default.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::AbortHandle;

use crate::clock::{SettleProbe, TestClock};

/// One recorded request.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    /// `METHOD /path?query`.
    pub line: String,
    pub body: String,
}

pub(crate) fn golden(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/api")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("a golden file of testdata/api");
    serde_json::from_str(&text).expect("golden JSON")
}

/// An answer: its HTTP status and body.
#[derive(Clone, Debug)]
pub(crate) struct Resp {
    pub status: u16,
    pub body: String,
}

impl Resp {
    pub(crate) fn json(body: serde_json::Value) -> Resp {
        Resp {
            status: 200,
            body: body.to_string(),
        }
    }

    pub(crate) fn file(name: &str) -> Resp {
        let body = golden(name);
        if name.starts_with("error-") {
            let status = match body["error"]["code"].as_str().unwrap_or("") {
                "bad_request" => 400,
                "not_found" | "unknown_route" => 404,
                "limit" | "no_session" => 409,
                _ => 503,
            };
            Resp {
                status,
                body: body.to_string(),
            }
        } else {
            Resp::json(body)
        }
    }
}

fn route(method: &str, target: &str) -> &'static str {
    let path = target.split('?').next().unwrap_or(target);
    match (method, path) {
        ("GET", "/v1/status") => "status",
        ("POST", "/v1/refresh") => "refresh",
        ("GET", "/v1/prs") => "list",
        ("POST", p) if p.starts_with("/v1/prs/") && p.ends_with("/notify") => "notify",
        ("DELETE", p) if p.starts_with("/v1/prs/") && p.ends_with("/notify") => "cancel",
        _ => "unknown",
    }
}

fn default_for(route: &str, version: &str) -> Resp {
    let file = match route {
        "status" => "status.json",
        "refresh" => "refresh.json",
        "list" => "list.json",
        "notify" => "notify-registered.json",
        "cancel" => "notify-cancelled.json",
        _ => "error-unknown_route.json",
    };
    let mut r = Resp::file(file);
    if route == "status" {
        // The probe's `StatusLite`: the CLI's own version, so nothing is replaced.
        let mut v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        v["version"] = version.into();
        r.body = v.to_string();
    }
    r
}

struct State {
    path: PathBuf,
    version: String,
    scripts: Mutex<HashMap<&'static str, VecDeque<Resp>>>,
    requests: Mutex<Vec<Seen>>,
    handling: AtomicUsize,
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

pub(crate) struct Stub(Arc<State>);

impl Stub {
    /// Binds `path`, answers with the CLI's `version` in its status, and tells `clock` when it's busy.
    pub(crate) fn start(path: &Path, version: &str, clock: &Arc<TestClock>) -> Stub {
        let std = std::os::unix::net::UnixListener::bind(path).expect("the stub binds its socket");
        std.set_nonblocking(true).unwrap();
        let listener = UnixListener::from_std(std).unwrap();
        let state = Arc::new(State {
            path: path.to_path_buf(),
            version: version.to_string(),
            scripts: Mutex::new(HashMap::new()),
            requests: Mutex::new(Vec::new()),
            handling: AtomicUsize::new(0),
            accept: Mutex::new(None),
        });
        clock.add_probe(state.clone());
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

    /// The next answers of `route` (`notify`, `cancel`, `list`, `status`, `refresh`), in order.
    pub(crate) fn script(&self, route: &'static str, resps: impl IntoIterator<Item = Resp>) {
        guard(&self.0.scripts)
            .entry(route)
            .or_default()
            .extend(resps);
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        guard(&self.0.requests).clone()
    }

    /// The requests of one route.
    pub(crate) fn of(&self, route_name: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|r| {
                let (m, t) = r.line.split_once(' ').unwrap_or((&r.line, ""));
                route(m, t) == route_name
            })
            .collect()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        if let Some(h) = guard(&self.0.accept).take() {
            h.abort();
            let _ = std::fs::remove_file(&self.0.path);
        }
    }
}

async fn read_request(s: &mut UnixStream) -> Option<Seen> {
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
    Some(Seen {
        line: format!("{} {}", parts.next()?, parts.next()?),
        body: String::from_utf8_lossy(&buf[end..]).to_string(),
    })
}

async fn serve(state: Arc<State>, mut s: UnixStream) {
    let Some(req) = read_request(&mut s).await else {
        state.handling.fetch_sub(1, Ordering::SeqCst);
        return;
    };
    let (method, target) = req.line.split_once(' ').unwrap_or((&req.line, ""));
    let name = route(method, target);
    guard(&state.requests).push(req.clone());
    let resp = guard(&state.scripts)
        .get_mut(name)
        .and_then(|q| q.pop_front())
        .unwrap_or_else(|| default_for(name, &state.version));
    let reason = if resp.status == 200 { "OK" } else { "Error" };
    let bytes = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        resp.status,
        resp.body.len(),
        resp.body
    );
    let _ = s.write_all(bytes.as_bytes()).await;
    let _ = s.shutdown().await;
    // Settled only once the client has read the answer and let the connection go.
    let mut b = [0u8; 256];
    while matches!(s.read(&mut b).await, Ok(n) if n > 0) {}
    state.handling.fetch_sub(1, Ordering::SeqCst);
}
