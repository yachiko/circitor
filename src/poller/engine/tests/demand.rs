//! The demand side (T25): POL-11, POL-14, POL-15 (D11), POL-16, API-13, API-15, POL-21's hot share.

use super::*;
use crate::model::{EventKind, ItemsOutcome, Mergeable, ReadMeta, Seen, Source, Status};
use crate::poller::{PollOutcome, ProblemCode, Refreshed, Refusal, RefusalCode};
use crate::state::{FileEntry, FileSource};
use crate::testkit::MARKER_TITLE;
use crate::token::TokenRead;
use tokio::task::JoinHandle;

fn secs(m: Mono) -> u64 {
    m.since(Mono::ZERO).as_secs()
}

/// `refresh_pr` on a task of its own.
fn refresh(r: &Rig, n: &str) -> JoinHandle<Refreshed> {
    let (e, i) = (r.engine.clone(), id(n));
    tokio::spawn(async move { e.refresh_pr(&i).await })
}

fn read_through(r: &Rig, n: &str) -> JoinHandle<Result<NodeId, Refusal>> {
    let (e, pr) = (r.engine.clone(), name(n));
    tokio::spawn(async move { e.read_through(&pr).await })
}

fn track(r: &Rig, n: &str) -> JoinHandle<Result<NodeId, Refusal>> {
    let (e, pr) = (r.engine.clone(), name(n));
    tokio::spawn(async move { e.track(&pr).await })
}

/// The requests from `from` on that name `n`: when each was sent, and whether it held the finding.
fn asks(r: &Rig, n: &str, from: usize) -> Vec<(u64, bool)> {
    let i = id(n);
    let calls = r.calls();
    let times = r.send_times();
    (from..times.len())
        .filter(|&k| named(&calls[k]).contains(&i))
        .map(|k| (secs(times[k]), has_find(&calls[k])))
        .collect()
}

/// An `asked` PR with a record, refreshed now, and not in the open list.
fn asked(r: &Rig, n: &str) {
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
    r.store.mark_sent(&[id(n)], r.now());
}

