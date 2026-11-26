#![cfg(feature = "test-endpoint")]
//! T26, milestone M1: the engine polls the fake GitHub through the real client, in one process on
//! the test clock. The fake shares no type with the client, so a wrong field name, alias or path on
//! either side fails here (TST-12). No API, no socket, no daemon: T28 adds those.

use std::sync::Arc;
use std::time::Duration;

use circitor::clock::{Clock, SharedClock, SleepWatch, TestClock, Time};
use circitor::config::Config;
use circitor::log::{BufferSink, Level, Logger};
use circitor::model::{
    CheckKind, ChecksState, EventKind, Mergeable, NodeId, Phase, PrName, PrState, Record, Review,
    Source, Status,
};
use circitor::poller::{Engine, EngineDeps, FirstPoll, PollerHandle, cadence};
use circitor::state::{FileContent, FileSource, Store, StoreDeps, View};
use circitor::status::Warnings;
use circitor::testkit::github::{AskedOf, PrId, World};
use circitor::testkit::server::{FakeGitHub, Fault, Request, Times};
use circitor::token::fake::FakeTokenSource;

/// The measured heads of PR A's second push (docs/05, 2026-11-24), made 40 characters long.
const OLD_HEAD: &str = "dce9dbe0000000000000000000000000000000a1";
const NEW_HEAD: &str = "cf54ec40000000000000000000000000000000a2";

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

fn name(n: &str) -> PrName {
    PrName::parse(n).unwrap()
}

/// The wiring: everything the engine needs, real but for GitHub (the fake, over loopback HTTP) and
/// the token source.
struct Rig {
    clock: Arc<TestClock>,
    gh: FakeGitHub,
    store: Arc<Store>,
    config: Arc<Config>,
    engine: Arc<Engine>,
    log: BufferSink,
}

impl Rig {
    /// The fake GitHub first, scripted by `script` before the engine's first request.
    fn start(script: impl FnOnce(&mut World)) -> Rig {
        let clock = TestClock::new(t0());
        let shared: SharedClock = clock.clone();
        let gh = FakeGitHub::start(clock.clone());
        gh.world(script);
        let log = BufferSink::default();
        let logger = Logger::new(shared.clone(), Arc::new(log.clone()), Level::Debug);
        let warnings = Warnings::new(shared.clone(), logger.clone());
        let config = Arc::new(Config::default());
        let store = Store::new(
            StoreDeps {
                clock: shared.clone(),
                logger: logger.clone(),
                config: config.clone(),
                seed: 26,
            },
            FileContent::default(),
        );
        let engine = Engine::spawn(EngineDeps {
            clock: shared.clone(),
            github: Arc::new(gh.client()),
            token_source: FakeTokenSource::new(shared.clone()),
            store: store.clone(),
            config: config.clone(),
            logger,
            warnings,
            activity: clock.activity(),
            sleep: SleepWatch::new(shared),
        });
        Rig {
            clock,
            gh,
            store,
            config,
            engine,
            log,
        }
    }

    async fn advance(&self, d: Duration) {
        self.clock.advance(d).await;
    }

    fn view(&self) -> View {
        self.store.view()
    }

    fn record(&self, n: &str) -> Arc<Record> {
        self.view()
            .find(&name(n))
            .unwrap_or_else(|| panic!("{n} is tracked with a record"))
            .record
            .clone()
    }

    fn id(&self, n: &str) -> NodeId {
        self.view().find(&name(n)).expect("tracked").id.clone()
    }

    /// MOD-16, by `cadence::stale` with the engine's tier now.
    fn stale(&self, n: &str) -> bool {
        let view = self.view();
        let pv = view.find(&name(n)).expect("tracked");
        let tier = self.engine.status().tier;
        cadence::stale(
            &pv.record,
            &pv.timing,
            tier,
            &self.config,
            self.clock.mono(),
        )
    }

    /// The PR's events, oldest first.
    fn events_of(&self, n: &str) -> Vec<circitor::model::Event> {
        let view = self.view();
        let id = view.find(&name(n)).expect("tracked").id.clone();
        view.events_after(0, Some(&id))
            .into_iter()
            .cloned()
            .collect()
    }

    /// The poll requests: those that hold a finding.
    fn polls(&self) -> Vec<Request> {
        self.gh
            .requests()
            .into_iter()
            .filter(|r| r.finding.is_some())
            .collect()
    }

