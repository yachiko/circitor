//! The merge's tests: tables over single records (TST-15), and read sequences with no clock and no
//! fake GitHub.

use std::time::Duration;

use super::derive::{self, checks, status};
use super::items::item;
use super::{apply_gone, apply_read, counts_rose};
use crate::clock::{Mono, Time};
use crate::model::tests::sample_record;
use crate::model::{
    Before, Carry, CheckItem, CheckKind, Checks, ChecksRead, ChecksState, Counts, EventKind,
    FailedItem, ItemKind, ItemRead, ItemsOutcome, ItemsRead, Known, Mergeable, Merged, NodeId,
    Phase, PrName, PrRead, PrState, ReadMeta, ReadState, Record, Review, Seen, SeenKind, Source,
    Status,
};

const SHA_A: &str = "aaaaaaa1111111111111111111111111111111aa";
const SHA_B: &str = "bbbbbbb2222222222222222222222222222222bb";

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

fn ms(secs: f64) -> Duration {
    Duration::from_millis((secs * 1000.0).round() as u64)
}

/// A request sent `secs` after the start on both clocks, with no `Date`.
fn meta(seq: u64, secs: f64) -> ReadMeta {
    ReadMeta {
        seq,
        sent_wall: t0().plus(ms(secs)),
        sent_mono: Mono::ZERO.plus(ms(secs)),
        github_date: None,
    }
}

fn run(id: u64, name: &str, phase: Phase, result: Option<&str>) -> CheckItem {
    CheckItem {
        kind: CheckKind::Run,
        id: Some(id),
        name: name.into(),
        phase,
        result: result.map(str::to_string),
        url: Some(format!("https://x/{id}")),
    }
}

fn done(id: u64, name: &str, result: &str) -> CheckItem {
    run(id, name, Phase::Completed, Some(result))
}

fn commit_status(name: &str, phase: Phase, result: Option<&str>) -> CheckItem {
    CheckItem {
        kind: CheckKind::Status,
        id: None,
        name: name.into(),
        phase,
        result: result.map(str::to_string),
        url: None,
    }
}

fn list(items: Vec<CheckItem>) -> ChecksRead {
    ChecksRead {
        total: items.len() as u32,
        items,
        rollup: None,
        next_page: None,
    }
}

/// An open PR that reads `ready` once settled: one passing check, mergeable, clean, no review.
fn read() -> PrRead {
    PrRead {
        id: NodeId("PR_1".into()),
        pr: PrName::parse("yachiko/circitor#12").unwrap(),
        url: "https://github.com/yachiko/circitor/pull/12".into(),
        title: "A title".into(),
        author: Some("someone".into()),
        state: ReadState::Open,
        draft: false,
        base: "main".into(),
        head: "feature".into(),
        head_sha: SHA_A.into(),
        created_at: Some(t0()),
        updated_at: Some(t0()),
        merged_at: None,
        closed_at: None,
        mergeable: Mergeable::Yes,
        merge_state: "clean".into(),
        review: Review::None,
        counts: Counts::default(),
        checks: list(vec![done(1, "test", "success")]),
    }
}

fn applied(m: Merged) -> (Record, Carry, Vec<EventKind>) {
    match m {
        Merged::Applied {
            record,
            carry,
            kinds,
        } => (record, carry, kinds),
        Merged::Older => panic!("expected an applied read"),
    }
}

const NO_ITEMS: ItemsRead = ItemsRead {
    comments: Vec::new(),
    reviews: Vec::new(),
};

/// A first read (`New`) at 0 s.
fn first(r: &PrRead) -> (Record, Carry) {
    let (rec, carry, _) = applied(apply_read(
        Before::New {
            source: Source::Authored,
        },
        &Carry::default(),
        r,
        ItemsOutcome::Read(&NO_ITEMS),
        &meta(1, 0.0),
    ));
    (rec, carry)
}

/// A later read of a known record, with no items asked.
fn again(rec: &Record, carry: &Carry, r: &PrRead, m: ReadMeta) -> (Record, Carry, Vec<EventKind>) {
    applied(apply_read(
        Before::Known(rec),
        carry,
        r,
        ItemsOutcome::NotAsked,
        &m,
    ))
}

/// A record that reads `ready`, settled.
fn settled_ready() -> (Record, Carry) {
    let (rec, carry) = first(&read());
    let (rec, carry, kinds) = again(&rec, &carry, &read(), meta(2, 20.0));
    assert_eq!(rec.status, Status::Ready);
    assert_eq!(kinds.len(), 1);
    (rec, carry)
}

fn words(kinds: &[EventKind]) -> Vec<&'static str> {
    kinds.iter().map(EventKind::word).collect()
}

fn comment(id: u64, at: Time) -> ItemRead {
    ItemRead {
        kind: ItemKind::Comment,
        id,
        author: Some("a".into()),
        at: Some(at),
        review_state: None,
        inline: None,
    }
}

fn review(id: u64, at: Time, inline: u32) -> ItemRead {
    ItemRead {
        kind: ItemKind::Review,
        id,
        author: Some("b".into()),
        at: Some(at),
        review_state: Some("commented".into()),
        inline: Some(inline),
    }
}

fn activity_items(kinds: &[EventKind]) -> (Vec<(ItemKind, u64)>, bool) {
    kinds
        .iter()
        .find_map(|k| match k {
            EventKind::Activity { items, more, .. } => {
                Some((items.iter().map(|i| (i.kind, i.id)).collect(), *more))
            }
            _ => None,
        })
        .expect("an activity kind")
}

// ---- tables (TST-15) ----

// TST-15, MOD-5: one case per row of MOD-5's table, and the results it names.
#[test]
fn tst15_checks_state() {
    use ChecksState::{Failing, Passing, Pending};
    let cases: Vec<(&str, Vec<CheckItem>, ChecksState)> = vec![
        ("no items", vec![], ChecksState::None),
        (
            "one running",
            vec![run(1, "a", Phase::InProgress, None)],
            Pending,
        ),
        (
            "one queued",
            vec![run(1, "a", Phase::Queued, None)],
            Pending,
        ),
        (
            "failure while another runs",
            vec![
                done(1, "a", "failure"),
                run(2, "b", Phase::InProgress, None),
            ],
            Pending,
        ),
        ("failure", vec![done(1, "a", "failure")], Failing),
        ("error", vec![done(1, "a", "error")], Failing),
        ("cancelled", vec![done(1, "a", "cancelled")], Failing),
        ("timed_out", vec![done(1, "a", "timed_out")], Failing),
        (
            "action_required",
            vec![done(1, "a", "action_required")],
            Failing,
        ),
        (
            "startup_failure",
            vec![done(1, "a", "startup_failure")],
            Failing,
        ),
        ("unlisted foo", vec![done(1, "a", "foo")], Failing),
        (
            "completed, no result",
            vec![run(1, "a", Phase::Completed, None)],
            Failing,
        ),
        (
            "success and a failure",
            vec![done(1, "a", "success"), done(2, "b", "failure")],
            Failing,
        ),
        ("success", vec![done(1, "a", "success")], Passing),
        ("skipped", vec![done(1, "a", "skipped")], Passing),
        ("neutral", vec![done(1, "a", "neutral")], Passing),
        (
            "success, skipped and neutral",
            vec![
                done(1, "a", "success"),
                done(2, "b", "skipped"),
                done(3, "c", "neutral"),
            ],
            Passing,
        ),
        (
            "a status, failure",
            vec![commit_status("ci", Phase::Completed, Some("failure"))],
            Failing,
        ),
    ];
    for (name, items, want) in cases {
        let c = checks(&list(items), false);
        assert_eq!(c.state, want, "{name}");
        assert!(!c.truncated, "{name}");
    }
    // A failure is counted while others run (MOD-5).
    let c = checks(
        &list(vec![
            done(1, "a", "failure"),
            run(2, "b", Phase::InProgress, None),
        ]),
        false,
    );
    assert_eq!((c.total, c.completed, c.failed), (2, 1, 1));
    assert_eq!(
        c.failed_items,
        vec![FailedItem {
            name: "a".into(),
            url: Some("https://x/1".into())
        }]
    );
}

