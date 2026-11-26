//! The notify routes' tests (T47): through `oneshot` on `api::router`, on the test clock, over a real
//! store filled through `reserve` and `apply`, with a real `Notifier` over a `RecordingPoster`.
//! Nothing here posts to a real session's socket.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Body;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::api::{AppState, router, test_state};
use crate::clock::{Clock, TestClock, Time};
use crate::github::fake::answers::pr_read;
use crate::log::{BufferSink, Level, Logger};
use crate::model::{
    CheckItem, CheckKind, ItemsOutcome, ItemsRead, NodeId, Phase, PrName, PrRead, ReadMeta,
    ReadState, Seen, Source,
};
use crate::notify::fake::RecordingPoster;
use crate::notify::{Notifier, NotifyDeps, Posted, Poster, RealPoster};
use crate::poller::fake::FakePoller;
use crate::poller::{DaemonProblem, FirstPoll, PollerHandle, ProblemCode, Refreshed};
use crate::state::store::{Applied, Store, StoreDeps};
use crate::state::{FileContent, FileEntry, MAX_PLACES, MAX_SUBSCRIPTIONS};
use crate::testkit::{MARKER_SESSION_TOKEN, ShortDir};

const SEC: Duration = Duration::from_secs(1);
const HEAD1: &str = "abcdef0123456789abcdef0123456789abcdef01";
const A: &str = "/tmp/ct-na.sock";
const B: &str = "/tmp/ct-nb.sock";
const C: &str = "/tmp/ct-nc.sock";
const URI1: &str = "/v1/prs/o/r/1/notify";

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

fn item(id: u64, phase: Phase, result: Option<&str>) -> CheckItem {
    CheckItem {
        kind: CheckKind::Run,
        id: Some(id),
        name: format!("job-{id}"),
        phase,
        result: result.map(str::to_string),
        url: None,
    }
}

fn read_of(name: &str, items: Vec<CheckItem>) -> PrRead {
    let mut r = pr_read(name);
    r.head_sha = HEAD1.to_string();
    r.checks.total = items.len() as u32;
    r.checks.items = items;
    r
}

/// One check running: `checks_pending`.
fn running(name: &str) -> PrRead {
    read_of(name, vec![item(1, Phase::InProgress, None)])
}

/// One check failed: settled at once (GitHub's `Date` is an hour on, MOD-6).
fn failed(name: &str) -> PrRead {
    read_of(name, vec![item(1, Phase::Completed, Some("failure"))])
}

fn with_state(mut r: PrRead, state: ReadState) -> PrRead {
    r.state = state;
    r
}

fn with_draft(mut r: PrRead, draft: bool) -> PrRead {
    r.draft = draft;
    r
}

fn put_on(store: &Store, clock: &TestClock, seq: &AtomicU64, read: &PrRead) -> Applied {
    let meta = ReadMeta {
        seq: seq.fetch_add(1, Ordering::SeqCst) + 1,
        sent_wall: clock.wall(),
        sent_mono: clock.mono(),
        github_date: Some(clock.wall().plus(Duration::from_secs(3_600))),
    };
    store.apply(
        &read.id,
        read,
        ItemsOutcome::Read(&ItemsRead::default()),
        &meta,
        Some("someone"),
    )
}

fn loaded_entry(name: &str) -> FileEntry {
    let read = pr_read(name);
    FileEntry {
        pr: read.pr,
        id: read.id,
        source: Source::Authored,
        login: Some("someone".into()),
        ended: None,
        seen: Seen::default(),
    }
}

struct Fix {
    clock: Arc<TestClock>,
    state: AppState,
    poller: Arc<FakePoller>,
    poster: Arc<RecordingPoster>,
    sink: BufferSink,
    seq: Arc<AtomicU64>,
}

fn fix() -> Fix {
    fix_with(FileContent::default(), None)
}

