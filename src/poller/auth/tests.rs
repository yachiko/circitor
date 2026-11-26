use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::clock::{TestClock, Time};
use crate::log::{BufferSink, Level};
use crate::testkit::{MARKER_TOKEN, MARKER_TOKEN_2};
use crate::token::fake::FakeTokenSource;

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn at(n: u64) -> Mono {
    Mono::ZERO.plus(s(n))
}

fn tok(v: &str) -> Token {
    Token::new(v.to_string())
}

struct Rig {
    clock: Arc<TestClock>,
    fake: Arc<FakeTokenSource>,
    sink: BufferSink,
    auth: Auth,
}

fn rig() -> Rig {
    let clock = TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap());
    let fake = FakeTokenSource::new(clock.clone());
    let sink = BufferSink::default();
    let logger = Logger::new(clock.clone(), Arc::new(sink.clone()), Level::Debug);
    let auth = Auth::new(fake.clone(), clock.clone(), logger);
    Rig {
        clock,
        fake,
        sink,
        auth,
    }
}

fn lines(rig: &Rig, word: &str) -> usize {
    rig.sink
        .contents()
        .lines()
        .filter(|l| l.contains(word))
        .count()
}

fn is_token(r: Result<Token, AuthProblem>, v: &str) -> bool {
    r.is_ok_and(|t| t.expose() == v)
}

/// OPS-5: no token in the log or in any message.
fn sweep(rig: &Rig) {
    let log = rig.sink.contents();
    assert!(!log.contains(MARKER_TOKEN));
    assert!(!log.contains(MARKER_TOKEN_2));
    if let Some(p) = rig.auth.problem() {
        let d = format!("{p:?}");
        assert!(!d.contains(MARKER_TOKEN) && !d.contains(MARKER_TOKEN_2));
    }
}

// OPS-5: the first read is waited for.
#[tokio::test]
async fn ops5_first_read_is_waited_for() {
    let mut r = rig();
    r.fake.set_delay(s(8));
    assert!(r.auth.problem().is_none());
    assert!(!r.auth.first_read_done());
    assert!(r.auth.last_read_ended().is_none());
    let mut auth = r.auth;
    let handle = tokio::spawn(async move {
        let t = auth.token(false).await;
        (auth, t)
    });
    r.clock.advance(s(7)).await;
    assert!(!handle.is_finished());
    r.clock.advance(s(1)).await;
    let (mut auth, t) = handle.await.unwrap();
    assert!(is_token(t, MARKER_TOKEN));
    assert_eq!(r.fake.reads(), 1);
    assert!(auth.first_read_done());
    assert_eq!(auth.last_read_ended(), Some(at(8)));
    assert!(auth.token(false).await.is_ok());
    assert!(auth.token(true).await.is_ok());
    assert_eq!(r.fake.reads(), 1);
    r.auth = auth;
    sweep(&r);
}

// POL-20, OPS-5: no login at start.
#[tokio::test]
async fn pol20_no_login_at_start() {
    let mut r = rig();
    r.fake.set(TokenRead::NoLogin);
    let e = r.auth.token(false).await.unwrap_err();
    assert!(e.message.contains("gh auth login"));
    assert!(r.auth.first_read_done());
    assert_eq!(r.auth.problem(), Some(e));
    assert_eq!(lines(&r, "problem begin code=auth"), 1);
}

// OPS-5: a failure names the command, the status and the path, never a login.
#[tokio::test]
async fn ops5_failure_message() {
    let mut r = rig();
    r.fake.set_gh_path("/opt/bin/gh");
    r.fake.set(TokenRead::Failed(GhFailure::Exit(3)));
    let e = r.auth.token(false).await.unwrap_err();
    assert!(e.message.contains("gh auth token"));
    assert!(e.message.contains('3'));
    assert!(e.message.contains("/opt/bin/gh"));
    assert!(!e.message.contains("gh auth login"));
    assert!(!e.message.contains("login"));
    assert!(r.auth.first_read_done());

    let mut r = rig();
    r.fake.set(TokenRead::Failed(GhFailure::Timeout));
    let e = r.auth.token(false).await.unwrap_err();
    assert!(e.message.contains("timed out"));
    assert!(e.message.contains("10 s"));
    assert!(!e.message.contains("login"));
}

