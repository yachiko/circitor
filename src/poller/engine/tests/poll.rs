//! The gate and the poll: POL-2, POL-4, POL-5, POL-7, POL-8, POL-10, POL-12, POL-13, STA-3.

use super::*;
use crate::github::Find;
use crate::model::{EventKind, ItemKind, ItemRead, Source};
use crate::poller::{PollOutcome, PollerHandle, ProblemCode};
use crate::state::{FileEntry, FileSource};
use crate::status::WarningCode;

fn comment(n: u64, secs: u64) -> ItemRead {
    ItemRead {
        kind: ItemKind::Comment,
        id: n,
        author: Some("someone".to_string()),
        at: Some(t0().plus(s(secs))),
        review_state: None,
        inline: None,
    }
}

fn wants(q: &Query) -> Vec<Want> {
    q.nodes.iter().map(|n| n.want.clone()).collect()
}

// POL-2: one request at a time, 1 s apart, 20 s at most.
#[tokio::test]
async fn pol2_gate() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let times = r.send_times();
    assert_eq!(times.len(), 2);
    assert!(times[1].since(times[0]) >= s(1));

    // The next poll (idle: 300 s) is held.
    let release = r.github.hold(&r.clock.activity());
    r.advance(299).await;
    assert_eq!(r.n_calls(), 3);
    r.engine.client_request();
    let joined = r.spawn_poll_now();
    r.advance(10).await;
    assert_eq!(
        r.n_calls(),
        3,
        "nothing else is sent while one is in flight"
    );
    assert!(!joined.is_finished());
    // POL-2: given up after 20 s of the clock.
    r.advance(10).await;
    let st = r.engine.status();
    assert_eq!(
        st.problem.map(|p| p.code),
        Some(ProblemCode::GithubUnavailable)
    );
    assert_eq!(r.n_calls(), 3);
    assert_eq!(joined.await.unwrap().outcome, PollOutcome::Shared);
    assert_eq!(
        r.lines("problem begin code=github_unavailable kind=timeout"),
        1
    );
    drop(release);
}

// POL-4, U2: found by polling, then refreshed with the items, then tracked in `pr` order.
#[tokio::test]
async fn pol4_first_poll() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    assert_eq!(r.n_calls(), 1);
    assert!(has_find(&r.calls()[0]) && r.calls()[0].nodes.is_empty());
    assert!(
        r.record("o/r#1").is_none(),
        "no record until its first refresh"
    );
    r.advance(1).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 2);
    assert!(!has_find(&calls[1]));
    assert_eq!(wants(&calls[1]), vec![Want::RecordAndItems; 2]);
    for n in ["o/r#1", "o/r#2"] {
        assert_eq!(r.record(n).unwrap().source, Source::Authored);
    }
    let tracked: Vec<PrName> = r
        .store
        .view()
        .events
        .iter()
        .filter(|e| matches!(e.kinds[..], [EventKind::Tracked { .. }]))
        .map(|e| e.pr.clone())
        .collect();
    assert_eq!(tracked, vec![name("o/r#1"), name("o/r#2")]);
}

// POL-4, STA-3: a failed first refresh frees the place; the PR is found again.
#[tokio::test]
async fn pol4_failed_first_refresh_is_found_again() {
    let mut w = world_of(&["o/r#1"]);
    w.errors.insert(
        id("o/r#1"),
        NodeAnswer::Error {
            type_: "INTERNAL".into(),
        },
    );
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    assert!(!r.tracked("o/r#1"));
    assert_eq!(r.store.places_free(), 200);
    r.world().errors.clear();
    r.advance(300).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(named(&calls[3]), vec![id("o/r#1")]);
    assert_eq!(wants(&calls[3]), vec![Want::RecordAndItems]);
    assert!(r.record("o/r#1").is_some());
}

