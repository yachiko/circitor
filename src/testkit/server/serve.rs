//! The accept loop, each connection's life, and one request from arrival to answer.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::{Method, StatusCode};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::testkit::github::{Asked, Rejected, World, cost_of, describe, execute};

use super::answer::{self, Body, Headers};
use super::fault::{self, Fault};
use super::log::{Request, summary};
use super::{Inner, MOVED_PATH, Pending, PrFault, State};

/// One connection: what closes it, and the `Pending` it holds until its first request takes it.
pub(super) struct Conn {
    kill: Notify,
    pending: Mutex<Option<Pending>>,
}

impl Conn {
    /// Closes the connection without another byte.
    pub fn kill(&self) {
        self.kill.notify_one();
    }

    fn take_pending(&self) -> Option<Pending> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

/// Starts accepting on `listener`; the task is kept so `refuse` can close it.
pub(super) fn spawn_accept(inner: &Arc<Inner>, listener: TcpListener) {
    let task = tokio::spawn(accept_loop(inner.clone(), listener));
    *inner.accept.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
}

async fn accept_loop(inner: Arc<Inner>, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        // TST-4: a connection accepted and not yet answered is never settled.
        let conn = Arc::new(Conn {
            kill: Notify::new(),
            pending: Mutex::new(Some(Pending::new(&inner.unsettled))),
        });
        inner.add_conn(&conn);
        tokio::spawn(serve_conn(inner.clone(), conn, stream));
    }
}

async fn serve_conn(inner: Arc<Inner>, conn: Arc<Conn>, stream: TcpStream) {
    let svc_conn = conn.clone();
    let svc = service_fn(move |req| {
        let inner = inner.clone();
        let conn = svc_conn.clone();
        async move { Ok::<_, Infallible>(handle(inner, conn, req).await) }
    });
    // TST-3: `Date` is GitHub's clock or nothing, never hyper's reading of the system's time.
    let serving = http1::Builder::new()
        .auto_date_header(false)
        .serve_connection(TokioIo::new(stream), svc);
    // TST-3: a request ends when it's answered or its connection closes. hyper notices a client
    // that goes away mid-request and drops the request's future, ending it.
    tokio::select! {
        _ = serving => {}
        () = conn.kill.notified() => {}
    }
    conn.take_pending();
}

/// Ends a request in the rules when dropped: answered, or its connection closed.
struct RequestEnd {
    inner: Arc<Inner>,
    key: Option<String>,
    epoch: u64,
}

impl Drop for RequestEnd {
    fn drop(&mut self) {
        let at = self.inner.clock.mono();
        self.inner.lock().rules.end(&self.key, self.epoch, at);
    }
}

fn header(req: &hyper::Request<Incoming>, name: &str) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

async fn handle(
    inner: Arc<Inner>,
    conn: Arc<Conn>,
    req: hyper::Request<Incoming>,
) -> http::Response<Body> {
    let mut pending = conn
        .take_pending()
        .unwrap_or_else(|| Pending::new(&inner.unsettled));
    if req.uri().path() == MOVED_PATH {
        // OPS-16: a redirect's target counts its hits.
        inner
            .redirect_hits
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        return answer::plain(StatusCode::OK, "moved");
    }
    if req.uri().path() != "/graphql" || req.method() != Method::POST {
        return answer::plain(StatusCode::NOT_FOUND, "Not Found");
    }
    let mono = inner.clock.mono();
    let wall = inner.clock.wall();
    let authorization = header(&req, "authorization");
    let pid = header(&req, "x-circitor-test-pid");
    let key = if inner.real { pid.clone() } else { None };
    let body: Bytes = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        // The client went away mid-body: nothing to answer.
        Err(_) => return answer::plain(StatusCode::BAD_REQUEST, "body unreadable"),
    };
    let asked = describe(&body);

    // TST-3: every request is logged, and the rules are checked as it arrives.
    let (index, _end) = {
        let mut st = inner.lock();
        let index = st.log.len();
        let (finding, nodes, lookup) = match &asked {
            Ok(a) => (a.finding.clone(), a.nodes.clone(), a.lookup.clone()),
            Err(_) => (None, Vec::new(), None),
        };
        let request = Request {
            index,
            mono,
            wall,
            cost: 0,
            finding,
            nodes,
            lookup,
            authorization: authorization.clone(),
            pid,
            fault: None,
            rejected: asked.as_ref().err().cloned(),
        };
        let mut broken = st.rules.arrive(&key, mono);
        if let Err(why) = &asked {
            broken.push(rejection(why));
        }
        let what = summary(&request);
        st.broken
            .extend(broken.into_iter().map(|b| format!("{b}: {what}")));
        st.log.push(request);
        let end = RequestEnd {
            inner: inner.clone(),
            key,
            epoch: st.rules.epoch,
        };
        (index, end)
    };

    // A hold by script: the request waits, excused, and is answered from the world as it is then.
    let slot = inner.lock().holds.pop_front();
    if let Some(slot) = slot {
        drop(pending);
        {
            let _held = inner.hold_guard();
            slot.arrived.send_replace(true);
            let _ = slot.release.await;
        }
        pending = Pending::new(&inner.unsettled);
    }

    let played = {
        let mut st = inner.lock();
        let f = fault::take(&mut st.faults, asked.as_ref().ok());
        if let Some(f) = &f {
            st.log[index].fault = Some(f.clone());
        }
        f
    };
    match played {
        Some(Fault::Hang) => {
            drop(pending);
            let mut cleared = inner.cleared.subscribe();
            {
                let _held = inner.hold_guard();
                // Until the client goes away (this future is dropped) or the fault is cleared.
                let _ = cleared.changed().await;
            }
            let _closing = Pending::new(&inner.unsettled);
            conn.kill();
            std::future::pending::<()>().await;
            unreachable!("a killed connection drops its requests")
        }
        Some(Fault::Drop) => {
            conn.kill();
            std::future::pending::<()>().await;
            unreachable!("a killed connection drops its requests")
        }
        Some(f) => {
            let st = inner.lock();
            let h = headers(&st, reported(&st));
            let moved = format!("http://{}{MOVED_PATH}", inner.addr);
            return answer::fault(&f, &h, &moved);
        }
        None => {}
    }

    let response = answer(&inner, &body, asked.ok(), index, authorization.as_deref());
    drop(pending);
    response
}