// TST-15, MOD-7: a truncated list's state, in MOD-7's order.
#[test]
fn tst15_truncated_state() {
    let passed: Vec<CheckItem> = (1..=300).map(|i| done(i, "c", "success")).collect();
    let with = |items: Vec<CheckItem>, total: u32, rollup: Option<&str>| ChecksRead {
        items,
        total,
        rollup: rollup.map(str::to_string),
        next_page: None,
    };
    for (rollup, want) in [
        (Some("pending"), ChecksState::Pending),
        (Some("expected"), ChecksState::Pending),
        (Some("success"), ChecksState::Passing),
        (Some("failure"), ChecksState::Failing),
        (Some("error"), ChecksState::Failing),
        (None, ChecksState::Failing),
    ] {
        let c = checks(&with(passed.clone(), 400, rollup), false);
        assert_eq!(c.state, want, "{rollup:?}");
        assert!(c.truncated);
        assert_eq!((c.total, c.completed, c.failed), (400, 300, 0));
    }
    // A read item not completed reads `pending` whatever the rollup.
    let mut one_running = passed.clone();
    one_running[0] = run(1, "c", Phase::InProgress, None);
    for rollup in ["success", "failure"] {
        let c = checks(&with(one_running.clone(), 400, Some(rollup)), false);
        assert_eq!(c.state, ChecksState::Pending, "{rollup}");
        assert_eq!(c.completed, 299);
    }
    // 300 of 300 isn't truncated; 300 of 301 is.
    let c = checks(&with(passed.clone(), 300, Some("failure")), false);
    assert!(!c.truncated);
    assert_eq!(c.state, ChecksState::Passing);
    let c = checks(&with(passed, 301, Some("failure")), false);
    assert!(c.truncated);
    assert_eq!(c.state, ChecksState::Failing);
    assert_eq!(c.total, 301);
}

/// An open record that reads `ready`.
fn ready_record() -> Record {
    let mut r = sample_record();
    r.state = PrState::Open;
    r.draft = false;
    r.mergeable = Mergeable::Yes;
    r.merge_state = "clean".into();
    r.review = Review::Approved;
    r.checks = checks(&list(vec![done(1, "a", "success")]), true);
    r
}

// TST-15, MOD-11: one case per row, and one per pair of adjacent rows that can hold together.
#[test]
fn tst15_status() {
    type Change = fn(&mut Record);
    let failing: Change = |r| r.checks = checks(&list(vec![done(1, "a", "failure")]), true);
    let pending: Change =
        |r| r.checks = checks(&list(vec![run(1, "a", Phase::InProgress, None)]), true);
    let cases: Vec<(&str, Vec<Change>, Status)> = vec![
        // One per row.
        (
            "merged",
            vec![|r| r.state = PrState::Merged],
            Status::Merged,
        ),
        (
            "closed",
            vec![|r| r.state = PrState::Closed],
            Status::Closed,
        ),
        ("gone", vec![|r| r.state = PrState::Gone], Status::Gone),
        (
            "conflict",
            vec![|r| r.mergeable = Mergeable::No],
            Status::Conflict,
        ),
        ("checks_failing", vec![failing], Status::ChecksFailing),
        (
            "checks_pending: pending",
            vec![pending],
            Status::ChecksPending,
        ),
        (
            "checks_pending: none unsettled",
            vec![|r| r.checks = checks(&list(vec![]), false)],
            Status::ChecksPending,
        ),
        (
            "checks_pending: passing unsettled",
            vec![|r| r.checks.settled = false],
            Status::ChecksPending,
        ),
        (
            "changes_requested",
            vec![|r| r.review = Review::ChangesRequested],
            Status::ChangesRequested,
        ),
        ("draft", vec![|r| r.draft = true], Status::Draft),
        (
            "review_required",
            vec![|r| r.review = Review::ReviewRequired],
            Status::ReviewRequired,
        ),
        (
            "blocked",
            vec![|r| r.merge_state = "blocked".into()],
            Status::Blocked,
        ),
        (
            "blocked: behind",
            vec![|r| r.merge_state = "behind".into()],
            Status::Blocked,
        ),
        (
            "unknown",
            vec![|r| r.mergeable = Mergeable::Unknown],
            Status::Unknown,
        ),
        ("ready", vec![], Status::Ready),
        (
            "ready: none settled",
            vec![|r| r.checks = checks(&list(vec![]), true)],
            Status::Ready,
        ),
        (
            "ready: review none",
            vec![|r| r.review = Review::None],
            Status::Ready,
        ),
        // Adjacent pairs: the earlier row wins.
        (
            "gone with a conflict",
            vec![|r| r.state = PrState::Gone, |r| r.mergeable = Mergeable::No],
            Status::Gone,
        ),
        (
            "conflict with failing checks",
            vec![|r| r.mergeable = Mergeable::No, failing],
            Status::Conflict,
        ),
        (
            "pending checks with changes requested",
            vec![pending, |r| r.review = Review::ChangesRequested],
            Status::ChecksPending,
        ),
        (
            "changes requested on a draft",
            vec![|r| r.review = Review::ChangesRequested, |r| r.draft = true],
            Status::ChangesRequested,
        ),
        (
            "a draft needing review",
            vec![|r| r.draft = true, |r| r.review = Review::ReviewRequired],
            Status::Draft,
        ),
        (
            "review required and blocked",
            vec![|r| r.review = Review::ReviewRequired, |r| {
                r.merge_state = "blocked".into()
            }],
            Status::ReviewRequired,
        ),
        (
            "blocked with mergeable unknown",
            vec![|r| r.merge_state = "blocked".into(), |r| {
                r.mergeable = Mergeable::Unknown
            }],
            Status::Blocked,
        ),
        (
            "a failing draft",
            vec![|r| r.draft = true, failing],
            Status::ChecksFailing,
        ),
        (
            "merged with a conflict",
            vec![|r| r.state = PrState::Merged, |r| {
                r.mergeable = Mergeable::No
            }],
            Status::Merged,
        ),
    ];
    for (name, changes, want) in cases {
        let mut r = ready_record();
        for change in changes {
            change(&mut r);
        }
        assert_eq!(status(&r), want, "{name}");
    }
}

