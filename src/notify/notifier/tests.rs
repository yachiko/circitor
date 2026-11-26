//! The notifier's tests (T46): on the test clock, a real `Store` filled through `reserve` and `apply`
//! with `github::fake::answers`, a `FakePoller`, a real `SleepWatch` and `Warnings`, and `RecordingPoster`.
//! Nothing here reads the environment or posts to a real session's socket.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::api::Until;
use crate::api::condition::{Condition, Progress, evaluate};
use crate::clock::{Clock, SharedClock, SleepWatch, TestClock, Time};
use crate::config::Config;
use crate::github::fake::answers::pr_read;
use crate::log::{BufferSink, Level, Logger};
use crate::model::{
    CheckItem, CheckKind, ChecksRead, ItemsOutcome, ItemsRead, Phase, PrRead, ReadMeta, ReadState,
    Source,
};
use crate::notify::fake::RecordingPoster;
use crate::notify::{DEFAULT_LIFE, NotifyDeps};
use crate::poller::fake::FakePoller;
use crate::poller::{DaemonProblem, PollerHandle};
use crate::state::FileContent;
use crate::state::store::{Store, StoreDeps};
use crate::status::Warnings;
use crate::testkit::{MARKER_CHECK, MARKER_LOGIN, MARKER_SESSION_TOKEN, MARKER_TITLE};

const SEC: Duration = Duration::from_secs(1);
const MIN: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3_600);
const A: &str = "/tmp/ct-a.sock";
const B: &str = "/tmp/ct-b.sock";
const C: &str = "/tmp/ct-c.sock";
const D: &str = "/tmp/ct-d.sock";
const PR1: &str = "o/r#1";
const PR2: &str = "o/r#2";

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