    /// TST-12: every answer the client got decoded. A body it couldn't read is a whole-request
    /// failure (`result=` not `ok`), a node it couldn't read a PR's error on the record.
    fn assert_decoded(&self) {
        let log = self.log.contents();
        let faulted = self
            .gh
            .requests()
            .iter()
            .filter(|r| r.fault.is_some())
            .count();
        let failed = log
            .lines()
            .filter(|l| l.contains(" request kind=") && !l.contains("result=ok"))
            .count();
        assert_eq!(
            failed, faulted,
            "every answer not faulted on purpose decodes:\n{log}"
        );
        for pv in &self.view().prs {
            assert!(
                pv.record.error.is_none(),
                "{}'s answer decoded, yet it holds an error: {:?}",
                pv.record.pr,
                pv.record.error
            );
        }
        assert!(
            self.gh.requests().iter().all(|r| r.rejected.is_none()),
            "every request is one the schema and the fake take"
        );
    }

    /// Every test ends here: the decode check, then the fake's rules (POL-1, POL-2).
    async fn finish(self) {
        self.assert_decoded();
        self.engine.stop().await;
        self.gh.check();
    }
}

fn kinds(e: &circitor::model::Event) -> Vec<&'static str> {
    e.kinds.iter().map(EventKind::word).collect()
}

fn status_to(e: &circitor::model::Event) -> Option<Status> {
    e.kinds.iter().find_map(|k| match k {
        EventKind::Status { to, .. } => Some(*to),
        _ => None,
    })
}

fn checks_state(e: &circitor::model::Event) -> Option<ChecksState> {
    e.kinds.iter().find_map(|k| match k {
        EventKind::Checks { state, .. } => Some(*state),
        _ => None,
    })
}

/// `n` open PRs of the viewer in `o/r`, untouched for an hour and more, `MERGEABLE`, no checks.
fn quiet_prs(w: &mut World, n: usize) -> Vec<PrId> {
    let repo = w.repo("o", "r");
    let now = w.now();
    let prs = w.open_prs(repo, n, s(60));
    for (i, &pr) in prs.iter().enumerate() {
        let t = now.minus(s(3_600 + 60 * i as u64));
        w.set_created_at(pr, t);
        w.set_updated_at(pr, t);
    }
    prs
}

// POL-4: one request that finds, a second straight after for the PRs found, each costed; MOD-6's
// shortcut by the `Date` header; MOD-15's `tracked` events in the order of `pr`.
#[tokio::test]
async fn m1_found_and_tracked() {
    let rig = Rig::start(|w| {
        let repo = w.repo("o", "r");
        let now = w.now();
        let one = w.open_pr(repo);
        w.set_created_at(one, now.minus(s(600)));
        w.set_updated_at(one, now.minus(s(120)));
        for check in ["build", "test"] {
            let run = w.run(one, check);
            w.finish(run, "SUCCESS");
        }
        let two = w.open_pr(repo);
        w.set_created_at(two, now.minus(s(5)));
        w.set_updated_at(two, now.minus(s(5)));
    });
    rig.advance(s(5)).await;

    let one = rig.record("o/r#1");
    let two = rig.record("o/r#2");
    assert_eq!(one.source, Source::Authored);
    assert_eq!(two.source, Source::Authored);
    // MOD-6: `updatedAt` 2 minutes old by the `Date` header counts as settled at once.
    assert!(one.checks.settled);
    assert_eq!(one.checks.state, ChecksState::Passing);
    assert_eq!(one.status, Status::Ready);
    // MOD-6: created 5 s ago with no checks: not settled.
    assert!(!two.checks.settled);
    assert_eq!(two.status, Status::ChecksPending);

    let view = rig.view();
    let events: Vec<_> = view.events_after(0, None).into_iter().cloned().collect();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(
        (events[0].rev, events[0].pr.to_string()),
        (1, "o/r#1".into())
    );
    assert_eq!(
        (events[1].rev, events[1].pr.to_string()),
        (2, "o/r#2".into())
    );
    for e in &events {
        assert_eq!(
            e.kinds,
            vec![EventKind::Tracked {
                source: Source::Authored
            }]
        );
    }
    assert_eq!(view.cursor().rev, 2);

    // POL-4: the finding alone, then both found PRs with their items, 1 s or more later.
    let reqs = rig.gh.requests();
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert_eq!(reqs[0].finding, Some(None));
    assert!(reqs[0].nodes.is_empty());
    assert_eq!(reqs[1].finding, None);
    let mut asked = reqs[1].nodes.clone();
    asked.sort_by(|a, b| a.0.cmp(&b.0));
    let mut want = vec![
        (rig.id("o/r#1").0, AskedOf::RecordAndItems),
        (rig.id("o/r#2").0, AskedOf::RecordAndItems),
    ];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(asked, want);
    assert!(reqs[1].mono.since(reqs[0].mono) >= s(1));
    assert_eq!((reqs[0].cost, reqs[1].cost), (1, 1));
    assert_eq!(*rig.engine.first_poll().borrow(), FirstPoll::Finished);
    rig.finish().await;
}

