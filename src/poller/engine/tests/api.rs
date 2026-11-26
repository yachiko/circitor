//! What the API sees: API-4, API-16, API-18, OPS-9, and stopping.

use super::*;
use crate::poller::{FirstPoll, PollOutcome, PollReport, ProblemCode, Tier};
use crate::testkit::{MARKER_TITLE, MARKER_TOKEN};
use crate::token::TokenRead;

// API-16: a whole poll and its points; inside the window `shared`; joined to one in flight; refused in a backoff.
#[tokio::test]
async fn api16_poll_now() {
    let w = World {
        rate: Some(answers::rate(3, 4_999)),
        ..World::default()
    };
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(30).await;
    {
        let mut w = r.world();
        w.open("o/r#1", 0);
        w.open("o/r#2", 1);
    }
    let polled = r.spawn_poll_now();
    r.advance(1).await;
    assert_eq!(
        polled.await.unwrap(),
        PollReport {
            outcome: PollOutcome::Polled,
            cost: 6
        }
    );
    assert!(r.tracked("o/r#1") && r.tracked("o/r#2"));
    r.advance(5).await;
    let shared = r.spawn_poll_now();
    r.clock.settle().await;
    assert_eq!(
        shared.await.unwrap(),
        PollReport {
            outcome: PollOutcome::Shared,
            cost: 0
        }
    );

    // One arriving during a held poll answers when that poll is done, with no request of its own.
    // The next ordinary poll is 300 s after the forced one was sent.
    let release = r.github.hold(&r.clock.activity());
    r.advance(294).await;
    let held_at = r.n_calls();
    assert_eq!(r.send_times().len(), held_at - 1, "held, not yet answered");
    let joined = r.spawn_poll_now();
    r.advance(3).await;
    assert!(!joined.is_finished());
    drop(release);
    r.advance(1).await;
    assert_eq!(
        joined.await.unwrap(),
        PollReport {
            outcome: PollOutcome::Shared,
            cost: 0
        }
    );
    assert_eq!(r.n_calls(), held_at);

    // During a backoff: `github_unavailable` at once.
    r.world().fail(unavailable(503));
    r.advance(300).await;
    let before = r.n_calls();
    let refused = r.engine.poll_now().await;
    assert_eq!(
        refused,
        PollReport {
            outcome: PollOutcome::GithubUnavailable,
            cost: 0
        }
    );
    assert_eq!(r.n_calls(), before);
}

// API-18: `Running` until the first refreshes are applied, then `Finished`.
#[tokio::test]
async fn api18_first_poll() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    let first = r.engine.first_poll();
    assert_eq!(r.n_calls(), 1);
    assert_eq!(*first.borrow(), FirstPoll::Running);
    r.advance(1).await;
    assert_eq!(*first.borrow(), FirstPoll::Finished);
    assert!(r.tracked("o/r#1"));
}

// API-4: the precedence of `problem`, with `until` and `last_poll_at`.
#[tokio::test]
async fn api4_status() {
    let mut w = world_of(&["o/r#1"]);
    w.rate = Some(answers::rate(1, 400));
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let st = r.engine.status();
    let p = st.problem.unwrap();
    assert_eq!((p.code, p.until), (ProblemCode::QuotaLow, None));
    assert_eq!(st.last_poll_at, Some(t0()));
    assert_eq!(st.remaining, Some(400));
    assert_eq!(st.login.as_deref(), Some("octo"));
    // `github_unavailable` over `quota_low`.
    r.world().fail(unavailable(502));
    r.advance(300).await;
    let st = r.engine.status();
    let p = st.problem.unwrap();
    assert_eq!(
        (p.code, p.until),
        (ProblemCode::GithubUnavailable, Some(t0().plus(s(305))))
    );
    assert_eq!(st.last_poll_at, Some(t0()), "the last poll applied");
    // `rate_limited` over `github_unavailable`.
    let reset = t0().plus(s(2_000));
    r.world().fail(Failure::Exhausted {
        reset_at: Some(reset),
    });
    r.advance(5).await;
    let p = r.engine.status().problem.unwrap();
    assert_eq!((p.code, p.until), (ProblemCode::RateLimited, Some(reset)));
    // `auth` over `rate_limited`: the periodic read at 600 s finds no login.
    r.token.set(TokenRead::NoLogin);
    r.advance(300).await;
    let p = r.engine.status().problem.unwrap();
    assert_eq!((p.code, p.until), (ProblemCode::Auth, None));
    assert!(p.message.contains("gh auth login"));
    assert!(!format!("{:?}", r.engine.status()).contains(MARKER_TOKEN));
}

// API-4, OPS-5: `status()` answers while the first token read is still running.
#[tokio::test]
async fn api4_status_during_the_first_read() {
    let r = start_with(Setup::default(), |t| t.set_delay(s(8))).await;
    let st = r.engine.status();
    assert_eq!(st.tier, Tier::Idle);
    assert_eq!(st.problem, None);
    assert_eq!(st.token_source.as_str(), "gh");
    assert_eq!(*r.engine.first_poll().borrow(), FirstPoll::Running);
    r.advance(7).await;
    assert_eq!(r.n_calls(), 0);
    let _ = r.engine.status();
    r.advance(1).await;
    assert_eq!(r.n_calls(), 1);
    assert_eq!(*r.engine.first_poll().borrow(), FirstPoll::Finished);
}

// OPS-9: one `request` line per request at `debug`; `login` at the first poll and on a change; no title.
#[tokio::test]
async fn ops9_log() {
    let mut w = world_of(&["o/r#1"]);
    w.pr("o/r#1").title = MARKER_TITLE.to_string();
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(300).await;
    r.world().login = "octo2".to_string();
    r.advance(300).await;
    assert_eq!(r.n_calls(), 4);
    assert_eq!(r.lines("debug request kind="), 4);
    let log = r.sink.contents();
    let first = log
        .lines()
        .find(|l| l.contains("request kind=poll"))
        .unwrap();
    assert!(
        first.contains("cost=1") && first.contains("duration_ms="),
        "{first}"
    );
    assert_eq!(r.lines("info login login=octo"), 2);
    assert_eq!(r.lines("login login=octo2"), 1);
    assert!(!log.contains(MARKER_TITLE));
    assert!(!log.contains(MARKER_TOKEN));

    // At `info`, no request lines.
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        level: Level::Info,
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    assert_eq!(r.lines("request kind="), 0);
}

// OPS-7: `stop()` returns with a request held, and nothing is sent after.
#[tokio::test]
async fn engine_stops() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let _release = r.github.hold(&r.clock.activity());
    r.advance(299).await;
    assert_eq!(r.n_calls(), 3);
    r.engine.stop().await;
    r.advance(3_600).await;
    assert_eq!(r.n_calls(), 3);
    let _ = r.engine.status();
}
