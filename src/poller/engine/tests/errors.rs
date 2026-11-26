//! Errors and limits: POL-19, POL-20, POL-21, POL-23, POL-24, POL-25.

use super::*;
use crate::model::PrState;
use crate::poller::{FirstPoll, PollOutcome, ProblemCode, RefusalCode, Tier, cadence};
use crate::status::WarningCode;
use crate::testkit::{MARKER_TOKEN, MARKER_TOKEN_2};
use crate::token::{Token, TokenRead};

fn unanswered_code(r: &Rig) -> Option<RefusalCode> {
    match &*r.engine.first_poll().borrow() {
        FirstPoll::Unanswered(refusal) => Some(refusal.code),
        _ => None,
    }
}

fn problem(r: &Rig) -> Option<ProblemCode> {
    r.engine.status().problem.map(|p| p.code)
}

// POL-19: the poll is abandoned, the backoff doubles to 5 minutes, one line each way.
#[tokio::test]
async fn pol19_backoff() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    {
        let mut w = r.world();
        for _ in 0..9 {
            w.fail(unavailable(503));
        }
    }
    r.advance(299).await;
    assert_eq!(r.n_calls(), 3);
    assert!(r.timing("o/r#1").attempt_failed && r.timing("o/r#2").attempt_failed);
    let st = r.engine.status();
    assert_eq!(
        st.problem.as_ref().map(|p| p.code),
        Some(ProblemCode::GithubUnavailable)
    );
    assert_eq!(st.problem.unwrap().until, Some(t0().plus(s(305))));
    r.advance(1_300).await;
    let times = r.send_times();
    let gaps: Vec<u64> = times[2..]
        .windows(2)
        .map(|w| w[1].since(w[0]).as_secs())
        .collect();
    assert_eq!(gaps[..8], [5, 10, 20, 40, 80, 160, 300, 300]);
    // The try after the ninth failure succeeds; it's a poll, and the backoff is over.
    let calls = r.calls();
    let last = calls.last().unwrap();
    assert!(has_find(last) && last.nodes.len() == 2);
    assert_eq!(problem(&r), None);
    assert!(!r.timing("o/r#1").attempt_failed);
    assert_eq!(
        r.lines("problem begin code=github_unavailable status=503"),
        1
    );
    assert_eq!(r.lines("problem end code=github_unavailable"), 1);
}

// POL-19, API-18: a first poll that fails is `Unanswered` with an `until`; POL-24: so does a 418.
#[tokio::test]
async fn pol19_first_poll_fails() {
    for status in [503, 418] {
        let mut w = world_of(&["o/r#1"]);
        w.fail(unavailable(status));
        let r = start(Setup {
            world: w,
            ..Setup::default()
        })
        .await;
        match &*r.engine.first_poll().borrow() {
            FirstPoll::Unanswered(refusal) => {
                assert_eq!(refusal.code, RefusalCode::GithubUnavailable);
                assert_eq!(refusal.until, Some(t0().plus(s(5))));
                assert!(refusal.message.contains(&format!("status {status}")));
            }
            other => panic!("{other:?}"),
        }
        // The try 5 s on answers, and the first poll has finished.
        r.advance(6).await;
        assert_eq!(*r.engine.first_poll().borrow(), FirstPoll::Finished);
        assert!(r.tracked("o/r#1"));
    }
}

// POL-20: a 401 reads the token again and repeats the request once; a second gives `auth`.
#[tokio::test]
async fn pol20_repeat() {
    let mut w = world_of(&["o/r#1"]);
    w.fail(Failure::Unauthorized);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    assert_eq!(r.token.reads(), 2);
    r.advance(2).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0], calls[1], "the same request, once more");
    assert!(r.tracked("o/r#1"));
    assert_eq!(problem(&r), None);

    let mut w = world_of(&["o/r#1"]);
    w.fail(Failure::Unauthorized);
    w.fail(Failure::Unauthorized);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    assert_eq!(r.n_calls(), 2);
    assert_eq!(problem(&r), Some(ProblemCode::Auth));
    assert_eq!(unanswered_code(&r), Some(RefusalCode::Auth));
    // Nothing is sent in `auth`: the PRs aren't polled.
    r.advance(600).await;
    assert_eq!(r.n_calls(), 2);
}