/// A store loaded from `loaded` (STA-8), and a real notifier over `real` or a `RecordingPoster`.
fn fix_with(loaded: FileContent, real: Option<Arc<dyn Poster>>) -> Fix {
    let clock = TestClock::new(t0());
    let (mut state, poller) = test_state(clock.clone());
    let sink = BufferSink::default();
    state.logger = Logger::new(state.clock.clone(), Arc::new(sink.clone()), Level::Info);
    state.store = Store::new(
        StoreDeps {
            clock: state.clock.clone(),
            logger: state.logger.clone(),
            config: state.config.clone(),
            seed: 1,
        },
        loaded,
    );
    let recording = RecordingPoster::new(state.clock.clone());
    let poster: Arc<dyn Poster> = real.unwrap_or_else(|| recording.clone());
    let handle: Arc<dyn PollerHandle> = poller.clone();
    state.notify = Notifier::spawn(NotifyDeps {
        store: state.store.clone(),
        clock: state.clock.clone(),
        logger: state.logger.clone(),
        activity: state.activity.clone(),
        poster,
        poller: handle,
        sleep: state.sleep.clone(),
        warnings: state.warnings.clone(),
        config: state.config.clone(),
    });
    Fix {
        clock,
        state,
        poller,
        poster: recording,
        sink,
        seq: Arc::new(AtomicU64::new(0)),
    }
}

struct Answer {
    status: u16,
    text: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap()
    }

    fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    fn message(&self) -> String {
        self.json()["error"]["message"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }
}