// POL-3: the periodic read.
#[tokio::test]
async fn pol3_periodic() {
    let mut r = rig();
    assert_eq!(r.auth.due(), Mono::ZERO);
    assert!(r.auth.token(false).await.is_ok());
    assert_eq!(r.auth.due(), at(600));

    r.clock.advance(s(600)).await;
    r.fake.set(TokenRead::Failed(GhFailure::Exit(1)));
    assert!(!r.auth.periodic().await);
    assert!(is_token(r.auth.token(false).await, MARKER_TOKEN));
    assert!(r.auth.problem().is_none());
    r.clock.advance(s(600)).await;
    assert!(!r.auth.periodic().await);
    assert_eq!(lines(&r, "token read failed"), 1);
    assert!(is_token(r.auth.token(false).await, MARKER_TOKEN));

    // Another token replaces it, silently.
    r.clock.advance(s(600)).await;
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN_2)));
    assert!(!r.auth.periodic().await);
    assert!(is_token(r.auth.token(false).await, MARKER_TOKEN_2));
    assert_eq!(lines(&r, "problem begin"), 0);

    // A later failure is a new run.
    r.clock.advance(s(600)).await;
    r.fake.set(TokenRead::Failed(GhFailure::Timeout));
    r.auth.periodic().await;
    assert_eq!(lines(&r, "token read failed"), 2);

    // No login drops it.
    r.clock.advance(s(600)).await;
    r.fake.set(TokenRead::NoLogin);
    r.auth.periodic().await;
    assert!(r.auth.problem().is_some());
    assert!(r.auth.token(false).await.is_err());
    assert_eq!(lines(&r, "problem begin code=auth"), 1);
    sweep(&r);
}

// POL-20: one repeat.
#[tokio::test]
async fn pol20_one_repeat() {
    let mut r = rig();
    let t = r.auth.token(false).await.unwrap();
    assert_eq!(r.fake.reads(), 1);
    r.clock.advance(s(1)).await;
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN_2)));
    match r.auth.unauthorized(&t, false).await {
        AfterRefusal::Retry(n) => assert_eq!(n.expose(), MARKER_TOKEN_2),
        AfterRefusal::InAuth(_) => panic!("expected a retry"),
    }
    assert_eq!(r.fake.reads(), 2);
    assert!(r.auth.problem().is_none());
    // The same token comes back as the repeat.
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN_2)));
    match r.auth.unauthorized(&t, false).await {
        AfterRefusal::Retry(n) => assert!(n.same_as(&tok(MARKER_TOKEN_2))),
        AfterRefusal::InAuth(_) => panic!("expected a retry"),
    }
    assert_eq!(r.fake.reads(), 3);

    let n = tok(MARKER_TOKEN_2);
    match r.auth.unauthorized(&n, true).await {
        AfterRefusal::InAuth(p) => assert!(p.message.contains("gh auth login")),
        AfterRefusal::Retry(_) => panic!("expected auth"),
    }
    assert_eq!(r.fake.reads(), 3);
    assert!(r.auth.problem().is_some());
    assert!(r.auth.token(false).await.is_err());
    assert_eq!(lines(&r, "problem begin code=auth"), 1);
    sweep(&r);
}

// POL-20: a failed re-read repeats with the token in hand.
#[tokio::test]
async fn pol20_failed_reread_repeats_with_the_kept_token() {
    let mut r = rig();
    let t = r.auth.token(false).await.unwrap();
    r.fake.set(TokenRead::Failed(GhFailure::Exit(2)));
    match r.auth.unauthorized(&t, false).await {
        AfterRefusal::Retry(n) => assert!(n.same_as(&t)),
        AfterRefusal::InAuth(_) => panic!("expected a retry"),
    }
    assert!(r.auth.problem().is_none());
    assert_eq!(lines(&r, "token read failed"), 1);

    // No login on the re-read is `auth`.
    r.fake.set(TokenRead::NoLogin);
    assert!(matches!(
        r.auth.unauthorized(&t, false).await,
        AfterRefusal::InAuth(_)
    ));
    assert!(r.auth.problem().is_some());
    sweep(&r);
}