// POL-6, MOD-2, MOD-3, TST-12: every field of the record, as scripted, through the real query and
// decoding.
#[tokio::test]
async fn m1_record_fields() {
    let mut ids = (0, 0, 0);
    let mut head = String::new();
    let rig = Rig::start(|w| {
        let repo = w.repo("o", "r");
        let now = w.now();
        let pr = w.open_pr(repo);
        w.set_base(pr, "release-2");
        w.set_head_ref(pr, "feature-x");
        w.set_draft(pr, true);
        w.set_review_decision(pr, Some("CHANGES_REQUESTED"));
        w.set_mergeable(pr, "CONFLICTING");
        let run = w.run(pr, "build");
        w.finish(run, "FAILURE");
        w.status(pr, "ci/lint", "SUCCESS");
        let comment = w.comment(pr);
        let review = w.review(pr, "CHANGES_REQUESTED", 1);
        w.set_created_at(pr, now.minus(s(3_600)));
        w.set_updated_at(pr, now.minus(s(600)));
        ids = (w.run_database_id(run), comment, review);
        head = w.head(pr).to_owned();
    });
    rig.advance(s(5)).await;

    let r = rig.record("o/r#1");
    assert_eq!(r.pr, name("o/r#1"));
    assert_eq!(r.url, "https://github.com/o/r/pull/1");
    assert_eq!(r.title, "MARKER-TITLE #1");
    assert_eq!(r.author.as_deref(), Some("marker-login"));
    assert_eq!(r.state, PrState::Open);
    assert!(r.draft);
    assert_eq!(r.base, "release-2");
    assert_eq!(r.head, "feature-x");
    assert_eq!(r.head_sha.len(), 40);
    assert_eq!(r.head_sha, head);
    assert_eq!(r.created_at, Some(t0().minus(s(3_600))));
    assert_eq!(r.updated_at, Some(t0().minus(s(600))));
    assert_eq!(r.merged_at, None);
    assert_eq!(r.closed_at, None);
    assert_eq!(r.mergeable, Mergeable::No);
    assert_eq!(r.merge_state, "dirty");
    assert_eq!(r.review, Review::ChangesRequested);
    assert_eq!(r.counts.comments, 1);
    assert_eq!(r.counts.reviews, 1);
    assert_eq!(r.counts.review_threads, 1);
    assert_eq!(r.status, Status::Conflict);
    assert_eq!(r.source, Source::Authored);
    assert!(r.error.is_none());

    // MOD-3: the run with its `databaseId`, the status without one.
    assert_eq!(r.checks.total, 2);
    assert_eq!(r.checks.completed, 2);
    assert_eq!(r.checks.failed, 1);
    assert_eq!(r.checks.state, ChecksState::Failing);
    let items = r.checks.items.as_ref().expect("the items are kept");
    assert_eq!(items.len(), 2, "{items:?}");
    let run = items.iter().find(|i| i.kind == CheckKind::Run).unwrap();
    assert_eq!(run.id, Some(ids.0));
    assert_eq!(run.name, "build");
    assert_eq!(run.phase, Phase::Completed);
    assert_eq!(run.result.as_deref(), Some("failure"));
    assert!(
        run.url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://github.com/o/r/"))
    );
    let status = items.iter().find(|i| i.kind == CheckKind::Status).unwrap();
    assert_eq!(status.id, None);
    assert_eq!(status.name, "ci/lint");
    assert_eq!(status.phase, Phase::Completed);
    assert_eq!(status.result.as_deref(), Some("success"));
    let failed: Vec<_> = r
        .checks
        .failed_items
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(failed, vec!["build"]);
    rig.finish().await;
}

