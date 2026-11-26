//! A PR's errors (T42): POL-21's `gone` and `pr_failing`, MOD-11's conditions, POL-24's line.

use super::*;
use crate::model::{EventKind, PrState, Seen, Source, Status};
use crate::state::{FileEntry, FileSource};
use crate::status::WarningCode;
use crate::testkit::MARKER_TITLE;

fn forbidden() -> NodeAnswer {
    NodeAnswer::Error {
        type_: "FORBIDDEN".to_string(),
    }
}

fn miss(r: &Rig, n: &str) {
    r.world().errors.insert(id(n), NodeAnswer::NotFound);
}

fn fail_with(r: &Rig, n: &str, a: NodeAnswer) {
    r.world().errors.insert(id(n), a);
}

fn mend(r: &Rig, n: &str) {
    r.world().errors.remove(&id(n));
}

fn is_gone(r: &Rig, n: &str) -> bool {
    r.record(n).unwrap().status == Status::Gone
}

fn is_stale(r: &Rig, n: &str, tier: Tier) -> bool {
    cadence::stale(
        &r.record(n).unwrap(),
        &r.timing(n),
        tier,
        &Config::default(),
        r.now(),
    )
}

fn warnings_of(r: &Rig) -> Vec<crate::status::Warning> {
    r.warnings
        .list()
        .into_iter()
        .filter(|w| w.code == WarningCode::PrFailing)
        .collect()
}

use crate::poller::Tier;
use crate::poller::cadence;

// POL-21: `NOT_FOUND` at every poll for 5 minutes and three polls: `gone`.
#[tokio::test]
async fn pol21_gone_after_five_minutes() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    // The polls at 0 to 270 s after the first.
    r.keep_active(327, 10).await;
    assert_eq!(r.record("o/r#1").unwrap().state, PrState::Open);
    assert!(is_stale(&r, "o/r#1", Tier::Active));
    assert!(
        r.events("o/r#1")
            .iter()
            .all(|e| !e.kinds.contains(&EventKind::Gone))
    );
    let before = r.record("o/r#1").unwrap();
    r.keep_active(3, 1).await;
    assert!(is_gone(&r, "o/r#1"));
    let events = r.events("o/r#1");
    let gone: Vec<_> = events
        .iter()
        .filter(|e| e.kinds.contains(&EventKind::Gone))
        .collect();
    assert_eq!(gone.len(), 1);
    let after = r.record("o/r#1").unwrap();
    assert_eq!(after.title, before.title);
    assert_eq!(after.pr, before.pr);
    assert_eq!(r.record("o/r#2").unwrap().state, PrState::Open);
    assert!(!is_stale(&r, "o/r#2", Tier::Active));
    assert_eq!(r.lines("gone pr=o/r#1"), 1);
}

// POL-21: polls 300 s apart: 0 and 300 s don't, the third does.
#[tokio::test]
async fn pol21_idle_tier() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    r.advance(2_000).await;
    assert!(is_gone(&r, "o/r#1"));
    let polls: Vec<u64> = r
        .calls()
        .iter()
        .zip(r.send_times())
        .filter(|(q, _)| has_find(q))
        .map(|(_, t)| t.since(Mono::ZERO).as_secs())
        .collect();
    let gaps: Vec<u64> = polls.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.iter().all(|g| *g == 300), "{gaps:?}");
    // The `NOT_FOUND`s counted: polls[1], polls[2], polls[3]; gone after the third.
    let e = r.events("o/r#1");
    let gone = e
        .iter()
        .find(|e| e.kinds.contains(&EventKind::Gone))
        .unwrap();
    assert_eq!(gone.at, t0().plus(s(polls[3])));
}

// POL-21: three in a minute, then an applied refresh: the run starts again from nothing.
#[tokio::test]
async fn pol21_three_in_a_minute() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    r.keep_active(65, 10).await;
    mend(&r, "o/r#1");
    r.keep_active(30, 10).await;
    assert_eq!(r.record("o/r#1").unwrap().state, PrState::Open);
    assert!(!is_stale(&r, "o/r#1", Tier::Active));
    miss(&r, "o/r#1");
    // 5 minutes from the new first one is needed, not from the old.
    r.keep_active(270, 10).await;
    assert!(!is_gone(&r, "o/r#1"));
    r.keep_active(60, 10).await;
    assert!(is_gone(&r, "o/r#1"));
}