// POL-20: in `auth` the token is read every 60 s, and on demand after 5 s.
#[tokio::test]
async fn pol20_reads_in_auth() {
    let mut r = rig();
    let t = r.auth.token(false).await.unwrap();
    r.fake.set(TokenRead::NoLogin);
    r.auth.unauthorized(&t, true).await;
    assert_eq!(r.fake.reads(), 1);
    assert_eq!(r.auth.due(), at(60));
    r.clock.advance(s(10)).await;
    assert!(r.auth.token(true).await.is_err());
    assert_eq!(r.fake.reads(), 2);
    assert_eq!(r.auth.due(), at(70));
    r.clock.advance(s(2)).await;
    assert!(r.auth.token(true).await.is_err());
    r.clock.advance(s(2)).await;
    assert!(r.auth.token(true).await.is_err());
    assert_eq!(r.fake.reads(), 2);
    assert!(r.auth.token(false).await.is_err());
    assert_eq!(r.fake.reads(), 2);
    r.clock.advance(s(1)).await;
    assert!(r.auth.token(true).await.is_err());
    assert_eq!(r.fake.reads(), 3);
    sweep(&r);
}

// POL-20: another token gets one try.
#[tokio::test]
async fn pol20_another_token_gets_one_try() {
    let mut r = rig();
    let t = r.auth.token(false).await.unwrap();
    r.auth.unauthorized(&t, true).await;
    r.clock.advance(s(60)).await;
    assert!(!r.auth.periodic().await);
    assert!(r.auth.token(false).await.is_err());

    r.clock.advance(s(60)).await;
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN_2)));
    assert!(r.auth.periodic().await);
    let n = r.auth.token(false).await.unwrap();
    assert_eq!(n.expose(), MARKER_TOKEN_2);
    assert!(r.auth.problem().is_some());
    let reads = r.fake.reads();
    assert!(matches!(
        r.auth.unauthorized(&n, false).await,
        AfterRefusal::InAuth(_)
    ));
    assert_eq!(r.fake.reads(), reads);
    assert!(r.auth.token(false).await.is_err());

    // Another try, accepted this time.
    r.clock.advance(s(60)).await;
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN)));
    // The first token was refused earlier, and the second just now: this one is another than the second.
    assert!(r.auth.periodic().await);
    let n = r.auth.token(false).await.unwrap();
    r.auth.accepted(&n);
    assert!(r.auth.problem().is_none());
    assert_eq!(lines(&r, "problem end code=auth"), 1);
    assert_eq!(lines(&r, "problem begin code=auth"), 1);
    assert_eq!(r.auth.due(), r.auth.last_read_ended().unwrap().plus(s(600)));
    sweep(&r);
}

// POL-20: the refused token is tried again every 10 minutes.
#[tokio::test]
async fn pol20_same_token_every_10_minutes() {
    let mut r = rig();
    let t = r.auth.token(false).await.unwrap();
    r.auth.unauthorized(&t, true).await;
    for _ in 0..9 {
        r.clock.advance(s(60)).await;
        assert!(!r.auth.periodic().await);
        assert!(r.auth.token(false).await.is_err());
    }
    r.clock.advance(s(60)).await;
    assert!(r.auth.periodic().await);
    assert!(is_token(r.auth.token(false).await, MARKER_TOKEN));
    sweep(&r);
}

