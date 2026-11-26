//! The routes' tests (T22): through `tower::ServiceExt::oneshot` on `api::router`, over the
//! `AppState` of `api::test_state`, the store filled through `reserve` and `apply`.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use serde::Deserialize;
use serde_json::Value;
use tower::ServiceExt;

use crate::api::{AppState, router, test_state};
use crate::clock::{Activity, Clock, Mono, TestClock, Time};
use crate::daemon::StopCause;
use crate::github::fake::answers::pr_read;
use crate::log::{Level, LogSink, Logger};
use crate::model::{
    CheckItem, CheckKind, ChecksRead, ItemsOutcome, ItemsRead, NodeId, Phase, PrName, PrRead,
    ReadMeta, ReadState, Source,
};
use crate::poller::fake::FakePoller;
use crate::poller::{
    DaemonProblem, FirstPoll, PollOutcome, PollReport, PollerHandle, PollerStatus, ProblemCode,
    Refreshed, Refusal, RefusalCode,
};
use crate::state::store::{Applied, Store};
use crate::state::{Flush, MAX_EVENTS};
use crate::testkit::TEST_VERSION;

const HOUR: Duration = Duration::from_secs(3_600);

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

fn name(s: &str) -> PrName {
    PrName::parse(s).unwrap()
}

/// Counts the flushes (STA-7).
#[derive(Default)]
struct CountingFlush(AtomicUsize);

#[async_trait::async_trait]
impl Flush for CountingFlush {
    async fn flush(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Applies reads to a store with rising sequence numbers: shared by the test and the poller's closures.
#[derive(Clone)]
struct Filler {
    store: Arc<Store>,
    clock: Arc<TestClock>,
    seq: Arc<AtomicU64>,
}

impl Filler {
    fn put(&self, read: &PrRead) -> Applied {
        let meta = ReadMeta {
            seq: self.seq.fetch_add(1, Ordering::SeqCst) + 1,
            sent_wall: self.clock.wall(),
            sent_mono: self.clock.mono(),
            github_date: None,
        };
        // Read as `me`: the PRs are `someone`'s, so none is the user's own (API-15).
        self.store.apply(
            &read.id,
            read,
            ItemsOutcome::Read(&ItemsRead::default()),
            &meta,
            Some("me"),
        )
    }

    fn track(&self, read: &PrRead, source: Source) -> NodeId {
        self.store.reserve(&read.id, &read.pr, source).unwrap();
        assert!(matches!(self.put(read), Applied::Changed { .. }));
        read.id.clone()
    }

    /// The same, for a PR that may be tracked already.
    fn track_again(&self, read: &PrRead, source: Source) -> NodeId {
        self.store.reserve(&read.id, &read.pr, source).unwrap();
        self.put(read);
        read.id.clone()
    }
}

struct Fix {
    clock: Arc<TestClock>,
    state: AppState,
    poller: Arc<FakePoller>,
    flush: Arc<CountingFlush>,
    fill: Filler,
}

fn fix() -> Fix {
    let clock = TestClock::new(t0());
    let (mut state, poller) = test_state(clock.clone());
    let flush = Arc::new(CountingFlush::default());
    state.flush = flush.clone();
    let fill = Filler {
        store: state.store.clone(),
        clock: clock.clone(),
        seq: Arc::new(AtomicU64::new(0)),
    };
    Fix {
        clock,
        state,
        poller,
        flush,
        fill,
    }
}

/// A PR read with one failing check, so `failed_items` and `checks.items` hold something.
fn read(n: &str) -> PrRead {
    let mut r = pr_read(n);
    r.checks = ChecksRead {
        items: vec![CheckItem {
            kind: CheckKind::Run,
            id: Some(7),
            name: "test".into(),
            phase: Phase::Completed,
            result: Some("failure".into()),
            url: Some("https://ci.example.test/7".into()),
        }],
        total: 1,
        rollup: Some("failure".into()),
        next_page: None,
    };
    r
}

fn updated(n: &str, minutes: u64) -> PrRead {
    let mut r = read(n);
    r.updated_at = Some(t0().plus(Duration::from_secs(60 * minutes)));
    r
}

fn ended(n: &str, state: ReadState, minutes: u64) -> PrRead {
    let mut r = updated(n, minutes);
    r.state = state;
    r
}

struct Answer {
    status: u16,
    json: Value,
    text: String,
}

impl Fix {
    fn app(&self) -> axum::Router {
        router(self.state.clone())
    }

    fn cursor(&self) -> String {
        self.state.store.view().cursor().to_string()
    }

    fn epoch(&self) -> String {
        self.state.store.view().epoch
    }

    async fn call(&self, method: &str, uri: &str, body: Option<&str>) -> Answer {
        send(self.app(), method, uri, body).await
    }

    async fn get(&self, uri: &str) -> Answer {
        self.call("GET", uri, None).await
    }

    fn set_problem(&self, problem: Option<DaemonProblem>) {
        let mut s: PollerStatus = self.poller.status();
        s.problem = problem;
        self.poller.set_status(s);
    }
}

fn request(method: &str, uri: &str, body: Option<&str>) -> http::Request<Body> {
    http::Request::builder()
        .method(method)
        .uri(uri)
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap()
}

async fn send(app: axum::Router, method: &str, uri: &str, body: Option<&str>) -> Answer {
    let response = app.oneshot(request(method, uri, body)).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 24)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let json = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    Answer { status, json, text }
}

fn prs(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|r| r["pr"].as_str().unwrap().to_string())
        .collect()
}