// ---- read sequences ----

// MOD-5: PR C's timeline from docs/measurements/2026-11-24/c1.log, sample by sample.
#[test]
fn mod5_pr_c_timeline() {
    let updated = Time::parse("2026-11-24T22:25:51Z").unwrap();
    let date0 = Time::parse("2026-11-24T22:25:54Z").unwrap();
    let ip = Phase::InProgress;
    let samples: Vec<(f64, Option<&str>, Vec<CheckItem>)> = vec![
        (0.0, None, vec![]),
        (
            7.5,
            Some("pending"),
            vec![
                run(1, "pass", ip, None),
                run(2, "fail", ip, None),
                done(3, "skipped", "skipped"),
                run(4, "slow", ip, None),
            ],
        ),
        (
            11.1,
            Some("failure"),
            vec![
                done(1, "pass", "success"),
                done(2, "fail", "failure"),
                done(3, "skipped", "skipped"),
                run(4, "slow", ip, None),
            ],
        ),
        (
            42.5,
            Some("failure"),
            vec![
                done(1, "pass", "success"),
                done(2, "fail", "failure"),
                done(3, "skipped", "skipped"),
                run(4, "slow", ip, None),
                commit_status("circitor/manual", ip, None),
            ],
        ),
        (
            57.5,
            Some("failure"),
            vec![
                done(1, "pass", "success"),
                done(2, "fail", "failure"),
                done(3, "skipped", "skipped"),
                run(4, "slow", ip, None),
                commit_status("circitor/manual", Phase::Completed, Some("failure")),
            ],
        ),
        (
            88.3,
            Some("failure"),
            vec![
                done(1, "pass", "success"),
                done(2, "fail", "failure"),
                done(3, "skipped", "skipped"),
                done(4, "slow", "cancelled"),
                commit_status("circitor/manual", Phase::Completed, Some("failure")),
            ],
        ),
    ];
    let mut state: Option<(Record, Carry)> = None;
    let mut got = Vec::new();
    for (i, (secs, rollup, items)) in samples.into_iter().enumerate() {
        let mut r = read();
        r.merge_state = if i == 0 { "clean" } else { "unstable" }.into();
        r.updated_at = Some(updated);
        r.checks = list(items);
        r.checks.rollup = rollup.map(str::to_string);
        let mut m = meta(i as u64 + 1, secs);
        m.github_date = Some(date0.plus(ms(secs)));
        let (rec, carry) = match &state {
            None => {
                let (rec, carry, kinds) = applied(apply_read(
                    Before::New {
                        source: Source::Authored,
                    },
                    &Carry::default(),
                    &r,
                    ItemsOutcome::Read(&NO_ITEMS),
                    &m,
                ));
                assert_eq!(words(&kinds), ["tracked"]);
                (rec, carry)
            }
            Some((rec, carry)) => {
                let (rec, carry, _) = again(rec, carry, &r, m);
                (rec, carry)
            }
        };
        let c = &rec.checks;
        let names: Vec<&str> = c.failed_items.iter().map(|f| f.name.as_str()).collect();
        got.push((
            secs,
            c.state,
            c.total,
            c.completed,
            c.failed,
            names.join(","),
            rec.status,
        ));
        state = Some((rec, carry));
    }
    use ChecksState::{Failing, Pending};
    use Status::{ChecksFailing, ChecksPending};
    assert_eq!(
        got,
        vec![
            (
                0.0,
                ChecksState::None,
                0,
                0,
                0,
                String::new(),
                ChecksPending
            ),
            (7.5, Pending, 4, 1, 0, String::new(), ChecksPending),
            (11.1, Pending, 4, 3, 1, "fail".into(), ChecksPending),
            (42.5, Pending, 5, 3, 1, "fail".into(), ChecksPending),
            (
                57.5,
                Pending,
                5,
                4,
                2,
                "fail,circitor/manual".into(),
                ChecksPending
            ),
            (
                88.3,
                Failing,
                5,
                5,
                3,
                "fail,slow,circitor/manual".into(),
                ChecksFailing
            ),
        ]
    );
}

// MOD-2: a read sent before the one last applied isn't applied.
#[test]
fn mod2_an_older_read_is_not_applied() {
    let (rec, mut carry) = first(&read());
    carry.applied_seq = 5;
    let mut r = read();
    r.title = "Newer".into();
    let older = apply_read(
        Before::Known(&rec),
        &carry,
        &r,
        ItemsOutcome::NotAsked,
        &meta(4, 10.0),
    );
    assert!(matches!(older, Merged::Older));
    let (rec2, carry2, kinds) = again(&rec, &carry, &r, meta(5, 10.0));
    assert_eq!(rec2.title, "Newer");
    assert_eq!(carry2.applied_seq, 5);
    assert_eq!(words(&kinds), ["edited"]);
    assert_eq!(rec2.fetched_at, t0().plus(ms(10.0)));
}

// MOD-2: the same read twice, at the next number with the same send times, makes no kinds.
#[test]
fn mod2_the_same_read_twice_makes_no_kinds() {
    let (rec, carry) = first(&read());
    let (rec2, carry2, kinds) = again(&rec, &carry, &read(), meta(2, 0.0));
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(rec2, rec);
    assert_eq!(carry2.applied_seq, 2);
}

// MOD-2, MOD-10: the record is the read, field for field; `error` cleared, `stale` false.
#[test]
fn mod2_the_record_takes_the_read() {
    let (mut rec, carry) = first(&read());
    rec.error = Some(crate::model::RecordError {
        type_: "FORBIDDEN".into(),
        since: t0(),
    });
    rec.stale = true;
    rec.source = Source::Manual;
    rec.rev = 9;
    let mut r = read();
    r.author = None;
    r.url = "https://github.com/yachiko/circitor/pull/12x".into();
    r.head = "other".into();
    r.review = Review::from_github(None);
    let (rec2, _, kinds) = again(&rec, &carry, &r, meta(2, 5.0));
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(rec2.error, None);
    assert!(!rec2.stale);
    assert_eq!((rec2.source, rec2.rev), (Source::Manual, 9));
    assert_eq!(rec2.author, None);
    assert_eq!(rec2.head, "other");
    assert_eq!(rec2.review, Review::None);
    assert_eq!(rec2.fetched_at, t0().plus(ms(5.0)));
}

// MOD-4, MOD-14: a check moving between `queued` and `in_progress` is no change.
#[test]
fn mod4_queued_and_in_progress_swap_makes_no_kind() {
    let mut r = read();
    r.checks = list(vec![run(1, "a", Phase::InProgress, None)]);
    let (rec, carry) = first(&r);
    r.checks = list(vec![run(1, "a", Phase::Queued, None)]);
    let (rec2, _, kinds) = again(&rec, &carry, &r, meta(2, 5.0));
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(rec2.checks.state, ChecksState::Pending);
    assert_eq!(rec2.checks.items.as_ref().unwrap()[0].phase, Phase::Queued);
}