async fn read_answer(response: axum::response::Response) -> Answer {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Answer {
        status,
        text: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

fn notify_request(uri: &str, body: &str) -> http::Request<Body> {
    http::Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

impl Fix {
    fn put(&self, read: &PrRead) -> Applied {
        put_on(&self.state.store, &self.clock, &self.seq, read)
    }

    fn track(&self, read: &PrRead) -> NodeId {
        self.state
            .store
            .reserve(&read.id, &read.pr, Source::Authored)
            .unwrap();
        assert!(matches!(self.put(read), Applied::Changed { .. }));
        read.id.clone()
    }

    /// A read-through applies `read` as `asked` (POL-16).
    fn read_through_applies(&self, read: PrRead) {
        let (store, clock, seq) = (
            self.state.store.clone(),
            self.clock.clone(),
            self.seq.clone(),
        );
        self.poller.on_read_through(move |_| {
            store.reserve(&read.id, &read.pr, Source::Asked).unwrap();
            put_on(&store, &clock, &seq, &read);
            Ok(read.id.clone())
        });
    }

    /// The arrival refresh applies `read`.
    fn refresh_applies(&self, read: PrRead) {
        let (store, clock, seq) = (
            self.state.store.clone(),
            self.clock.clone(),
            self.seq.clone(),
        );
        self.poller.on_refresh(move |_| {
            put_on(&store, &clock, &seq, &read);
            Refreshed::Applied
        });
    }

    async fn post(&self, uri: &str, body: Value) -> Answer {
        self.post_raw(uri, &body.to_string()).await
    }

    async fn post_raw(&self, uri: &str, body: &str) -> Answer {
        let response = router(self.state.clone())
            .oneshot(notify_request(uri, body))
            .await
            .unwrap();
        let a = read_answer(response).await;
        self.clock.settle().await;
        a
    }

    async fn delete(&self, uri: &str) -> Answer {
        let response = router(self.state.clone())
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let a = read_answer(response).await;
        self.clock.settle().await;
        a
    }

    fn cursor(&self) -> String {
        self.state.store.view().cursor().to_string()
    }

    fn subscriptions(&self) -> u32 {
        self.state.store.view().subscriptions
    }

    fn set_auth(&self) {
        let mut status = self.poller.status();
        status.problem = Some(DaemonProblem {
            code: ProblemCode::Auth,
            message: "There is no usable token: run `gh auth login`.".into(),
            until: None,
        });
        status.problem_since = Some(self.clock.mono());
        self.poller.set_status(status);
    }
}

fn uri(n: u32) -> String {
    format!("/v1/prs/o/r/{n}/notify")
}

fn checks_done(socket: &str) -> Value {
    json!({"until": "checks-done", "socket": socket})
}

// ---- NTF-2: the body ----

// NTF-2, API-1: the body is strict; each bad value is `bad_request` naming it, before anything else.
#[tokio::test]
async fn ntf2_strict_body() {
    let f = fix();
    f.track(&running("o/r#1"));
    let cases: Vec<(&str, String)> = vec![
        ("until", json!({"socket": A}).to_string()),
        ("socket", json!({"until": "merged"}).to_string()),
        (
            "colour",
            json!({"until": "merged", "socket": A, "colour": 1}).to_string(),
        ),
        (
            "fail_fast",
            json!({"until": "checks-done", "socket": A, "fail_fast": 1}).to_string(),
        ),
        (
            "kind",
            json!({"until": "changed", "socket": A, "kind": ["activity"]}).to_string(),
        ),
        (
            "for",
            json!({"until": "merged", "socket": A, "for": 59}).to_string(),
        ),
        (
            "for",
            json!({"until": "merged", "socket": A, "for": 86401}).to_string(),
        ),
        (
            "limit",
            json!({"until": "merged", "socket": A, "limit": 0}).to_string(),
        ),
        (
            "sha",
            json!({"until": "merged", "socket": A, "sha": "abcdef0"}).to_string(),
        ),
        ("JSON", "{\"until\": ".to_string()),
        (
            "socket",
            json!({"until": "merged", "socket": "relative/s.sock"}).to_string(),
        ),
    ];
    for (name, body) in cases {
        let a = f.post_raw(URI1, &body).await;
        assert_eq!(a.status, 400, "{body}");
        assert_eq!(a.code(), "bad_request", "{body}");
        assert!(a.message().contains(name), "{body}: {}", a.message());
    }
    assert_eq!(f.poller.read_throughs(), 0);
    assert_eq!(f.poller.refreshes(), 0);
    assert!(f.poster.posts().is_empty());
    assert_eq!(f.subscriptions(), 0);
}

// ---- NTF-2: the socket ----

// NTF-2, API-3: a socket that isn't the user's is `no_session`, before GitHub and before an answer
// already known; a confirmation that can't be posted is `no_session` too, and nothing is kept.
#[tokio::test]
async fn ntf2_no_session() {
    let f = fix();
    f.poster.gone(std::path::Path::new(A));
    // An untracked PR: no read-through.
    let a = f.post(&uri(9), checks_done(A)).await;
    assert_eq!((a.status, a.code()), (409, "no_session".to_string()));
    assert!(a.message().contains("socket"), "{}", a.message());
    assert_eq!(f.poller.read_throughs(), 0);
    assert_eq!(f.poller.refreshes(), 0);
    // An answer already known: still `no_session`.
    f.track(&failed("o/r#1"));
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!((a.status, a.code()), (409, "no_session".to_string()));
    assert_eq!(f.poller.refreshes(), 0);

    // The confirmation fails: `no_session` naming the error, nothing kept.
    f.track(&running("o/r#2"));
    f.poster.answer_next(Posted::Failed("boom-error".into()));
    let a = f.post(&uri(2), checks_done(B)).await;
    assert_eq!((a.status, a.code()), (409, "no_session".to_string()));
    assert!(a.message().contains("boom-error"), "{}", a.message());
    assert_eq!(f.poster.posts().len(), 1);
    assert_eq!(f.subscriptions(), 0);
    assert_eq!(f.state.notify.count(), 0);

    // A regular file, through the real poster.
    let dir = ShortDir::new();
    let file = dir.path().join("s.sock");
    std::fs::write(&file, b"").unwrap();
    let real: Arc<dyn Poster> = RealPoster::new();
    let f = fix_with(FileContent::default(), Some(real));
    f.track(&running("o/r#1"));
    let a = f.post(URI1, checks_done(file.to_str().unwrap())).await;
    assert_eq!((a.status, a.code()), (409, "no_session".to_string()));
    assert!(!a.message().contains(file.to_str().unwrap()));
    assert_eq!(f.poller.refreshes(), 0);
    assert_eq!(f.subscriptions(), 0);
}

// ---- NTF-2: the `auth` state ----

// NTF-2, API-3: no outcome in the `auth` state is `auth`, before the limit; an answer already known
// still answers.
#[tokio::test]
async fn ntf2_auth() {
    let f = fix();
    f.track(&running("o/r#1"));
    f.set_auth();
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!((a.status, a.code()), (503, "auth".to_string()));
    assert!(f.poster.posts().is_empty());
    assert_eq!(f.subscriptions(), 0);

    // With 64 held: `auth`, not `limit`.
    let f = fix();
    for n in 1..=MAX_SUBSCRIPTIONS {
        f.track(&running(&format!("o/r#{n}")));
        let socket = if n <= 32 { A } else { B };
        let a = f.post(&uri(n), checks_done(socket)).await;
        assert_eq!(a.status, 200, "{}", a.text);
    }
    f.track(&running("o/r#65"));
    f.set_auth();
    let posts = f.poster.posts().len();
    let a = f.post(&uri(65), checks_done(C)).await;
    assert_eq!((a.status, a.code()), (503, "auth".to_string()));
    assert_eq!(f.poster.posts().len(), posts);
    assert_eq!(f.subscriptions(), MAX_SUBSCRIPTIONS);

    // Failed and settled checks: the answer is known.
    f.track(&failed("o/r#66"));
    let a = f.post(&uri(66), checks_done(C)).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json()["registered"], false);
    assert_eq!(a.json()["outcome"], "satisfied");
}

// ---- NTF-2: kept ----

// NTF-2: kept, the confirmation posted before the answer; the answer's keys.
#[tokio::test]
async fn ntf2_registered() {
    let f = fix();
    let id = f.track(&running("o/r#1"));
    let before = f.cursor();
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!(a.status, 200, "{}", a.text);
    let posts = f.poster.posts();
    assert_eq!(posts.len(), 1);
    assert!(posts[0].text.contains("watching"), "{}", posts[0].text);
    let v = a.json();
    assert_eq!(v["registered"], true);
    assert_eq!(v["replaced"], false);
    assert_eq!(v["expires_at"], "2026-11-25T22:00:00Z");
    assert!(v["outcome"].is_null() && v["reason"].is_null());
    assert_eq!(v["record"]["pr"], "o/r#1");
    assert!(v["record"]["checks"].get("items").is_some());
    assert_eq!(v["events"], json!([]));
    assert_eq!(v["records"], json!([]));
    assert_eq!(v["reset"], false);
    assert_eq!(v["cursor"], before);
    assert_eq!(f.subscriptions(), 1);
    assert!(f.state.store.view().get(&id).is_some());
}

// NTF-2, STA-9: a `since` too old on the registration's view: `reset`, `records`, and let go: the
// subscription's cursor is the answer's.
#[tokio::test]
async fn ntf2_since_too_old() {
    let f = fix();
    f.track(&running("o/r#1"));
    let a = f
        .post(
            URI1,
            json!({"until": "checks-done", "socket": A, "since": "zzzzzzzz-5", "for": 60}),
        )
        .await;
    assert_eq!(a.status, 200, "{}", a.text);
    let v = a.json();
    assert_eq!(v["registered"], true);
    assert_eq!(v["reset"], true);
    let records = v["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["pr"], "o/r#1");
    assert!(records[0]["checks"].get("items").is_none());
    let cursor = v["cursor"].as_str().unwrap().to_string();
    // Keep the record fresh so the expiry isn't held for a catch-up.
    for _ in 0..30 {
        f.clock.advance(10 * SEC).await;
        f.put(&running("o/r#1"));
        f.clock.settle().await;
        if f.poster.posts().len() > 1 {
            break;
        }
    }
    let posts = f.poster.posts();
    assert_eq!(posts.len(), 2, "{posts:?}");
    assert!(posts[1].text.contains("expired"), "{}", posts[1].text);
    assert!(
        posts[1].text.ends_with(&format!("--since {cursor}")),
        "{}",
        posts[1].text
    );
    assert!(!posts[1].text.contains("zzzzzzzz"));
}

// NTF-2: an answer already known is the `wait`'s: nothing kept, nothing posted.
#[tokio::test]
async fn ntf2_already_known() {
    let f = fix();
    f.track(&failed("o/r#1"));
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!(a.status, 200, "{}", a.text);
    let v = a.json();
    assert_eq!(v["registered"], false);
    assert_eq!(v["replaced"], false);
    assert!(v["expires_at"].is_null());
    assert_eq!(v["outcome"], "satisfied");
    assert!(v["reason"].is_null());
    assert_eq!(v["record"]["checks"]["state"], "failing");
    assert!(v["record"]["checks"].get("items").is_some());
    assert!(f.poster.posts().is_empty());
    assert_eq!(f.subscriptions(), 0);

    f.track(&with_state(running("o/r#2"), ReadState::Closed));
    let a = f
        .post(&uri(2), json!({"until": "merged", "socket": A}))
        .await;
    let v = a.json();
    assert_eq!(
        (v["outcome"].clone(), v["reason"].clone()),
        ("ended".into(), "closed".into())
    );
    assert!(f.poster.posts().is_empty());
    assert_eq!(f.subscriptions(), 0);
}

// NTF-2, API-12: one evaluation as `timeout=0`: the arrival refresh once, the read-through when due
// (a loaded PR with no record too), and `NotSent` evaluates on the record in memory.
#[tokio::test]
async fn ntf2_evaluates_as_timeout_0() {
    let f = fix();
    f.track(&running("o/r#1"));
    f.post(URI1, checks_done(A)).await;
    assert_eq!(f.poller.refreshes(), 1);
    assert_eq!(f.poller.read_throughs(), 0);

    // Untracked: read through once, then the arrival refresh.
    f.read_through_applies(running("o/r#2"));
    let a = f.post(&uri(2), checks_done(A)).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json()["registered"], true);
    assert_eq!(f.poller.read_throughs(), 1);
    assert_eq!(f.poller.refreshes(), 2);

    // A loaded PR with no record: read through too, unlike a wait's (NTF-2, STA-8).
    let f = fix_with(
        FileContent {
            prs: vec![loaded_entry("o/r#1")],
        },
        None,
    );
    f.read_through_applies(failed("o/r#1"));
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(f.poller.read_throughs(), 1);
    assert_eq!(a.json()["outcome"], "satisfied");

    // A refusal answers as API-3.
    let f = fix();
    let a = f.post(&uri(3), checks_done(A)).await;
    assert_eq!(
        (a.status, a.code()),
        (503, "github_unavailable".to_string())
    );
    assert!(f.poster.posts().is_empty());

    // `NotSent` (the fake's default): evaluated on the record in memory.
    let f = fix();
    f.track(&failed("o/r#1"));
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!(f.poller.refreshes(), 1);
    assert_eq!(a.json()["outcome"], "satisfied");

    // An arrival refresh applied is evaluated.
    let f = fix();
    f.track(&running("o/r#1"));
    f.refresh_applies(failed("o/r#1"));
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!(a.json()["outcome"], "satisfied");
    assert!(f.poster.posts().is_empty());
}

// NTF-2: the same request again replaces; the same as the daemon reads it (`sha` without case,
// `fail_fast: false` as left out); and an answer already known still replaces.
#[tokio::test]
async fn ntf2_replaced() {
    let f = fix();
    f.track(&running("o/r#1"));
    let body = json!({"until": "checks-done", "socket": A, "sha": "abcdef0"});
    let a = f.post(URI1, body.clone()).await;
    assert_eq!(a.json()["replaced"], false);
    let a = f.post(URI1, body.clone()).await;
    assert_eq!(a.json()["registered"], true);
    assert_eq!(a.json()["replaced"], true);
    assert_eq!(f.poster.posts().len(), 2);
    assert_eq!(f.subscriptions(), 1);

    let same = json!({"until": "checks-done", "socket": A, "sha": "ABCDEF0", "fail_fast": false});
    let a = f.post(URI1, same.clone()).await;
    assert_eq!(a.json()["replaced"], true);
    assert_eq!(f.poster.posts().len(), 3);
    assert_eq!(f.subscriptions(), 1);

    // Another socket is held side by side.
    let a = f
        .post(
            URI1,
            json!({"until": "checks-done", "socket": B, "sha": "abcdef0"}),
        )
        .await;
    assert_eq!(a.json()["replaced"], false);
    assert_eq!(f.subscriptions(), 2);

    // Now known at the arrival refresh: `registered: false`, `replaced: true`, nothing posted.
    f.refresh_applies(failed("o/r#1"));
    let a = f.post(URI1, same).await;
    let v = a.json();
    assert_eq!(v["registered"], false, "{}", a.text);
    assert_eq!(v["replaced"], true);
    assert_eq!(v["outcome"], "satisfied");
    assert_eq!(f.poster.posts().len(), 5, "{:?}", f.poster.posts());
    // The other socket's was told by the notifier; A's earlier one is gone without a notice.
    let to_a = f
        .poster
        .posts()
        .iter()
        .filter(|p| p.socket.to_str() == Some(A))
        .count();
    assert_eq!(to_a, 3);
}

// NTF-2: the 64 and the 32 are checked by `register`, after the evaluation; STA-3's 200 before.
#[tokio::test]
async fn ntf2_limit_after_the_evaluation() {
    let f = fix();
    for n in 1..=MAX_SUBSCRIPTIONS {
        f.track(&running(&format!("o/r#{n}")));
        let socket = if n <= 32 { A } else { B };
        let a = f.post(&uri(n), checks_done(socket)).await;
        assert_eq!(a.status, 200, "{}", a.text);
    }
    f.track(&running("o/r#65"));
    let a = f.post(&uri(65), checks_done(C)).await;
    assert_eq!((a.status, a.code()), (409, "limit".to_string()));
    f.track(&failed("o/r#66"));
    let a = f.post(&uri(66), checks_done(C)).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json()["registered"], false);
    // A replacement is never refused.
    let a = f.post(&uri(1), checks_done(A)).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json()["replaced"], true);

    // 32 for one socket.
    let f = fix();
    for n in 1..=32 {
        f.track(&running(&format!("o/r#{n}")));
        assert_eq!(f.post(&uri(n), checks_done(A)).await.status, 200);
    }
    f.track(&running("o/r#33"));
    let a = f.post(&uri(33), checks_done(A)).await;
    assert_eq!((a.status, a.code()), (409, "limit".to_string()));
    assert_eq!(f.post(&uri(33), checks_done(B)).await.status, 200);

    // STA-3: an untracked PR with no place left: `limit`, no read-through.
    let f = fix();
    for n in 1..=MAX_PLACES {
        let r = pr_read(&format!("o/p#{n}"));
        f.state.store.reserve(&r.id, &r.pr, Source::Asked).unwrap();
    }
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!((a.status, a.code()), (409, "limit".to_string()));
    assert_eq!(f.poller.read_throughs(), 0);
}