// MOD-11: the login the PR was read with isn't the poll's.
#[tokio::test]
async fn mod11_same_login() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    r.world().login = "someone-else".to_string();
    r.keep_active(600, 10).await;
    assert_eq!(r.record("o/r#1").unwrap().state, PrState::Open);
    assert!(is_stale(&r, "o/r#1", Tier::Active));
    // Without regard to case.
    r.world().login = "OCTO".to_string();
    r.keep_active(330, 10).await;
    assert!(is_gone(&r, "o/r#1"));
}

// MOD-11: not when every PR of the poll is missing; a lone one can go.
#[tokio::test]
async fn mod11_not_every_pr() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    miss(&r, "o/r#2");
    r.keep_active(600, 10).await;
    for n in ["o/r#1", "o/r#2"] {
        assert_eq!(r.record(n).unwrap().state, PrState::Open);
        assert!(is_stale(&r, n, Tier::Active));
    }
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    r.keep_active(330, 10).await;
    assert!(is_gone(&r, "o/r#1"));
}

// MOD-11: hot refreshes don't count, and don't end a run begun in polls.
#[tokio::test]
async fn mod11_only_polls_count() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    let waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    miss(&r, "o/r#1");
    // The polls' login differs, so only the hot refreshes are left to count.
    r.world().login = "someone-else".to_string();
    r.keep_active(360, 5).await;
    assert!(r.calls().iter().filter(|q| !has_find(q)).count() > 20);
    assert_eq!(r.record("o/r#1").unwrap().state, PrState::Open);
    // Now the polls count, and the hot ones between don't end the run.
    r.world().login = "octo".to_string();
    r.keep_active(330, 5).await;
    assert!(is_gone(&r, "o/r#1"));
    drop(waiter);
}

// MOD-11: polls whose finding fails count nothing.
#[tokio::test]
async fn mod11_no_login_no_count() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    // The finding is the first request of a poll: fail every one, and the poll is abandoned.
    for _ in 0..40 {
        r.world().fail(unavailable(503));
    }
    r.advance(900).await;
    assert_eq!(r.record("o/r#1").unwrap().state, PrState::Open);
    assert!(warnings_of(&r).is_empty());
}

// POL-21: another type in a poll ends the run.
#[tokio::test]
async fn pol21_other_type_ends_the_run() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    r.keep_active(240, 10).await;
    fail_with(&r, "o/r#1", forbidden());
    r.keep_active(30, 10).await;
    miss(&r, "o/r#1");
    r.keep_active(290, 10).await;
    assert!(!is_gone(&r, "o/r#1"));
    r.keep_active(40, 10).await;
    assert!(is_gone(&r, "o/r#1"));
}

// STA-8: a loaded PR saved open, under its saved login.
#[tokio::test]
async fn sta8_loaded_entry() {
    let loaded = FileContent {
        prs: vec![FileEntry {
            pr: name("o/r#9"),
            id: id("o/r#9"),
            source: Source::Manual,
            login: Some("octo".into()),
            ended: None,
            seen: Seen::default(),
        }],
    };
    let mut w = World::default();
    w.open("o/r#9", 0);
    w.unlisted.insert(id("o/r#9"));
    w.errors.insert(id("o/r#9"), NodeAnswer::NotFound);
    let r = start(Setup {
        world: w,
        loaded,
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    r.keep_active(297, 10).await;
    assert!(r.store.file_content().prs[0].ended.is_none());
    r.keep_active(3, 1).await;
    let content = r.store.file_content();
    let ended = content.prs[0].ended.clone().unwrap();
    assert_eq!(ended.state, PrState::Gone);
    let sent = r.send_times();
    let at = ended.seen_at.since(t0()).as_secs();
    assert!(sent.iter().any(|m| m.since(Mono::ZERO).as_secs() == at));
    assert!((299..=330).contains(&at), "{at}");
    assert_eq!(r.lines("gone pr=o/r#9"), 1);
    assert!(r.store.view().events.is_empty());
    r.advance(600).await;
    assert_eq!(r.lines("gone pr=o/r#9"), 1);
    assert_eq!(
        r.store.file_content().prs[0]
            .ended
            .as_ref()
            .unwrap()
            .seen_at,
        ended.seen_at
    );
}

// STA-2: the retention clock runs from the send time of the request that completed the run.
#[tokio::test]
async fn sta2_seen_at() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    r.keep_active(400, 10).await;
    assert!(is_gone(&r, "o/r#1"));
    let content = r.store.file_content();
    let e = content.prs.iter().find(|e| e.id == id("o/r#1")).unwrap();
    let ended = e.ended.clone().unwrap();
    assert_eq!(ended.state, PrState::Gone);
    let event = r
        .events("o/r#1")
        .into_iter()
        .find(|e| e.kinds.contains(&EventKind::Gone))
        .unwrap();
    // Sent when the run completed: the event is made as the answer is handled, at the same time.
    assert_eq!(ended.seen_at, event.at);
    assert!(ended.seen_at.since(t0()).as_secs() >= 330);
}

// POL-24: one line while a run lasts; a second after an applied refresh.
#[tokio::test]
async fn pol24_one_log_line() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    fail_with(&r, "o/r#1", forbidden());
    r.keep_active(600, 10).await;
    assert_eq!(r.lines("pr error pr=o/r#1 type=FORBIDDEN"), 1);
    mend(&r, "o/r#1");
    r.keep_active(40, 10).await;
    fail_with(&r, "o/r#1", forbidden());
    r.keep_active(60, 10).await;
    assert_eq!(r.lines("pr error pr=o/r#1 type=FORBIDDEN"), 2);
    assert_eq!(r.lines("pr error"), 2);
}