fn path(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn name(s: &str) -> PrName {
    PrName::parse(s).unwrap()
}

fn id(s: &str) -> NodeId {
    pr_read(s).id
}

fn check(phase: Phase, result: Option<&str>) -> CheckItem {
    CheckItem {
        kind: CheckKind::Run,
        id: Some(1),
        name: MARKER_CHECK.into(),
        phase,
        result: result.map(str::to_string),
        url: Some("https://x/1".into()),
    }
}

/// An open PR whose one check runs; its title and author are markers (NTF-8).
fn running(n: &str) -> PrRead {
    let mut r = pr_read(n);
    r.title = MARKER_TITLE.into();
    r.author = Some(MARKER_LOGIN.into());
    r.checks = ChecksRead {
        items: vec![check(Phase::InProgress, None)],
        total: 1,
        rollup: None,
        next_page: None,
    };
    r
}

fn finished(n: &str, result: &str) -> PrRead {
    let mut r = running(n);
    r.checks.items = vec![check(Phase::Completed, Some(result))];
    r
}

fn merged(n: &str) -> PrRead {
    let mut r = running(n);
    r.state = ReadState::Merged;
    r
}

fn closed(n: &str) -> PrRead {
    let mut r = running(n);
    r.state = ReadState::Closed;
    r
}

fn ask(until: Until) -> Ask {
    Ask::new(until)
}

fn sha_ask(sha: &str) -> Ask {
    let mut a = Ask::new(Until::ChecksDone);
    a.sha = Some(sha.into());
    a
}

struct Fix {
    clock: Arc<TestClock>,
    store: Arc<Store>,
    sink: BufferSink,
    poster: Arc<RecordingPoster>,
    poller: Arc<FakePoller>,
    sleep: Arc<SleepWatch>,
    warnings: Warnings,
    n: Arc<Notifier>,
    seq: u64,
}

async fn fix() -> Fix {
    let clock = TestClock::new(t0());
    let shared: SharedClock = clock.clone();
    let sink = BufferSink::default();
    let logger = Logger::new(shared.clone(), Arc::new(sink.clone()), Level::Info);
    let config = Arc::new(Config::default());
    let store = Store::new(
        StoreDeps {
            clock: shared.clone(),
            logger: logger.clone(),
            config: config.clone(),
            seed: 1,
        },
        FileContent::default(),
    );
    let poster = RecordingPoster::new(shared.clone());
    let poller = FakePoller::new();
    let sleep = SleepWatch::new(shared.clone());
    let warnings = Warnings::new(shared.clone(), logger.clone());
    let n = Notifier::spawn(NotifyDeps {
        store: store.clone(),
        clock: shared,
        logger,
        activity: clock.activity(),
        poster: poster.clone(),
        poller: poller.clone(),
        sleep: sleep.clone(),
        warnings: warnings.clone(),
        config,
    });
    let f = Fix {
        clock,
        store,
        sink,
        poster,
        poller,
        sleep,
        warnings,
        n,
        seq: 0,
    };
    f.clock.settle().await;
    f
}

impl Fix {
    /// Applies `read` as sent now; GitHub's `Date` an hour on, so a first read is settled at once (MOD-6).
    fn put(&mut self, read: &PrRead) {
        self.seq += 1;
        let meta = ReadMeta {
            seq: self.seq,
            sent_wall: self.clock.wall(),
            sent_mono: self.clock.mono(),
            github_date: Some(self.clock.wall().plus(HOUR)),
        };
        self.store.apply(
            &read.id,
            read,
            ItemsOutcome::Read(&ItemsRead::default()),
            &meta,
            Some("someone"),
        );
    }

    fn track(&mut self, read: &PrRead) {
        self.store
            .reserve(&read.id, &read.pr, Source::Asked)
            .unwrap();
        self.put(read);
    }

    /// What the route hands over (T47): checked, evaluated once on a view, its socket looked at.
    async fn new_sub(
        &self,
        pr: &str,
        ask: Ask,
        socket: &str,
        life: Duration,
        token: Option<&str>,
    ) -> NewSubscription {
        let id = id(pr);
        let cond = Condition::check(ask).unwrap();
        let view = self.store.view();
        let mut progress = Progress::new(self.clock.mono());
        assert!(evaluate(&cond, &view, &id, &mut progress).is_none());
        let cursor = match &cond.since {
            Some(c) if !view.too_old(c) => c.clone(),
            _ => view.cursor(),
        };
        let socket = path(socket);
        let Looked::Socket(socket_id) = self.n.look(&socket).await else {
            panic!("the fake's socket");
        };
        NewSubscription {
            id,
            pr: name(pr),
            cond,
            progress,
            cursor,
            life,
            life_given: life != DEFAULT_LIFE,
            socket,
            socket_id,
            token: token.map(|t| SessionToken::new(t.to_string())),
        }
    }

    async fn reg(&self, pr: &str, ask: Ask, socket: &str) -> Result<Registered, RegisterError> {
        self.reg_for(pr, ask, socket, DEFAULT_LIFE).await
    }

    async fn reg_for(
        &self,
        pr: &str,
        ask: Ask,
        socket: &str,
        life: Duration,
    ) -> Result<Registered, RegisterError> {
        let new = self.new_sub(pr, ask, socket, life, None).await;
        let r = self.n.register(new).await;
        self.clock.settle().await;
        r
    }

    /// `register` on its own task, for a stalled confirmation.
    async fn reg_spawned(
        &self,
        pr: &str,
        ask: Ask,
        socket: &str,
    ) -> JoinHandle<Result<Registered, RegisterError>> {
        let new = self.new_sub(pr, ask, socket, DEFAULT_LIFE, None).await;
        let n = self.n.clone();
        let h = tokio::spawn(async move { n.register(new).await });
        self.clock.settle().await;
        h
    }

    fn texts(&self) -> Vec<String> {
        self.poster.posts().into_iter().map(|p| p.text).collect()
    }

    fn posts(&self) -> usize {
        self.poster.posts().len()
    }

    fn last(&self) -> String {
        self.texts().last().cloned().unwrap_or_default()
    }

    fn log(&self) -> String {
        self.sink.contents()
    }

    fn log_count(&self, s: &str) -> usize {
        self.log().lines().filter(|l| l.contains(s)).count()
    }

    fn held(&self) -> u32 {
        let n = self.n.count();
        // API-5: `count()` is what `status` shows.
        assert_eq!(n, self.store.view().subscriptions);
        n
    }

    fn told(&self, pr: &str, ask: Ask, cursor: Cursor) -> Told {
        Told {
            pr: name(pr),
            cond: ask,
            life: DEFAULT_LIFE,
            life_given: false,
            cursor,
        }
    }

    fn set_problem(&self, since: Option<Mono>) {
        let mut s = self.poller.status();
        s.problem = since.map(|_| DaemonProblem {
            code: ProblemCode::Auth,
            message: "no login".into(),
            until: None,
        });
        s.problem_since = since;
        self.poller.set_status(s);
    }
}

// NTF-2: the confirmation is posted before `register` answers; `expires_at` is now plus `for`.
#[tokio::test]
async fn ntf2_register_posts_the_confirmation_first() {
    let mut f = fix().await;
    f.track(&running(PR1));
    let cursor = f.store.view().cursor();
    let r = f.reg(PR1, sha_ask("1a2b3c4"), A).await.unwrap();
    assert_eq!(
        r,
        Registered {
            expires_at: t0().plus(DEFAULT_LIFE),
            replaced: false
        }
    );
    let posts = f.poster.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].text,
        text::confirmation(&f.told(PR1, sha_ask("1a2b3c4"), cursor))
    );
    assert_eq!(posts[0].socket, path(A));
    assert_eq!(f.held(), 1);
    assert_eq!(f.store.view().subscriptions, 1);
    assert_eq!(
        f.log_count("subscription registered pr=o/r#1 until=checks-done"),
        1
    );
}