// NTF-5, STA-1: registering doesn't make the PR hot: no `named`, no waiter, no nudge.
#[tokio::test]
async fn ntf5_never_hot() {
    let f = fix();
    let id = f.track(&running("o/r#1"));
    let a = f.post(URI1, checks_done(A)).await;
    assert_eq!(a.json()["registered"], true);
    let view = f.state.store.view();
    let timing = &view.get(&id).unwrap().timing;
    assert!(timing.named.is_none());
    assert_eq!(timing.waiters, 0);
    assert_eq!(timing.subscriptions, 1);
    assert_eq!(f.poller.nudges(), 0);
    // NTF-3's cancel doesn't either.
    f.delete(&format!("{URI1}?socket={A}")).await;
    let view = f.state.store.view();
    assert!(view.get(&id).unwrap().timing.named.is_none());
    assert_eq!(f.poller.nudges(), 0);
}

// NTF-2, API-8: the events since `since`, cut at `limit`.
#[tokio::test]
async fn ntf2_events_since() {
    let f = fix();
    f.track(&running("o/r#1"));
    let since = f.cursor();
    for i in 0..3 {
        assert!(matches!(
            f.put(&with_draft(running("o/r#1"), i % 2 == 0)),
            Applied::Changed { .. }
        ));
    }
    let a = f
        .post(
            URI1,
            json!({"until": "checks-done", "socket": A, "since": since}),
        )
        .await;
    let v = a.json();
    assert_eq!(v["registered"], true);
    assert_eq!(v["events"].as_array().unwrap().len(), 3);
    assert_eq!(v["more_events"], false);
    let a = f
        .post(
            URI1,
            json!({"until": "checks-done", "socket": A, "since": since, "limit": 2}),
        )
        .await;
    let v = a.json();
    assert_eq!(v["events"].as_array().unwrap().len(), 2);
    assert_eq!(v["more_events"], true);
}

