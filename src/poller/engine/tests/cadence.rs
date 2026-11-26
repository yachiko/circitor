//! Cadence: POL-13, POL-15, POL-22, POL-26, NTF-5.

use super::*;
use crate::model::{ItemsOutcome, ReadMeta, Source};
use crate::poller::{PollOutcome, PollerHandle, Tier};

/// An `asked` PR with a record, tracked through the store as a read-through would (T25's), and not
/// in the open list.
fn track_asked(r: &Rig, n: &str) {
    let read = {
        let mut w = r.world();
        let i = w.open(n, 0);
        w.unlisted.insert(i);
        w.prs[&id(n)].clone()
    };
    r.store.reserve(&id(n), &name(n), Source::Asked).unwrap();
    let meta = ReadMeta {
        seq: 0,
        sent_wall: r.clock.wall(),
        sent_mono: r.now(),
        github_date: None,
    };
    r.store.apply(
        &id(n),
        &read,
        ItemsOutcome::Read(&ItemsRead::default()),
        &meta,
        None,
    );
}

/// The polls (requests with a finding) sent since `from`.
fn polls_since(r: &Rig, from: usize) -> Vec<Query> {
    r.calls()[from..]
        .iter()
        .filter(|q| has_find(q))
        .cloned()
        .collect()
}

fn poll_times(r: &Rig, from: usize) -> Vec<Mono> {
    let calls = r.calls();
    let times = r.send_times();
    (from..calls.len())
        .filter(|&i| has_find(&calls[i]))
        .map(|i| times[i])
        .collect()
}

fn gaps(times: &[Mono]) -> Vec<u64> {
    times
        .windows(2)
        .map(|w| w[1].since(w[0]).as_secs())
        .collect()
}

// NTF-5, POL-13: a subscription on an open PR keeps the active tier; it isn't hot.
#[tokio::test]
async fn ntf5_a_subscription_keeps_the_tier_active() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.engine.client_request();
    r.advance(1).await;
    let guard = r.store.add_subscription(&id("o/r#1")).unwrap();
    let from = r.n_calls();
    for _ in 0..240 {
        r.advance(30).await;
        assert_eq!(r.engine.status().tier, Tier::Active);
    }
    let times = poll_times(&r, from);
    assert!(gaps(&times).iter().all(|&g| g == 30));
    assert_eq!(times.len(), 240);
    // Never refreshed outside a poll (it isn't hot).
    assert!(
        r.calls()[from..]
            .iter()
            .filter(|q| named(q).contains(&id("o/r#1")))
            .all(has_find)
    );
    // Dropped, the last client request 2 hours old: idle at once, polls 300 s apart.
    drop(guard);
    assert_eq!(r.engine.status().tier, Tier::Idle);
    let from = r.n_calls();
    r.advance(1_000).await;
    let times = poll_times(&r, from);
    assert!(gaps(&times).iter().all(|&g| g == 300));
    assert!(times.len() >= 3);
}

// NTF-5, POL-13: an end isn't a request; the tier follows the last client request.
#[tokio::test]
async fn ntf5_the_end_is_not_a_request() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let guard = r.store.add_subscription(&id("o/r#1")).unwrap();
    r.advance(1_000).await;
    r.engine.client_request();
    r.advance(200).await;
    drop(guard);
    r.advance(399).await;
    assert_eq!(r.engine.status().tier, Tier::Active);
    r.advance(1).await;
    assert_eq!(r.engine.status().tier, Tier::Idle);
}

// NTF-5: a subscription on a PR that has ended doesn't keep the tier.
#[tokio::test]
async fn ntf5_an_ended_pr_does_not_keep_the_tier() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let _guard = r.store.add_subscription(&id("o/r#1")).unwrap();
    r.advance(100).await;
    assert_eq!(r.engine.status().tier, Tier::Active);
    r.world().set_state("o/r#1", ReadState::Merged);
    let from = r.n_calls();
    r.advance(30).await;
    assert_eq!(r.n_calls(), from + 1);
    assert_eq!(
        r.engine.status().tier,
        Tier::Idle,
        "from the refresh that showed the merge"
    );
}

// NTF-5, POL-10: an `asked` PR with a subscription is in every poll; without one, one poll in ten.
#[tokio::test]
async fn ntf5_asked_pr_with_a_subscription_in_every_poll() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    track_asked(&r, "x/y#2");
    let _keep = r.store.add_subscription(&id("o/r#1")).unwrap();
    let from = r.n_calls();
    r.advance(600).await;
    let polls = polls_since(&r, from);
    assert_eq!(polls.len(), 20);
    let with = polls
        .iter()
        .filter(|q| named(q).contains(&id("x/y#2")))
        .count();
    assert_eq!(with, 2);
    let _sub = r.store.add_subscription(&id("x/y#2")).unwrap();
    let from = r.n_calls();
    r.advance(300).await;
    let polls = polls_since(&r, from);
    assert_eq!(polls.len(), 10);
    assert!(polls.iter().all(|q| named(q).contains(&id("x/y#2"))));
}