// MOD-4: a step back from `completed` is believed at once.
#[test]
fn mod4_a_step_back_from_completed_is_a_checks_kind() {
    let (rec, carry) = settled_ready();
    let mut r = read();
    r.checks = list(vec![run(1, "test", Phase::InProgress, None)]);
    let (rec2, _, kinds) = again(&rec, &carry, &r, meta(3, 30.0));
    assert_eq!(words(&kinds), ["status", "checks"]);
    assert_eq!(rec2.checks.state, ChecksState::Pending);
    assert_eq!(rec2.status, Status::ChecksPending);
    // A commit status posted again as pending is a change too.
    let mut r = read();
    r.checks = list(vec![commit_status("ci", Phase::Completed, Some("success"))]);
    let (rec, carry, _) = again(&rec2, &carry, &r, meta(4, 31.0));
    r.checks = list(vec![commit_status("ci", Phase::InProgress, None)]);
    let (_, _, kinds) = again(&rec, &carry, &r, meta(5, 32.0));
    assert!(words(&kinds).contains(&"checks"), "{kinds:?}");
}

// MOD-4, MOD-14: a re-run with a new `id` changes which items are in the list.
#[test]
fn mod4_a_rerun_with_a_new_id_is_a_checks_kind() {
    let mut r = read();
    r.checks = list(vec![run(1, "a", Phase::InProgress, None)]);
    let (rec, carry) = first(&r);
    r.checks = list(vec![run(2, "a", Phase::InProgress, None)]);
    let (_, _, kinds) = again(&rec, &carry, &r, meta(2, 5.0));
    assert_eq!(
        kinds,
        vec![EventKind::Checks {
            state: ChecksState::Pending,
            total: 1,
            completed: 0,
            failed: 0,
            failed_items: vec![],
        }]
    );
}

// MOD-14: a check's `url` alone is no change, nor `updated_at`, `url`, `author`, `head`.
#[test]
fn mod14_a_url_change_alone_makes_no_kind() {
    let (rec, carry) = first(&read());
    let mut r = read();
    r.checks.items[0].url = Some("https://elsewhere/1".into());
    r.updated_at = Some(t0().plus(ms(3.0)));
    r.url = "https://github.com/yachiko/circitor/pull/12?moved".into();
    r.author = Some("renamed".into());
    r.head = "renamed-branch".into();
    r.counts.review_threads = 4;
    let (rec2, _, kinds) = again(&rec, &carry, &r, meta(2, 1.0));
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(
        rec2.checks.items.as_ref().unwrap()[0].url.as_deref(),
        Some("https://elsewhere/1")
    );
    assert_eq!(rec2.counts.review_threads, 4);
}

// MOD-6: 20 s with checks, 60 s with none; the read that sets the start is false (TST-8 `no-ci`).
#[test]
fn mod6_settles_at_20_s_with_checks_and_60_s_without() {
    for (items, settle) in [(vec![done(1, "t", "success")], 20.0), (vec![], 60.0)] {
        let mut r = read();
        r.checks = list(items.clone());
        let (rec, carry) = first(&r);
        assert!(!rec.checks.settled);
        assert_eq!(rec.status, Status::ChecksPending);
        let (rec, carry, kinds) = again(&rec, &carry, &r, meta(2, settle - 0.001));
        assert!(!rec.checks.settled, "{settle}");
        assert!(kinds.is_empty(), "{kinds:?}");
        let (rec, carry, kinds) = again(&rec, &carry, &r, meta(3, settle));
        assert!(rec.checks.settled, "{settle}");
        assert_eq!(rec.status, Status::Ready);
        assert_eq!(
            kinds,
            vec![EventKind::Status {
                from: Status::ChecksPending,
                to: Status::Ready
            }]
        );
        // Once true it stays true, also if the list later reads empty.
        let mut empty = r.clone();
        empty.checks = list(vec![]);
        let (rec, _, _) = again(&rec, &carry, &empty, meta(4, settle + 1.0));
        assert!(rec.checks.settled);
    }
    // The empty list's 60 s go by the list as each read shows it: checks appearing at 30 s settle it.
    let mut r = read();
    r.checks = list(vec![]);
    let (rec, carry) = first(&r);
    let (rec, carry, _) = again(&rec, &carry, &r, meta(2, 19.0));
    assert!(!rec.checks.settled);
    let (rec, _, _) = again(&rec, &carry, &read(), meta(3, 30.0));
    assert!(rec.checks.settled);
}

fn first_with(before: Before<'_>, r: &PrRead, date: Option<Time>) -> Record {
    let mut m = meta(1, 0.0);
    m.github_date = date;
    applied(apply_read(
        before,
        &Carry::default(),
        r,
        ItemsOutcome::Read(&NO_ITEMS),
        &m,
    ))
    .0
}

// MOD-6: the shortcut for `New` and `Loaded`, by GitHub's `Date` (TST-8 `settled-at-once`).
#[test]
fn mod6_shortcut() {
    let date = t0().plus(Duration::from_secs(600));
    let befores = [
        Before::New {
            source: Source::Authored,
        },
        Before::Loaded {
            source: Source::Authored,
            saved_open: true,
        },
    ];
    for before in befores {
        for (items, age, want) in [
            (vec![done(1, "t", "success")], 120.0, true),
            (vec![done(1, "t", "success")], 25.0, false),
            (vec![done(1, "t", "success")], 26.0, true),
            (vec![], 65.0, false),
            (vec![], 66.0, true),
        ] {
            let mut r = read();
            r.checks = list(items);
            r.updated_at = Some(date.minus(ms(age)));
            let rec = first_with(before.clone(), &r, Some(date));
            assert_eq!(rec.checks.settled, want, "{before:?} {age}");
            if want {
                assert_eq!(rec.status, Status::Ready);
            }
        }
        // No `Date`, never; no `updated_at`, never.
        let mut r = read();
        r.updated_at = Some(date.minus(ms(120.0)));
        assert!(!first_with(before.clone(), &r, None).checks.settled);
        r.updated_at = None;
        assert!(!first_with(before.clone(), &r, Some(date)).checks.settled);
    }
    // Never for `Known`: a new head on an old PR starts at false.
    let (rec, carry) = settled_ready();
    let mut r = read();
    r.head_sha = SHA_B.into();
    r.updated_at = Some(date.minus(ms(120.0)));
    let mut m = meta(3, 30.0);
    m.github_date = Some(date);
    let (rec, _, _) = again(&rec, &carry, &r, m);
    assert!(!rec.checks.settled);
}