// POL-20: the end of the last read.
#[tokio::test]
async fn pol20_last_read_ended() {
    let mut r = rig();
    r.fake.set_delay(s(8));
    assert_eq!(r.auth.last_read_ended(), None);
    let mut auth = r.auth;
    let h = tokio::spawn(async move {
        let t = auth.token(false).await;
        (auth, t)
    });
    r.clock.advance(s(7)).await;
    assert!(!h.is_finished());
    r.clock.advance(s(1)).await;
    let (mut auth, t) = h.await.unwrap();
    let t = t.unwrap();
    assert_eq!(auth.last_read_ended(), Some(at(8)));
    r.fake.set_delay(s(0));

    r.clock.advance(s(100)).await;
    auth.token(false).await.unwrap();
    assert_eq!(auth.last_read_ended(), Some(at(8)));
    auth.periodic().await;
    assert_eq!(auth.last_read_ended(), Some(at(108)));

    r.clock.advance(s(10)).await;
    auth.unauthorized(&t, false).await;
    assert_eq!(auth.last_read_ended(), Some(at(118)));

    r.clock.advance(s(10)).await;
    auth.unauthorized(&t, true).await;
    assert_eq!(auth.last_read_ended(), Some(at(118)));

    r.clock.advance(s(10)).await;
    let _ = auth.token(true).await;
    assert_eq!(auth.last_read_ended(), Some(at(138)));
    r.clock.advance(s(1)).await;
    let _ = auth.token(true).await;
    assert_eq!(auth.last_read_ended(), Some(at(138)));
    r.auth = auth;
    sweep(&r);
}

// POL-20: a login after no login opens a try at once.
#[tokio::test]
async fn pol20_login_after_no_login() {
    let mut r = rig();
    r.fake.set(TokenRead::NoLogin);
    assert!(r.auth.token(false).await.is_err());
    r.clock.advance(s(5)).await;
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN)));
    assert!(is_token(r.auth.token(true).await, MARKER_TOKEN));
    assert!(r.auth.problem().is_some());
    let t = r.auth.token(false).await.unwrap();
    r.auth.accepted(&t);
    assert!(r.auth.problem().is_none());
    sweep(&r);
}

// API-3: the message by source.
#[tokio::test]
async fn api3_message_per_source() {
    for (kind, word) in [
        (TokenSourceKind::GhTokenVar, "GH_TOKEN"),
        (TokenSourceKind::GithubTokenVar, "GITHUB_TOKEN"),
    ] {
        let mut r = rig();
        r.fake.set_kind(kind);
        let t = r.auth.token(false).await.unwrap();
        let AfterRefusal::InAuth(p) = r.auth.unauthorized(&t, true).await else {
            panic!("expected auth");
        };
        assert!(p.message.contains(word));
        assert!(p.message.contains("restart"));
        assert!(!p.message.contains("gh auth login"));
    }
    let mut r = rig();
    let t = r.auth.token(false).await.unwrap();
    let AfterRefusal::InAuth(p) = r.auth.unauthorized(&t, true).await else {
        panic!("expected auth");
    };
    assert!(p.message.contains("gh auth login"));
    assert!(p.message.contains("refused"));
}

// OPS-5: the token is nowhere in the log or in a problem, whatever the path.
#[tokio::test]
async fn ops5_no_token_anywhere() {
    let mut r = rig();
    let t = r.auth.token(false).await.unwrap();
    r.fake.set(TokenRead::Failed(GhFailure::Exit(1)));
    r.clock.advance(s(600)).await;
    r.auth.periodic().await;
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN_2)));
    r.auth.unauthorized(&t, false).await;
    r.auth.unauthorized(&tok(MARKER_TOKEN_2), true).await;
    sweep(&r);
    r.clock.advance(s(60)).await;
    r.fake.set(TokenRead::Token(tok(MARKER_TOKEN)));
    r.auth.periodic().await;
    let n = r.auth.token(false).await.unwrap();
    sweep(&r);
    r.auth.accepted(&n);
    r.fake.set(TokenRead::NoLogin);
    r.auth.periodic().await;
    sweep(&r);
    assert!(!format!("{:?}", r.auth.problem()).contains("gho_"));
    assert!(!r.sink.contents().contains("gho_"));
}