// MOD-13, NTF-2: the answer is the registration's view; a refresh applied during the confirmation
// is the notice's, posted after it.
#[tokio::test]
async fn ntf2_answer_from_the_registration_view() {
    let f = fix();
    f.track(&running("o/r#1"));
    let before = f.cursor();
    let release = f.poster.stall(&f.state.activity);
    let app = router(f.state.clone());
    let task = tokio::spawn(async move {
        let r = app
            .oneshot(notify_request(URI1, &checks_done(A).to_string()))
            .await
            .unwrap();
        read_answer(r).await
    });
    f.clock.settle().await;
    assert_eq!(f.poster.posts().len(), 1);
    assert!(!task.is_finished());
    f.put(&failed("o/r#1"));
    f.clock.settle().await;
    assert_eq!(
        f.poster.posts().len(),
        1,
        "the outcome waits for the confirmation"
    );
    drop(release);
    let a = task.await.unwrap();
    f.clock.settle().await;
    let v = a.json();
    assert_eq!(v["registered"], true, "{}", a.text);
    assert_eq!(v["record"]["checks"]["state"], "pending");
    assert_eq!(v["cursor"], before);
    let posts = f.poster.posts();
    assert_eq!(posts.len(), 2, "{posts:?}");
    assert!(posts[0].text.contains("watching"));
    assert!(posts[1].text.contains("Details"), "{}", posts[1].text);
}

