//! The `wait` route's tests (T23): through `oneshot` on `api::router`, reading the body as a stream,
//! on the test clock, over a real store.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Body;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use tokio::task::JoinHandle;
use tower::ServiceExt;

use super::*;
use crate::api::{router, test_state};
use crate::clock::{Clock, TestClock, Time};
use crate::github::fake::answers::pr_read;
use crate::model::{
    CheckItem, CheckKind, ItemKind, ItemRead, ItemsOutcome, ItemsRead, Phase, PrRead, ReadMeta,
    ReadState, Seen, Source,
};
use crate::poller::fake::FakePoller;
use crate::poller::{FirstPoll, PollReport, PollerStatus, Refreshed, Refusal, RefusalCode};
use crate::state::store::{Applied, Store, StoreDeps};
use crate::state::{FileContent, FileEntry};

const SEC: Duration = Duration::from_secs(1);
const HEAD1: &str = "1111111111111111111111111111111111111111";
const HEAD2: &str = "2222222222222222222222222222222222222222";

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

/// Applies `read` to `store` as a refresh sent now. GitHub's `Date` is an hour after the PR's
/// `updated_at`, so a first sight is settled at once (MOD-6).
fn put_with(
    store: &Store,
    clock: &TestClock,
    seq: &AtomicU64,
    read: &PrRead,
    items: &ItemsRead,
) -> Applied {
    let meta = ReadMeta {
        seq: seq.fetch_add(1, Ordering::SeqCst) + 1,
        sent_wall: clock.wall(),
        sent_mono: clock.mono(),
        github_date: Some(t0().plus(Duration::from_secs(3_600))),
    };
    store.apply(
        &read.id,
        read,
        ItemsOutcome::Read(items),
        &meta,
        Some("someone"),
    )
}