// NTF-2: a confirmation that isn't posted, for any reason, keeps nothing and isn't tried again.
#[tokio::test]
async fn ntf2_confirmation_not_posted_keeps_nothing() {
    let mut f = fix().await;
    f.track(&running(PR1));
    for answer in [
        Posted::SessionGone("refused".into()),
        Posted::Failed("EAGAIN".into()),
    ] {
        let before = f.posts();
        f.poster.answer_next(answer);
        let r = f.reg(PR1, ask(Until::Merged), A).await;
        assert!(matches!(r, Err(RegisterError::NoSession(_))), "{r:?}");
        assert_eq!(f.posts(), before + 1);
        assert_eq!(f.held(), 0);
        assert_eq!(f.store.subscriptions(), 0);
    }

    // Stalled past 2 s of the clock: the limit passes, nothing kept.
    let release = f.poster.stall(&f.clock.activity());
    let h = f.reg_spawned(PR1, ask(Until::Merged), A).await;
    assert_eq!(f.held(), 1);
    f.clock.advance(POST_LIMIT - Duration::from_millis(1)).await;
    assert!(!h.is_finished());
    f.clock.advance(Duration::from_millis(1)).await;
    let r = h.await.unwrap();
    assert!(matches!(r, Err(RegisterError::NoSession(_))), "{r:?}");
    assert_eq!(f.posts(), 3);
    assert_eq!(f.held(), 0);
    assert_eq!(f.store.subscriptions(), 0);
    drop(release);
    assert_eq!(f.log_count("reason=timeout"), 1);

    // As a replacement: the one it replaced is gone too.
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    assert_eq!(f.held(), 1);
    f.poster.answer_next(Posted::Failed("EAGAIN".into()));
    let r = f.reg(PR1, ask(Until::Merged), A).await;
    assert!(matches!(r, Err(RegisterError::NoSession(_))));
    assert_eq!(f.held(), 0);
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 5);
}

// NTF-2: an end found while confirming waits for the confirmation, and goes with it if it fails.
#[tokio::test]
async fn ntf2_an_outcome_during_the_confirmation() {
    let mut f = fix().await;
    f.track(&running(PR1));
    let release = f.poster.stall(&f.clock.activity());
    let h = f.reg_spawned(PR1, ask(Until::Merged), A).await;
    f.put(&merged(PR1));
    f.clock.settle().await;
    assert_eq!(f.held(), 1);
    assert_eq!(f.posts(), 1);
    drop(release);
    f.clock.settle().await;
    let r = h.await.unwrap();
    assert!(r.is_ok(), "{r:?}");
    let texts = f.texts();
    assert_eq!(texts.len(), 2);
    assert!(texts[0].starts_with("circitor: watching o/r#1 until merged"));
    assert!(
        texts[1].starts_with("circitor: o/r#1 merged. Details:"),
        "{}",
        texts[1]
    );
    assert_eq!(f.held(), 0);

    // The same, with the confirmation then answered `Failed`: no outcome notice, ever.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.poster.answer_next(Posted::Failed("EAGAIN".into()));
    let release = f.poster.stall(&f.clock.activity());
    let h = f.reg_spawned(PR1, ask(Until::Merged), A).await;
    f.put(&merged(PR1));
    f.clock.settle().await;
    assert_eq!(f.held(), 1);
    drop(release);
    f.clock.settle().await;
    assert!(matches!(h.await.unwrap(), Err(RegisterError::NoSession(_))));
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 1);
    assert_eq!(f.held(), 0);
}

// NTF-2: "the same request", compared as the daemon reads it.
#[tokio::test]
async fn ntf2_replace() {
    let mut f = fix().await;
    f.track(&running(PR1));
    assert!(!f.reg(PR1, sha_ask("1a2b3c4"), A).await.unwrap().replaced);
    let again = f.reg(PR1, sha_ask("1a2b3c4"), A).await.unwrap();
    assert!(again.replaced);
    assert_eq!(f.posts(), 2);
    assert!(f.texts()[1].starts_with("circitor: watching"));
    assert_eq!(f.held(), 1);
    // `sha` lower-cased as given.
    assert!(f.reg(PR1, sha_ask("1A2B3C4"), A).await.unwrap().replaced);
    assert_eq!(f.held(), 1);
    // A short and a full SHA differ.
    let full = format!("1a2b3c4{}", "0".repeat(33));
    assert!(!f.reg(PR1, sha_ask(&full), A).await.unwrap().replaced);
    assert_eq!(f.held(), 2);
    // `fail_fast` differs; left out is `false`, which `Ask` holds as `false`.
    let mut ff = sha_ask("1a2b3c4");
    ff.fail_fast = true;
    assert!(!f.reg(PR1, ff, A).await.unwrap().replaced);
    assert_eq!(f.held(), 3);
    let mut not_ff = sha_ask("1a2b3c4");
    not_ff.fail_fast = false;
    assert!(f.reg(PR1, not_ff, A).await.unwrap().replaced);
    assert_eq!(f.held(), 3);
    // `kind` as a set.
    let mut ab = ask(Until::Changed);
    ab.kind = Some("review,checks".into());
    assert!(!f.reg(PR1, ab, A).await.unwrap().replaced);
    assert_eq!(f.held(), 4);
    let mut ba = ask(Until::Changed);
    ba.kind = Some("checks,review".into());
    assert!(f.reg(PR1, ba, A).await.unwrap().replaced);
    let mut other = ask(Until::Changed);
    other.kind = Some("review".into());
    assert!(!f.reg(PR1, other, A).await.unwrap().replaced);
    assert_eq!(f.held(), 5);
    // Another socket is another request.
    assert!(!f.reg(PR1, sha_ask("1a2b3c4"), B).await.unwrap().replaced);
    assert_eq!(f.held(), 6);
}