// ---- NTF-3 ----

// NTF-3: cancel drops that socket's, never reads through, never waits for the first poll, posts
// nothing; a renamed PR's subscription is found by the old name.
#[tokio::test]
async fn ntf3_cancel() {
    let f = fix();
    f.track(&running("o/r#1"));
    f.post(URI1, checks_done(A)).await;
    f.post(URI1, checks_done(B)).await;
    assert_eq!(f.subscriptions(), 2);
    let posts = f.poster.posts().len();
    let a = f.delete(&format!("{URI1}?socket={A}")).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(a.json()["cancelled"], 1);
    let a = f.delete(&format!("{URI1}?socket={A}")).await;
    assert_eq!(a.json()["cancelled"], 0);
    assert_eq!(f.subscriptions(), 1);
    let a = f.delete(URI1).await;
    assert_eq!((a.status, a.code()), (400, "bad_request".to_string()));
    assert!(a.message().contains("socket"));
    let a = f.delete(&format!("{URI1}?socket={A}&x=1")).await;
    assert_eq!(a.status, 400);
    // Untracked: 0, no read-through.
    let a = f.delete(&format!("/v1/prs/o/r/9/notify?socket={A}")).await;
    assert_eq!(a.json()["cancelled"], 0);
    assert_eq!(f.poller.read_throughs(), 0);
    // API-18: never `starting`.
    f.poller.set_first_poll(FirstPoll::Running);
    let a = f.delete(&format!("{URI1}?socket={A}")).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(f.poster.posts().len(), posts);

    // NTF-1: renamed after registering; cancelled by the old name.
    let f = fix();
    f.track(&running("o/r#1"));
    f.post(URI1, checks_done(A)).await;
    let mut renamed = running("o/r#1");
    renamed.pr = PrName::parse("o/new#1").unwrap();
    f.put(&renamed);
    f.clock.settle().await;
    assert!(
        f.state
            .store
            .lookup(&PrName::parse("o/r#1").unwrap())
            .is_none()
    );
    let a = f.delete(&format!("{URI1}?socket={A}")).await;
    assert_eq!(a.json()["cancelled"], 1, "{}", a.text);
    assert_eq!(f.subscriptions(), 0);
}