// POL-4, MOD-6, MOD-8, MOD-15: PR A's second push (docs/05, 2026-11-24, 22:13:50 to 22:14:43) as
// the active tier's polls see it: the old head served for 3 s, then the new head with `UNKNOWN` and
// no rollup, `MERGEABLE` and `PENDING` from +6 s, both checks passed at +52 s.
#[tokio::test]
async fn m1_push() {
    // The push lands 1.5 s before the poll at 90 s, so that poll still reads the old head.
    let push_at = t0().plus(ms(88_500));
    let rig = Rig::start(|w| {
        let repo = w.repo("o", "r");
        let now = w.now();
        let a = w.open_pr(repo);
        w.push_sha(a, OLD_HEAD);
        for check in ["fast", "slow"] {
            let run = w.run(a, check);
            w.finish(run, "SUCCESS");
        }
        // docs/05: created at 22:09:25, last pushed at 22:09:23; here as long before the push.
        w.set_created_at(a, now.minus(s(200)));
        w.set_updated_at(a, now.minus(s(200)));
        w.at(push_at, move |w| {
            w.push_lagged(a, Some(NEW_HEAD), s(3));
        });
        w.at(push_at.plus(s(3)), move |w| w.set_mergeable(a, "UNKNOWN"));
        w.at(push_at.plus(s(6)), move |w| {
            w.set_mergeable(a, "MERGEABLE");
            let runs: Vec<_> = ["fast", "slow"].iter().map(|c| w.run(a, c)).collect();
            for &run in &runs {
                w.start(run);
            }
            w.at(push_at.plus(s(52)), move |w| {
                for run in runs {
                    w.finish(run, "SUCCESS");
                }
            });
        });
    });
    rig.engine.client_request();
    rig.advance(s(60)).await;
    let before = rig.record("o/r#1");
    assert_eq!(before.status, Status::Ready);
    assert_eq!(before.head_sha, OLD_HEAD);
    let rev_before = rig.view().rev;

    // The poll at 90 s, 1.5 s after the push, reads the old head: no event.
    rig.engine.client_request();
    rig.advance(s(31)).await;
    assert_eq!(rig.polls().last().unwrap().mono.as_duration(), s(90));
    assert_eq!(rig.record("o/r#1").head_sha, OLD_HEAD);
    assert_eq!(
        rig.view().rev,
        rev_before,
        "a poll that read the old head made no event"
    );

    // Over the next two minutes.
    rig.engine.client_request();
    rig.advance(s(120)).await;
    let events: Vec<_> = rig
        .events_of("o/r#1")
        .into_iter()
        .filter(|e| e.rev > rev_before)
        .collect();
    let head = events
        .iter()
        .position(|e| {
            e.kinds.iter().any(|k| {
                matches!(k, EventKind::Head { from, to } if from == "dce9dbe" && to == "cf54ec4")
            })
        })
        .unwrap_or_else(|| panic!("a head event: {events:?}"));
    assert_eq!(head, 0, "{events:?}");
    assert_eq!(
        status_to(&events[head]),
        Some(Status::ChecksPending),
        "{events:?}"
    );
    let pending = (head..events.len())
        .find(|&i| checks_state(&events[i]) == Some(ChecksState::Pending))
        .unwrap_or_else(|| panic!("a checks pending event: {events:?}"));
    let passing = (pending + 1..events.len())
        .find(|&i| checks_state(&events[i]) == Some(ChecksState::Passing))
        .unwrap_or_else(|| panic!("a checks passing event after pending: {events:?}"));
    // MOD-6: `ready` waits for a settled list.
    for e in &events[..passing] {
        assert_ne!(
            status_to(e),
            Some(Status::Ready),
            "ready before passing: {events:?}"
        );
    }
    let end = rig.record("o/r#1");
    assert_eq!(end.status, Status::Ready);
    assert_eq!(end.head_sha, NEW_HEAD);
    rig.finish().await;
}