// POL-15: a forced poll's window runs from the last forced poll, not the last poll.
#[tokio::test]
async fn pol15_forced_poll_window() {
    let r = start(Setup::default()).await;
    assert_eq!(r.n_calls(), 1);
    r.advance(2).await;
    let first = r.spawn_poll_now();
    r.clock.settle().await;
    assert_eq!(first.await.unwrap().outcome, PollOutcome::Polled);
    assert_eq!(r.send_times()[1], at(2));
    r.advance(9).await;
    let second = r.spawn_poll_now();
    r.clock.settle().await;
    assert_eq!(
        second.await.unwrap(),
        crate::poller::PollReport {
            outcome: PollOutcome::Shared,
            cost: 0
        }
    );
    r.advance(2).await;
    let third = r.spawn_poll_now();
    r.clock.settle().await;
    assert_eq!(third.await.unwrap().outcome, PollOutcome::Polled);
    assert_eq!(r.n_calls(), 3);
}

// POL-13: idle from the start; 30 s while requests come; 300 s after 600 s without one.
#[tokio::test]
async fn pol13_tiers() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    assert_eq!(r.engine.status().tier, Tier::Idle);
    r.advance(300).await;
    assert_eq!(poll_times(&r, 0), vec![at(0), at(300)]);
    assert_eq!(r.engine.status().tier, Tier::Idle);
    r.keep_active(120, 10).await;
    assert_eq!(r.engine.status().tier, Tier::Active);
    // The last request at 410: active until 1010.
    r.advance(910).await;
    let times = poll_times(&r, 0);
    let active: Vec<Mono> = times.iter().copied().filter(|t| *t > at(300)).collect();
    assert_eq!(active.first(), Some(&at(330)));
    assert!(gaps(&active[..active.len() - 1]).iter().all(|&g| g == 30));
    assert!(active.contains(&at(990)));
    assert!(!active.contains(&at(1_020)));
    assert_eq!(times.last(), Some(&at(1_290)));
    // A request in the idle tier starts a poll at once when the last is over 30 s old.
    r.engine.client_request();
    r.clock.settle().await;
    assert_eq!(poll_times(&r, 0).last(), Some(&at(1_330)));
    assert_eq!(r.lines("tier tier=active"), 2);
    assert_eq!(r.lines("tier tier=idle"), 1);
}

// POL-22: a sleep noticed by the tick: one line, every record slept, no poll without a reason.
#[tokio::test]
async fn pol22_sleep() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(15).await;
    let before = r.n_calls();
    r.clock.sleep_wall_only(s(7_200)).await;
    r.advance(5).await;
    assert_eq!(r.lines("sleep seconds=7200"), 1);
    assert_eq!(r.sleep.total(), s(7_200));
    assert!(r.timing("o/r#1").slept);
    assert_eq!(r.n_calls(), before, "no waiter, no subscription, no client");
}

// POL-22: one poll at once when a client asked within `idle_after`, a waiter or a subscription is held.
#[tokio::test]
async fn pol22_sleep_polls_when_wanted() {
    for case in 0..3 {
        let r = start(Setup {
            world: world_of(&["o/r#1"]),
            ..Setup::default()
        })
        .await;
        r.advance(1).await;
        let _waiter = (case == 1).then(|| r.store.add_waiter(&id("o/r#1")).unwrap());
        let _sub = (case == 2).then(|| r.store.add_subscription(&id("o/r#1")).unwrap());
        r.advance(14).await;
        let before = r.n_calls();
        r.clock.sleep_wall_only(s(7_200)).await;
        if case == 0 {
            // The client request notices it.
            r.engine.client_request();
            r.clock.settle().await;
        } else {
            r.advance(5).await;
        }
        assert_eq!(r.lines("sleep seconds="), 1, "case {case}");
        let polls = polls_since(&r, before);
        assert_eq!(polls.len(), 1, "case {case}");
    }
}

// POL-22: a running backoff starts again at 5 s; a sleep both clocks count sends one poll.
#[tokio::test]
async fn pol22_backoff_and_sleep_both() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    for _ in 0..3 {
        r.world().fail(unavailable(503));
    }
    // Fails at 300, 305, 315; the next try would be at 335.
    r.advance(316).await;
    assert_eq!(r.n_calls(), 5);
    r.clock.sleep_wall_only(s(7_200)).await;
    r.advance(10).await;
    assert_eq!(
        r.send_times()[5],
        at(325),
        "the tick at 320 noticed it: 5 s on"
    );

    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let before = r.n_calls();
    r.clock.sleep_both(s(7_200)).await;
    assert_eq!(
        r.n_calls(),
        before + 1,
        "one poll, not one per missed interval"
    );
    r.advance(100).await;
    assert_eq!(r.n_calls(), before + 1);
}

// POL-26: the ticks send nothing; an ended PR past `keep_done` is dropped at a tick.
#[tokio::test]
async fn pol26_tick() {
    let config = Config {
        keep_done: s(60),
        ..Config::default()
    };
    let r = start(Setup {
        config,
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    r.world().set_state("o/r#1", ReadState::Merged);
    r.advance(299).await;
    let after_poll = r.n_calls();
    assert!(r.tracked("o/r#1"));
    r.advance(59).await;
    assert!(r.tracked("o/r#1"));
    r.advance(10).await;
    assert!(!r.tracked("o/r#1"), "dropped at the tick after keep_done");
    assert!(r.tracked("o/r#2"));
    r.advance(220).await;
    assert_eq!(r.n_calls(), after_poll, "ticks between polls send nothing");
    let _ = r.handle().idle_since();
}
