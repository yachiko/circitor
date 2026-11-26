//! A scripted stub daemon for the commands' tests. T24's stub (`conn::stub`) answers one fixed file per
//! route; these tests also need error answers, a `circitor-cursor` header, answers on the test clock,
//! and a wait whose connection drops after its head. Each route has a queue of answers, then its default.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::AbortHandle;

use crate::clock::{Clock, SettleProbe, TestClock};

/// What the stub does with one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    Answer,
    /// Reads the request and never answers.
    Stall,
    /// Reads the request and closes without an answer.
    Close,
    /// Sends the status line and headers, then closes.
    HeadThenClose,
    /// Sends the head, then stops serving (socket file removed), then closes: no daemon comes back.
    HeadThenVanish,
}

#[derive(Clone, Debug)]
pub(crate) struct Resp {
    pub status: u16,
    pub body: String,
    pub cursor: Option<String>,
    pub act: Act,
    /// Waited on the test clock before acting.
    pub delay: Duration,
}

impl Resp {
    pub(crate) fn ok(body: serde_json::Value) -> Resp {
        let cursor = body
            .get("cursor")
            .and_then(|c| c.as_str())
            .map(str::to_string);
        Resp {
            status: 200,
            body: body.to_string(),
            cursor,
            act: Act::Answer,
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn file(name: &str) -> Resp {
        let body = golden(name);
        if name.starts_with("error-") {
            Resp::error(body)
        } else {
            Resp::ok(body)
        }
    }

    /// An API-3 error with its code's HTTP status.
    pub(crate) fn error(body: serde_json::Value) -> Resp {
        let code = body["error"]["code"].as_str().unwrap_or("").to_string();
        let status = match code.as_str() {
            "bad_request" => 400,
            "not_found" | "unknown_route" => 404,
            "limit" | "no_session" => 409,
            _ => 503,
        };
        Resp {
            status,
            body: body.to_string(),
            cursor: None,
            act: Act::Answer,
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn act(mut self, act: Act) -> Resp {
        self.act = act;
        self
    }

    pub(crate) fn after(mut self, d: Duration) -> Resp {
        self.delay = d;
        self
    }

    pub(crate) fn cursor(mut self, c: &str) -> Resp {
        self.cursor = Some(c.to_string());
        self
    }
}

pub(crate) fn golden(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/api")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("a golden file of testdata/api");
    serde_json::from_str(&text).expect("golden JSON")
}

/// The route's name, for the scripts.
pub(crate) fn route(method: &str, target: &str) -> &'static str {
    let path = target.split('?').next().unwrap_or(target);
    match (method, path) {
        ("GET", "/v1/status") => "status",
        ("POST", "/v1/refresh") => "refresh",
        ("POST", "/v1/track") => "track",
        ("POST", "/v1/shutdown") => "shutdown",
        ("DELETE", p) if p.starts_with("/v1/track/") => "untrack",
        ("GET", "/v1/prs") => "list",
        ("GET", "/v1/changes") => "changes",
        ("GET", p) if p.starts_with("/v1/prs/") && p.ends_with("/changes") => "changes",
        ("GET", p) if p.starts_with("/v1/prs/") && p.ends_with("/wait") => "wait",
        ("GET", p) if p.starts_with("/v1/prs/") => "show",
        _ => "unknown",
    }
}

fn default_for(route: &str, version: &str) -> Resp {
    let file = match route {
        "status" => "status.json",
        "refresh" => "refresh.json",
        "track" => "track.json",
        "untrack" => "untrack-null.json",
        "list" => "list.json",
        "changes" => "changes.json",
        "wait" => "wait-satisfied.json",
        "show" => "show.json",
        "shutdown" => return Resp::ok(serde_json::json!({ "version": version })),
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
    clock: Arc<TestClock>,
    version: String,
    scripts: Mutex<HashMap<&'static str, VecDeque<Resp>>>,
    defaults: Mutex<HashMap<&'static str, Resp>>,
    requests: Mutex<Vec<String>>,
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
            clock: clock.clone(),
            version: version.to_string(),
            scripts: Mutex::new(HashMap::new()),
            defaults: Mutex::new(HashMap::new()),
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

    /// The next answers of `route`, in order.
    pub(crate) fn script(&self, route: &'static str, resps: impl IntoIterator<Item = Resp>) {
        guard(&self.0.scripts)
            .entry(route)
            .or_default()
            .extend(resps);
    }

    /// What `route` answers once its script is used up.
    pub(crate) fn default(&self, route: &'static str, resp: Resp) {
        guard(&self.0.defaults).insert(route, resp);
    }

    /// Every request, as `METHOD /path?query`.
    pub(crate) fn requests(&self) -> Vec<String> {
        guard(&self.0.requests).clone()
    }

    /// The requests of one route, as `METHOD /path?query`.
    pub(crate) fn of(&self, route_name: &str) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|r| {
                let (m, t) = r.split_once(' ').unwrap_or((r, ""));
                route(m, t) == route_name
            })
            .collect()
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0.path
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        vanish(&self.0);
    }
}

fn vanish(state: &State) {
    if let Some(h) = guard(&state.accept).take() {
        h.abort();
        let _ = std::fs::remove_file(&state.path);
    }
}

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

fn head(resp: &Resp) -> String {
    let reason = if resp.status == 200 { "OK" } else { "Error" };
    let cursor = resp
        .cursor
        .as_ref()
        .map_or(String::new(), |c| format!("circitor-cursor: {c}\r\n"));
    format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{cursor}Connection: close\r\n\r\n",
        resp.status,
        resp.body.len()
    )
}

async fn serve(state: Arc<State>, mut s: UnixStream) {
    let done = |state: &State| {
        state.handling.fetch_sub(1, Ordering::SeqCst);
    };
    let Some(req) = read_request(&mut s).await else {
        done(&state);
        return;
    };
    guard(&state.requests).push(req.clone());
    let (method, target) = req.split_once(' ').unwrap_or((&req, ""));
    let name = route(method, target);
    let scripted = guard(&state.scripts)
        .get_mut(name)
        .and_then(|q| q.pop_front());
    let resp = scripted
        .or_else(|| guard(&state.defaults).get(name).cloned())
        .unwrap_or_else(|| default_for(name, &state.version));
    if !resp.delay.is_zero() {
        // The timer is set while still busy, so the clock can't move past it unseen.
        let at = state.clock.mono().plus(resp.delay);
        let sleep = state.clock.sleep_until(at);
        done(&state);
        sleep.await;
        state.handling.fetch_add(1, Ordering::SeqCst);
    }
    match resp.act {
        Act::Stall => {
            done(&state);
            std::future::pending::<()>().await;
        }
        Act::Close => {
            drop(s);
            done(&state);
        }
        Act::HeadThenClose | Act::HeadThenVanish => {
            let _ = s.write_all(head(&resp).as_bytes()).await;
            let _ = s.flush().await;
            if resp.act == Act::HeadThenVanish {
                vanish(&state);
            }
            drop(s);
            done(&state);
        }
        Act::Answer => {
            let mut bytes = head(&resp).into_bytes();
            bytes.extend_from_slice(resp.body.as_bytes());
            let _ = s.write_all(&bytes).await;
            let _ = s.shutdown().await;
            // Settled only once the client has read the answer and let the connection go.
            let mut b = [0u8; 256];
            while matches!(s.read(&mut b).await, Ok(n) if n > 0) {}
            done(&state);
        }
    }
}