// POL-4, STA-8: a loaded PR is in the first request, with the items.
#[tokio::test]
async fn pol4_loaded_pr_in_request_one() {
    let loaded = FileContent {
        prs: vec![FileEntry {
            pr: name("o/r#9"),
            id: id("o/r#9"),
            source: Source::Manual,
            login: Some("octo".into()),
            ended: None,
            seen: Default::default(),
        }],
    };
    let mut w = World::default();
    w.open("o/r#9", 0);
    w.unlisted.insert(id("o/r#9"));
    let r = start(Setup {
        world: w,
        loaded,
        ..Setup::default()
    })
    .await;
    let calls = r.calls();
    assert_eq!(calls.len(), 1);
    assert!(has_find(&calls[0]));
    assert_eq!(named(&calls[0]), vec![id("o/r#9")]);
    assert_eq!(wants(&calls[0]), vec![Want::RecordAndItems]);
    assert_eq!(r.record("o/r#9").unwrap().source, Source::Manual);
}

// POL-5: the owners filter, after finding; a transferred PR is demoted; none on a failed page.
#[tokio::test]
async fn pol5_owners() {
    let config = Config {
        owners: vec!["Acme".to_string()],
        ..Config::default()
    };
    let r = start(Setup {
        config,
        world: world_of(&["acme/x#1", "other/x#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    assert!(r.tracked("acme/x#1"));
    assert!(!r.tracked("other/x#1"));
    assert_eq!(r.calls()[1].nodes.len(), 1);
    // Transferred: same node ID, listed under `other/`.
    r.world().pr("acme/x#1").pr = name("other/y#7");
    r.advance(300).await;
    let rec = r.store.view().get(&id("acme/x#1")).unwrap().record.clone();
    assert_eq!(rec.pr, name("other/y#7"));
    assert_eq!(rec.source, Source::Asked);
}

// POL-5: no PR is demoted when page 2 of the finding fails.
#[tokio::test]
async fn pol5_no_demotion_when_page_two_fails() {
    let mut w = World::default();
    w.open("a/a#1", 0);
    // 101 more: with `a/a#1` unlisted, the finding still has two pages.
    for i in 1..=101 {
        w.open(&format!("o/r#{i}"), i);
    }
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    // Two finding pages, three first-refresh requests.
    r.advance(10).await;
    assert_eq!(r.n_calls(), 5);
    assert!(r.tracked("a/a#1"));
    {
        let mut w = r.world();
        w.unlisted.insert(id("a/a#1"));
        w.pass();
        w.fail(unavailable(502));
    }
    r.advance(292).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 7);
    assert_eq!(named(&calls[5])[0], id("a/a#1"));
    assert_eq!(
        calls[6].find,
        Some(Find {
            after: Some("o100".into())
        })
    );
    assert_eq!(r.record("a/a#1").unwrap().source, Source::Authored);
}

// POL-5: a PR merged between polls is unlisted, refreshed by ID, read merged, and keeps `authored`.
#[tokio::test]
async fn pol5_merged_keeps_authored() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    {
        let mut w = r.world();
        w.set_state("o/r#1", ReadState::Merged);
        w.unlisted.insert(id("o/r#2"));
        w.errors.insert(
            id("o/r#2"),
            NodeAnswer::Error {
                type_: "INTERNAL".into(),
            },
        );
    }
    r.advance(300).await;
    let calls = r.calls();
    assert_eq!(named(&calls[2]), vec![id("o/r#1"), id("o/r#2")]);
    let merged = r.record("o/r#1").unwrap();
    assert_eq!(merged.state, crate::model::PrState::Merged);
    assert_eq!(merged.source, Source::Authored);
    assert_eq!(r.record("o/r#2").unwrap().source, Source::Authored);
    let file = r.store.file_content();
    for n in ["o/r#1", "o/r#2"] {
        let e = file.prs.iter().find(|e| e.id == id(n)).unwrap();
        assert_eq!(e.source, Source::Authored);
    }
}

// POL-7: 40 a request; 100 found a page, the second with the first's cursor.
#[tokio::test]
async fn pol7_batches_and_pages() {
    let names: Vec<String> = (1..=41).map(|i| format!("o/r#{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let r = start(Setup {
        world: world_of(&refs),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!((calls[1].nodes.len(), calls[2].nodes.len()), (40, 1));
    r.advance(300).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 5);
    assert!(has_find(&calls[3]) && !has_find(&calls[4]));
    assert_eq!((calls[3].nodes.len(), calls[4].nodes.len()), (40, 1));

    let names: Vec<String> = (1..=150).map(|i| format!("o/r#{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let r = start(Setup {
        world: world_of(&refs),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    let calls = r.calls();
    assert_eq!(calls[0].find, Some(Find { after: None }));
    assert_eq!(
        calls[1].find,
        Some(Find {
            after: Some("o100".into())
        })
    );
    r.advance(10).await;
    assert_eq!(r.store.places_free(), 50);
}

// STA-3: the newest found get the places; paging stops when they're all taken; the warning clears.
#[tokio::test]
async fn sta3_places() {
    let names: Vec<String> = (1..=198).map(|i| format!("o/r#{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let r = start(Setup {
        world: world_of(&refs),
        ..Setup::default()
    })
    .await;
    r.advance(20).await;
    assert_eq!(r.store.places_free(), 2);
    {
        let mut w = r.world();
        for i in 1..=5u64 {
            w.open(&format!("n/n#{i}"), 1_000 + i);
        }
    }
    r.advance(280).await;
    r.advance(20).await;
    assert_eq!(r.store.places_free(), 0);
    assert!(r.tracked("n/n#5") && r.tracked("n/n#4"));
    for i in 1..=3 {
        assert!(!r.tracked(&format!("n/n#{i}")));
    }
    assert_eq!(r.warnings.list()[0].code, WarningCode::PrLimit);

    // Every place taken: page 2 isn't asked.
    let before = r.n_calls();
    r.advance(300).await;
    let calls = r.calls();
    assert!(calls[before..].iter().all(|q| q.find
        != Some(Find {
            after: Some("o100".into())
        })));
    assert!(calls[before..].iter().filter(|q| has_find(q)).count() == 1);

    // A poll that leaves none out clears it.
    {
        let mut w = r.world();
        for i in 1..=3 {
            w.prs.remove(&id(&format!("n/n#{i}")));
        }
        for i in 1..=103 {
            w.set_state(&format!("o/r#{i}"), ReadState::Closed);
        }
    }
    r.advance(300).await;
    r.advance(20).await;
    assert!(r.warnings.list().is_empty());
}

// POL-8: a PR's check pages are one read.
#[tokio::test]
async fn pol8_check_pages() {
    let mut w = world_of(&["o/r#1", "o/r#2", "o/r#3"]);
    w.checks.insert(id("o/r#1"), 187);
    w.checks.insert(id("o/r#2"), 400);
    w.checks.insert(id("o/r#3"), 50);
    let r = start(Setup {
        world: w,
        ..Setup::default()
    })
    .await;
    r.advance(5).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        wants(&calls[2]),
        vec![
            Want::ChecksPage {
                after: "k100".into()
            },
            Want::ChecksPage {
                after: "k100".into()
            }
        ]
    );
    assert_eq!(
        wants(&calls[3]),
        vec![Want::ChecksPage {
            after: "k200".into()
        }]
    );
    let one = r.record("o/r#1").unwrap();
    assert_eq!(
        (one.checks.total, one.checks.items.as_ref().unwrap().len()),
        (187, 187)
    );
    assert!(!one.checks.truncated);
    assert_eq!(r.events("o/r#1").len(), 1, "applied once");
    let two = r.record("o/r#2").unwrap();
    assert_eq!(two.checks.items.as_ref().unwrap().len(), 300);
    assert!(two.checks.truncated);

    // A failed second page applies nothing, and leaves `error` as it was.
    {
        let mut w = r.world();
        w.checks.insert(id("o/r#3"), 150);
        w.page_errors.insert(id("o/r#3"));
    }
    let rev = r.store.view().rev;
    r.advance(300).await;
    let three = r.record("o/r#3").unwrap();
    assert_eq!(three.checks.total, 50);
    assert_eq!(three.error, None);
    assert!(r.timing("o/r#3").attempt_failed);
    assert!(r.events("o/r#3").iter().all(|e| e.rev <= rev));
}

// POL-12: a raised count fetches the items straight after; one event carries both.
#[tokio::test]
async fn pol12_items() {
    let r = start(Setup {
        world: world_of(&["o/r#1"]),
        ..Setup::default()
    })
    .await;
    r.advance(1).await;
    {
        let mut w = r.world();
        w.pr("o/r#1").counts.comments = 1;
        w.items.insert(
            id("o/r#1"),
            ItemsRead {
                comments: vec![comment(11, 100)],
                reviews: vec![],
            },
        );
    }
    r.advance(300).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(wants(&calls[3]), vec![Want::Items]);
    assert!(
        r.send_times()[3].since(r.send_times()[2]) == s(1),
        "straight after"
    );
    let events = r.events("o/r#1");
    let last = events.last().unwrap();
    assert_eq!(events.len(), 2);
    assert!(last.kinds.iter().any(|k| matches!(k, EventKind::Activity { items, counts, .. } if items.len() == 1 && counts.comments == 1)));

    // A failed part: applied with the old counts; the next refresh asks again.
    {
        let mut w = r.world();
        w.pr("o/r#1").counts.comments = 2;
        w.item_errors.insert(id("o/r#1"));
    }
    r.advance(300).await;
    assert_eq!(r.n_calls(), 6);
    assert_eq!(r.record("o/r#1").unwrap().counts.comments, 1);
    r.world().item_errors.clear();
    r.advance(300).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 8);
    assert_eq!(wants(&calls[7]), vec![Want::Items]);
    assert_eq!(r.record("o/r#1").unwrap().counts.comments, 2);
}

// POL-12, POL-7: 41 PRs with raised counts are two `Items` requests.
#[tokio::test]
async fn pol12_items_batched() {
    let names: Vec<String> = (1..=41).map(|i| format!("o/r#{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let r = start(Setup {
        world: world_of(&refs),
        ..Setup::default()
    })
    .await;
    r.advance(2).await;
    {
        let mut w = r.world();
        for n in &names {
            w.pr(n).counts.reviews = 1;
        }
    }
    r.advance(310).await;
    let calls = r.calls();
    let items: Vec<usize> = calls
        .iter()
        .filter(|q| q.nodes.iter().all(|n| n.want == Want::Items) && !q.nodes.is_empty())
        .map(|q| q.nodes.len())
        .collect();
    assert_eq!(items, vec![40, 1]);
}

// POL-13: with the world unchanged, a poll is one request (P10).
#[tokio::test]
async fn pol13_one_request_per_poll() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2", "o/r#3"]),
        ..Setup::default()
    })
    .await;
    r.engine.client_request();
    r.advance(1).await;
    assert_eq!(r.n_calls(), 2);
    let t = r.timing("o/r#1");
    assert_eq!(t.last_sent, Some(at(0)));
    assert_eq!(t.fetched, Some(at(1)));
    r.keep_active(95, 5).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 5);
    for q in &calls[2..] {
        assert!(has_find(q));
        assert_eq!(q.nodes.len(), 3);
    }
    assert_eq!(&r.send_times()[2..], &[at(30), at(60), at(90)]);
}