// MOD-6: a new head, a head back to an earlier one, and a reopening each start settling again
// (TST-8 `head-back`, `reopened`).
#[test]
fn mod6_a_new_head_an_old_head_back_and_a_reopening_start_again() {
    let (rec, carry) = settled_ready(); // started at 0, settled at 20
    let mut b = read();
    b.head_sha = SHA_B.into();
    let (rec, carry, kinds) = again(&rec, &carry, &b, meta(3, 25.0));
    assert!(!rec.checks.settled);
    assert_eq!(carry.settle_start, Some(Mono::ZERO.plus(ms(25.0))));
    assert_eq!(words(&kinds), ["status", "head"]);
    assert_eq!(
        kinds[1],
        EventKind::Head {
            from: "aaaaaaa".into(),
            to: "bbbbbbb".into()
        }
    );
    let (rec, carry, _) = again(&rec, &carry, &b, meta(4, 44.9));
    assert!(!rec.checks.settled);
    let (rec, carry, _) = again(&rec, &carry, &b, meta(5, 45.0));
    assert!(rec.checks.settled);
    // Back to the earlier head.
    let (rec, carry, kinds) = again(&rec, &carry, &read(), meta(6, 50.0));
    assert!(!rec.checks.settled);
    assert_eq!(rec.status, Status::ChecksPending);
    assert_eq!(words(&kinds), ["status", "head"]);
    let (rec, carry, _) = again(&rec, &carry, &read(), meta(7, 70.0));
    assert!(rec.checks.settled);
    // A close keeps it; a reopening starts again.
    let mut closed = read();
    closed.state = ReadState::Closed;
    let (rec, carry, kinds) = again(&rec, &carry, &closed, meta(8, 75.0));
    assert!(rec.checks.settled);
    assert_eq!(words(&kinds), ["status", "closed"]);
    let (rec, carry, kinds) = again(&rec, &carry, &read(), meta(9, 80.0));
    assert!(!rec.checks.settled);
    assert_eq!(words(&kinds), ["status", "reopened"]);
    assert_eq!(rec.status, Status::ChecksPending);
    let (rec, _, _) = again(&rec, &carry, &read(), meta(10, 100.0));
    assert!(rec.checks.settled);
    assert_eq!(rec.status, Status::Ready);
}

fn unknown_read() -> PrRead {
    let mut r = read();
    r.mergeable = Mergeable::Unknown;
    r.merge_state = "unknown".into();
    r
}

// MOD-8: a known `yes` is kept through `UNKNOWN` for 60 s on the wall clock (TST-8
// `mergeable-unknown`, `base-changed`).
#[test]
fn mod8_unknown_keeps_yes_for_60_s() {
    let (rec, carry) = settled_ready(); // `yes` last read at 20 s
    let (kept, carry_kept, kinds) = again(&rec, &carry, &unknown_read(), meta(3, 80.0));
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(kept.mergeable, Mergeable::Yes);
    assert_eq!(kept.merge_state, "clean");
    assert_eq!(kept.status, Status::Ready);
    let (gone, carry_after, kinds) = again(&kept, &carry_kept, &unknown_read(), meta(4, 81.0));
    assert_eq!(gone.mergeable, Mergeable::Unknown);
    assert_eq!(carry_after.known, None);
    assert_eq!(
        kinds,
        vec![
            EventKind::Status {
                from: Status::Ready,
                to: Status::Unknown
            },
            EventKind::Mergeable {
                mergeable: Mergeable::Unknown,
                merge_state: "unknown".into()
            }
        ]
    );
    // A changed head reads `unknown` at once.
    let mut r = unknown_read();
    r.head_sha = SHA_B.into();
    let (rec2, _, _) = again(&rec, &carry, &r, meta(3, 21.0));
    assert_eq!(rec2.mergeable, Mergeable::Unknown);
    // A changed base too.
    let mut r = unknown_read();
    r.base = "develop".into();
    let (rec2, _, kinds) = again(&rec, &carry, &r, meta(3, 21.0));
    assert_eq!(rec2.mergeable, Mergeable::Unknown);
    assert!(words(&kinds).contains(&"mergeable"));
    // A clock set back: `sent_wall` before `read_at` is kept.
    let mut m = meta(3, 30.0);
    m.sent_wall = t0().minus(Duration::from_secs(3600));
    let (rec2, _, kinds) = again(&rec, &carry, &unknown_read(), m);
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(rec2.mergeable, Mergeable::Yes);
}

// MOD-9: `merge_state` is kept with `mergeable`, and passes through otherwise.
#[test]
fn mod9_merge_state_follows() {
    let (rec, carry) = settled_ready();
    let (rec2, _, _) = again(&rec, &carry, &unknown_read(), meta(3, 30.0));
    assert_eq!(rec2.merge_state, "clean");
    let (rec2, _, _) = again(&rec, &carry, &unknown_read(), meta(3, 90.0));
    assert_eq!(rec2.merge_state, "unknown");
    let mut r = read();
    r.mergeable = Mergeable::No;
    r.merge_state = "dirty".into();
    let (rec2, carry2, kinds) = again(&rec, &carry, &r, meta(3, 30.0));
    assert_eq!(
        words(&kinds),
        ["status", "mergeable"],
        "a conflict changes the status"
    );
    assert_eq!(rec2.merge_state, "dirty");
    let known = carry2.known.unwrap();
    assert_eq!(
        (known.mergeable, known.merge_state.as_str()),
        (Mergeable::No, "dirty")
    );
    // An unlisted value is taken as read, never kept.
    let mut r = read();
    r.merge_state = "has_hooks".into();
    let (rec2, _, kinds) = again(&rec, &carry, &r, meta(3, 30.0));
    assert_eq!(rec2.merge_state, "has_hooks");
    assert_eq!(words(&kinds), ["mergeable"]);
}

// MOD-8: a merged PR reads `unknown` for both, with no `mergeable` kind beside `merged`.
#[test]
fn mod8_merged_reads_unknown_with_no_mergeable_kind() {
    let (rec, carry) = settled_ready();
    let mut r = read();
    r.state = ReadState::Merged;
    r.merged_at = Some(t0().plus(Duration::from_secs(25)));
    r.closed_at = r.merged_at;
    let (rec2, carry2, kinds) = again(&rec, &carry, &r, meta(3, 30.0));
    assert_eq!(words(&kinds), ["status", "merged"]);
    assert_eq!(rec2.mergeable, Mergeable::Unknown);
    assert_eq!(rec2.merge_state, "unknown");
    assert_eq!(rec2.status, Status::Merged);
    assert_eq!(carry2.known, None);
}

// MOD-11: an unlisted value reads on the safe side for `status` only.
#[test]
fn mod11_unlisted_values_read_on_the_safe_side() {
    let (rec, carry) = settled_ready();
    let mut r = read();
    r.mergeable = Mergeable::Other("foo".into());
    let (rec2, _, _) = again(&rec, &carry, &r, meta(3, 30.0));
    assert_eq!(rec2.status, Status::Unknown);
    assert_eq!(rec2.mergeable, Mergeable::Other("foo".into()));
    let mut r = read();
    r.review = Review::Other("foo".into());
    let (rec2, _, _) = again(&rec, &carry, &r, meta(3, 30.0));
    assert_eq!(rec2.status, Status::ReviewRequired);
    assert_eq!(rec2.review, Review::Other("foo".into()));
    let json = serde_json::to_value(&rec2).unwrap();
    assert_eq!(json["review"], "foo");
}