// ---- NTF-9 ----

// NTF-9: the session's token reaches the poster and nothing else: no answer, no log line.
#[tokio::test]
async fn ntf9_token_not_in_answers() {
    let f = fix();
    f.track(&running("o/r#1"));
    let body = json!({"until": "checks-done", "socket": A, "token": MARKER_SESSION_TOKEN});
    let a = f.post(URI1, body.clone()).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert!(!a.text.contains(MARKER_SESSION_TOKEN));
    // An error answer doesn't hold it either.
    f.poster.answer_next(Posted::Failed("boom".into()));
    let a = f
        .post(
            &uri(1),
            json!({"until": "merged", "socket": A, "token": MARKER_SESSION_TOKEN}),
        )
        .await;
    assert_eq!(a.status, 409);
    assert!(!a.text.contains(MARKER_SESSION_TOKEN));
    let a = f
        .post(URI1, json!({"until": "merged", "socket": A, "token": 5}))
        .await;
    assert_eq!(a.status, 400);
    assert!(a.message().contains("token"));
    assert!(!f.sink.contents().contains(MARKER_SESSION_TOKEN));
    let posts = f.poster.posts();
    assert_eq!(posts[0].token.as_deref(), Some(MARKER_SESSION_TOKEN));
    assert!(posts.iter().all(|p| !p.text.contains(MARKER_SESSION_TOKEN)));
    // `null` is no token.
    let a = f
        .post(
            &uri(1),
            json!({"until": "changed", "socket": B, "token": null}),
        )
        .await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(f.poster.posts().last().unwrap().token, None);
}