// POL-10: a PR the open list shows is refreshed in that poll as an open one.
#[tokio::test]
async fn pol10_listed_as_open() {
    let r = start(Setup {
        world: world_of(&["o/r#1", "o/r#2"]),
        ..Setup::default()
    })
    .await;
    r.engine.client_request();
    r.advance(1).await;
    r.world().set_state("o/r#1", ReadState::Merged);
    r.keep_active(35, 5).await;
    assert_eq!(
        r.record("o/r#1").unwrap().state,
        crate::model::PrState::Merged
    );
    assert_eq!(r.timing("o/r#1").last_sent, Some(at(30)));
    // Not listed at 60: not due either (300 s).
    r.keep_active(30, 5).await;
    assert_eq!(r.n_calls(), 4);
    assert_eq!(named(&r.calls()[3]), vec![id("o/r#2")]);
    // Reopened: listed at 90, sent 60 s ago, refreshed in that poll.
    r.world().set_state("o/r#1", ReadState::Open);
    r.keep_active(30, 5).await;
    let calls = r.calls();
    assert_eq!(calls.len(), 6);
    assert_eq!(named(&calls[5]), vec![id("o/r#1")]);
    assert_eq!(
        r.record("o/r#1").unwrap().state,
        crate::model::PrState::Open
    );
    let _ = r.handle().status();
}