fn names(prefix: &str, n: usize) -> Vec<String> {
    (1..=n).map(|k| format!("{prefix}#{k}")).collect()
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn with_rate(mut w: World, cost: u32, remaining: u32) -> World {
    w.rate = Some(answers::rate(cost, remaining));
    w
}

fn loaded(n: &str, source: Source) -> FileContent {
    FileContent {
        prs: vec![FileEntry {
            pr: name(n),
            id: id(n),
            source,
            login: Some("octo".to_string()),
            ended: None,
            seen: Seen::default(),
        }],
    }
}

fn code(r: &Result<NodeId, Refusal>) -> Option<RefusalCode> {
    r.as_ref().err().map(|e| e.code)
}

// D11, POL-15: a forced poll refreshes every tracked PR, whatever its period; an ordinary one doesn't.
#[tokio::test]
async fn d11_forced_poll_refreshes_every_pr() {
    let config = Config {
        interval: s(60),
        ..Config::default()
    };
    let r = start(Setup {
        config,
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    for n in ["o/r#1", "o/r#2", "o/r#3"] {
        asked(&r, n);
    }
    // The ordinary poll at 300 (idle) asks none of them: `asked`, refreshed 299 s ago.
    r.advance(299).await;
    let last = r.calls().last().cloned().unwrap();
    assert!(has_find(&last) && last.nodes.is_empty());
    // 60 s after it, a forced poll: the finding and all three, in one request.
    r.advance(60).await;
    let from = r.n_calls();
    let polled = r.spawn_poll_now();
    r.advance(1).await;
    assert_eq!(polled.await.unwrap().outcome, PollOutcome::Polled);
    let calls = r.calls();
    assert_eq!(calls.len(), from + 1);
    assert!(has_find(&calls[from]));
    assert_eq!(
        named(&calls[from]),
        vec![id("o/r#1"), id("o/r#2"), id("o/r#3")]
    );
    // An ordinary poll 60 s after that one (active, `interval` 60 s) asks none of them.
    r.engine.client_request();
    let from = r.n_calls();
    r.advance(60).await;
    let calls = r.calls();
    assert_eq!(calls.len(), from + 1);
    assert!(has_find(&calls[from]) && calls[from].nodes.is_empty());
    // Beyond 40 PRs, a second request.
    for k in 4..=41 {
        asked(&r, &format!("o/r#{k}"));
    }
    r.advance(11).await;
    let from = r.n_calls();
    let polled = r.spawn_poll_now();
    r.advance(3).await;
    assert_eq!(polled.await.unwrap().outcome, PollOutcome::Polled);
    let calls = r.calls();
    assert_eq!(calls.len(), from + 2);
    assert!(has_find(&calls[from]) && !has_find(&calls[from + 1]));
    assert_eq!(
        (calls[from].nodes.len(), calls[from + 1].nodes.len()),
        (40, 1)
    );
}

// POL-14: a waiter's PR is refreshed every `hot_interval` between polls; a poll counts.
#[tokio::test]
async fn pol14_hot_between_polls() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.engine.client_request();
    r.advance(2).await;
    let waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(43).await;
    // The poll at 0 is t: t + 10, t + 20, the poll at t + 30, then t + 40.
    assert_eq!(
        asks(&r, "o/r#1", from),
        vec![(10, false), (20, false), (30, true), (40, false)]
    );
    for q in &r.calls()[from..] {
        if !has_find(q) {
            assert_eq!(q.nodes.len(), 1);
            assert_eq!(q.nodes[0].want, Want::Record);
        }
    }
    drop(waiter);
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(100).await;
    assert!(r.calls()[from..].iter().all(has_find), "only polls");
}

// POL-14: named while `checks_pending`: hot for 2 minutes; a `ready` PR named never is.
#[tokio::test]
async fn pol14_named_while_pending() {
    let mut w = world_of(&["o/r#1", "o/r#2"]);
    w.pending.insert(id("o/r#1"));
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    // MOD-6: an empty list settles 60 s after the first read (sent at 1); then #2 is `ready`.
    r.advance(65).await;
    let settled = refresh(&r, "o/r#2");
    r.clock.settle().await;
    assert_eq!(settled.await.unwrap(), Refreshed::Applied);
    assert_eq!(r.record("o/r#1").unwrap().status, Status::ChecksPending);
    assert_eq!(r.record("o/r#2").unwrap().status, Status::Ready);
    r.advance(1).await;
    drop(r.store.name(&id("o/r#1")));
    drop(r.store.name(&id("o/r#2")));
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(200).await;
    // Named at 66: every 10 s while that was less than 120 s ago.
    let every_10: Vec<(u64, bool)> = (0..12).map(|k| (66 + k * 10, false)).collect();
    assert_eq!(asks(&r, "o/r#1", from), every_10);
    assert!(asks(&r, "o/r#2", from).is_empty());
}

// POL-14, POL-7: 41 hot PRs are two requests.
#[tokio::test]
async fn pol14_many_hot() {
    let all = names("o/r", 41);
    let r = start(Setup {
        world: world_of(&refs(&all)),
        ..Setup::default()
    })
    .await;
    r.advance(3).await;
    let _waiters: Vec<_> = all
        .iter()
        .map(|n| r.store.add_waiter(&id(n)).unwrap())
        .collect();
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(9).await;
    let calls = &r.calls()[from..];
    let sizes: Vec<usize> = calls.iter().map(|q| q.nodes.len()).collect();
    assert_eq!(sizes, vec![40, 1]);
    assert!(calls.iter().all(|q| !has_find(q)));
}

// POL-15: one PR is forced only when its last refresh was sent more than 5 s ago.
#[tokio::test]
async fn pol15_window() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    let from = r.n_calls();
    let first = refresh(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(first.await.unwrap(), Refreshed::Applied);
    assert_eq!(r.n_calls(), from + 1);
    r.advance(4).await;
    let second = refresh(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(second.await.unwrap(), Refreshed::NotSent);
    assert_eq!(r.n_calls(), from + 1);
    r.clock.advance(Duration::from_millis(1_001)).await;
    let third = refresh(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(third.await.unwrap(), Refreshed::Applied);
    let calls = r.calls();
    assert_eq!(calls.len(), from + 2);
    assert_eq!(named(&calls[from + 1]), vec![id("o/r#1")]);
    assert!(!has_find(&calls[from + 1]));
}

// POL-15: callers of one PR share the request in flight and its result.
#[tokio::test]
async fn pol15_shared() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    for (fail, want) in [(false, Refreshed::Applied), (true, Refreshed::Failed)] {
        r.advance(10).await;
        if fail {
            r.world().fail(unavailable(503));
        }
        let release = r.github.hold(&r.clock.activity());
        let from = r.n_calls();
        let a = refresh(&r, "o/r#1");
        r.clock.settle().await;
        assert_eq!(r.n_calls(), from + 1, "sent and held");
        let b = refresh(&r, "o/r#1");
        let c = refresh(&r, "o/r#1");
        r.clock.settle().await;
        assert!(!b.is_finished() && !c.is_finished());
        drop(release);
        r.clock.settle().await;
        for h in [a, b, c] {
            assert_eq!(h.await.unwrap(), want);
        }
        assert_eq!(r.n_calls(), from + 1);
    }
}

// POL-15: a forced refresh goes next, between a poll's requests; the poll then leaves it out.
#[tokio::test]
async fn pol15_goes_next() {
    let all = names("o/r", 200);
    let r = start(Setup {
        world: world_of(&refs(&all)),
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    assert_eq!(r.store.places_free(), 0);
    r.advance(289).await;
    let release = r.github.hold(&r.clock.activity());
    r.advance(1).await;
    let held = r.n_calls() - 1;
    {
        let calls = r.calls();
        assert!(has_find(&calls[held]) && calls[held].nodes.len() == 40);
        assert!(!named(&calls[held]).contains(&id("o/r#100")));
    }
    let forced = refresh(&r, "o/r#100");
    r.clock.settle().await;
    drop(release);
    r.advance(10).await;
    assert_eq!(forced.await.unwrap(), Refreshed::Applied);
    let calls = r.calls();
    assert_eq!(named(&calls[held + 1]), vec![id("o/r#100")]);
    assert!(!has_find(&calls[held + 1]));
    // Updated for D15 (T56): every PR on page 1 is tracked, so the finding reads page 2, then the
    // batches go on.
    assert!(has_find(&calls[held + 2]) && calls[held + 2].nodes.is_empty());
    assert!(!has_find(&calls[held + 3]) && calls[held + 3].nodes.len() == 40);
    let later: Vec<NodeId> = calls[held + 2..].iter().flat_map(named).collect();
    assert!(!later.contains(&id("o/r#100")));
    assert_eq!(later.len() + 40, 199, "every other PR, once");
}

// POL-15, POL-19, POL-20, POL-23: not sent in a backoff, in `auth`, exhausted; sent capped, in 30 s.
#[tokio::test]
async fn pol15_not_sent() {
    // A backoff.
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    r.world().fail(unavailable(503));
    r.advance(290).await;
    let n = r.n_calls();
    let x = refresh(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(x.await.unwrap(), Refreshed::NotSent);
    assert_eq!(r.n_calls(), n);

    // `auth`.
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    r.world().fail(Failure::Unauthorized);
    r.world().fail(Failure::Unauthorized);
    r.advance(291).await;
    assert_eq!(
        r.engine.status().problem.map(|p| p.code),
        Some(ProblemCode::Auth)
    );
    r.advance(10).await;
    let n = r.n_calls();
    let x = refresh(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(x.await.unwrap(), Refreshed::NotSent);
    assert_eq!(r.n_calls(), n);

    // Exhausted.
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    r.world().fail(Failure::Exhausted {
        reset_at: Some(t0().plus(s(1_500))),
    });
    r.advance(290).await;
    let n = r.n_calls();
    let x = refresh(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(x.await.unwrap(), Refreshed::NotSent);
    assert_eq!(r.n_calls(), n);

    // Capped: sent, in a 30 s window.
    let r = start(Setup {
        world: with_rate(world_of(&["o/r#1"]), 600, 4_999),
        ..Setup::default()
    })
    .await;
    r.world().rate = Some(answers::rate(0, 4_999));
    r.advance(31).await;
    let from = r.n_calls();
    for (wait, want) in [
        (0, Refreshed::Applied),
        (10, Refreshed::NotSent),
        (21, Refreshed::Applied),
    ] {
        r.advance(wait).await;
        let x = refresh(&r, "o/r#1");
        r.clock.settle().await;
        assert_eq!(x.await.unwrap(), want);
    }
    assert_eq!(asks(&r, "o/r#1", from), vec![(31, false), (62, false)]);
}

// POL-16: an untracked PR is read through, tracked `asked`, and answered; then it's in memory.
#[tokio::test]
async fn pol16_read_through() {
    let mut w = World::default();
    let i = w.open("o/r#1", 0);
    w.unlisted.insert(i);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(5).await;
    let from = r.n_calls();
    let got = read_through(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(got.await.unwrap(), Ok(id("o/r#1")));
    let calls = r.calls();
    assert_eq!(calls.len(), from + 1);
    assert_eq!(calls[from].lookup, Some(name("o/r#1")));
    assert!(!has_find(&calls[from]) && calls[from].nodes.is_empty());
    assert_eq!(
        r.store.lookup(&name("o/r#1")).unwrap().source,
        Source::Asked
    );
    let events = r.events("o/r#1");
    assert_eq!(events.len(), 1);
    assert!(events[0].kinds.iter().any(|k| matches!(
        k,
        EventKind::Tracked {
            source: Source::Asked
        }
    )));
    assert!(
        !events[0]
            .kinds
            .iter()
            .any(|k| matches!(k, EventKind::Activity { .. }))
    );
    assert_eq!(r.timing("o/r#1").last_sent, Some(at(5)));
    let again = read_through(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(again.await.unwrap(), Ok(id("o/r#1")));
    assert_eq!(r.n_calls(), from + 1);
}

// POL-16, API-3, STA-3: each refusal, and none tracks the PR or holds a title.
#[tokio::test]
async fn pol16_refusals() {
    let mut messages = Vec::new();
    let mut w = World::default();
    w.lookup_errors.insert(
        name("o/f#1"),
        LookupAnswer::Error {
            type_: "FORBIDDEN".to_string(),
        },
    );
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(5).await;
    let n = r.n_calls();
    let missing = read_through(&r, "o/missing#1");
    r.advance(2).await;
    let missing = missing.await.unwrap();
    assert_eq!(code(&missing), Some(RefusalCode::NotFound));
    assert!(!r.tracked("o/missing#1"));
    assert_eq!(r.n_calls(), n + 1);
    let forbidden = read_through(&r, "o/f#1");
    r.advance(2).await;
    let forbidden = forbidden.await.unwrap().unwrap_err();
    assert_eq!(forbidden.code, RefusalCode::GithubUnavailable);
    assert!(forbidden.message.contains("FORBIDDEN") && forbidden.message.contains("gh"));
    assert_eq!(r.n_calls(), n + 2);
    messages.push(missing.unwrap_err().message);
    messages.push(forbidden.message);

    // `limit` at 200, nothing sent.
    let all = names("o/r", 200);
    let mut w = world_of(&refs(&all));
    for read in w.prs.values_mut() {
        read.title = MARKER_TITLE.to_string();
    }
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    let n = r.n_calls();
    let limit = read_through(&r, "o/x#1");
    r.clock.settle().await;
    let limit = limit.await.unwrap().unwrap_err();
    assert_eq!(limit.code, RefusalCode::Limit);
    assert_eq!(r.n_calls(), n);
    messages.push(limit.message);

    // `auth`.
    let r = start_with(Setup::default(), |t| t.set(TokenRead::NoLogin)).await;
    r.advance(10).await;
    let auth = read_through(&r, "o/r#1");
    r.clock.settle().await;
    let auth = auth.await.unwrap().unwrap_err();
    assert_eq!(auth.code, RefusalCode::Auth);
    assert_eq!(r.n_calls(), 0);
    messages.push(auth.message);

    // `rate_limited`, with `until`.
    let r = start(Setup::default()).await;
    r.advance(10).await;
    let reset = t0().plus(s(1_500));
    r.world().fail(Failure::Exhausted {
        reset_at: Some(reset),
    });
    r.advance(290).await;
    let n = r.n_calls();
    let limited = read_through(&r, "o/r#1");
    r.clock.settle().await;
    let limited = limited.await.unwrap().unwrap_err();
    assert_eq!(
        (limited.code, limited.until),
        (RefusalCode::RateLimited, Some(reset))
    );
    assert_eq!(r.n_calls(), n);
    messages.push(limited.message);

    for m in messages {
        assert!(!m.contains(MARKER_TITLE), "{m}");
    }
}

// POL-16, POL-19: one read-through a step of the backoff; its failure doesn't move it; its success ends it.
#[tokio::test]
async fn pol16_one_per_backoff_step() {
    let mut w = world_of(&["o/r#1"]);
    let i = w.open("o/r#2", 0);
    w.unlisted.insert(i);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    r.world().fail(unavailable(503));
    r.world().fail(unavailable(503));
    r.advance(291).await;
    let until = Some(t0().plus(s(305)));
    assert_eq!(r.engine.status().problem.unwrap().until, until);
    let n = r.n_calls();
    let first = read_through(&r, "o/r#2");
    r.clock.settle().await;
    let first = first.await.unwrap().unwrap_err();
    assert_eq!(first.code, RefusalCode::GithubUnavailable);
    assert_eq!(r.n_calls(), n + 1, "sent");
    assert_eq!(r.engine.status().problem.unwrap().until, until, "unmoved");
    let second = read_through(&r, "o/r#2");
    r.clock.settle().await;
    let second = second.await.unwrap().unwrap_err();
    assert_eq!(
        (second.code, second.until),
        (RefusalCode::GithubUnavailable, until)
    );
    assert_eq!(r.n_calls(), n + 1, "not sent");
    // The try at 305 fails: a new step, and one more read-through.
    r.world().fail(unavailable(503));
    r.advance(5).await;
    assert_eq!(r.n_calls(), n + 2);
    let third = read_through(&r, "o/r#2");
    r.clock.settle().await;
    assert_eq!(third.await.unwrap(), Ok(id("o/r#2")));
    assert_eq!(r.engine.status().problem, None, "the backoff is over");
    // A poll follows at once.
    r.advance(2).await;
    let calls = r.calls();
    assert_eq!(calls[n + 2].lookup, Some(name("o/r#2")));
    assert!(has_find(&calls[n + 3]));
    assert_eq!(secs(r.send_times()[n + 3]), 307);
}

// POL-16, STA-8, MOD-1: a loaded PR keeps its saved source; an ID tracked under another name is that PR.
#[tokio::test]
async fn pol16_loaded_and_renamed() {
    let mut w = World::default();
    w.open("o/r#1", 0);
    w.fail(unavailable(503));
    let r = start(Setup {
        world: w,
        loaded: loaded("o/r#1", Source::Manual),
        ..Setup::default()
    })
    .await;
    assert!(r.record("o/r#1").is_none(), "the first poll failed");
    let got = read_through(&r, "o/r#1");
    r.advance(2).await;
    assert_eq!(got.await.unwrap(), Ok(id("o/r#1")));
    assert_eq!(r.record("o/r#1").unwrap().source, Source::Manual);

    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(5).await;
    r.world().aliases.insert(name("o/new#1"), id("o/r#1"));
    let free = r.store.places_free();
    let got = read_through(&r, "o/new#1");
    r.clock.settle().await;
    assert_eq!(got.await.unwrap(), Ok(id("o/r#1")));
    assert_eq!(r.store.places_free(), free, "no new place");
    assert_eq!(r.record("o/r#1").unwrap().pr, name("o/new#1"));
}

// API-15: `track` reads through as `manual`; with a record it sends nothing, also at the limit.
#[tokio::test]
async fn api15_track() {
    let all = names("o/r", 199);
    let mut w = world_of(&refs(&all));
    let i = w.open("o/x#1", 0);
    w.unlisted.insert(i);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    let n = r.n_calls();
    let t = track(&r, "o/x#1");
    r.clock.settle().await;
    assert_eq!(t.await.unwrap(), Ok(id("o/x#1")));
    assert_eq!(r.n_calls(), n + 1);
    assert_eq!(r.calls()[n].lookup, Some(name("o/x#1")));
    assert_eq!(
        r.store.lookup(&name("o/x#1")).unwrap().source,
        Source::Manual
    );
    assert_eq!(r.store.places_free(), 0);
    let again = track(&r, "o/x#1");
    r.clock.settle().await;
    assert_eq!(again.await.unwrap(), Ok(id("o/x#1")));
    // An `authored` PR becomes `manual` too, at the limit, with nothing sent.
    let own = track(&r, "o/r#5");
    r.clock.settle().await;
    assert_eq!(own.await.unwrap(), Ok(id("o/r#5")));
    assert_eq!(
        r.store.lookup(&name("o/r#5")).unwrap().source,
        Source::Manual
    );
    let full = track(&r, "o/y#1");
    r.clock.settle().await;
    assert_eq!(code(&full.await.unwrap()), Some(RefusalCode::Limit));
    assert_eq!(r.n_calls(), n + 1);

    // A loaded PR while GitHub is unreachable: `manual` is saved first.
    let mut w = World::default();
    w.open("o/r#1", 0);
    w.fail(unavailable(503));
    w.fail(unavailable(503));
    let r = start(Setup {
        world: w,
        loaded: loaded("o/r#1", Source::Authored),
        ..Setup::default()
    })
    .await;
    let t = track(&r, "o/r#1");
    r.advance(2).await;
    assert_eq!(
        code(&t.await.unwrap()),
        Some(RefusalCode::GithubUnavailable)
    );
    assert_eq!(r.n_calls(), 2, "the read-through was sent");
    assert_eq!(r.store.file_content().prs[0].source, Source::Manual);
}

// API-13: what follows a waiter's arrival, by quota state.
#[tokio::test]
async fn api13_waiter_cadence() {
    // Slowed: every 20 s.
    let r = start(Setup {
        world: with_rate(world_of(&["o/r#1"]), 481, 4_999),
        ..Setup::default()
    })
    .await;
    r.world().rate = Some(answers::rate(0, 4_999));
    r.advance(2).await;
    let _waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(290).await;
    let every_20: Vec<(u64, bool)> = (1..=14).map(|k| (k * 20, false)).collect();
    assert_eq!(asks(&r, "o/r#1", from), every_20);

    // In reserve: every `idle_interval`, the polls' refreshes.
    let r = start(Setup {
        world: with_rate(world_of(&["o/r#1"]), 1, 400),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    let _waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(1_000).await;
    assert_eq!(
        asks(&r, "o/r#1", from),
        vec![(300, true), (600, true), (900, true)]
    );

    // Capped: no hot refresh; `refresh_pr` still sent, once per 30 s.
    let r = start(Setup {
        world: with_rate(world_of(&["o/r#1"]), 600, 4_999),
        ..Setup::default()
    })
    .await;
    r.world().rate = Some(answers::rate(0, 4_999));
    r.advance(2).await;
    let _waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(29).await;
    assert_eq!(r.n_calls(), from, "no hot refresh");
    for (wait, want) in [
        (0, Refreshed::Applied),
        (10, Refreshed::NotSent),
        (21, Refreshed::Applied),
    ] {
        r.advance(wait).await;
        let x = refresh(&r, "o/r#1");
        r.clock.settle().await;
        assert_eq!(x.await.unwrap(), want);
    }
    r.advance(100).await;
    assert_eq!(asks(&r, "o/r#1", from), vec![(31, false), (62, false)]);

    // Exhausted: nothing, and `refresh_pr` isn't sent.
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    r.world().fail(Failure::Exhausted {
        reset_at: Some(t0().plus(s(1_500))),
    });
    r.advance(290).await;
    let _waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    let from = r.n_calls();
    r.advance(100).await;
    let x = refresh(&r, "o/r#1");
    r.clock.settle().await;
    assert_eq!(x.await.unwrap(), Refreshed::NotSent);
    assert_eq!(r.n_calls(), from);
}

// POL-11: an `UNKNOWN` read gets one follow-up 3 s later, which never chains.
#[tokio::test]
async fn pol11_follow_up() {
    // A waiter's PR: each hot refresh that reads `UNKNOWN` gets its own; MOD-8 keeps `yes` meanwhile.
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    let _waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    r.world().pr("o/r#1").mergeable = Mergeable::Unknown;
    let from = r.n_calls();
    r.advance(38).await;
    assert_eq!(
        asks(&r, "o/r#1", from),
        vec![
            (10, false),
            (13, false),
            (23, false),
            (26, false),
            (36, false),
            (39, false)
        ]
    );
    assert_eq!(r.record("o/r#1").unwrap().mergeable, Mergeable::Yes);

    // At start: three PRs of one answer share one follow-up, as `Record`; later polls get none.
    let mut w = world_of(&["o/r#1", "o/r#2", "o/r#3"]);
    for n in ["o/r#1", "o/r#2", "o/r#3"] {
        w.pr(n).mergeable = Mergeable::Unknown;
    }
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(10).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(secs(r.send_times()[2]), 4);
    assert!(!has_find(&calls[2]) && calls[2].nodes.len() == 3);
    assert!(calls[2].nodes.iter().all(|n| n.want == Want::Record));
    r.advance(600).await;
    assert!(
        r.calls()[3..].iter().all(has_find),
        "no waiter, not named, not new"
    );

    // An ended PR gets none.
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    let _waiter = r.store.add_waiter(&id("o/r#1")).unwrap();
    r.engine.nudge();
    {
        let mut w = r.world();
        w.pr("o/r#1").mergeable = Mergeable::Unknown;
        w.set_state("o/r#1", ReadState::Merged);
    }
    let from = r.n_calls();
    r.advance(28).await;
    assert_eq!(
        asks(&r, "o/r#1", from),
        vec![(10, false), (20, false), (30, false)]
    );

    // Capped, or in reserve: none.
    for w in [
        with_rate(world_of(&["o/r#1", "o/r#2"]), 600, 4_999),
        with_rate(world_of(&["o/r#1", "o/r#2"]), 1, 400),
    ] {
        let mut w = w;
        w.pr("o/r#1").mergeable = Mergeable::Unknown;
        w.pr("o/r#2").mergeable = Mergeable::Unknown;
        let r = start(Setup {
            world: w,
            ..Setup::default()
        })
        .await;
        r.advance(10).await;
        assert_eq!(r.n_calls(), 2, "the finding and the first refresh");
    }
}

// POL-21: a node's `NOT_FOUND` on a hot refresh stays with that PR; the others are applied.
#[tokio::test]
async fn pol21_hot_node_error() {
    let prs = ["o/r#1", "o/r#2", "o/r#3"];
    let r = start(Setup {
        world: world_of(&prs),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    let _waiters: Vec<_> = prs
        .iter()
        .map(|n| r.store.add_waiter(&id(n)).unwrap())
        .collect();
    r.engine.nudge();
    r.world().errors.insert(id("o/r#2"), NodeAnswer::NotFound);
    let from = r.n_calls();
    r.advance(9).await;
    let calls = &r.calls()[from..];
    assert_eq!(calls.len(), 1);
    assert!(!has_find(&calls[0]) && calls[0].nodes.len() == 3);
    assert!(r.timing("o/r#2").attempt_failed);
    let err = r.record("o/r#2").unwrap().error.unwrap();
    assert_eq!(err.type_, "NOT_FOUND");
    for n in ["o/r#1", "o/r#3"] {
        let t = r.timing(n);
        assert!(!t.attempt_failed);
        assert_eq!(t.fetched, Some(at(10)));
    }
    assert_ne!(
        r.record("o/r#2").unwrap().state,
        crate::model::PrState::Gone
    );
}