// POL-10, MOD-8, POL-5: PR B merges between two polls; the next poll's finding no longer lists it,
// and the same poll refreshes it by ID.
#[tokio::test]
async fn m1_merge() {
    let rig = Rig::start(|w| {
        let repo = w.repo("o", "r");
        let now = w.now();
        let pr = w.open_pr(repo);
        w.set_created_at(pr, now.minus(s(120)));
        w.set_updated_at(pr, now.minus(s(120)));
        // docs/05: merged 15.7 s after `gh pr create`; here between the polls at 30 and 60 s.
        w.at(now.plus(s(45)), move |w| w.merge(pr));
    });
    rig.engine.client_request();
    rig.advance(s(31)).await;
    let id = rig.id("o/r#1");
    let open = rig.record("o/r#1");
    assert_eq!(open.state, PrState::Open);
    assert_eq!(open.mergeable, Mergeable::Yes);
    let rev_open = rig.view().rev;
    let first_refreshes = rig
        .gh
        .requests()
        .iter()
        .filter(|r| r.nodes.iter().any(|n| n.1 == AskedOf::RecordAndItems))
        .count();

    rig.engine.client_request();
    rig.advance(s(30)).await;
    // The poll at 60 s: one request, its finding and B by ID.
    let poll = rig.polls().last().cloned().unwrap();
    assert_eq!(poll.mono.as_duration(), s(60));
    assert_eq!(poll.nodes, vec![(id.0.clone(), AskedOf::Record)]);
    assert_eq!(rig.gh.requests().last(), Some(&poll), "nothing after it");
    // Off the list, it isn't found again as new: no further first refresh.
    let after = rig
        .gh
        .requests()
        .iter()
        .filter(|r| r.nodes.iter().any(|n| n.1 == AskedOf::RecordAndItems))
        .count();
    assert_eq!(after, first_refreshes);

    let r = rig.record("o/r#1");
    assert_eq!(r.state, PrState::Merged);
    assert_eq!(r.status, Status::Merged);
    assert_eq!(r.mergeable, Mergeable::Unknown);
    assert_eq!(r.merge_state, "unknown");
    assert!(r.merged_at.is_some());
    let events: Vec<_> = rig
        .events_of("o/r#1")
        .into_iter()
        .filter(|e| e.rev > rev_open)
        .collect();
    assert_eq!(events.len(), 1, "{events:?}");
    let words = kinds(&events[0]);
    assert!(
        words.contains(&"status") && words.contains(&"merged"),
        "{words:?}"
    );
    assert!(
        !words.contains(&"mergeable"),
        "MOD-8: no mergeable kind: {words:?}"
    );
    assert_eq!(status_to(&events[0]), Some(Status::Merged));
    // POL-5: not demoted for leaving the list.
    assert_eq!(r.source, Source::Authored);
    let saved = rig.store.file_content();
    let entry = saved.prs.iter().find(|e| e.id == id).expect("saved");
    assert_eq!(entry.source, Source::Authored);
    rig.finish().await;
}

// POL-13: the tiers, counted from the fake's request log.
#[tokio::test]
async fn m1_tiers() {
    let rig = Rig::start(|w| {
        quiet_prs(w, 10);
    });
    // A client request every 5 minutes for 15 minutes: the last at 900 s.
    for _ in 0..4 {
        rig.engine.client_request();
        rig.advance(s(300)).await;
    }
    // Then nothing: idle from 10 minutes after the last, 1 500 s.
    rig.advance(s(1_500)).await;
    let reqs = rig.gh.requests();
    // The first poll's two requests, then one request a poll (P10).
    assert_eq!(reqs[0].mono.as_duration(), s(0));
    assert!(reqs[0].finding.is_some() && reqs[1].finding.is_none());
    for r in &reqs[2..] {
        assert!(
            r.finding.is_some(),
            "every later poll is one request: {r:?}"
        );
        assert_eq!(r.nodes.len(), 10, "{r:?}");
    }
    let sent: Vec<u64> = reqs[2..]
        .iter()
        .map(|r| r.mono.as_duration().as_secs())
        .collect();
    let mut last = 0;
    for &t in &sent {
        let gap = t - last;
        last = t;
        if t <= 1_500 {
            assert_eq!(gap, 30, "active: 30 s apart, at {t}: {sent:?}");
        } else {
            assert_eq!(gap, 300, "idle: 300 s apart, at {t}: {sent:?}");
        }
    }
    assert!(sent.contains(&1_470), "{sent:?}");
    assert!(sent.iter().filter(|&&t| t > 1_500).count() >= 4, "{sent:?}");
    rig.finish().await;
}