// NTF-2: 64 in all, 32 for one socket; a replacement is never refused.
#[tokio::test]
async fn ntf2_limit() {
    let mut f = fix().await;
    f.track(&running(PR1));
    let sha = |i: usize| format!("{i:07x}");
    for i in 0..64 {
        let socket = [A, B, C][i % 3];
        f.reg(PR1, sha_ask(&sha(i)), socket).await.unwrap();
    }
    assert_eq!(f.held(), 64);
    assert_eq!(
        f.reg(PR1, sha_ask(&sha(99)), D).await,
        Err(RegisterError::Limit)
    );
    assert_eq!(f.posts(), 64);
    let r = f.reg(PR1, sha_ask(&sha(0)), A).await.unwrap();
    assert!(r.replaced);
    assert_eq!(f.held(), 64);

    let mut f = fix().await;
    f.track(&running(PR1));
    for i in 0..32 {
        f.reg(PR1, sha_ask(&sha(i)), A).await.unwrap();
    }
    assert_eq!(
        f.reg(PR1, sha_ask(&sha(32)), A).await,
        Err(RegisterError::Limit)
    );
    assert_eq!(f.posts(), 32);
    assert!(f.reg(PR1, sha_ask(&sha(32)), B).await.is_ok());
    assert!(f.reg(PR1, sha_ask(&sha(0)), A).await.unwrap().replaced);
    assert_eq!(f.held(), 33);
}

// NTF-2: `drop_same` ends the same one as `Replaced`, posting nothing.
#[tokio::test]
async fn ntf2_drop_same() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, sha_ask("1a2b3c4"), A).await.unwrap();
    assert!(f.n.drop_same(&id(PR1), &path(A), &sha_ask("1A2B3C4")));
    assert_eq!(f.held(), 0);
    assert_eq!(f.posts(), 1);
    assert_eq!(f.log_count("end=replaced"), 1);
    assert!(!f.n.drop_same(&id(PR1), &path(A), &sha_ask("1a2b3c4")));
}

// NTF-3: that socket's subscriptions on that PR; nothing posted.
// NTF-1: held by node ID, a renamed PR's subscription is found by its old name.
#[tokio::test]
async fn ntf3_ntf1_cancel() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, sha_ask("1a2b3c4"), A).await.unwrap();
    f.reg(PR1, sha_ask("5a6b7c8"), A).await.unwrap();
    f.reg(PR1, sha_ask("1a2b3c4"), B).await.unwrap();
    assert_eq!(f.n.cancel(&name(PR1), Some(&id(PR1)), &path(A)), 2);
    assert_eq!(f.held(), 1);
    assert_eq!(f.n.cancel(&name(PR1), Some(&id(PR1)), &path(A)), 0);
    assert_eq!(f.posts(), 3);
    assert_eq!(f.log_count("end=cancelled"), 2);

    // After a rename, the old name and no node ID still find it.
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    let mut renamed = running(PR1);
    renamed.pr = name("o/new#1");
    f.put(&renamed);
    f.clock.settle().await;
    assert_eq!(
        f.store.view().get(&id(PR1)).unwrap().record.pr,
        name("o/new#1")
    );
    assert_eq!(f.n.cancel(&name("O/R#1"), None, &path(A)), 1);
    assert_eq!(f.held(), 1);
    assert_eq!(f.posts(), 4);
}

// NTF-4: evaluated after every applied refresh, with the waiters' own `evaluate`.
#[tokio::test]
async fn ntf4_evaluated_after_every_applied_refresh() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.track(&running(PR2));
    let cursor = f.store.view().cursor();
    f.reg(PR1, ask(Until::ChecksDone), A).await.unwrap();
    f.put(&finished(PR2, "success"));
    f.clock.settle().await;
    f.put(&running(PR1));
    f.clock.settle().await;
    assert_eq!(f.posts(), 1);
    assert_eq!(f.held(), 1);

    f.put(&finished(PR1, "success"));
    let view = f.store.view();
    f.clock.settle().await;
    assert_eq!(f.posts(), 2);
    assert_eq!(f.held(), 0);
    // What a wait with the same parameters gives on the same view.
    let cond = Condition::check(ask(Until::ChecksDone)).unwrap();
    let mut progress = Progress::new(Mono::ZERO);
    let end = evaluate(&cond, &view, &id(PR1), &mut progress).unwrap();
    let expected = text::outcome(
        &f.told(PR1, ask(Until::ChecksDone), cursor),
        end,
        Some(&view.get(&id(PR1)).unwrap().record),
        true,
    );
    assert_eq!(f.last(), expected);
    assert!(
        expected.contains("checks passed (1/1) on 0123456"),
        "{expected}"
    );
    assert_eq!(f.log_count("subscription ended pr=o/r#1 end=satisfied"), 1);
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 2);
}