// POL-20, NTF-4: `problem_since` is when the code last changed, not each try.
#[tokio::test]
async fn pol20_problem_since() {
    let mut w = World::default();
    w.fail(Failure::Unauthorized);
    w.fail(Failure::Unauthorized);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    assert_eq!(problem(&r), Some(ProblemCode::Auth));
    assert_eq!(r.engine.status().problem_since, Some(at(1)));
    // Five minutes on, a new token's try gets a 401: still `auth`, since unchanged.
    r.advance(300).await;
    r.token
        .set(TokenRead::Token(Token::new(MARKER_TOKEN_2.to_string())));
    r.world().fail(Failure::Unauthorized);
    let before = r.n_calls();
    r.advance(60).await;
    assert_eq!(r.n_calls(), before + 1, "one try, no repeat");
    assert_eq!(problem(&r), Some(ProblemCode::Auth));
    assert_eq!(r.engine.status().problem_since, Some(at(1)));
    // A success clears both.
    r.token
        .set(TokenRead::Token(Token::new(MARKER_TOKEN.to_string())));
    r.advance(60).await;
    assert_eq!(r.n_calls(), before + 2);
    let st = r.engine.status();
    assert_eq!((st.problem, st.problem_since), (None, None));
    // A backoff that follows sets it anew.
    r.world().fail(unavailable(500));
    let sent = r.now();
    r.advance(300).await;
    let st = r.engine.status();
    assert_eq!(
        st.problem.map(|p| p.code),
        Some(ProblemCode::GithubUnavailable)
    );
    let since = st.problem_since.unwrap();
    assert!(since > sent && since <= r.now());
    assert_eq!(Some(since), r.send_times().last().copied());
}

// POL-20: a client request in `auth` reads the token again when the last read is 5 s old.
#[tokio::test]
async fn pol20_client_request_in_auth() {
    let r = start_with(Setup::default(), |t| t.set(TokenRead::NoLogin)).await;
    assert_eq!(r.token.reads(), 1);
    assert_eq!(unanswered_code(&r), Some(RefusalCode::Auth));
    let first = r.engine.first_poll();
    r.advance(5).await;
    r.engine.client_request();
    assert_eq!(*first.borrow(), FirstPoll::Running, "before it returns");
    r.clock.settle().await;
    assert_eq!(r.token.reads(), 2);
    assert_eq!(unanswered_code(&r), Some(RefusalCode::Auth));
    // 2 s after the last read: nothing.
    r.advance(2).await;
    r.engine.client_request();
    assert_eq!(unanswered_code(&r), Some(RefusalCode::Auth));
    r.clock.settle().await;
    assert_eq!(r.token.reads(), 2);
    assert_eq!(r.n_calls(), 0);
    // The source has a token: the next request leads to a poll, then `Finished`.
    r.token
        .set(TokenRead::Token(Token::new(MARKER_TOKEN.to_string())));
    r.advance(3).await;
    r.engine.client_request();
    assert_eq!(*first.borrow(), FirstPoll::Running);
    r.clock.settle().await;
    assert_eq!(r.n_calls(), 1);
    assert_eq!(*first.borrow(), FirstPoll::Finished);
    assert_eq!(problem(&r), None);
}

// POL-21, POL-24: a node's error stays with its PR; the others are applied.
#[tokio::test]
async fn pol21_node_errors_stay_with_the_pr() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2", "o/r#3"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    {
        let mut w = r.world();
        w.errors.insert(id("o/r#1"), NodeAnswer::NotFound);
        w.errors.insert(
            id("o/r#2"),
            NodeAnswer::Error {
                type_: "FORBIDDEN".into(),
            },
        );
        for n in ["o/r#1", "o/r#2", "o/r#3"] {
            w.pr(n).draft = true;
        }
    }
    r.advance(300).await;
    assert!(r.record("o/r#3").unwrap().draft);
    let cfg = Config::default();
    for (n, t) in [("o/r#1", "NOT_FOUND"), ("o/r#2", "FORBIDDEN")] {
        let rec = r.record(n).unwrap();
        assert!(!rec.draft, "keeps its values");
        assert_eq!(rec.error.as_ref().unwrap().type_, t);
        assert!(cadence::stale(
            &rec,
            &r.timing(n),
            Tier::Idle,
            &cfg,
            r.now()
        ));
    }
    // Neither becomes `gone` here, however long it lasts (T42).
    r.advance(3_600).await;
    for n in ["o/r#1", "o/r#2"] {
        assert_eq!(r.record(n).unwrap().state, PrState::Open);
    }
}