// TST-6's first line, as a smoke test: an idle hour and an active hour, from `points()`.
#[tokio::test]
async fn m1_hour_in_budget() {
    let rig = Rig::start(|w| {
        quiet_prs(w, 10);
    });
    rig.advance(s(5)).await;
    assert_eq!(rig.gh.requests().len(), 2);
    let (p0, n0) = (rig.gh.points(), rig.polls().len());

    rig.advance(s(3_600)).await;
    let (p1, n1) = (rig.gh.points(), rig.polls().len());
    let (idle_points, idle_polls) = (p1 - p0, n1 - n0);
    assert!(idle_points <= 15, "an idle hour costs {idle_points}");
    assert!(idle_polls >= 11, "an idle hour sends {idle_polls} polls");

    for _ in 0..12 {
        rig.engine.client_request();
        rig.advance(s(300)).await;
    }
    let (p2, n2) = (rig.gh.points(), rig.polls().len());
    let (active_points, active_polls) = (p2 - p1, n2 - n1);
    assert!(active_points <= 130, "an active hour costs {active_points}");
    assert!(
        active_polls >= 110,
        "an active hour sends {active_polls} polls"
    );
    // P10: every poll after the first is one request.
    assert_eq!(rig.gh.requests().len(), 2 + n2 - n0);
    rig.finish().await;
}

// POL-19: one 500; the records turn stale, the next try comes 5 s later, and the poll after it
// clears them.
#[tokio::test]
async fn m1_one_bad_answer() {
    let rig = Rig::start(|w| {
        quiet_prs(w, 2);
    });
    rig.engine.client_request();
    rig.advance(s(5)).await;
    assert!(!rig.stale("o/r#1") && !rig.stale("o/r#2"));
    rig.gh.fault(Fault::Status(500), Times::Requests(1));

    rig.advance(s(26)).await;
    let reqs = rig.gh.requests();
    let bad = reqs.last().unwrap();
    assert_eq!(bad.mono.as_duration(), s(30));
    assert_eq!(bad.fault, Some(Fault::Status(500)));
    assert!(
        rig.stale("o/r#1") && rig.stale("o/r#2"),
        "MOD-16: the failed request's PRs"
    );

    rig.advance(s(5)).await;
    let reqs = rig.gh.requests();
    let retry = reqs.last().unwrap();
    assert_eq!(retry.mono.as_duration(), s(35), "the next try 5 s later");
    assert_eq!(retry.fault, None);
    assert!(retry.finding.is_some(), "the try is a poll");
    // POL-13: the failed request counts as sent at 30 s, so the try at 35 s asks no node and the
    // records stay stale until the poll after it, the next poll due (30 s after the try).
    assert!(retry.nodes.is_empty(), "{retry:?}");
    assert!(rig.stale("o/r#1") && rig.stale("o/r#2"));

    rig.advance(s(30)).await;
    let reqs = rig.gh.requests();
    let after = reqs.last().unwrap();
    assert_eq!(after.mono.as_duration(), s(65), "{reqs:?}");
    assert_eq!(after.nodes.len(), 2, "{after:?}");
    assert!(
        !rig.stale("o/r#1") && !rig.stale("o/r#2"),
        "the poll after it clears them"
    );
    rig.finish().await;
}

// POL-1, POL-2: across a busy timeline the fake recorded no mutation, no overlap, no gap under 1 s
// and no query the schema refuses. `check()` asserts it at every test's end; this one says so too.
#[tokio::test]
async fn m1_rules_held() {
    let rig = Rig::start(|w| {
        let prs = quiet_prs(w, 3);
        let (a, b) = (prs[0], prs[1]);
        let now = w.now();
        w.at(now.plus(s(40)), move |w| {
            w.push(a);
            let run = w.run(a, "build");
            w.start(run);
        });
        w.at(now.plus(s(75)), move |w| {
            w.comment(b);
            w.review(b, "APPROVED", 2);
        });
        w.at(now.plus(s(130)), move |w| w.merge(b));
    });
    rig.engine.client_request();
    rig.advance(s(240)).await;
    assert!(rig.gh.broken().is_empty(), "{:?}", rig.gh.broken());
    let reqs = rig.gh.requests();
    assert!(reqs.len() >= 9, "{reqs:?}");
    for pair in reqs.windows(2) {
        assert!(pair[1].mono.since(pair[0].mono) >= s(1), "{pair:?}");
    }
    rig.finish().await;
}