// NTF-4: a close, and `for` passing at the first tick at or after it.
#[tokio::test]
async fn ntf4_ends() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.put(&closed(PR1));
    f.clock.settle().await;
    assert!(
        f.last()
            .starts_with("circitor: o/r#1 ended: closed. Details:"),
        "{}",
        f.last()
    );
    assert_eq!(f.held(), 0);
    assert_eq!(f.log_count("end=ended"), 1);

    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg_for(PR1, ask(Until::Merged), A, MIN).await.unwrap();
    f.clock.advance(50 * SEC).await;
    assert_eq!(f.posts(), 1);
    f.clock.advance(5 * SEC).await;
    // An applied refresh between two ticks isn't a tick.
    f.put(&running(PR1));
    f.clock.settle().await;
    assert_eq!(f.posts(), 1);
    f.clock.advance(5 * SEC).await;
    assert_eq!(f.posts(), 2);
    let post = f.poster.posts().pop().unwrap();
    assert_eq!(post.at, t0().plus(MIN));
    assert!(
        post.text
            .starts_with("circitor: notify for o/r#1 expired after 1m; it's checks_pending. Ask again: circitor notify o/r#1 --until merged --for 60 --since "),
        "{}",
        post.text
    );
    assert_eq!(f.held(), 0);
    assert_eq!(f.log_count("end=expired"), 1);
}

// NTF-4, TST-8 `clock-back` and `sleep`: `for` never reads the wall clock, and never beats a catch-up.
#[tokio::test]
async fn ntf4_for_and_the_clocks() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg_for(PR1, ask(Until::Merged), A, MIN).await.unwrap();
    f.clock.set_wall_back(HOUR).await;
    assert_eq!(f.sleep.check(), None);
    f.clock.advance(50 * SEC).await;
    assert_eq!(f.posts(), 1);
    // The wall clock back again, alone: the notifier never reads it for `for`.
    f.clock.sleep_wall_only(HOUR).await;
    f.clock.advance(9 * SEC).await;
    assert_eq!(f.posts(), 1);
    f.clock.advance(SEC).await;
    assert_eq!(f.posts(), 2);
    assert!(f.last().contains("expired after 1m"));

    // `for` of an hour; a 2 h sleep at 10 minutes; the PR's next refresh shows it merged.
    for refresh in [true, false] {
        let mut f = fix().await;
        f.track(&running(PR1));
        f.reg_for(PR1, ask(Until::Merged), A, HOUR).await.unwrap();
        f.clock.advance(10 * MIN).await;
        f.clock.sleep_wall_only(2 * HOUR).await;
        assert_eq!(f.sleep.check(), Some(2 * HOUR));
        f.store.mark_slept();
        f.clock.settle().await;
        assert_eq!(f.posts(), 1, "nothing at the wake");
        f.clock.advance(20 * SEC).await;
        if refresh {
            f.put(&merged(PR1));
            f.clock.settle().await;
            assert_eq!(f.posts(), 2);
            assert!(
                f.last().starts_with("circitor: o/r#1 merged."),
                "{}",
                f.last()
            );
            continue;
        }
        f.clock.advance(30 * SEC).await;
        assert_eq!(f.posts(), 1);
        f.clock.advance(10 * SEC).await;
        assert_eq!(f.posts(), 2);
        let post = f.poster.posts().pop().unwrap();
        assert!(
            post.text
                .contains("expired after 1h; it's checks_pending (stale, "),
            "{}",
            post.text
        );
        assert_eq!(post.at, t0().plus(10 * MIN + 2 * HOUR + MIN));
    }
}

// NTF-4: `for` passing while the record is stale waits for the PR's next applied refresh, or 60 s.
#[tokio::test]
async fn ntf4_expiry_waits_on_a_stale_record() {
    for refresh in [true, false] {
        let mut f = fix().await;
        f.track(&running(PR1));
        f.reg_for(PR1, ask(Until::Merged), A, MIN).await.unwrap();
        f.clock.advance(30 * SEC).await;
        f.store.attempt_failed(&id(PR1), None, f.clock.wall());
        f.clock.advance(30 * SEC).await;
        assert_eq!(f.posts(), 1, "held at 60 s");
        f.clock.advance(30 * SEC).await;
        assert_eq!(f.posts(), 1);
        if refresh {
            f.put(&running(PR1));
            f.clock.settle().await;
            assert_eq!(f.posts(), 2);
            let text = f.last();
            assert!(
                text.contains("expired after 1m; it's checks_pending. Ask again"),
                "{text}"
            );
            assert!(!text.contains("stale"));
        } else {
            f.clock.advance(20 * SEC).await;
            assert_eq!(f.posts(), 1);
            f.clock.advance(10 * SEC).await;
            assert_eq!(f.posts(), 2);
            let text = f.last();
            assert!(
                text.contains("it's checks_pending (stale, 2m old)"),
                "{text}"
            );
        }
        assert_eq!(f.held(), 0);
    }
}