fn strings(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn revs(events: &Value) -> Vec<u64> {
    events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["rev"].as_u64().unwrap())
        .collect()
}

fn assert_error(a: &Answer, status: u16, code: &str) {
    assert_eq!(a.status, status, "{}", a.text);
    assert_eq!(a.json["version"], TEST_VERSION, "{}", a.text);
    assert_eq!(a.json["error"]["code"], code, "{}", a.text);
}

/// A PollerHandle that notes the work in flight inside `client_request` (TST-4).
struct Watching {
    inner: Arc<FakePoller>,
    activity: Activity,
    seen: Mutex<Vec<usize>>,
}

#[async_trait::async_trait]
impl PollerHandle for Watching {
    fn client_request(&self) {
        self.seen.lock().unwrap().push(self.activity.in_flight());
        self.inner.client_request();
    }
    fn idle_since(&self) -> Mono {
        self.inner.idle_since()
    }
    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal> {
        self.inner.read_through(name).await
    }
    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal> {
        self.inner.track(name).await
    }
    async fn refresh_pr(&self, id: &NodeId) -> Refreshed {
        self.inner.refresh_pr(id).await
    }
    async fn poll_now(&self) -> PollReport {
        self.inner.poll_now().await
    }
    fn nudge(&self) {
        self.inner.nudge()
    }
    fn first_poll(&self) -> tokio::sync::watch::Receiver<FirstPoll> {
        self.inner.first_poll()
    }
    fn status(&self) -> PollerStatus {
        self.inner.status()
    }
}

// API-1, API-3: `router`'s fallbacks answer `unknown_route` for an unknown path and for a known
// path with another method, never a bare 405; its layer counts the request's work and checks for a sleep.
#[tokio::test]
async fn api1_router_fallback_and_layer() {
    let mut f = fix();
    for (method, path) in [
        ("GET", "/v1/nope"),
        ("GET", "/v2/prs"),
        ("POST", "/v1/prs/o/r/1/wait"),
        ("DELETE", "/v1/prs"),
        ("PUT", "/v1/status"),
    ] {
        let a = f.call(method, path, None).await;
        assert_error(&a, 404, "unknown_route");
    }

    // TST-4: one guard per request, held inside the handler, gone with the response.
    let watching = Arc::new(Watching {
        inner: f.poller.clone(),
        activity: f.state.activity.clone(),
        seen: Mutex::new(Vec::new()),
    });
    f.state.poller = watching.clone();
    let a = f.get("/v1/prs").await;
    assert_eq!(a.status, 200);
    assert_eq!(*watching.seen.lock().unwrap(), [1]);
    assert_eq!(f.state.activity.in_flight(), 0);

    // POL-22: the layer's `sleep.check()` sees a suspend at the next request, `status` included.
    f.clock.sleep_wall_only(HOUR).await;
    assert_eq!(f.state.sleep.total(), Duration::ZERO);
    let a = f.get("/v1/status").await;
    assert_eq!(a.status, 200);
    assert_eq!(f.state.sleep.total(), HOUR);
}

// API-1: values are percent-decoded; a bad escape is `bad_request`.
#[tokio::test]
async fn api1_encoded_value() {
    let f = fix();
    for n in 1..=5 {
        f.fill.track(&read(&format!("o/r#{n}")), Source::Authored);
    }
    let a = f.get(&format!("/v1/changes?since={}%2D3", f.epoch())).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(revs(&a.json["events"]), [4, 5]);
    assert_eq!(a.json["reset"], false);
    let a = f.get("/v1/changes?since=%zz").await;
    assert_error(&a, 400, "bad_request");
    assert!(
        a.json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("since")
    );
}