// MOD-11: `apply_gone`, and a gone record read again.
#[test]
fn mod11_gone() {
    let (mut rec, carry) = settled_ready();
    rec.error = Some(crate::model::RecordError {
        type_: "NOT_FOUND".into(),
        since: t0(),
    });
    let (gone, kinds) = apply_gone(&rec).unwrap();
    assert_eq!(
        kinds,
        vec![
            EventKind::Status {
                from: Status::Ready,
                to: Status::Gone
            },
            EventKind::Gone
        ]
    );
    assert_eq!((gone.state, gone.status), (PrState::Gone, Status::Gone));
    let mut same = gone.clone();
    same.state = rec.state;
    same.status = rec.status;
    assert_eq!(same, rec, "every other field stays");
    for state in [PrState::Merged, PrState::Closed, PrState::Gone] {
        let mut r = rec.clone();
        r.state = state;
        assert!(apply_gone(&r).is_none(), "{state:?}");
    }
    for (state, word) in [
        (ReadState::Open, "reopened"),
        (ReadState::Merged, "merged"),
        (ReadState::Closed, "closed"),
    ] {
        let mut r = read();
        r.state = state;
        let (back, carry2, kinds) = again(&gone, &carry, &r, meta(3, 30.0));
        assert_eq!(words(&kinds), ["status", word], "{state:?}");
        assert_eq!(back.error, None);
        if state == ReadState::Open {
            assert!(!back.checks.settled);
            assert_eq!(carry2.settle_start, Some(Mono::ZERO.plus(ms(30.0))));
        }
    }
}

// MOD-1: a change of case alone isn't a change; the record takes GitHub's spelling.
#[test]
fn mod1_names_compare_without_case() {
    let (rec, carry) = first(&read());
    let mut r = read();
    r.pr = PrName::parse("Yachiko/Circitor#12").unwrap();
    let (rec2, _, kinds) = again(&rec, &carry, &r, meta(2, 1.0));
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(rec2.pr.to_string(), "Yachiko/Circitor#12");
    // A rename is `edited` naming `pr`.
    r.pr = PrName::parse("yachiko/renamed#12").unwrap();
    let (rec3, _, kinds) = again(&rec2, &carry, &r, meta(3, 2.0));
    assert_eq!(
        kinds,
        vec![EventKind::Edited {
            fields: vec!["pr".into()]
        }]
    );
    assert_eq!(rec3.pr.to_string(), "yachiko/renamed#12");
}

// MOD-15: `edited` names `pr`, `title`, `base`, in that order.
#[test]
fn mod15_edited_names_pr_title_base_in_that_order() {
    let (rec, carry) = first(&read());
    let mut r = read();
    r.base = "develop".into();
    r.title = "Another".into();
    r.pr = PrName::parse("other/circitor#12").unwrap();
    let (_, _, kinds) = again(&rec, &carry, &r, meta(2, 1.0));
    assert_eq!(
        kinds,
        vec![EventKind::Edited {
            fields: vec!["pr".into(), "title".into(), "base".into()]
        }]
    );
}

// MOD-15: kinds come in `EventKind`'s declared order.
#[test]
fn mod15_kinds_keep_the_declared_order() {
    let (rec, carry) = settled_ready();
    let mut r = read();
    r.title = "New".into();
    r.head_sha = SHA_B.into();
    r.draft = true;
    r.review = Review::ChangesRequested;
    r.mergeable = Mergeable::No;
    r.merge_state = "dirty".into();
    r.checks = list(vec![run(9, "t", Phase::Queued, None)]);
    r.counts.comments = 1;
    let got = ItemsRead {
        comments: vec![comment(1, t0())],
        reviews: vec![],
    };
    let (_, _, kinds) = applied(apply_read(
        Before::Known(&rec),
        &carry,
        &r,
        ItemsOutcome::Read(&got),
        &meta(3, 30.0),
    ));
    assert_eq!(
        words(&kinds),
        [
            "status",
            "head",
            "checks",
            "draft",
            "mergeable",
            "review",
            "activity",
            "edited"
        ]
    );
}

// ---- items (MOD-12, STA-5, POL-12) ----

fn new_read(r: &PrRead, got: &ItemsRead) -> (Record, Carry, Vec<EventKind>) {
    applied(apply_read(
        Before::New {
            source: Source::Asked,
        },
        &Carry::default(),
        r,
        ItemsOutcome::Read(got),
        &meta(1, 0.0),
    ))
}

// STA-5: first sight is a baseline (TST-8 `baseline`).
#[test]
fn sta5_new_is_a_baseline() {
    let mut r = read();
    r.counts.comments = 5;
    let t = |s: u64| t0().plus(Duration::from_secs(s));
    let got = ItemsRead {
        comments: vec![
            comment(1, t(1)),
            comment(2, t(2)),
            comment(3, t(3)),
            comment(5, t(4)),
            comment(4, t(4)),
        ],
        reviews: vec![],
    };
    let (rec, carry, kinds) = new_read(&r, &got);
    assert_eq!(
        kinds,
        vec![EventKind::Tracked {
            source: Source::Asked
        }]
    );
    assert_eq!(rec.counts.comments, 5);
    assert_eq!(
        carry.seen.comment,
        SeenKind {
            at: Some(t(4)),
            ids: vec![4, 5]
        }
    );
    assert_eq!(carry.seen.review, SeenKind::default());
}

fn known_with_seen(seen: Seen, comments: u32) -> (Record, Carry) {
    let (mut rec, mut carry) = settled_ready();
    rec.counts.comments = comments;
    carry.seen = seen;
    (rec, carry)
}

fn read_items(
    rec: &Record,
    carry: &Carry,
    r: &PrRead,
    got: &ItemsRead,
) -> (Record, Carry, Vec<EventKind>) {
    applied(apply_read(
        Before::Known(rec),
        carry,
        r,
        ItemsOutcome::Read(got),
        &meta(3, 30.0),
    ))
}

// STA-5: an item at the seen time with an ID not seen is new.
#[test]
fn sta5_same_time_other_id_is_new() {
    let seen = Seen {
        comment: SeenKind {
            at: Some(t0()),
            ids: vec![1],
        },
        review: SeenKind::default(),
    };
    let (rec, carry) = known_with_seen(seen, 1);
    let mut r = read();
    r.counts.comments = 2;
    let got = ItemsRead {
        comments: vec![comment(1, t0()), comment(2, t0())],
        reviews: vec![],
    };
    let (rec2, carry2, kinds) = read_items(&rec, &carry, &r, &got);
    assert_eq!(words(&kinds), ["activity"]);
    assert_eq!(
        activity_items(&kinds),
        (vec![(ItemKind::Comment, 2)], false)
    );
    assert_eq!(carry2.seen.comment.ids, vec![1, 2]);
    assert_eq!(rec2.counts.comments, 2);
    // `seen` never moves back.
    let older = ItemsRead {
        comments: vec![comment(0, t0().minus(Duration::from_secs(5)))],
        reviews: vec![],
    };
    let (_, carry3, _) = read_items(&rec2, &carry2, &r, &older);
    assert_eq!(carry3.seen, carry2.seen);
}