// NTF-4, POL-26: `problem` at a tick: `auth` for 10 minutes from `problem_since`, or `pr_failing`.
#[tokio::test]
async fn ntf4_problem() {
    // `auth` for 9 minutes 50 s and then not: nothing.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.set_problem(Some(Mono::ZERO));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.clock.advance(9 * MIN + 50 * SEC).await;
    f.set_problem(None);
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 1);
    assert_eq!(f.held(), 1);

    // Past `problem_since` + 10 minutes: at the first tick at or after it, for every subscription.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.track(&running(PR2));
    let since = Mono::ZERO.plus(5 * SEC);
    f.set_problem(Some(since));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.reg(PR2, ask(Until::Merged), B).await.unwrap();
    f.clock.advance(5 * MIN).await;
    // A failed try with a new token: `auth` to `auth`, the same `problem_since`.
    f.set_problem(Some(since));
    f.clock.advance(5 * MIN + 5 * SEC).await;
    // A wake that isn't a tick, past the 10 minutes.
    f.put(&running(PR1));
    f.clock.settle().await;
    assert_eq!(f.posts(), 2);
    f.clock.advance(5 * SEC).await;
    assert_eq!(f.posts(), 4);
    let posts = f.poster.posts();
    assert_eq!(posts[2].at, t0().plus(10 * MIN + 10 * SEC));
    for p in &posts[2..] {
        assert!(
            p.text.contains("from GitHub (auth); notify is cancelled"),
            "{}",
            p.text
        );
    }
    assert_eq!(f.log_count("end=problem why=auth"), 2);

    // `problem_since` 5 minutes before the registration: the notice 5 minutes after it.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.set_problem(Some(Mono::ZERO));
    f.clock.advance(5 * MIN).await;
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.clock.advance(5 * MIN - 10 * SEC).await;
    assert_eq!(f.posts(), 1);
    f.clock.advance(10 * SEC).await;
    assert_eq!(f.posts(), 2);
    assert_eq!(f.poster.posts()[1].at, t0().plus(10 * MIN));

    // `pr_failing` for its PR's node ID: at the next tick; one on another PR stays.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.track(&running(PR2));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.reg(PR2, ask(Until::Merged), A).await.unwrap();
    f.clock.advance(3 * SEC).await;
    f.warnings
        .raise(WarningCode::PrFailing, &id(PR1).0, "o/r#1".into());
    f.clock.settle().await;
    assert_eq!(f.posts(), 2);
    f.clock.advance(7 * SEC).await;
    assert_eq!(f.posts(), 3);
    assert!(
        f.last()
            .contains("can't read o/r#1 from GitHub (pr_failing)"),
        "{}",
        f.last()
    );
    assert_eq!(f.held(), 1);
    // One registered while it's raised posts at the first tick.
    f.reg(PR1, ask(Until::Merged), B).await.unwrap();
    f.clock.advance(10 * SEC).await;
    assert_eq!(f.posts(), 5);
    assert!(f.last().contains("(pr_failing)"));
    assert_eq!(f.held(), 1);
}

// NTF-4: every socket is looked at each tick, a `stat`, never a connection.
#[tokio::test]
async fn ntf4_socket_checked_at_each_tick() {
    for swap in [false, true] {
        let mut f = fix().await;
        f.track(&running(PR1));
        f.reg(PR1, ask(Until::Merged), A).await.unwrap();
        if swap {
            f.poster.swapped(&path(A));
        } else {
            f.poster.gone(&path(A));
        }
        f.clock.settle().await;
        assert_eq!(f.held(), 1);
        f.clock.advance(10 * SEC).await;
        assert_eq!(f.held(), 0);
        assert_eq!(f.posts(), 1);
        assert_eq!(f.log_count("end=session_gone"), 1);
        // One line for its end, beside its registration; no notice was tried.
        assert_eq!(f.log_count("subscription "), 2);
        assert_eq!(f.log_count("notice not posted"), 0);
    }

    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.poster.stat_fails(&path(A));
    f.clock.advance(100 * SEC).await;
    assert_eq!(f.held(), 1);
    assert_eq!(f.posts(), 1);
    assert_eq!(f.log_count("socket not checked pr=o/r#1"), 1);
}