// POL-21: `pr_failing` after 5 minutes of errors.
#[tokio::test]
async fn pol21_pr_failing() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    fail_with(&r, "o/r#1", forbidden());
    r.keep_active(327, 10).await;
    assert!(warnings_of(&r).is_empty());
    r.keep_active(3, 1).await;
    let w = warnings_of(&r);
    assert_eq!(w.len(), 1);
    assert!(w[0].message.contains("o/r#1"), "{}", w[0].message);
    assert!(w[0].message.contains("FORBIDDEN") && w[0].message.contains("gh"));
    assert!(!w[0].message.contains(&id("o/r#1").0));
    let since = w[0].since;
    // The type changes: `since` stays, the message follows.
    miss(&r, "o/r#1");
    r.keep_active(70, 10).await;
    let w = warnings_of(&r);
    assert_eq!(w[0].since, since);
    assert!(w[0].message.contains("NOT_FOUND"));
    // Never gone: its `NOT_FOUND`s began as another type's run, and the other PR answers.
    mend(&r, "o/r#1");
    r.keep_active(40, 10).await;
    assert!(warnings_of(&r).is_empty());
    assert_eq!(r.record("o/r#1").unwrap().state, PrState::Open);
}

// POL-21: cleared when the PR becomes gone, and when it's untracked while failing.
#[tokio::test]
async fn pol21_pr_failing_cleared() {
    let mut w = world_of(&["o/r#1", "o/r#2"]);
    let third = w.open("o/r#3", 0);
    w.unlisted.insert(third);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    // A manual PR the user doesn't author: `untrack` drops it.
    let e = r.engine.clone();
    let tracking = tokio::spawn(async move { e.track(&name("o/r#3")).await });
    r.advance(1).await;
    tracking.await.unwrap().unwrap();
    miss(&r, "o/r#1");
    fail_with(&r, "o/r#3", forbidden());
    r.keep_active(400, 10).await;
    assert!(is_gone(&r, "o/r#1"));
    let w = warnings_of(&r);
    assert_eq!(w.len(), 1, "only the other PR's: {w:?}");
    assert!(w[0].message.contains("o/r#3"));
    // Untracked while failing: gone by the next tick.
    r.store.untrack(&name("o/r#3"));
    assert!(!r.tracked("o/r#3"));
    r.advance(11).await;
    assert!(warnings_of(&r).is_empty());
}

// POL-21: a gone PR's later `NOT_FOUND`s add nothing.
#[tokio::test]
async fn pol21_already_gone() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    miss(&r, "o/r#1");
    r.keep_active(400, 10).await;
    assert!(is_gone(&r, "o/r#1"));
    let lines = r.lines("pr error");
    assert_eq!(r.lines("gone pr=o/r#1"), 1);
    r.keep_active(3_000, 60).await;
    assert_eq!(r.lines("pr error"), lines);
    assert_eq!(r.lines("gone pr=o/r#1"), 1);
    assert!(warnings_of(&r).is_empty());
    assert_eq!(
        r.events("o/r#1")
            .iter()
            .filter(|e| e.kinds.contains(&EventKind::Gone))
            .count(),
        1
    );
}

// OPS-9: no title in a line or a warning.
#[tokio::test]
async fn ops9_no_text() {
    let mut w = world_of(&["o/r#1", "o/r#2"]);
    for read in w.prs.values_mut() {
        read.title = MARKER_TITLE.to_string();
    }
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    fail_with(&r, "o/r#1", forbidden());
    r.keep_active(400, 10).await;
    assert_eq!(warnings_of(&r).len(), 1);
    miss(&r, "o/r#1");
    r.keep_active(400, 10).await;
    assert!(!r.sink.contents().contains(MARKER_TITLE));
    for w in r.warnings.list() {
        assert!(!w.message.contains(MARKER_TITLE));
    }
}