// STA-5: with nothing seen of a kind, every item of it is new; ordered by `at`, comments first, `id`.
#[test]
fn sta5_nothing_seen_makes_every_item_new() {
    let t = |s: u64| t0().plus(Duration::from_secs(s));
    let mut r = read();
    r.counts.comments = 3;
    r.counts.reviews = 2;
    let got = ItemsRead {
        comments: vec![comment(30, t(2)), comment(10, t(1)), comment(20, t(1))],
        reviews: vec![
            review(5, t(1), 0),
            review(6, t(3), 1),
            ItemRead {
                at: None,
                ..review(7, t(0), 0)
            },
        ],
    };
    let (_, carry, kinds) = applied(apply_read(
        Before::Loaded {
            source: Source::Authored,
            saved_open: true,
        },
        &Carry::default(),
        &r,
        ItemsOutcome::Read(&got),
        &meta(1, 0.0),
    ));
    assert_eq!(words(&kinds), ["activity", "tracked"]);
    use ItemKind::{Comment as C, Review as R};
    assert_eq!(
        activity_items(&kinds),
        (vec![(C, 10), (C, 20), (R, 5), (C, 30), (R, 6)], false)
    );
    // The review with no time is neither seen nor new.
    assert_eq!(
        carry.seen.review,
        SeenKind {
            at: Some(t(3)),
            ids: vec![6]
        }
    );
}

// STA-5, MOD-15: a PR loaded from the file reports what came meanwhile (TST-8 `restart`).
#[test]
fn sta5_loaded_reports_what_came_meanwhile() {
    let t = |s: u64| t0().plus(Duration::from_secs(s));
    let carry = Carry {
        seen: Seen {
            comment: SeenKind {
                at: Some(t(10)),
                ids: vec![1],
            },
            review: SeenKind::default(),
        },
        ..Carry::default()
    };
    let mut r = read();
    r.state = ReadState::Merged;
    r.counts.comments = 3;
    let got = ItemsRead {
        comments: vec![comment(1, t(10)), comment(2, t(20)), comment(3, t(30))],
        reviews: vec![],
    };
    let (rec, _, kinds) = applied(apply_read(
        Before::Loaded {
            source: Source::Manual,
            saved_open: true,
        },
        &carry,
        &r,
        ItemsOutcome::Read(&got),
        &meta(1, 0.0),
    ));
    assert_eq!(words(&kinds), ["merged", "activity", "tracked"]);
    assert_eq!(
        activity_items(&kinds),
        (vec![(ItemKind::Comment, 2), (ItemKind::Comment, 3)], false)
    );
    assert_eq!(
        kinds[2],
        EventKind::Tracked {
            source: Source::Manual
        }
    );
    assert_eq!(rec.source, Source::Manual);
    // Saved merged: no `merged` again; nothing new: no `activity`.
    let none_new = ItemsRead {
        comments: vec![comment(1, t(10))],
        reviews: vec![],
    };
    let (_, _, kinds) = applied(apply_read(
        Before::Loaded {
            source: Source::Manual,
            saved_open: false,
        },
        &carry,
        &r,
        ItemsOutcome::Read(&none_new),
        &meta(1, 0.0),
    ));
    assert_eq!(words(&kinds), ["tracked"]);
    // Saved open, now closed.
    r.state = ReadState::Closed;
    let (_, _, kinds) = applied(apply_read(
        Before::Loaded {
            source: Source::Manual,
            saved_open: true,
        },
        &carry,
        &r,
        ItemsOutcome::Read(&none_new),
        &meta(1, 0.0),
    ));
    assert_eq!(words(&kinds), ["closed", "tracked"]);
}

// POL-12, STA-5: `more` for `Loaded` and for `Known`.
#[test]
fn sta5_more() {
    let t = |s: u64| t0().plus(Duration::from_secs(s));
    let twenty: Vec<ItemRead> = (1..=20).map(|i| comment(i, t(100 + i))).collect();
    let got = ItemsRead {
        comments: twenty.clone(),
        reviews: vec![],
    };
    let loaded = |carry: &Carry| {
        let mut r = read();
        r.counts.comments = 40;
        let (_, _, kinds) = applied(apply_read(
            Before::Loaded {
                source: Source::Authored,
                saved_open: true,
            },
            carry,
            &r,
            ItemsOutcome::Read(&got),
            &meta(1, 0.0),
        ));
        activity_items(&kinds)
    };
    // Loaded: all 20 new → `more`; 19 of 20 → not.
    let (items, more) = loaded(&Carry::default());
    assert_eq!((items.len(), more), (20, true));
    let one_seen = Carry {
        seen: Seen {
            comment: SeenKind {
                at: Some(t(101)),
                ids: vec![1],
            },
            review: SeenKind::default(),
        },
        ..Carry::default()
    };
    let (items, more) = loaded(&one_seen);
    assert_eq!((items.len(), more), (19, false));
    // Known: all 20 new and a rise of more than 20 → `more`; a rise of 20 → not.
    for (rise, want) in [(21, true), (20, false)] {
        let (rec, carry) = known_with_seen(
            Seen {
                comment: SeenKind {
                    at: Some(t(1)),
                    ids: vec![99],
                },
                review: SeenKind::default(),
            },
            5,
        );
        let mut r = read();
        r.counts.comments = 5 + rise;
        let (_, _, kinds) = read_items(&rec, &carry, &r, &got);
        let (items, more) = activity_items(&kinds);
        assert_eq!((items.len(), more), (20, want), "rise {rise}");
    }
    // Known: 19 of 20 new with a big rise → not.
    let (rec, carry) = known_with_seen(one_seen.seen.clone(), 5);
    let mut r = read();
    r.counts.comments = 50;
    let (_, _, kinds) = read_items(&rec, &carry, &r, &got);
    assert!(!activity_items(&kinds).1);
}

// POL-12: a `Known` read with no rise makes no `activity`, but `seen` still moves; a count that
// falls, or `review_threads` alone, makes no kind, and the record takes the new numbers.
#[test]
fn pol12_no_rise_no_activity() {
    let (rec, carry) = known_with_seen(Seen::default(), 4);
    let mut r = read();
    r.counts.comments = 3;
    r.counts.review_threads = 7;
    let got = ItemsRead {
        comments: vec![comment(1, t0())],
        reviews: vec![],
    };
    let (rec2, carry2, kinds) = read_items(&rec, &carry, &r, &got);
    assert!(kinds.is_empty(), "{kinds:?}");
    assert_eq!(rec2.counts.comments, 3);
    assert_eq!(rec2.counts.review_threads, 7);
    assert_eq!(carry2.seen.comment.at, Some(t0()));
    // The rise is seen from the lower value.
    r.counts.comments = 4;
    assert!(counts_rose(&rec2, &r));
}