// NTF-8, STA-9: a cursor too old as the notice is built names `show`.
#[tokio::test]
async fn ntf4_cursor_too_old_names_show() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.track(&running(PR2));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    for i in 0..=crate::state::MAX_EVENTS {
        let mut r = running(PR2);
        r.draft = i % 2 == 0;
        f.put(&r);
    }
    f.put(&merged(PR1));
    f.clock.settle().await;
    assert_eq!(
        f.last(),
        "circitor: o/r#1 merged. History too old; details: circitor show o/r#1"
    );
}

// NTF-6: a notice not posted ends its subscription at once, logged, and is never tried again.
#[tokio::test]
async fn ntf6_not_posted() {
    for (answer, reason) in [
        (Posted::SessionGone("refused".into()), "session_gone"),
        (Posted::Failed("EAGAIN".into()), "failed"),
    ] {
        let mut f = fix().await;
        f.track(&running(PR1));
        f.reg(PR1, ask(Until::Merged), A).await.unwrap();
        f.poster.answer_next(answer);
        f.put(&merged(PR1));
        f.clock.settle().await;
        assert_eq!(f.posts(), 2);
        assert_eq!(f.held(), 0);
        assert_eq!(
            f.log_count(&format!("notice not posted pr=o/r#1 reason={reason}")),
            1
        );
        f.clock.advance(10 * MIN).await;
        assert_eq!(f.posts(), 2);
    }

    // Stalled past 2 s; a second subscription's notice goes out while the first is stalled.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.track(&running(PR2));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.reg(PR2, ask(Until::Merged), B).await.unwrap();
    let release = f.poster.stall(&f.clock.activity());
    f.put(&merged(PR1));
    f.clock.settle().await;
    assert_eq!(f.posts(), 3);
    assert_eq!(f.held(), 2);
    f.clock.advance(SEC).await;
    f.put(&merged(PR2));
    f.clock.settle().await;
    assert_eq!(f.posts(), 4, "asked for while the first is stalled");
    f.clock.advance(SEC).await;
    assert_eq!(f.held(), 1);
    assert_eq!(f.log_count("notice not posted pr=o/r#1 reason=timeout"), 1);
    f.clock.advance(SEC).await;
    assert_eq!(f.held(), 0);
    drop(release);
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 4);

    // The path checked before connecting: another socket there means no post at all.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.poster.swapped(&path(A));
    f.put(&merged(PR1));
    f.clock.settle().await;
    assert_eq!(f.posts(), 1);
    assert_eq!(f.held(), 0);
    assert_eq!(
        f.log_count("notice not posted pr=o/r#1 reason=session_gone"),
        1
    );
}

// NTF-6: once written, a notice is never sent again.
#[tokio::test]
async fn ntf6_done_is_final() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.put(&merged(PR1));
    f.clock.settle().await;
    assert_eq!(f.posts(), 2);
    f.put(&merged(PR1));
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 2);
    assert_eq!(f.held(), 0);
    assert_eq!(f.log_count("notice not posted"), 0);
}

// NTF-10: a stop in order tells every session, all at once, within 1 s for all.
#[tokio::test]
async fn ntf10_stop() {
    // Three open: `stopped` to each, with `--no-spawn`.
    let mut f = fix().await;
    f.track(&running(PR1));
    for s in [A, B, C] {
        f.reg(PR1, ask(Until::Merged), s).await.unwrap();
    }
    f.n.stop(true).await;
    let texts = f.texts();
    assert_eq!(texts.len(), 6);
    for t in &texts[3..] {
        assert!(t.starts_with("circitor: the daemon is stopping; notify for o/r#1 is cancelled."));
        assert!(t.ends_with(" --no-spawn"), "{t}");
    }
    assert_eq!(f.n.count(), 0);
    assert_eq!(f.store.subscriptions(), 0);
    assert_eq!(f.log_count("reason=stop"), 0);
    assert_eq!(
        f.n.register(
            f.new_sub(PR1, ask(Until::Merged), A, DEFAULT_LIFE, None)
                .await
        )
        .await,
        Err(RegisterError::Stopping)
    );

    // `stop(false)`: without `--no-spawn`.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.n.stop(false).await;
    assert!(
        f.last()
            .ends_with(&format!("--since {}", f.store.view().cursor())),
        "{}",
        f.last()
    );

    // Stalled: three open and a fourth whose outcome post is under way.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.track(&running(PR2));
    for s in [A, B, C] {
        f.reg(PR1, ask(Until::Merged), s).await.unwrap();
    }
    f.reg(PR2, ask(Until::Merged), D).await.unwrap();
    let release = f.poster.stall(&f.clock.activity());
    f.put(&merged(PR2));
    f.clock.settle().await;
    assert_eq!(f.posts(), 5);
    let n = f.n.clone();
    let h = tokio::spawn(async move { n.stop(true).await });
    f.clock.settle().await;
    assert_eq!(f.n.count(), 0);
    let late = f
        .new_sub(PR1, ask(Until::Merged), B, DEFAULT_LIFE, None)
        .await;
    assert_eq!(f.n.register(late).await, Err(RegisterError::Stopping));
    let posts = f.poster.posts();
    assert_eq!(posts.len(), 8);
    assert!(
        posts.iter().filter(|p| p.socket == path(D)).count() == 2,
        "no `stopped` to D"
    );
    f.clock.advance(STOP_LIMIT - Duration::from_millis(1)).await;
    assert!(!h.is_finished());
    f.clock.advance(Duration::from_millis(1)).await;
    h.await.unwrap();
    assert_eq!(f.log_count("notice not posted reason=stop count=4"), 1);
    drop(release);
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 8);
    assert_eq!(f.store.subscriptions(), 0);

    // Dropping the future half-way logs it too.
    let mut f = fix().await;
    f.track(&running(PR1));
    f.reg(PR1, ask(Until::Merged), A).await.unwrap();
    f.reg(PR1, ask(Until::Merged), B).await.unwrap();
    let release = f.poster.stall(&f.clock.activity());
    let n = f.n.clone();
    let h = tokio::spawn(async move { n.stop(true).await });
    f.clock.settle().await;
    h.abort();
    let _ = h.await;
    assert_eq!(f.log_count("notice not posted reason=stop count=2"), 1);
    drop(release);
}