fn rejection(why: &Rejected) -> String {
    match why {
        Rejected::Mutation => "POL-1: a mutation".to_owned(),
        Rejected::NotInSchema(what) => format!("TST-3: a query the schema refuses ({what})"),
        Rejected::NoResolver(what) => format!("TST-3: a field the fake can't answer ({what})"),
        Rejected::Unparsable(what) => format!("TST-3: a query that doesn't parse ({what})"),
    }
}

/// The headers' rate as answers report it: lagged when a lag is set (docs/02 §2).
fn reported(st: &State) -> (u32, crate::clock::Time) {
    if st.lag == 0 || st.rates.is_empty() {
        return st.world.rate();
    }
    let i = st.rates.len().saturating_sub(st.lag + 1);
    st.rates[i]
}

fn headers(st: &State, rate: (u32, crate::clock::Time)) -> Headers {
    Headers {
        date: st.date_header.then(|| st.world.now()),
        remaining: rate.0,
        reset: Some(rate.1),
    }
}

/// Whether the `Authorization` value carries one of the accepted tokens.
fn token_accepted(tokens: &Option<Vec<String>>, authorization: Option<&str>) -> bool {
    let Some(tokens) = tokens else {
        return true;
    };
    let Some(value) = authorization else {
        return false;
    };
    let token = value.split_once(' ').map_or(value, |(scheme, rest)| {
        if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("token") {
            rest.trim()
        } else {
            value
        }
    });
    tokens.iter().any(|t| t == token)
}

/// Executes the request against the world, spends its cost, and answers it.
fn answer(
    inner: &Arc<Inner>,
    body: &[u8],
    asked: Option<Asked>,
    index: usize,
    authorization: Option<&str>,
) -> http::Response<Body> {
    let mut st = inner.lock();
    inner.sync_world(&mut st);
    if !token_accepted(&st.tokens, authorization) {
        let h = headers(&st, reported(&st));
        return answer::unauthorized(&h);
    }
    if asked.is_none() {
        let h = headers(&st, reported(&st));
        return answer::refused(&h, "the query can't be run");
    }

    // The world's faults under an aim, for this request only.
    let applied = fault::take_all(&mut st.pr_faults, asked.as_ref());
    for f in &applied {
        set_pr_fault(&mut st.world, f, true);
    }
    // docs/02 §2: with a lag, the `rateLimit` block reports the rate some requests old too.
    let before = st.world.rate();
    let mut lagged = None;
    if st.lag > 0
        && let Ok(cost) = cost_of(body)
    {
        let after = (before.0.saturating_sub(cost), before.1);
        st.rates.push_back(after);
        while st.rates.len() > st.lag + 1 {
            st.rates.pop_front();
        }
        let shown = reported(&st);
        st.world.set_rate(shown.0.saturating_add(cost), shown.1);
        lagged = Some(shown);
    }
    let result = execute(&st.world, body);
    if lagged.is_some() {
        st.world.set_rate(before.0, before.1);
    }
    for f in &applied {
        set_pr_fault(&mut st.world, f, false);
    }
    match result {
        Ok(ex) => {
            st.world.spend(ex.cost);
            st.log[index].cost = ex.cost;
            let rate = lagged.unwrap_or_else(|| st.world.rate());
            let h = headers(&st, rate);
            answer::json_ok(&h, &ex.body)
        }
        Err(why) => {
            let what = summary(&st.log[index]);
            st.broken.push(format!("{}: {what}", rejection(&why)));
            st.log[index].rejected = Some(why);
            let h = headers(&st, reported(&st));
            answer::refused(&h, "the query can't be run")
        }
    }
}

fn set_pr_fault(world: &mut World, f: &PrFault, on: bool) {
    match f {
        PrFault::Node { pr, type_ } => world.node_error(*pr, on.then_some(type_.as_str())),
        PrFault::Field { pr, field, type_ } => {
            world.field_error(*pr, field, on.then_some(type_.as_str()));
        }
    }
}