// POL-12: failed items keep the old counts, so the next read sees the rise again.
#[test]
fn pol12_failed_items_keep_the_old_counts() {
    let (rec, carry) = known_with_seen(Seen::default(), 1);
    let mut r = read();
    r.counts.comments = 3;
    r.counts.reviews = 1;
    r.counts.review_threads = 2;
    assert!(counts_rose(&rec, &r));
    for outcome in [ItemsOutcome::Failed, ItemsOutcome::NotAsked] {
        let (rec2, carry2, kinds) = applied(apply_read(
            Before::Known(&rec),
            &carry,
            &r,
            outcome,
            &meta(3, 30.0),
        ));
        assert!(kinds.is_empty(), "{outcome:?}: {kinds:?}");
        assert_eq!(
            (
                rec2.counts.comments,
                rec2.counts.reviews,
                rec2.counts.review_threads
            ),
            (1, 0, 2)
        );
        assert!(counts_rose(&rec2, &r), "{outcome:?}");
        assert_eq!(carry2.seen, carry.seen);
    }
    // Failed with no rise: the old counts too.
    let mut fell = read();
    fell.counts.comments = 0;
    let (rec2, _, _) = applied(apply_read(
        Before::Known(&rec),
        &carry,
        &fell,
        ItemsOutcome::Failed,
        &meta(3, 30.0),
    ));
    assert_eq!(rec2.counts.comments, 1);
    // A first read without items: applied, `seen` alone, no activity.
    let seen = Seen {
        comment: SeenKind {
            at: Some(t0()),
            ids: vec![1],
        },
        review: SeenKind::default(),
    };
    let start = Carry {
        seen: seen.clone(),
        ..Carry::default()
    };
    let (rec3, carry3, kinds) = applied(apply_read(
        Before::Loaded {
            source: Source::Authored,
            saved_open: true,
        },
        &start,
        &r,
        ItemsOutcome::Failed,
        &meta(1, 0.0),
    ));
    assert_eq!(words(&kinds), ["tracked"]);
    assert_eq!(carry3.seen, seen);
    assert_eq!(rec3.counts.comments, 3);
}

// MOD-12: an item's REST paths.
#[test]
fn mod12_paths() {
    let pr = PrName::parse("Yachiko/circitor#12").unwrap();
    let c = item(&comment(101, t0()), t0(), &pr);
    assert_eq!(c.fetch, "repos/Yachiko/circitor/issues/comments/101");
    assert_eq!((c.inline, c.fetch_inline.as_deref()), (None, None));
    let r0 = item(&review(202, t0(), 0), t0(), &pr);
    assert_eq!(r0.fetch, "repos/Yachiko/circitor/pulls/12/reviews/202");
    assert_eq!((r0.inline, r0.fetch_inline.as_deref()), (Some(0), None));
    let r1 = item(&review(203, t0(), 1), t0(), &pr);
    assert_eq!(r1.inline, Some(1));
    assert_eq!(
        r1.fetch_inline.as_deref(),
        Some("repos/Yachiko/circitor/pulls/12/reviews/203/comments")
    );
    let no_inline = ItemRead {
        inline: None,
        ..review(204, t0(), 0)
    };
    assert_eq!(item(&no_inline, t0(), &pr).inline, Some(0));
    // Built from the record's current name.
    let (rec, carry) = known_with_seen(Seen::default(), 0);
    let mut r = read();
    r.pr = PrName::parse("yachiko/renamed#12").unwrap();
    r.counts.reviews = 1;
    let got = ItemsRead {
        comments: vec![],
        reviews: vec![review(7, t0(), 2)],
    };
    let (_, _, kinds) = read_items(&rec, &carry, &r, &got);
    let EventKind::Activity { items, .. } = &kinds[0] else {
        panic!("{kinds:?}")
    };
    assert_eq!(items[0].fetch, "repos/yachiko/renamed/pulls/12/reviews/7");
}

// U5, MOD-12: an item carries who, when and an ID, never text: its JSON keys are MOD-12's only.
#[test]
fn u5_an_item_has_no_text() {
    let pr = PrName::parse("o/r#1").unwrap();
    let allowed = [
        "kind",
        "id",
        "author",
        "at",
        "review_state",
        "inline",
        "fetch",
        "fetch_inline",
    ];
    for (read, want) in [
        (
            comment(1, t0()),
            vec!["at", "author", "fetch", "id", "kind"],
        ),
        (
            review(2, t0(), 3),
            vec![
                "at",
                "author",
                "fetch",
                "fetch_inline",
                "id",
                "inline",
                "kind",
                "review_state",
            ],
        ),
    ] {
        let json = serde_json::to_value(item(&read, t0(), &pr)).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, want);
        assert!(keys.iter().all(|k| allowed.contains(k)));
    }
}

// U4: the record holds the state, the checks, the review state and whether it can merge.
#[test]
fn u4_the_record_knows_state_checks_review_and_mergeability() {
    let (rec, _) = settled_ready();
    let json = serde_json::to_value(&rec).unwrap();
    assert_eq!(json["state"], "open");
    assert_eq!(json["checks"]["state"], "passing");
    assert_eq!(json["review"], "none");
    assert_eq!(json["mergeable"], "yes");
    assert_eq!(json["status"], "ready");
}

// POL-8: a page joined to a read.
#[test]
fn pol8_append_page() {
    let mut a = ChecksRead {
        items: vec![done(1, "a", "success")],
        total: 101,
        rollup: Some("pending".into()),
        next_page: Some("c1".into()),
    };
    a.append_page(ChecksRead {
        items: vec![done(2, "b", "failure")],
        total: 102,
        rollup: Some("failure".into()),
        next_page: None,
    });
    assert_eq!(a.items.len(), 2);
    assert_eq!(
        (a.total, a.rollup.as_deref(), a.next_page),
        (102, Some("failure"), None)
    );
}

// MOD-8: a known value is not kept on a closed PR, and `Known` holds what was read.
#[test]
fn mod8_known_is_what_was_read() {
    let (_, carry) = settled_ready();
    assert_eq!(
        carry.known,
        Some(Known {
            mergeable: Mergeable::Yes,
            merge_state: "clean".into(),
            head_sha: SHA_A.into(),
            base: "main".into(),
            read_at: t0().plus(Duration::from_secs(20)),
        })
    );
    let mut r = unknown_read();
    r.state = ReadState::Closed;
    let (m, _, k) = derive::mergeability(&r, carry.known.as_ref(), t0());
    assert_eq!((m, k), (Mergeable::Unknown, None));
}

#[test]
fn checks_items_are_kept() {
    let c: Checks = checks(&list(vec![done(1, "a", "success")]), true);
    assert_eq!(c.items.as_ref().map(Vec::len), Some(1));
}