// NTF-10: a crash posts nothing and logs nothing.
#[tokio::test]
async fn ntf10_abort_posts_nothing() {
    let mut f = fix().await;
    f.track(&running(PR1));
    for s in [A, B, C] {
        f.reg(PR1, ask(Until::Merged), s).await.unwrap();
    }
    let lines = f.log().lines().count();
    f.n.abort();
    f.clock.settle().await;
    assert_eq!(f.n.count(), 0);
    assert_eq!(f.store.subscriptions(), 0);
    f.put(&merged(PR1));
    f.clock.advance(10 * MIN).await;
    assert_eq!(f.posts(), 3);
    assert!(
        !f.log()
            .lines()
            .skip(lines)
            .any(|l| l.contains("subscription") || l.contains("notice"))
    );
}

// NTF-9, OPS-5, OPS-9: the session's token reaches the poster and nothing else; the log holds no path.
#[tokio::test]
async fn ntf9_token_is_nowhere() {
    let mut f = fix().await;
    f.track(&running(PR1));
    let new = f
        .new_sub(
            PR1,
            ask(Until::Merged),
            A,
            DEFAULT_LIFE,
            Some(MARKER_SESSION_TOKEN),
        )
        .await;
    let mut shown = format!("{new:?} {new:#?}");
    let r = f.n.register(new).await;
    shown.push_str(&format!("{r:?}"));
    f.clock.settle().await;
    f.poster.answer_next(Posted::Failed("EAGAIN".into()));
    let again = f
        .new_sub(
            PR1,
            ask(Until::Changed),
            A,
            DEFAULT_LIFE,
            Some(MARKER_SESSION_TOKEN),
        )
        .await;
    let err = f.n.register(again).await.unwrap_err();
    shown.push_str(&format!("{err:?}"));
    if let RegisterError::NoSession(m) = &err {
        shown.push_str(m);
    }
    f.put(&merged(PR1));
    f.clock.settle().await;
    let posts = f.poster.posts();
    assert_eq!(posts.len(), 3);
    for p in &posts {
        assert_eq!(p.token.as_deref(), Some(MARKER_SESSION_TOKEN));
        assert!(!p.text.contains(MARKER_SESSION_TOKEN));
    }
    let told = f.told(PR1, ask(Until::Merged), f.store.view().cursor());
    shown.push_str(&format!(
        "{told:?} {:?} {:?} {:?} {:?}",
        Posted::Done,
        Looked::Missing,
        End::Problem(ProblemWhy::Auth),
        SessionToken::new(MARKER_SESSION_TOKEN.into())
    ));
    assert!(!shown.contains(MARKER_SESSION_TOKEN), "{shown}");
    assert!(!f.log().contains(MARKER_SESSION_TOKEN));
    assert!(!f.log().contains(A), "never a socket's path");
}

// NTF-8: no notice built for a record holds its title, its author or a check's name.
#[tokio::test]
async fn ntf8_no_github_text() {
    let mut f = fix().await;
    f.track(&running(PR1));
    f.track(&running(PR2));
    f.reg(PR1, ask(Until::ChecksDone), A).await.unwrap();
    f.reg_for(PR2, ask(Until::Merged), A, MIN).await.unwrap();
    f.put(&finished(PR1, "failure"));
    f.clock.advance(MIN).await;
    f.reg(PR2, ask(Until::Changed), B).await.unwrap();
    f.n.stop(true).await;
    let texts = f.texts();
    assert_eq!(texts.len(), 6);
    assert!(texts.iter().any(|t| t.contains("checks failed (1 of 1)")));
    for t in &texts {
        for m in [MARKER_TITLE, MARKER_LOGIN, MARKER_CHECK] {
            assert!(!t.contains(m), "{t}");
        }
    }
    for m in [MARKER_TITLE, MARKER_CHECK] {
        assert!(!f.log().contains(m));
    }
}