// ---- API-19: the shapes ----

/// A JSON document's shape with its keys in their order (serde_json's `Value` sorts them).
enum Shape {
    Leaf,
    List(Vec<Shape>),
    Map(Vec<(String, Shape)>),
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Shape, D::Error> {
        d.deserialize_any(ShapeVisitor)
    }
}

struct ShapeVisitor;

impl<'de> Visitor<'de> for ShapeVisitor {
    type Value = Shape;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E>(self, _: bool) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }
    fn visit_i64<E>(self, _: i64) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }
    fn visit_u64<E>(self, _: u64) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }
    fn visit_f64<E>(self, _: f64) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }
    fn visit_str<E>(self, _: &str) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }
    fn visit_unit<E>(self) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Shape, A::Error> {
        let mut all = Vec::new();
        while let Some(s) = seq.next_element()? {
            all.push(s);
        }
        Ok(Shape::List(all))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Shape, A::Error> {
        let mut all = Vec::new();
        while let Some((k, v)) = map.next_entry::<String, Shape>()? {
            all.push((k, v));
        }
        Ok(Shape::Map(all))
    }
}

/// The same keys in the same order at every level both have: objects by key, lists by their first
/// element. An event's `kinds` are compared only by their first, when both have one of the same kind.
fn same_keys(path: &str, got: &Shape, want: &Shape) {
    match (got, want) {
        (Shape::Map(a), Shape::Map(b)) => {
            let ka: Vec<&str> = a.iter().map(|(k, _)| k.as_str()).collect();
            let kb: Vec<&str> = b.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(ka, kb, "keys at {path}");
            for ((k, va), (_, vb)) in a.iter().zip(b) {
                if k == "kinds" {
                    continue;
                }
                same_keys(&format!("{path}.{k}"), va, vb);
            }
        }
        (Shape::List(a), Shape::List(b)) => {
            if let (Some(x), Some(y)) = (a.first(), b.first()) {
                same_keys(&format!("{path}[0]"), x, y);
            }
        }
        _ => {}
    }
}

fn shape_of(text: &str) -> Shape {
    serde_json::from_str(text).unwrap()
}

fn golden_shape(file: &str) -> Shape {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    shape_of(&std::fs::read_to_string(path).unwrap())
}

// API-19: the bodies have the golden files' key names, at every level and in order.
#[tokio::test]
async fn api19_shapes() {
    let f = fix();
    f.track(&read_of(
        "o/r#1",
        vec![
            item(1, Phase::InProgress, None),
            item(2, Phase::Queued, None),
        ],
    ));
    let a = f.post(URI1, checks_done(A)).await;
    same_keys(
        "",
        &shape_of(&a.text),
        &golden_shape("testdata/api/notify-registered.json"),
    );

    let since = f.cursor();
    let mut r = failed("o/r#2");
    r.checks
        .items
        .push(item(2, Phase::Completed, Some("success")));
    f.track(&r);
    let a = f
        .post(
            &uri(2),
            json!({"until": "checks-done", "socket": A, "since": since}),
        )
        .await;
    assert_eq!(
        a.json()["events"].as_array().unwrap().len(),
        1,
        "{}",
        a.text
    );
    same_keys(
        "",
        &shape_of(&a.text),
        &golden_shape("testdata/api/notify-already.json"),
    );

    let a = f.delete(&format!("{URI1}?socket={A}")).await;
    same_keys(
        "",
        &shape_of(&a.text),
        &golden_shape("testdata/api/notify-cancelled.json"),
    );

    f.poster.gone(std::path::Path::new(B));
    let a = f.post(URI1, checks_done(B)).await;
    same_keys(
        "",
        &shape_of(&a.text),
        &golden_shape("testdata/api/error-no_session.json"),
    );

    f.track(&running("o/r#3"));
    for n in 4..=35 {
        f.track(&running(&format!("o/r#{n}")));
        f.post(&uri(n), checks_done(C)).await;
    }
    let a = f.post(&uri(3), checks_done(C)).await;
    assert_eq!(a.code(), "limit");
    same_keys(
        "",
        &shape_of(&a.text),
        &golden_shape("testdata/api/error-limit.json"),
    );
}