// POL-25: exhausted, nothing is sent until the reset; `remaining: 0` is applied first.
#[tokio::test]
async fn pol25_limits() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let reset = t0().plus(s(1_500));
    r.world().fail(Failure::Exhausted {
        reset_at: Some(reset),
    });
    r.advance(299).await;
    assert_eq!(r.n_calls(), 3);
    let st = r.engine.status();
    let p = st.problem.unwrap();
    assert_eq!((p.code, p.until), (ProblemCode::RateLimited, Some(reset)));
    let report = r.engine.poll_now().await;
    assert_eq!((report.outcome, report.cost), (PollOutcome::RateLimited, 0));
    r.advance(1_199).await;
    assert_eq!(r.n_calls(), 3, "nothing until the reset");
    r.advance(11).await;
    assert_eq!(r.n_calls(), 4);
    assert!(r.send_times()[3] >= at(1_500));
    assert_eq!(problem(&r), None);
    assert_eq!(r.lines("problem begin code=rate_limited"), 1);
    assert_eq!(r.lines("problem end code=rate_limited"), 1);

    // An answer with `remaining: 0` is applied, then the poll stops.
    let mut w = world_of(&["o/r#1"]);
    w.rate = Some(Rate {
        cost: 1,
        remaining: 0,
        reset_at: Some(t0().plus(s(1_200))),
    });
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(300).await;
    assert_eq!(r.n_calls(), 1, "the first refresh isn't sent");
    assert!(!r.tracked("o/r#1"));
    assert_eq!(r.engine.status().login.as_deref(), Some("octo"), "applied");
    assert_eq!(problem(&r), Some(ProblemCode::RateLimited));
}

// POL-25: a secondary limit with no `Retry-After` holds everything for 60 s.
#[tokio::test]
async fn pol25_secondary() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.keep_active(25, 5).await;
    r.world().fail(Failure::Secondary { retry_after: None });
    r.keep_active(10, 5).await;
    assert_eq!(r.send_times().last(), Some(&at(30)));
    assert_eq!(problem(&r), Some(ProblemCode::RateLimited));
    let report = r.engine.poll_now().await;
    assert_eq!(report.outcome, PollOutcome::RateLimited);
    r.keep_active(50, 5).await;
    assert_eq!(r.n_calls(), 3, "nothing for 60 s");
    r.keep_active(15, 5).await;
    assert_eq!(r.n_calls(), 4);
    let last = *r.send_times().last().unwrap();
    assert!(last >= at(90) && last <= at(100));
    assert_eq!(r.lines("problem begin code=rate_limited"), 1);
    assert_eq!(r.lines("problem end code=rate_limited"), 1);
}

// POL-23: slowed, capped (with `poll_now` still served), reserve; each warning on its own condition.
#[tokio::test]
async fn pol23_policy_obeyed() {
    let w = World {
        rate: Some(answers::rate(481, 4_999)),
        ..World::default()
    };
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.world().rate = Some(answers::rate(0, 4_999));
    r.keep_active(130, 10).await;
    assert_eq!(r.send_times(), vec![at(0), at(60), at(120)]);
    r.world().rate = Some(answers::rate(119, 4_999));
    r.keep_active(60, 10).await;
    assert_eq!(r.n_calls(), 4);
    assert_eq!(r.engine.status().cost_last_hour, 600);
    let codes =
        |r: &Rig| -> Vec<WarningCode> { r.warnings.list().iter().map(|w| w.code).collect() };
    assert_eq!(codes(&r), vec![WarningCode::QuotaCapped]);
    assert_eq!(problem(&r), Some(ProblemCode::QuotaLow));
    r.keep_active(120, 10).await;
    assert_eq!(r.n_calls(), 4, "capped: no polls");
    // `poll_now` runs when capped.
    r.world().rate = Some(answers::rate(0, 400));
    let report = r.spawn_poll_now();
    r.advance(10).await;
    assert_eq!(report.await.unwrap().outcome, PollOutcome::Polled);
    assert_eq!(r.n_calls(), 5);
    assert_eq!(
        codes(&r),
        vec![WarningCode::QuotaCapped, WarningCode::QuotaReserve]
    );
    // An hour after the first spend, capped clears; reserve stays.
    r.keep_active(3_300, 60).await;
    assert_eq!(codes(&r), vec![WarningCode::QuotaReserve]);
    r.world().rate = Some(answers::rate(0, 4_999));
    r.keep_active(320, 60).await;
    assert!(codes(&r).is_empty());
}