fn put_at(store: &Store, clock: &TestClock, seq: &AtomicU64, read: &PrRead) -> Applied {
    put_with(store, clock, seq, read, &ItemsRead::default())
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

/// An open read of `name` at `head`, its checks as given.
fn read_of(name: &str, head: &str, items: Vec<CheckItem>) -> PrRead {
    let mut r = pr_read(name);
    r.head_sha = head.to_string();
    r.checks.total = items.len() as u32;
    r.checks.items = items;
    r
}

/// One check still queued: `pending`.
fn pending(name: &str) -> PrRead {
    read_of(name, HEAD1, vec![item(1, Phase::Queued, None)])
}

fn with_state(mut r: PrRead, state: ReadState) -> PrRead {
    r.state = state;
    r
}

fn with_draft(mut r: PrRead, draft: bool) -> PrRead {
    r.draft = draft;
    r
}

/// A started wait: the head, and the body read to its end by a task of its own.
struct Started {
    status: u16,
    cursor: Option<String>,
    body: JoinHandle<String>,
}

impl Started {
    fn done(&self) -> bool {
        self.body.is_finished()
    }

    /// The body as JSON. A 200's must have come already; an error's is whole from the start.
    async fn json(self) -> Value {
        serde_json::from_str(&self.text().await).unwrap()
    }

    async fn text(self) -> String {
        if self.status == 200 {
            assert!(self.done(), "the wait hasn't answered");
        }
        self.body.await.unwrap()
    }
}

/// Reads `response`'s head, and its body in a task of its own (the body completes later).
fn started(response: Response) -> Started {
    let status = response.status().as_u16();
    let cursor = response
        .headers()
        .get(CURSOR_HEADER)
        .map(|v| v.to_str().unwrap().to_string());
    let body = tokio::spawn(async move {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    });
    Started {
        status,
        cursor,
        body,
    }
}

fn get(uri: &str) -> http::Request<Body> {
    http::Request::builder()
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

struct Fix {
    clock: Arc<TestClock>,
    state: AppState,
    poller: Arc<FakePoller>,
    seq: Arc<AtomicU64>,
}

fn fix() -> Fix {
    fix_loaded(FileContent::default())
}

/// A fixture whose store starts from `loaded` (STA-8).
fn fix_loaded(loaded: FileContent) -> Fix {
    let clock = TestClock::new(t0());
    let (mut state, poller) = test_state(clock.clone());
    state.store = Store::new(
        StoreDeps {
            clock: state.clock.clone(),
            logger: state.logger.clone(),
            config: state.config.clone(),
            seed: 1,
        },
        loaded,
    );
    Fix {
        clock,
        state,
        poller,
        seq: Arc::new(AtomicU64::new(0)),
    }
}

impl Fix {
    fn put(&self, read: &PrRead) -> Applied {
        put_at(&self.state.store, &self.clock, &self.seq, read)
    }

    fn put_items(&self, read: &PrRead, items: &ItemsRead) -> Applied {
        put_with(&self.state.store, &self.clock, &self.seq, read, items)
    }

    /// Tracks `read` as `authored`, with its first refresh applied.
    fn track(&self, read: &PrRead) -> NodeId {
        self.state
            .store
            .reserve(&read.id, &read.pr, Source::Authored)
            .unwrap();
        assert!(matches!(self.put(read), Applied::Changed { .. }));
        read.id.clone()
    }

    /// The arrival refresh applies `read` (`Applied`).
    fn refresh_applies(&self, read: PrRead) {
        let (store, clock, seq) = (
            self.state.store.clone(),
            self.clock.clone(),
            self.seq.clone(),
        );
        self.poller.on_refresh(move |_| {
            put_at(&store, &clock, &seq, &read);
            Refreshed::Applied
        });
    }

    async fn send(&self, uri: &str) -> Response {
        router(self.state.clone()).oneshot(get(uri)).await.unwrap()
    }

    async fn start(&self, uri: &str) -> Started {
        started(self.send(uri).await)
    }

    fn cursor(&self) -> String {
        self.state.store.view().cursor().to_string()
    }

    fn waiters(&self) -> u32 {
        self.state.store.view().waiters
    }
}

// ---- API-10, API-11 ----

// API-10, API-11: a few cases end to end; the table is T44's.
#[tokio::test]
async fn api10_api11_through_the_route() {
    // `merged` answers when the PR merges.
    let f = fix();
    f.track(&pending("o/r#1"));
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    f.clock.settle().await;
    assert_eq!(w.status, 200);
    assert!(!w.done());
    f.put(&with_state(pending("o/r#1"), ReadState::Merged));
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(v["outcome"], "satisfied");
    assert!(v["reason"].is_null());
    assert_eq!(v["record"]["state"], "merged");

    // `checks-done` on a closed PR: `ended`, `closed`.
    f.track(&with_state(pending("o/r#2"), ReadState::Closed));
    let w = f.start("/v1/prs/o/r/2/wait?until=checks-done").await;
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(
        (v["outcome"].clone(), v["reason"].clone()),
        ("ended".into(), "closed".into())
    );

    // `fail_fast=1`: satisfied at the first failed check while another runs; without it, it goes on.
    f.track(&read_of(
        "o/r#3",
        HEAD1,
        vec![
            item(1, Phase::Completed, Some("failure")),
            item(2, Phase::InProgress, None),
        ],
    ));
    let fast = f
        .start("/v1/prs/o/r/3/wait?until=checks-done&fail_fast=1")
        .await;
    let slow = f.start("/v1/prs/o/r/3/wait?until=checks-done").await;
    f.clock.settle().await;
    assert!(!slow.done());
    let v = fast.json().await;
    assert_eq!(v["outcome"], "satisfied");
    assert_eq!(v["record"]["checks"]["state"], "pending");
    slow.body.abort();

    // `kind=activity`: not answered by a `checks` event, answered by an `activity` one.
    f.track(&pending("o/r#4"));
    let w = f
        .start("/v1/prs/o/r/4/wait?until=changed&kind=activity")
        .await;
    f.clock.settle().await;
    let passed = read_of(
        "o/r#4",
        HEAD1,
        vec![item(1, Phase::Completed, Some("success"))],
    );
    assert!(matches!(f.put(&passed), Applied::Changed { .. }));
    f.clock.settle().await;
    assert!(!w.done(), "a `checks` event doesn't answer `kind=activity`");
    let mut commented = passed.clone();
    commented.counts.comments = 1;
    let items = ItemsRead {
        comments: vec![ItemRead {
            kind: ItemKind::Comment,
            id: 7,
            author: Some("someone".into()),
            at: Some(t0().plus(SEC)),
            review_state: None,
            inline: None,
        }],
        reviews: Vec::new(),
    };
    assert!(matches!(
        f.put_items(&commented, &items),
        Applied::Changed { .. }
    ));
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(v["outcome"], "satisfied");
    assert!(v["events"].as_array().unwrap().iter().any(|e| {
        e["kinds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k["kind"] == "activity")
    }));

    // And a merge ends it: `ended`, `merged`.
    let w = f
        .start("/v1/prs/o/r/4/wait?until=changed&kind=activity")
        .await;
    f.clock.settle().await;
    assert!(!w.done());
    f.put(&with_state(commented, ReadState::Merged));
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(
        (v["outcome"].clone(), v["reason"].clone()),
        ("ended".into(), "merged".into())
    );
}

// ---- API-9 ----

// API-9, U6: the arrival's order: parameters, limits, read-through, registration and the head, which goes
// out while the request is held open.
#[tokio::test]
async fn api9_arrival_order() {
    let f = fix();
    for query in [
        "until=merged&timeout=3601",
        "until=checks-done&sha=xyz",
        "until=merged&sha=abcdef1",
        "until=merged&fail_fast=1",
        "until=changed&kind=foo",
        "until=checks-done&fail_fast=true",
        "timeout=5",
        "until=merged&limit=0",
        "until=merged&other=1",
    ] {
        let w = f.start(&format!("/v1/prs/o/r/1/wait?{query}")).await;
        assert_eq!(w.status, 400, "{query}");
        assert_eq!(w.json().await["error"]["code"], "bad_request", "{query}");
    }
    assert_eq!(
        (
            f.poller.read_throughs(),
            f.poller.refreshes(),
            f.poller.nudges()
        ),
        (0, 0, 0)
    );

    // STA-3: an untracked PR at 200 places is `limit`, with no read-through.
    let full = fix();
    for i in 0..crate::state::MAX_PLACES {
        let id = NodeId(format!("X{i}"));
        let name = PrName::parse(&format!("o/x#{}", i + 1)).unwrap();
        full.state
            .store
            .reserve(&id, &name, Source::Authored)
            .unwrap();
    }
    assert_eq!(full.state.store.places_free(), 0);
    let w = full.start("/v1/prs/o/r/1/wait?until=merged").await;
    assert_eq!(w.status, 409);
    assert!(w.cursor.is_none());
    assert_eq!(w.json().await["error"]["code"], "limit");
    assert_eq!(full.poller.read_throughs(), 0);

    // API-3: a refused read-through is its error.
    let refused = fix();
    let w = refused.start("/v1/prs/o/r/1/wait?until=merged").await;
    assert_eq!(w.status, 503);
    assert_eq!(w.json().await["error"]["code"], "github_unavailable");
    refused.poller.on_read_through(|_| {
        Err(Refusal {
            code: RefusalCode::NotFound,
            message: "no such pull request".into(),
            until: None,
        })
    });
    let w = refused.start("/v1/prs/o/r/1/wait?until=merged").await;
    assert_eq!(w.status, 404);
    assert_eq!(w.json().await["error"]["code"], "not_found");
    assert_eq!(refused.waiters(), 0);

    // Otherwise the head is 200, with the cursor at registration (after the read-through), before any body.
    let ok = fix();
    let (store, clock, seq) = (ok.state.store.clone(), ok.clock.clone(), ok.seq.clone());
    ok.poller.on_read_through(move |name| {
        let read = pending(&name.to_string());
        store.reserve(&read.id, &read.pr, Source::Asked).unwrap();
        put_at(&store, &clock, &seq, &read);
        Ok(read.id.clone())
    });
    let before = ok.cursor();
    let w = ok.start("/v1/prs/o/r/1/wait?until=merged").await;
    assert_eq!(w.status, 200);
    let after = ok.cursor();
    assert_ne!(before, after);
    assert_eq!(w.cursor.as_deref(), Some(after.as_str()));
    ok.clock.settle().await;
    assert!(!w.done());
    assert_eq!(ok.poller.client_requests(), 1);
    assert_eq!(ok.poller.read_throughs(), 1);
    w.body.abort();
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

// API-9, STA-8: a loaded PR's wait is held with no read-through and no arrival refresh, and first
// evaluated once its first refresh is applied.
#[tokio::test]
async fn api9_loaded_is_held() {
    let f = fix_loaded(FileContent {
        prs: vec![loaded_entry("o/r#1")],
    });
    let held = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    let early = f.start("/v1/prs/o/r/1/wait?until=merged&timeout=5").await;
    f.clock.settle().await;
    assert_eq!((held.status, early.status), (200, 200));
    f.clock.advance(5 * SEC).await;
    assert!(!held.done());
    let v = early.json().await;
    assert_eq!(v["outcome"], "timeout");
    assert!(v["record"].is_null());

    f.put(&with_state(pending("o/r#1"), ReadState::Merged));
    f.clock.settle().await;
    let v = held.json().await;
    assert_eq!(v["outcome"], "satisfied");
    assert_eq!(v["record"]["state"], "merged");
    assert_eq!((f.poller.read_throughs(), f.poller.refreshes()), (0, 0));
}

// API-10: `changed` without `since` takes its baseline after the arrival refresh.
#[tokio::test]
async fn api10_changed_baseline() {
    let f = fix();
    f.track(&pending("o/r#1"));
    f.refresh_applies(with_draft(pending("o/r#1"), true));
    let w = f.start("/v1/prs/o/r/1/wait?until=changed").await;
    f.clock.settle().await;
    assert_eq!(f.poller.refreshes(), 1);
    assert!(!w.done(), "the arrival refresh's change doesn't answer");
    f.put(&with_draft(pending("o/r#1"), false));
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(v["outcome"], "satisfied");
    let events = v["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(v["cursor"], f.cursor());
}

// ---- API-11 ----

// API-11: `head_moved`, by the refreshes' send times, never the clock alone.
#[tokio::test]
async fn api11_head_moved() {
    let f = fix();
    // Matched, then another head.
    f.track(&pending("o/r#1"));
    let w = f
        .start(&format!(
            "/v1/prs/o/r/1/wait?until=checks-done&sha={}",
            &HEAD1[..7]
        ))
        .await;
    f.clock.settle().await;
    assert!(!w.done());
    f.put(&read_of("o/r#1", HEAD2, vec![item(1, Phase::Queued, None)]));
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(
        (v["outcome"].clone(), v["reason"].clone()),
        ("ended".into(), "head_moved".into())
    );

    // Never matched: not on a refresh sent 59 s after arrival, yes on one sent at 60 s.
    let f = fix();
    f.track(&pending("o/r#2"));
    let w = f
        .start(&format!(
            "/v1/prs/o/r/2/wait?until=checks-done&sha={}",
            &HEAD2[..7]
        ))
        .await;
    f.clock.settle().await;
    f.clock.advance(59 * SEC).await;
    f.put(&pending("o/r#2"));
    f.clock.settle().await;
    assert!(!w.done());
    f.clock.advance(SEC).await;
    f.put(&pending("o/r#2"));
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(
        (v["outcome"].clone(), v["reason"].clone()),
        ("ended".into(), "head_moved".into())
    );

    // With no refresh, the clock at 200 s of a 3 600 s wait ends nothing.
    let f = fix();
    f.track(&pending("o/r#3"));
    let w = f
        .start(&format!(
            "/v1/prs/o/r/3/wait?until=checks-done&sha={}&timeout=3600",
            &HEAD2[..7]
        ))
        .await;
    f.clock.advance(200 * SEC).await;
    assert!(!w.done());
    w.body.abort();
}

// ---- API-12 ----

/// A `PollerHandle` of the test's own: `FakePoller`, with a read-through or an arrival refresh that
/// takes time on the clock.
struct SlowPoller {
    inner: Arc<FakePoller>,
    clock: SharedClock,
    read_through: Duration,
    refresh: Duration,
}

#[async_trait::async_trait]
impl PollerHandle for SlowPoller {
    fn client_request(&self) {
        self.inner.client_request();
    }
    fn idle_since(&self) -> Mono {
        self.inner.idle_since()
    }
    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal> {
        crate::clock::sleep(&self.clock, self.read_through).await;
        self.inner.read_through(name).await
    }
    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal> {
        self.inner.track(name).await
    }
    async fn refresh_pr(&self, id: &NodeId) -> Refreshed {
        crate::clock::sleep(&self.clock, self.refresh).await;
        self.inner.refresh_pr(id).await
    }
    async fn poll_now(&self) -> PollReport {
        self.inner.poll_now().await
    }
    fn nudge(&self) {
        self.inner.nudge();
    }
    fn first_poll(&self) -> tokio::sync::watch::Receiver<FirstPoll> {
        self.inner.first_poll()
    }
    fn status(&self) -> PollerStatus {
        self.inner.status()
    }
}

fn slow(f: &mut Fix, read_through: Duration, refresh: Duration) {
    f.state.poller = Arc::new(SlowPoller {
        inner: f.poller.clone(),
        clock: f.state.clock.clone(),
        read_through,
        refresh,
    });
}

// API-12: the deadline: monotonic time plus noticed sleep, from the arrival.
#[tokio::test]
async fn api12_timeout() {
    // The default ends at 100 s, not at 99 s.
    let f = fix();
    f.track(&pending("o/r#1"));
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    f.clock.advance(99 * SEC).await;
    assert!(!w.done());
    f.clock.advance(SEC).await;
    let v = w.json().await;
    assert_eq!(v["outcome"], "timeout");
    assert_eq!(v["record"]["pr"], "o/r#1");

    // `timeout=0` answers after the arrival refresh, with the clock unmoved.
    let f = fix();
    f.track(&pending("o/r#1"));
    f.refresh_applies(pending("o/r#1"));
    let at = f.clock.mono();
    let w = f.start("/v1/prs/o/r/1/wait?until=merged&timeout=0").await;
    f.clock.settle().await;
    assert_eq!(f.clock.mono(), at);
    assert_eq!(f.poller.refreshes(), 1);
    assert_eq!(w.json().await["outcome"], "timeout");

    // Two hours of sleep the monotonic clock didn't count, once noticed, end a 100 s wait.
    let f = fix();
    f.track(&pending("o/r#1"));
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    f.clock.settle().await;
    f.clock.sleep_wall_only(2 * 3_600 * SEC).await;
    assert!(f.state.sleep.check().is_some());
    f.clock.settle().await;
    assert_eq!(w.json().await["outcome"], "timeout");

    // A wall clock set back an hour doesn't lengthen it.
    let f = fix();
    f.track(&pending("o/r#1"));
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    f.clock.settle().await;
    f.clock.set_wall_back(3_600 * SEC).await;
    assert!(f.state.sleep.check().is_none());
    f.clock.advance(99 * SEC).await;
    assert!(!w.done());
    f.clock.advance(SEC).await;
    assert_eq!(w.json().await["outcome"], "timeout");

    // A read-through that outlasts the deadline: 200, `timeout`, `record: null`, with the header.
    let mut f = fix();
    slow(&mut f, 200 * SEC, Duration::ZERO);
    let app = router(f.state.clone());
    let request =
        tokio::spawn(async move { app.oneshot(get("/v1/prs/o/r/1/wait?until=merged")).await });
    f.clock.settle().await;
    assert!(!request.is_finished());
    f.clock.advance(99 * SEC).await;
    assert!(!request.is_finished());
    f.clock.advance(SEC).await;
    assert!(request.is_finished());
    let w = started(request.await.unwrap().unwrap());
    assert_eq!(w.status, 200);
    assert_eq!(w.cursor.as_deref(), Some(f.cursor().as_str()));
    f.clock.settle().await;
    let v = w.json().await;
    assert_eq!(v["outcome"], "timeout");
    assert!(v["record"].is_null());
    assert_eq!(v["cursor"], f.cursor());
    assert_eq!(f.waiters(), 0);
}

// ---- API-13 ----

// API-13: a waiter makes its PR hot and forces an arrival refresh; one not sent counts as failed.
#[tokio::test]
async fn api13_hot() {
    let f = fix();
    let id = f.track(&pending("o/r#1"));
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    f.clock.settle().await;
    assert_eq!(f.state.store.view().get(&id).unwrap().timing.waiters, 1);
    assert_eq!((f.poller.nudges(), f.poller.refreshes()), (1, 1));
    f.put(&with_state(pending("o/r#1"), ReadState::Merged));
    f.clock.settle().await;
    assert_eq!(w.json().await["outcome"], "satisfied");
    // Again when it leaves.
    assert_eq!(f.poller.nudges(), 2);
    assert_eq!(f.state.store.view().get(&id).unwrap().timing.waiters, 0);

    // `NotSent` (the fake's default) and a merged record: answered at once.
    let f = fix();
    f.track(&with_state(pending("o/r#1"), ReadState::Merged));
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    f.clock.settle().await;
    assert_eq!(f.poller.refreshes(), 1);
    assert_eq!(w.json().await["outcome"], "satisfied");
}

// ---- API-14 ----

// API-14: at most 64, and a waiter whose client goes away leaves nothing behind.
#[tokio::test]
async fn api14_64_and_cleanup() {
    let f = fix();
    f.track(&pending("o/r#1"));
    let mut held = Vec::new();
    for _ in 0..MAX_WAITERS {
        let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
        assert_eq!(w.status, 200);
        held.push(w);
    }
    f.clock.settle().await;
    assert_eq!(f.waiters(), 64);
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    assert_eq!(w.status, 409);
    assert_eq!(w.json().await["error"]["code"], "limit");

    // Dropping one body (the client closed) frees its place.
    let one = held.pop().unwrap();
    one.body.abort();
    f.clock.settle().await;
    assert_eq!(f.waiters(), 63);
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    assert_eq!(w.status, 200);
    held.push(w);
    f.clock.settle().await;
    assert_eq!(f.waiters(), 64);
    for w in held {
        w.body.abort();
    }
    f.clock.settle().await;
    assert_eq!(f.waiters(), 0);

    // Dropping a response during the arrival refresh leaves no waiter.
    let mut f = fix();
    let id = f.track(&pending("o/r#1"));
    slow(&mut f, Duration::ZERO, 30 * SEC);
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    f.clock.settle().await;
    assert_eq!(f.waiters(), 1);
    w.body.abort();
    f.clock.settle().await;
    assert_eq!(f.waiters(), 0);
    assert_eq!(f.state.store.view().get(&id).unwrap().timing.waiters, 0);
    // A response dropped before its body was ever read, too.
    let response = f.send("/v1/prs/o/r/1/wait?until=merged").await;
    assert_eq!(f.waiters(), 1);
    drop(response);
    assert_eq!(f.waiters(), 0);
    f.clock.settle().await;
}

// ---- API-19: the shapes ----

/// A JSON document's shape with its keys in their order (serde_json's `Value` sorts them).
enum Shape {
    Leaf(Option<String>),
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
        Ok(Shape::Leaf(None))
    }
    fn visit_i64<E>(self, _: i64) -> Result<Shape, E> {
        Ok(Shape::Leaf(None))
    }
    fn visit_u64<E>(self, _: u64) -> Result<Shape, E> {
        Ok(Shape::Leaf(None))
    }
    fn visit_f64<E>(self, _: f64) -> Result<Shape, E> {
        Ok(Shape::Leaf(None))
    }
    fn visit_str<E>(self, s: &str) -> Result<Shape, E> {
        Ok(Shape::Leaf(Some(s.to_string())))
    }
    fn visit_unit<E>(self) -> Result<Shape, E> {
        Ok(Shape::Leaf(None))
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

fn kind_of(map: &[(String, Shape)]) -> Option<&str> {
    map.iter().find_map(|(k, v)| match (k.as_str(), v) {
        ("kind", Shape::Leaf(Some(s))) => Some(s.as_str()),
        _ => None,
    })
}

/// The same keys in the same order at every level both have: objects by key, lists by their first
/// element. Two objects tagged with different `kind`s aren't compared.
fn same_keys(path: &str, got: &Shape, want: &Shape) {
    match (got, want) {
        (Shape::Map(a), Shape::Map(b)) => {
            if let (Some(x), Some(y)) = (kind_of(a), kind_of(b))
                && x != y
            {
                return;
            }
            let ka: Vec<&str> = a.iter().map(|(k, _)| k.as_str()).collect();
            let kb: Vec<&str> = b.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(ka, kb, "keys at {path}");
            for ((k, va), (_, vb)) in a.iter().zip(b) {
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

/// Whether `key` is anywhere in the shape, so a test can tell the comparison reached it.
fn has_key(shape: &Shape, key: &str) -> bool {
    match shape {
        Shape::Map(m) => m.iter().any(|(k, v)| k == key || has_key(v, key)),
        Shape::List(l) => l.iter().any(|s| has_key(s, key)),
        Shape::Leaf(_) => false,
    }
}

// API-19: the bodies have the golden files' key names, at every level and in order.
#[tokio::test]
async fn api19_shapes() {
    // Satisfied, with a record holding check items and an event.
    let f = fix();
    let since = f.cursor();
    let read = read_of(
        "o/r#1",
        HEAD1,
        vec![
            item(1, Phase::Completed, Some("success")),
            item(2, Phase::Completed, Some("failure")),
        ],
    );
    f.track(&read);
    let w = f
        .start(&format!(
            "/v1/prs/o/r/1/wait?until=checks-done&since={since}"
        ))
        .await;
    f.clock.settle().await;
    let text = w.body.await.unwrap();
    let got = shape_of(&text);
    assert!(has_key(&got, "failed_items") && has_key(&got, "items"));
    same_keys("", &got, &golden_shape("testdata/api/wait-satisfied.json"));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["events"].as_array().unwrap().len(), 1);

    // Timeout with no record: a loaded PR, before its first refresh.
    let f = fix_loaded(FileContent {
        prs: vec![loaded_entry("o/r#1")],
    });
    let w = f.start("/v1/prs/o/r/1/wait?until=merged&timeout=0").await;
    f.clock.settle().await;
    let text = w.body.await.unwrap();
    same_keys(
        "",
        &shape_of(&text),
        &golden_shape("testdata/api/wait-timeout-no-record.json"),
    );
    assert!(serde_json::from_str::<Value>(&text).unwrap()["record"].is_null());

    // A refusal: `error-limit.json`'s.
    let f = fix();
    f.track(&pending("o/r#1"));
    let mut held = Vec::new();
    for _ in 0..MAX_WAITERS {
        held.push(f.start("/v1/prs/o/r/1/wait?until=merged").await);
    }
    f.clock.settle().await;
    let w = f.start("/v1/prs/o/r/1/wait?until=merged").await;
    let text = w.body.await.unwrap();
    same_keys(
        "",
        &shape_of(&text),
        &golden_shape("testdata/api/error-limit.json"),
    );
    for w in held {
        w.body.abort();
    }
    f.clock.settle().await;
}

// ---- STA-9, API-8 ----

// STA-9, API-19: too old is judged when the answer is built; a reset's `records` lack `checks.items`.
#[tokio::test]
async fn sta9_reset_at_answer() {
    let f = fix();
    f.track(&pending("o/r#1"));
    f.track(&pending("o/r#2"));
    let w = f.start("/v1/prs/o/r/1/wait?until=checks-done").await;
    f.clock.settle().await;
    for i in 0..=crate::state::MAX_EVENTS {
        f.put(&with_draft(pending("o/r#2"), i % 2 == 0));
    }
    f.clock.settle().await;
    assert!(!w.done());
    f.clock.advance(100 * SEC).await;
    let text = w.text().await;
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["outcome"], "timeout");
    assert_eq!(v["reset"], true);
    let records = v["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["pr"], "o/r#1");
    assert!(records[0]["checks"].get("items").is_none());
    assert!(v["record"]["checks"].get("items").is_some());
    same_keys(
        "",
        &shape_of(&text),
        &golden_shape("testdata/api/wait-reset.json"),
    );
}

// API-8: `limit` reaches `window`.
#[tokio::test]
async fn api8_limit() {
    let f = fix();
    f.track(&pending("o/r#1"));
    let since = f.cursor();
    let w = f
        .start(&format!(
            "/v1/prs/o/r/1/wait?until=checks-done&since={since}&limit=2&timeout=1"
        ))
        .await;
    f.clock.settle().await;
    let mut revs = Vec::new();
    for i in 0..5 {
        match f.put(&with_draft(pending("o/r#1"), i % 2 == 0)) {
            Applied::Changed { rev } => revs.push(rev),
            other => panic!("{other:?}"),
        }
    }
    f.clock.advance(SEC).await;
    let v = w.json().await;
    let events = v["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["rev"], revs[1]);
    assert_eq!(v["more_events"], true);
    let epoch = f.state.store.view().epoch;
    assert_eq!(v["cursor"], format!("{epoch}-{}", revs[1]));
}