/// A JSON value's shape, its object keys in the order written.
#[derive(Debug)]
enum Shape {
    Null,
    Leaf,
    List(Vec<Shape>),
    Object(Vec<(String, Shape)>),
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Shape, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Shape;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> Result<Shape, E> {
                Ok(Shape::Null)
            }
            fn visit_none<E>(self) -> Result<Shape, E> {
                Ok(Shape::Null)
            }
            fn visit_bool<E>(self, _: bool) -> Result<Shape, E> {
                Ok(Shape::Leaf)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Shape, E> {
                Ok(Shape::Leaf)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Shape, E> {
                Ok(Shape::Leaf)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Shape, E> {
                Ok(Shape::Leaf)
            }
            fn visit_str<E>(self, _: &str) -> Result<Shape, E> {
                Ok(Shape::Leaf)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut s: A) -> Result<Shape, A::Error> {
                let mut out = Vec::new();
                while let Some(x) = s.next_element::<Shape>()? {
                    out.push(x);
                }
                Ok(Shape::List(out))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Shape, A::Error> {
                let mut out = Vec::new();
                while let Some(k) = m.next_key::<String>()? {
                    out.push((k, m.next_value::<Shape>()?));
                }
                Ok(Shape::Object(out))
            }
        }
        d.deserialize_any(V)
    }
}

/// Compares two shapes: the same keys in the same order at every level; a `null` on either side
/// stands for any value; two lists compare their first elements when both have one.
fn same_shape(at: &str, ours: &Shape, golden: &Shape) -> Result<(), String> {
    match (ours, golden) {
        (Shape::Null, _) | (_, Shape::Null) | (Shape::Leaf, Shape::Leaf) => Ok(()),
        (Shape::List(a), Shape::List(b)) => match (a.first(), b.first()) {
            (Some(x), Some(y)) => same_shape(&format!("{at}[0]"), x, y),
            _ => Ok(()),
        },
        (Shape::Object(a), Shape::Object(b)) => {
            let ka: Vec<&str> = a.iter().map(|(k, _)| k.as_str()).collect();
            let kb: Vec<&str> = b.iter().map(|(k, _)| k.as_str()).collect();
            if ka != kb {
                return Err(format!("{at}: keys {ka:?}, the golden file's {kb:?}"));
            }
            for ((k, x), (_, y)) in a.iter().zip(b) {
                same_shape(&format!("{at}.{k}"), x, y)?;
            }
            Ok(())
        }
        _ => Err(format!(
            "{at}: {ours:?} where the golden file has {golden:?}"
        )),
    }
}

fn assert_shape(file: &str, a: &Answer) {
    let golden_text =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
            .unwrap();
    let golden: Shape = serde_json::from_str(&golden_text).unwrap();
    let ours: Shape = serde_json::from_str(&a.text).unwrap();
    if let Err(e) = same_shape("", &ours, &golden) {
        panic!("{file}: {e}\n{}", a.text);
    }
}

// API-19: each route's body has its golden file's keys, at every level and in order, with every
// list and flag there when empty.
#[tokio::test]
async fn api19_shapes() {
    let f = fix();
    let a_id = f.fill.track(&read("o/r#1"), Source::Authored);
    f.fill.track(&read("o/r#2"), Source::Manual);
    f.poller.set_status(PollerStatus {
        last_poll: Some((t0(), PollOutcome::Polled)),
        last_poll_at: Some(t0()),
        remaining: Some(4_000),
        reset_at: Some(t0().plus(HOUR)),
        login: Some("me".into()),
        ..f.poller.status()
    });

    assert_shape("testdata/api/list.json", &f.get("/v1/prs").await);
    let a = f.get(&format!("/v1/prs?since={}-999", f.epoch())).await;
    assert_eq!(a.json["reset"], true);
    assert_shape("testdata/api/list-reset.json", &a);
    // Empty lists and flags are there too.
    let empty = fix();
    let a = empty.get("/v1/prs").await;
    assert_eq!(a.json["records"], Value::Array(Vec::new()));
    assert_shape("testdata/api/list.json", &a);

    assert_shape("testdata/api/show.json", &f.get("/v1/prs/o/r/1").await);
    assert_shape(
        "testdata/api/changes.json",
        &f.get(&format!("/v1/changes?since={}-0", f.epoch())).await,
    );
    assert_shape(
        "testdata/api/changes.json",
        &f.get(&format!("/v1/prs/o/r/1/changes?since={}-0", f.epoch()))
            .await,
    );

    let fill = f.fill.clone();
    f.poller
        .on_track(move |n| Ok(fill.track(&read(&n.to_string()), Source::Manual)));
    assert_shape(
        "testdata/api/track.json",
        &f.call("POST", "/v1/track", Some(r#"{"pr":"o/r#3"}"#)).await,
    );
    assert_shape(
        "testdata/api/untrack-null.json",
        &f.call("DELETE", "/v1/track/o/r/99", None).await,
    );
    // An untrack that answers a record has `show`'s shape.
    assert_shape(
        "testdata/api/show.json",
        &f.call("DELETE", "/v1/track/o/r/2", None).await,
    );
    assert_shape(
        "testdata/api/refresh.json",
        &f.call("POST", "/v1/refresh", None).await,
    );
    assert_shape("testdata/api/status.json", &f.get("/v1/status").await);

    // A problem and two warnings.
    f.set_problem(Some(DaemonProblem {
        code: ProblemCode::RateLimited,
        message: "limited".into(),
        until: Some(t0().plus(HOUR)),
    }));
    f.state
        .warnings
        .raise(crate::status::WarningCode::QuotaCapped, "", "capped".into());
    f.state.warnings.raise(
        crate::status::WarningCode::PrFailing,
        &a_id.0,
        "o/r#1 fails".into(),
    );
    assert_shape(
        "testdata/api/status-problem.json",
        &f.get("/v1/status").await,
    );

    // Errors.
    assert_shape(
        "testdata/api/error-bad_request.json",
        &f.get("/v1/prs?bogus=1").await,
    );
    assert_shape(
        "testdata/api/error-unknown_route.json",
        &f.get("/v1/nope").await,
    );
    for (code, file) in [
        (RefusalCode::NotFound, "not_found"),
        (RefusalCode::Auth, "auth"),
        (RefusalCode::GithubUnavailable, "github_unavailable"),
        (RefusalCode::RateLimited, "rate_limited"),
        (RefusalCode::Limit, "limit"),
    ] {
        f.poller.on_read_through(move |_| {
            Err(Refusal {
                code,
                message: "refused".into(),
                until: Some(t0().plus(HOUR)),
            })
        });
        assert_shape(
            &format!("testdata/api/error-{file}.json"),
            &f.get("/v1/prs/o/r/50").await,
        );
    }
    f.poller.set_first_poll(FirstPoll::Running);
    let app = f.app();
    let call = tokio::spawn(send(app, "GET", "/v1/prs", None));
    f.clock.advance(Duration::from_secs(10)).await;
    assert_shape("testdata/api/error-starting.json", &call.await.unwrap());

    // `shutdown`: the envelope and nothing else.
    let a = f.call("POST", "/v1/shutdown", None).await;
    let keys: Vec<String> = a.json.as_object().unwrap().keys().cloned().collect();
    let mut want = vec!["cursor", "daemon", "version"];
    want.sort();
    assert_eq!(keys, want);
}

// API-1: an unknown parameter, a parameter twice, a value out of range, a bad body, a bad path:
// each is `bad_request` naming it.
#[tokio::test]
async fn api1_strict() {
    let f = fix();
    f.fill.track(&read("o/r#1"), Source::Authored);
    let cases: &[(&str, &str, Option<&str>, &str)] = &[
        ("GET", "/v1/prs?bogus=1", None, "bogus"),
        ("GET", "/v1/prs?checks=1&checks=1", None, "checks"),
        ("GET", "/v1/prs?checks=2", None, "checks"),
        ("GET", "/v1/changes?limit=0", None, "limit"),
        ("GET", "/v1/changes?limit=501", None, "limit"),
        ("GET", "/v1/prs/o/r/1/changes?limit=501", None, "limit"),
        ("GET", "/v1/prs?state=closed", None, "state"),
        ("GET", "/v1/prs?since=abc", None, "since"),
        ("GET", "/v1/prs/o/r/1?fresh=yes", None, "fresh"),
        ("GET", "/v1/prs/o/r/1?bogus=1", None, "bogus"),
        ("GET", "/v1/status?bogus=1", None, "bogus"),
        ("POST", "/v1/refresh?bogus=1", None, "bogus"),
        ("POST", "/v1/shutdown?bogus=1", None, "bogus"),
        (
            "POST",
            "/v1/track",
            Some(r#"{"pr":"o/r#1","extra":1}"#),
            "extra",
        ),
        ("POST", "/v1/track", Some("not json"), "body"),
        ("POST", "/v1/track", Some(r#"{"pr":"o/r"}"#), "pr"),
        ("GET", "/v1/prs/o/r/x", None, "x"),
        ("GET", "/v1/prs/o/r/x/changes", None, "x"),
        ("DELETE", "/v1/track/o/r/x", None, "x"),
    ];
    for (method, uri, body, named) in cases {
        let a = f.call(method, uri, *body).await;
        assert_error(&a, 400, "bad_request");
        let message = a.json["error"]["message"].as_str().unwrap();
        assert!(message.contains(named), "{method} {uri}: {message}");
    }
    // Nothing was asked of GitHub, and the shutdown wasn't taken.
    assert_eq!(f.poller.tracks(), 0);
    assert_eq!(f.poller.read_throughs(), 0);
    assert_eq!(f.state.stop.requested(), None);
}

// API-2, API-4: every 200 carries `version`, the view's `cursor` and `daemon`; every error `version`.
#[tokio::test]
async fn api2_api4_envelope() {
    let f = fix();
    f.fill.track(&read("o/r#1"), Source::Authored);
    f.fill.track(&read("o/r#2"), Source::Manual);
    let fill = f.fill.clone();
    f.poller
        .on_track(move |n| Ok(fill.track_again(&read(&n.to_string()), Source::Manual)));
    let until = t0().plus(HOUR);
    for problem in [
        None,
        Some(DaemonProblem {
            code: ProblemCode::GithubUnavailable,
            message: "GitHub can't be reached".into(),
            until: Some(until),
        }),
    ] {
        f.set_problem(problem.clone());
        let routes: &[(&str, &str, Option<&str>)] = &[
            ("GET", "/v1/prs", None),
            ("GET", "/v1/prs/o/r/1", None),
            ("GET", "/v1/changes", None),
            ("GET", "/v1/prs/o/r/1/changes", None),
            ("POST", "/v1/refresh", None),
            ("GET", "/v1/status", None),
            ("POST", "/v1/track", Some(r#"{"pr":"o/r#3"}"#)),
            ("DELETE", "/v1/track/o/r/1", None),
            ("POST", "/v1/shutdown", None),
        ];
        for (method, uri, body) in routes {
            let a = f.call(method, uri, *body).await;
            assert_eq!(a.status, 200, "{method} {uri}: {}", a.text);
            assert_eq!(a.json["version"], TEST_VERSION);
            assert_eq!(a.json["cursor"], f.cursor(), "{method} {uri}");
            assert!(!a.text.contains('\n'));
            let d = &a.json["daemon"];
            assert_eq!(d["tier"], "active");
            match &problem {
                None => {
                    assert!(
                        d["problem"].is_null() && d["message"].is_null() && d["until"].is_null()
                    )
                }
                Some(_) => {
                    assert_eq!(d["problem"], "github_unavailable");
                    assert_eq!(d["message"], "GitHub can't be reached");
                    assert_eq!(d["until"], until.rfc3339());
                }
            }
        }
    }
    for (method, uri) in [("GET", "/v1/prs?x=1"), ("GET", "/v1/nope")] {
        let a = f.call(method, uri, None).await;
        assert_eq!(a.json["version"], TEST_VERSION);
    }
}

// API-3: a refused read-through's code and HTTP status; a PR with a record answers from memory.
#[tokio::test]
async fn api3_errors() {
    let f = fix();
    let until = t0().plus(HOUR);
    for (code, status, word, has_until) in [
        (RefusalCode::NotFound, 404, "not_found", false),
        (RefusalCode::Auth, 503, "auth", false),
        (
            RefusalCode::GithubUnavailable,
            503,
            "github_unavailable",
            true,
        ),
        (RefusalCode::RateLimited, 503, "rate_limited", true),
        (RefusalCode::Limit, 409, "limit", false),
    ] {
        f.poller.on_read_through(move |_| {
            Err(Refusal {
                code,
                message: format!("refused as {word}"),
                until: has_until.then_some(until),
            })
        });
        let a = f.get("/v1/prs/o/r/1").await;
        assert_error(&a, status, word);
        assert_eq!(a.json["error"]["message"], format!("refused as {word}"));
        if has_until {
            assert_eq!(a.json["error"]["until"], until.rfc3339());
        } else {
            assert!(a.json["error"]["until"].is_null());
        }
    }
    assert_error(&f.get("/v1/track").await, 404, "unknown_route");

    // A record answers 200 while the poller reports a problem, with no read-through.
    f.fill.track(&read("o/r#2"), Source::Authored);
    f.set_problem(Some(DaemonProblem {
        code: ProblemCode::Auth,
        message: "run gh auth login".into(),
        until: None,
    }));
    let before = f.poller.read_throughs();
    let a = f.get("/v1/prs/o/r/2").await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json["record"]["pr"], "o/r#2");
    assert_eq!(a.json["daemon"]["problem"], "auth");
    assert_eq!(f.poller.read_throughs(), before);
}

/// A sink that refuses every line.
struct Refusing;

impl LogSink for Refusing {
    fn write_line(&self, _line: &str) -> std::io::Result<()> {
        Err(std::io::Error::other("disk full"))
    }
}

// API-5, NTF-9: health and counts; `subscriptions` is their number and nothing else; no client request.
#[tokio::test]
async fn api5_status() {
    let mut f = fix();
    let a_id = f.fill.track(&read("o/r#1"), Source::Authored);
    let b_id = f.fill.track(&read("o/r#2"), Source::Authored);
    f.fill.track(&read("o/r#3"), Source::Manual);
    f.fill.track(&read("o/r#4"), Source::Asked);
    // API-18: `status` answers while the first poll runs.
    f.poller.set_first_poll(FirstPoll::Running);

    let a = f.get("/v1/status").await;
    assert_eq!(a.status, 200, "{}", a.text);
    let j = &a.json;
    assert_eq!(j["mode"], "spawned");
    assert_eq!(j["started_at"], t0().rfc3339());
    assert_eq!(
        j["tracked"],
        serde_json::json!({"authored": 2, "manual": 1, "asked": 1})
    );
    assert_eq!(j["waiters"], 0);
    assert_eq!(j["subscriptions"], 0);
    assert_eq!(j["token_source"], "gh");
    assert_eq!(j["gh_path"], "gh");
    assert_eq!(j["paths"]["state"], "/tmp/ct/state.json");
    assert_eq!(j["warnings"], Value::Array(Vec::new()));
    assert!(j["last_poll"].is_null());

    // NTF-9: two subscriptions are counted, and aren't waiters.
    let s1 = f.state.store.add_subscription(&a_id).unwrap();
    let s2 = f.state.store.add_subscription(&b_id).unwrap();
    let a = f.get("/v1/status").await;
    assert_eq!(a.json["subscriptions"], 2);
    assert_eq!(a.json["waiters"], 0);
    // NTF-9: nothing else about a subscription is in the answer.
    assert!(!a.text.contains("subscription\""));
    drop((s1, s2));
    assert_eq!(f.get("/v1/status").await.json["subscriptions"], 0);

    // OPS-9, API-5: `log_unwritable` while the logger fails.
    let failing = Logger::new(f.state.clock.clone(), Arc::new(Refusing), Level::Info);
    f.state.logger = failing.clone();
    failing.info("a line");
    let a = f.get("/v1/status").await;
    let warnings = a.json["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{}", a.text);
    assert_eq!(warnings[0]["code"], "log_unwritable");
    assert_eq!(warnings[0]["since"], t0().rfc3339());

    // API-5: `status` isn't a client request.
    assert_eq!(f.poller.client_requests(), 0);
}

// API-6: the order, `state`, `since` with `untracked`, a reset, `checks`, `pending`.
#[tokio::test]
async fn api6_list() {
    let f = fix();
    // Empty lists are `[]`.
    let a = f.get("/v1/prs").await;
    assert_eq!(a.status, 200);
    for key in ["records", "untracked", "pending"] {
        assert_eq!(a.json[key], Value::Array(Vec::new()), "{key}");
    }
    assert_eq!(a.json["reset"], false);

    f.fill.track(&updated("o/a#1", 10), Source::Authored);
    f.fill.track(&updated("o/b#2", 30), Source::Authored);
    f.fill.track(&updated("o/c#3", 10), Source::Manual);
    f.fill
        .track(&ended("o/d#4", ReadState::Merged, 50), Source::Authored);
    f.fill
        .track(&ended("o/e#5", ReadState::Closed, 40), Source::Authored);
    let gone = f.fill.track(&updated("o/f#6", 60), Source::Authored);
    f.state.store.mark_gone(&gone, t0());
    // Two PRs found and not yet refreshed: `pending`, sorted.
    for n in ["o/z#9", "o/y#8"] {
        let r = pr_read(n);
        f.state
            .store
            .reserve(&r.id, &r.pr, Source::Authored)
            .unwrap();
    }

    // Open first, newest `updated_at` first, then `pr`.
    let a = f.get("/v1/prs").await;
    assert_eq!(prs(&a.json["records"]), ["o/b#2", "o/a#1", "o/c#3"]);
    assert_eq!(strings(&a.json["pending"]), ["o/y#8", "o/z#9"]);
    let a = f.get("/v1/prs?state=all").await;
    assert_eq!(
        prs(&a.json["records"]),
        ["o/b#2", "o/a#1", "o/c#3", "o/f#6", "o/d#4", "o/e#5"]
    );

    // `checks.items` only with `checks=1`; `failed_items` always.
    let a = f.get("/v1/prs").await;
    for r in a.json["records"].as_array().unwrap() {
        assert!(r["checks"].get("items").is_none());
        assert_eq!(r["checks"]["failed_items"][0]["name"], "test");
    }
    let a = f.get("/v1/prs?checks=1").await;
    for r in a.json["records"].as_array().unwrap() {
        assert_eq!(r["checks"]["items"][0]["name"], "test");
    }

    // `since`: a PR merged after the cursor is listed; a PR dropped after it is named in
    // `untracked`, but not one tracked again.
    let dropped = f.fill.track(&updated("o/m#7", 5), Source::Manual);
    let again = f.fill.track(&updated("o/n#8", 5), Source::Manual);
    let c = f.cursor();
    f.fill.put(&ended("o/a#1", ReadState::Merged, 70));
    assert!(f.state.store.untrack(&name("o/m#7")).record.is_some());
    assert!(!f.state.store.is_tracked(&dropped));
    f.state.store.untrack(&name("o/n#8"));
    f.fill.track(&updated("o/n#8", 5), Source::Manual);
    assert!(f.state.store.is_tracked(&again));
    let a = f.get(&format!("/v1/prs?since={c}")).await;
    assert_eq!(a.json["reset"], false);
    assert_eq!(prs(&a.json["records"]), ["o/n#8", "o/a#1"]);
    assert_eq!(a.json["records"][1]["state"], "merged");
    assert_eq!(strings(&a.json["untracked"]), ["o/m#7"]);

    // A too-old cursor: `reset`, the records `state` asks for, nothing untracked.
    let old = format!("/v1/prs?since={}-999", f.epoch());
    let a = f.get(&old).await;
    assert_eq!(a.json["reset"], true);
    assert_eq!(prs(&a.json["records"]), ["o/b#2", "o/c#3", "o/n#8"]);
    assert_eq!(a.json["untracked"], Value::Array(Vec::new()));
    let a = f.get(&format!("{old}&state=all")).await;
    assert_eq!(a.json["reset"], true);
    assert_eq!(a.json["records"].as_array().unwrap().len(), 7);
}

// API-7: one record with `checks.items`; read through once without one; `fresh=1` refreshes once.
#[tokio::test]
async fn api7_show() {
    let f = fix();
    let id = f.fill.track(&read("o/r#1"), Source::Authored);
    let a = f.get("/v1/prs/o/r/1").await;
    assert_eq!(a.status, 200);
    assert_eq!(a.json["record"]["pr"], "o/r#1");
    assert_eq!(a.json["record"]["checks"]["items"][0]["name"], "test");
    assert_eq!(f.poller.read_throughs(), 0);
    assert_eq!(f.poller.refreshes(), 0);
    // STA-1: the request named it.
    assert!(
        f.state
            .store
            .view()
            .get(&id)
            .unwrap()
            .timing
            .named
            .is_some()
    );

    // No record: one read-through.
    let fill = f.fill.clone();
    f.poller
        .on_read_through(move |n| Ok(fill.track(&read(&n.to_string()), Source::Asked)));
    let a = f.get("/v1/prs/o/r/2").await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json["record"]["source"], "asked");
    assert_eq!(f.poller.read_throughs(), 1);
    let named = f
        .state
        .store
        .view()
        .find(&name("o/r#2"))
        .unwrap()
        .timing
        .named;
    assert!(named.is_some());

    // `fresh=1`: one refresh, and 200 from memory when it failed.
    f.poller.on_refresh(|_| Refreshed::Failed);
    let a = f.get("/v1/prs/o/r/1?fresh=1").await;
    assert_eq!(a.status, 200);
    assert_eq!(a.json["record"]["pr"], "o/r#1");
    assert_eq!(f.poller.refreshes(), 1);
    assert_eq!(f.poller.read_throughs(), 1);
}

// API-8: `window` through the route.
#[tokio::test]
async fn api8_changes() {
    let f = fix();
    for n in 1..=5 {
        f.fill.track(&read(&format!("o/r#{n}")), Source::Authored);
    }
    let pending = pr_read("o/p#1");
    f.state
        .store
        .reserve(&pending.id, &pending.pr, Source::Authored)
        .unwrap();
    let epoch = f.epoch();

    // No `since`: no events, the view's cursor.
    let a = f.get("/v1/changes").await;
    assert_eq!(a.json["events"], Value::Array(Vec::new()));
    assert_eq!(a.json["cursor"], f.cursor());
    assert_eq!(strings(&a.json["pending"]), ["o/p#1"]);

    // `limit=2`: the two oldest, `more_events`, and the second's cursor.
    let a = f.get(&format!("/v1/changes?since={epoch}-0&limit=2")).await;
    assert_eq!(revs(&a.json["events"]), [1, 2]);
    assert_eq!(a.json["more_events"], true);
    assert_eq!(a.json["cursor"], format!("{epoch}-2"));
    assert_eq!(a.json["records"], Value::Array(Vec::new()));

    // The PR's route: its own events, `pending: []`.
    let a = f
        .get(&format!("/v1/prs/o/r/3/changes?since={epoch}-0"))
        .await;
    assert_eq!(revs(&a.json["events"]), [3]);
    assert_eq!(a.json["events"][0]["pr"], "o/r#3");
    assert_eq!(a.json["pending"], Value::Array(Vec::new()));

    // A cursor older than the history: `reset`, every record in scope, none with `checks.items`.
    let mut r = read("o/r#1");
    for _ in 0..=MAX_EVENTS {
        r.draft = !r.draft;
        assert!(matches!(f.fill.put(&r), Applied::Changed { .. }));
    }
    let a = f.get(&format!("/v1/changes?since={epoch}-1")).await;
    assert_eq!(a.json["reset"], true);
    assert_eq!(a.json["events"], Value::Array(Vec::new()));
    let records = a.json["records"].as_array().unwrap();
    assert_eq!(records.len(), 5);
    assert!(records.iter().all(|r| r["checks"].get("items").is_none()));
    let a = f
        .get(&format!("/v1/prs/o/r/2/changes?since={epoch}-1"))
        .await;
    assert_eq!(a.json["reset"], true);
    assert_eq!(prs(&a.json["records"]), ["o/r#2"]);
    assert!(a.json["records"][0]["checks"].get("items").is_none());
}

// API-15, STA-7: `track` through the poller, then the file; `untrack` through the store alone.
#[tokio::test]
async fn api15_track_untrack() {
    let f = fix();
    let fill = f.fill.clone();
    f.poller
        .on_track(move |n| Ok(fill.track(&read(&n.to_string()), Source::Manual)));
    let a = f
        .call("POST", "/v1/track", Some(r#"{"pr": "o/r#1"}"#))
        .await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json["record"]["source"], "manual");
    assert_eq!(a.json["record"]["checks"]["items"][0]["name"], "test");
    assert_eq!(f.poller.tracks(), 1);
    assert_eq!(f.flush.0.load(Ordering::SeqCst), 1);

    // A refused `track` answers its error, and was written first all the same.
    f.poller.on_track(|_| {
        Err(Refusal {
            code: RefusalCode::GithubUnavailable,
            message: "unreachable".into(),
            until: None,
        })
    });
    let a = f
        .call("POST", "/v1/track", Some(r#"{"pr": "o/r#2"}"#))
        .await;
    assert_error(&a, 503, "github_unavailable");
    assert_eq!(f.poller.tracks(), 2);
    assert_eq!(f.flush.0.load(Ordering::SeqCst), 2);

    // `untrack` of an untracked PR: `null`, neither the poller nor the file.
    let a = f.call("DELETE", "/v1/track/o/r/9", None).await;
    assert_eq!(a.status, 200);
    assert!(a.json["record"].is_null());
    assert_eq!(f.flush.0.load(Ordering::SeqCst), 2);

    // Of a `manual` one that's dropped: its last record, and the file.
    let a = f.call("DELETE", "/v1/track/o/r/1", None).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json["record"]["pr"], "o/r#1");
    assert_eq!(a.json["record"]["stale"], false);
    let last = f.state.store.view().events.last().cloned().unwrap();
    assert_eq!(a.json["record"]["rev"], last.rev);
    assert_eq!(f.flush.0.load(Ordering::SeqCst), 3);
    assert!(f.state.store.lookup(&name("o/r#1")).is_none());

    // Of an `authored` one: no change.
    f.fill.track(&read("o/r#3"), Source::Authored);
    let a = f.call("DELETE", "/v1/track/o/r/3", None).await;
    assert_eq!(a.status, 200);
    assert_eq!(a.json["record"]["source"], "authored");
    assert_eq!(f.flush.0.load(Ordering::SeqCst), 3);

    // `untrack` never reaches the poller.
    assert_eq!(f.poller.tracks(), 2);
    assert_eq!(f.poller.read_throughs(), 0);
    assert_eq!(f.poller.refreshes(), 0);
}

// API-16: the poll's outcome and cost, HTTP 200 whatever it is.
#[tokio::test]
async fn api16_refresh() {
    let f = fix();
    for (outcome, cost, word) in [
        (PollOutcome::Polled, 4, "polled"),
        (PollOutcome::Shared, 0, "shared"),
        (PollOutcome::GithubUnavailable, 1, "github_unavailable"),
    ] {
        f.poller.on_poll(move || PollReport { outcome, cost });
        let a = f.call("POST", "/v1/refresh", None).await;
        assert_eq!(a.status, 200, "{}", a.text);
        assert_eq!(a.json["outcome"], word);
        assert_eq!(a.json["cost"], cost);
    }
    assert_eq!(f.poller.polls(), 3);
}

// API-17: the envelope alone; the stop is asked for once the body has been taken.
#[tokio::test]
async fn api17_shutdown() {
    let f = fix();
    let response = f
        .app()
        .oneshot(request("POST", "/v1/shutdown", None))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(f.state.stop.requested(), None);
    let body = response.into_body();
    assert_eq!(f.state.stop.requested(), None);
    let bytes = axum::body::to_bytes(body, 1 << 16).await.unwrap();
    assert_eq!(f.state.stop.requested(), Some(StopCause::Request));
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["cursor", "daemon", "version"]);
    assert_eq!(v["version"], TEST_VERSION);
    // `shutdown` isn't a client request.
    assert_eq!(f.poller.client_requests(), 0);
}

// API-18: `list` and `changes` wait up to 10 s for the first poll; the others don't wait.
#[tokio::test]
async fn api18_starting() {
    let f = fix();
    f.fill.track(&read("o/r#1"), Source::Authored);
    f.poller.set_first_poll(FirstPoll::Running);

    // Still running at 10 s: `starting`.
    for uri in ["/v1/prs", "/v1/changes"] {
        let call = tokio::spawn(send(f.app(), "GET", uri, None));
        f.clock.advance(Duration::from_secs(9)).await;
        assert!(!call.is_finished(), "{uri}");
        f.clock.advance(Duration::from_secs(1)).await;
        let a = call.await.unwrap();
        assert_error(&a, 503, "starting");
        assert!(a.json["error"]["until"].is_null());
    }

    // Finished at 3 s: answered then.
    let call = tokio::spawn(send(f.app(), "GET", "/v1/prs", None));
    f.clock.advance(Duration::from_secs(3)).await;
    assert!(!call.is_finished());
    f.poller.set_first_poll(FirstPoll::Finished);
    f.clock.settle().await;
    let a = call.await.unwrap();
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(prs(&a.json["records"]), ["o/r#1"]);

    // `Unanswered` with an `auth` refusal: that error, at once.
    f.poller.set_first_poll(FirstPoll::Unanswered(Refusal {
        code: RefusalCode::Auth,
        message: "gh has no login; run gh auth login".into(),
        until: None,
    }));
    for uri in ["/v1/prs", "/v1/changes"] {
        let a = f.get(uri).await;
        assert_error(&a, 503, "auth");
    }

    // A PR's route, `status` and `shutdown` don't wait.
    f.poller.set_first_poll(FirstPoll::Running);
    for (method, uri) in [
        ("GET", "/v1/prs/o/r/1"),
        ("GET", "/v1/prs/o/r/1/changes"),
        ("GET", "/v1/status"),
        ("POST", "/v1/shutdown"),
    ] {
        let call = tokio::spawn(send(f.app(), method, uri, None));
        f.clock.settle().await;
        assert!(call.is_finished(), "{method} {uri}");
        assert_eq!(call.await.unwrap().status, 200, "{method} {uri}");
    }
}
