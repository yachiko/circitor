use std::path::Path;
use std::time::Duration;

use super::*;
use crate::api::{DaemonBlock, ListBody, NotifyBody, RecordBody};
use crate::model::{
    ChecksState, Counts, Event, EventKind, FailedItem, Item, ItemKind, Mergeable, NodeId, Review,
    Source, Status,
};
use crate::poller::{ProblemCode, Tier};
use crate::testkit::golden;

// ---- inputs ----

fn t(s: &str) -> Time {
    Time::parse(s).unwrap()
}

fn ctx() -> RenderCtx {
    RenderCtx {
        now: t("2026-11-24T22:00:00Z"),
        tz: chrono::FixedOffset::east_opt(0).unwrap(),
        cli_version: "1.2.3+4f9a1c2".into(),
    }
}

fn ctx_plus1() -> RenderCtx {
    RenderCtx {
        tz: chrono::FixedOffset::east_opt(3600).unwrap(),
        ..ctx()
    }
}

fn gold(name: &str, text: &str) {
    golden::check(&format!("tests/golden/render/{name}.txt"), text);
}

fn pr(s: &str) -> PrName {
    PrName::parse(s).unwrap()
}

macro_rules! api_json {
    ($name:literal) => {
        serde_json::from_str(include_str!(concat!(
            "../../../testdata/api/",
            $name,
            ".json"
        )))
        .unwrap()
    };
}

fn list_env() -> Envelope<ListBody> {
    api_json!("list")
}
fn wait_env() -> Envelope<WaitBody> {
    api_json!("wait-satisfied")
}
fn changes_env() -> Envelope<ChangesBody> {
    api_json!("changes")
}
fn already_env() -> Envelope<NotifyBody> {
    api_json!("notify-already")
}
fn registered_env() -> Envelope<NotifyBody> {
    api_json!("notify-registered")
}

/// The spec's example record: `yachiko/circitor#12`, checks 3/5, none failed.
fn spec_record() -> Record {
    let mut r = list_env().body.records[0].clone();
    r.pr = pr("yachiko/circitor#12");
    r.title = "Add the poller".into();
    r.head_sha = "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d".into();
    r.status = Status::ChecksPending;
    r.review = Review::None;
    r.checks.state = ChecksState::Pending;
    r.checks.total = 5;
    r.checks.completed = 3;
    r.checks.failed = 0;
    r.checks.failed_items = vec![];
    r.checks.items = None;
    r.stale = false;
    r.error = None;
    r
}

fn with_checks(state: ChecksState, total: u32, completed: u32, failed: u32) -> Record {
    let mut r = spec_record();
    r.checks.state = state;
    r.checks.total = total;
    r.checks.completed = completed;
    r.checks.failed = failed;
    r
}

fn stale(mut r: Record) -> Record {
    r.stale = true;
    r.fetched_at = t("2026-11-24T21:56:00Z");
    r
}

fn wait_body(outcome: Outcome, reason: Option<Reason>, record: Option<Record>) -> WaitBody {
    WaitBody {
        outcome,
        reason,
        record,
        events: vec![],
        records: vec![],
        more_events: false,
        reset: false,
    }
}

fn env_of<T>(body: T) -> Envelope<T> {
    Envelope {
        version: "1.2.3+4f9a1c2".into(),
        cursor: "k3x9q2mz-418".into(),
        daemon: DaemonBlock {
            tier: Tier::Active,
            last_poll_at: None,
            problem: None,
            message: None,
            until: None,
        },
        body,
    }
}

fn ask(cond: Ask) -> WaitAsk {
    WaitAsk {
        pr: pr("yachiko/circitor#12"),
        cond,
        timeout: 100,
        timeout_given: false,
        json: false,
    }
}

fn checks_done_sha() -> Ask {
    let mut c = Ask::new(Until::ChecksDone);
    c.sha = Some("1a2b3c4".into());
    c
}

fn event(pr_: &str, kinds: Vec<EventKind>) -> Event {
    Event {
        rev: 1,
        pr: pr(pr_),
        at: t("2026-11-24T21:00:00Z"),
        kinds,
        id: NodeId::default(),
    }
}

// ---- CLI-3 ----

// CLI-3: the record line.
#[test]
fn cli3_list_line() {
    let line = record_line_of(&spec_record());
    assert_eq!(
        line,
        "yachiko/circitor#12  checks_pending  1a2b3c4  checks 3/5, 0 failed  review none  \"Add the poller\""
    );
    gold("list", &list(&list_env(), false, &ctx()));
}

fn record_line_of(r: &Record) -> String {
    let e = env_of(RecordBody {
        record: Some(r.clone()),
    });
    tracked(&e, &r.pr, &ctx()).trim_end().to_string()
}

fn every_event_kind() -> Vec<Event> {
    let p = "yachiko/circitor#12";
    let review = Item {
        kind: ItemKind::Review,
        id: 604123,
        author: Some("octocat".into()),
        at: t("2026-11-24T21:00:00Z"),
        review_state: Some("changes_requested".into()),
        inline: Some(3),
        fetch: "repos/yachiko/circitor/pulls/12/reviews/604123".into(),
        fetch_inline: Some("repos/yachiko/circitor/pulls/12/reviews/604123/comments".into()),
    };
    let comment = Item {
        kind: ItemKind::Comment,
        id: 918273,
        author: Some("octocat".into()),
        at: t("2026-11-24T21:00:00Z"),
        review_state: None,
        inline: None,
        fetch: "repos/yachiko/circitor/issues/comments/918273".into(),
        fetch_inline: None,
    };
    let activity = |items| EventKind::Activity {
        counts: Counts::default(),
        items,
        more: false,
    };
    vec![event(
        p,
        vec![
            EventKind::Status {
                from: Status::ChecksPending,
                to: Status::ChecksFailing,
            },
            EventKind::Head {
                from: "1a2b3c4".into(),
                to: "9f8e7d6".into(),
            },
            EventKind::Checks {
                state: ChecksState::Pending,
                total: 5,
                completed: 3,
                failed: 1,
                failed_items: vec![],
            },
            EventKind::Checks {
                state: ChecksState::Failing,
                total: 5,
                completed: 5,
                failed: 1,
                failed_items: vec![FailedItem {
                    name: "build".into(),
                    url: Some("https://github.com/yachiko/circitor/runs/1".into()),
                }],
            },
            EventKind::Merged,
            EventKind::Draft { draft: false },
            EventKind::Mergeable {
                mergeable: Mergeable::No,
                merge_state: "dirty".into(),
            },
            EventKind::Review {
                review: Review::ChangesRequested,
            },
            activity(vec![review]),
            activity(vec![comment]),
            EventKind::Edited {
                fields: vec!["title".into(), "base".into()],
            },
            EventKind::Tracked {
                source: Source::Manual,
            },
            EventKind::Closed,
            EventKind::Reopened,
            EventKind::Gone,
            EventKind::Untracked {
                source: Source::Manual,
            },
        ],
    )]
}

// CLI-3: one line per kind, byte for byte.
#[test]
fn cli3_event_lines() {
    let mut e = changes_env();
    e.body.events = every_event_kind();
    let text = changes(&e, Some(&pr("yachiko/circitor#12")), &ctx());
    let want = "\
yachiko/circitor#12  status checks_pending -> checks_failing
yachiko/circitor#12  head 1a2b3c4 -> 9f8e7d6
yachiko/circitor#12  checks pending (3 of 5, 1 failed)
yachiko/circitor#12  checks failing (1 of 5): build https://github.com/yachiko/circitor/runs/1
yachiko/circitor#12  merged
yachiko/circitor#12  draft false
yachiko/circitor#12  mergeable no (dirty)
yachiko/circitor#12  review changes_requested
yachiko/circitor#12  new review changes_requested by octocat, 3 inline  gh api repos/yachiko/circitor/pulls/12/reviews/604123  gh api repos/yachiko/circitor/pulls/12/reviews/604123/comments
yachiko/circitor#12  new comment by octocat  gh api repos/yachiko/circitor/issues/comments/918273
yachiko/circitor#12  edited title, base
yachiko/circitor#12  tracked (manual)
yachiko/circitor#12  closed
yachiko/circitor#12  reopened
yachiko/circitor#12  gone
yachiko/circitor#12  untracked
cursor: k3x9q2ab-28
";
    assert_eq!(text, want);
    gold("changes-every-kind", &text);
    gold("changes", &changes(&changes_env(), None, &ctx()));
}

// CLI-3: a review without inline comments, a ghost, `more`, an activity with nothing.
#[test]
fn cli3_event_edge_cases() {
    let p = "yachiko/circitor#12";
    let item = |kind, author: Option<&str>, inline| Item {
        kind,
        id: 5,
        author: author.map(Into::into),
        at: t("2026-11-24T21:00:00Z"),
        review_state: Some("approved".into()),
        inline,
        fetch: "repos/o/r/pulls/12/reviews/5".into(),
        fetch_inline: Some("repos/o/r/pulls/12/reviews/5/comments".into()),
    };
    let act = |items, more| EventKind::Activity {
        counts: Counts::default(),
        items,
        more,
    };
    let mut e = changes_env();
    e.body.events = vec![event(
        p,
        vec![
            act(vec![item(ItemKind::Review, None, Some(0))], false),
            act(vec![], false),
            act(vec![], true),
        ],
    )];
    let text = changes(&e, None, &ctx());
    assert_eq!(
        text,
        "\
yachiko/circitor#12  new review approved by ghost  gh api repos/o/r/pulls/12/reviews/5
yachiko/circitor#12  more new items than shown  gh api repos/yachiko/circitor/issues/12/comments  gh api repos/yachiko/circitor/pulls/12/reviews
cursor: k3x9q2ab-28
"
    );
    gold("changes-edge-cases", &text);
}

// CLI-3: durations.
#[test]
fn cli3_durations() {
    let cases = [
        (0, "0s"),
        (45, "45s"),
        (59, "59s"),
        (60, "1m"),
        (61, "1m1s"),
        (240, "4m"),
        (270, "4m30s"),
        (3_599, "59m59s"),
        (3_600, "1h"),
        (3_780, "1h3m"),
        (86_399, "23h59m"),
        (86_400, "1d"),
        (190_800, "2d5h"),
    ];
    for (s, want) in cases {
        assert_eq!(duration(Duration::from_secs(s)), want, "{s}");
    }
}

// CLI-3: a title is quoted and escaped, and never cut.
#[test]
fn cli3_title_escapes() {
    let mut r = spec_record();
    r.title = "a \"b\" \\ c\t\u{1b}".into();
    let line = record_line_of(&r);
    assert!(line.ends_with(r#"  "a \"b\" \\ c\u{09}\u{1b}""#), "{line}");
    r.title = "x".repeat(300);
    let line = record_line_of(&r);
    assert!(line.ends_with(&format!("\"{}\"", "x".repeat(300))));
    r.title = "a\nb".into();
    assert!(record_line_of(&r).ends_with(r#""a\u{0a}b""#));
    gold(
        "title-escapes",
        &format!(
            "{}\n",
            record_line_of(&{
                r.title = "a \"b\" \\ c\t\u{1b}".into();
                r
            })
        ),
    );
}

// CLI-3: `more events:` names the PR after an answer about one PR (D9).
#[test]
fn cli3_more_events_names_the_pr() {
    let p = pr("yachiko/circitor#12");
    let mut c = changes_env();
    c.body.more_events = true;
    assert!(
        changes(&c, Some(&p), &ctx())
            .contains("more events: circitor changes yachiko/circitor#12 --since k3x9q2ab-28\n")
    );
    assert!(
        changes(&c, None, &ctx()).contains("more events: circitor changes --since k3x9q2ab-28\n")
    );
    let mut w = wait_env();
    w.body.more_events = true;
    let text = wait(&w, &ask(checks_done_sha()), &ctx());
    assert!(
        text.contains("more events: circitor changes yachiko/circitor#12 --since k3x9q2ab-28\n")
    );
}

// CLI-3: the tail, in order.
#[test]
fn cli3_tail_order() {
    let mut c = changes_env();
    c.body.more_events = true;
    c.body.pending = vec![pr("octo/lib#31")];
    c.daemon.problem = Some(ProblemCode::Auth);
    c.daemon.message = Some("Run gh auth login.".into());
    let text = changes(&c, None, &ctx());
    let tail: Vec<&str> = text.lines().rev().take(4).collect();
    assert_eq!(
        tail,
        [
            "cursor: k3x9q2ab-28",
            "problem: auth, Run gh auth login.",
            "more events: circitor changes --since k3x9q2ab-28",
            "pending octo/lib#31",
        ]
    );
    gold("changes-tail", &text);

    let mut l = list_env();
    l.body.untracked = vec![pr("octo/old#2")];
    l.body.pending = vec![pr("octo/lib#31")];
    let text = list(&l, true, &ctx());
    assert!(text.contains("pending octo/lib#31\nuntracked octo/old#2\ncursor: k3x9q2ab-28\n"));
    assert_eq!(text.lines().last(), Some("cursor: k3x9q2ab-28"));
    gold("list-since", &text);
}

// CLI-3: `list` without `--since` prints 50.
#[test]
fn cli3_list_50() {
    let mut l = list_env();
    let one = l.body.records[0].clone();
    l.body.records = (0..51)
        .map(|i| {
            let mut r = one.clone();
            r.pr = pr(&format!("octo/app#{}", i + 1));
            r
        })
        .collect();
    let without = list(&l, false, &ctx());
    assert_eq!(
        without
            .lines()
            .filter(|l| l.starts_with("octo/app#"))
            .count(),
        50
    );
    assert!(without.contains("\n1 more PR (all are in circitor list --json)\n"));
    let with = list(&l, true, &ctx());
    assert_eq!(
        with.lines().filter(|l| l.starts_with("octo/app#")).count(),
        51
    );
    assert!(!with.contains("more PR"));
    l.body.records.push(one);
    assert!(list(&l, false, &ctx()).contains("\n2 more PRs "));
    gold("list-more", &without);
}

// CLI-3: a reset is never silent.
#[test]
fn cli3_reset() {
    let text = list(&api_json!("list-reset"), false, &ctx());
    assert!(text.starts_with("reset: cursor too old, current state follows\nocto/app#12  "));
    gold("list-reset", &text);

    let mut w = api_json!("wait-reset");
    w = Envelope { ..w };
    let text = wait(&w, &ask(checks_done_sha()), &ctx());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "reset: cursor too old, current state follows");
    assert!(lines[1].starts_with("octo/app#12  checks_failing"));
    gold("wait-reset", &text);

    let mut c = changes_env();
    c.body.reset = true;
    c.body.records = list_env().body.records;
    let text = changes(&c, None, &ctx());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "reset: cursor too old, current state follows");
    assert!(lines[1].starts_with("octo/app#12  "));
    assert!(lines[2].starts_with("octo/app#9  "));
    assert!(lines[3].starts_with("octo/app#12  tracked"));
    gold("changes-reset", &text);
}

// CLI-3: the stale mark.
#[test]
fn cli3_stale() {
    let r = stale(spec_record());
    assert!(record_line_of(&r).ends_with("\"Add the poller\" (stale, 4m old)"));
    let mut r = r;
    r.error = Some(crate::model::RecordError {
        type_: "FORBIDDEN".into(),
        since: t("2026-11-24T21:57:00Z"),
    });
    assert!(record_line_of(&r).ends_with("\"Add the poller\" (stale, 4m old, error FORBIDDEN)"));
}

#[test]
fn cli3_stale_mark_alone() {
    let c = ctx();
    let r = stale(spec_record());
    assert_eq!(stale_mark(&r, c.now), " (stale, 4m old)");
    let mut r2 = r.clone();
    r2.error = Some(crate::model::RecordError {
        type_: "FORBIDDEN".into(),
        since: c.now,
    });
    assert_eq!(stale_mark(&r2, c.now), " (stale, 4m old, error FORBIDDEN)");
    assert_eq!(stale_mark(&spec_record(), c.now), "");
}

// CLI-3: a daemon problem is never silent.
#[test]
fn cli3_problem_line() {
    let mut c = changes_env();
    c.daemon.problem = Some(ProblemCode::RateLimited);
    c.daemon.until = Some(t("2026-11-24T22:12:00Z"));
    c.daemon.message = Some("GitHub's rate limit is used up.".into());
    let text = changes(&c, None, &ctx_plus1());
    assert!(
        text.contains(
            "problem: rate_limited for 12m (until 23:12 local), GitHub's rate limit is used up.\n"
        ),
        "{text}"
    );
    gold("problem-rate-limited", &text);

    c.daemon.problem = Some(ProblemCode::Auth);
    c.daemon.until = None;
    c.daemon.message = Some("Run gh auth login.".into());
    let text = changes(&c, None, &ctx());
    assert!(text.contains("problem: auth, Run gh auth login.\n"));
    assert!(!text.contains("local"));
}

// CLI-3: `show`.
#[test]
fn cli3_show() {
    let e = api_json!("show");
    gold("show", &show(&e, &ctx()));

    let mut e = e;
    let r = e.body.record.as_mut().unwrap();
    r.mergeable = Mergeable::Unknown;
    r.checks
        .items
        .as_mut()
        .unwrap()
        .push(crate::model::CheckItem {
            kind: crate::model::CheckKind::Run,
            id: Some(9),
            name: "deploy".into(),
            phase: crate::model::Phase::InProgress,
            result: None,
            url: Some("https://ci.example.test/octo/runs/9".into()),
        });
    e.daemon.problem = Some(ProblemCode::GithubUnavailable);
    e.daemon.message = Some("GitHub can't be reached.".into());
    gold("show-unknown-running", &show(&e, &ctx()));
}

// CLI-3: refresh, stop, tracked.
#[test]
fn cli3_refresh_stop_tracked() {
    let mut out = String::new();
    let r: Envelope<RefreshBody> = api_json!("refresh");
    out += &refresh(&r, &ctx());
    let mut one = r.clone();
    one.body.cost = 1;
    out += &refresh(&one, &ctx());
    let mut shared = r.clone();
    shared.body.outcome = PollOutcome::Shared;
    shared.body.cost = 0;
    out += &refresh(&shared, &ctx());
    let mut bad = r;
    bad.body.outcome = PollOutcome::RateLimited;
    bad.body.cost = 0;
    out += &refresh(&bad, &ctx());
    out += &stop(true);
    out += &stop(false);
    let tr: Envelope<RecordBody> = api_json!("track");
    out += &tracked(&tr, &pr("octo/lib#5"), &ctx());
    let un: Envelope<RecordBody> = api_json!("untrack-null");
    out += &tracked(&un, &pr("octo/lib#5"), &ctx());
    assert!(out.contains("refresh: polled, 4 points\nrefresh: polled, 1 point\nrefresh: shared\nrefresh: rate_limited, 0 points\nstopped\nnot running\n"));
    assert!(out.ends_with("octo/lib#5 not tracked\n"));
    gold("refresh-stop-tracked", &out);
}

// CLI-3: status.
#[test]
fn cli3_status() {
    let mut e: Envelope<StatusBody> = api_json!("status");
    let mut c = ctx();
    assert!(!status(&e, &c).contains("cli_version"));
    gold("status", &status(&e, &c));
    e.body.last_poll = None;
    e.body.github.remaining = None;
    e.body.owners = vec![];
    gold("status-nulls", &status(&e, &c));
    let p: Envelope<StatusBody> = api_json!("status-problem");
    gold("status-problem", &status(&p, &c));
    c.cli_version = "1.3.0+x".into();
    let text = status(&p, &c);
    assert!(text.ends_with("cli_version: 1.3.0+x\n"));
    gold("status-cli-differs", &text);
}

// ---- CLI-6 ----

fn line(outcome: Outcome, reason: Option<Reason>, r: Option<&Record>, until: Until) -> String {
    outcome_line(outcome, reason, r, until, Some("1a2b3c4"))
}

fn six_cases() -> Vec<(&'static str, String)> {
    let cd = Until::ChecksDone;
    let mut truncated = with_checks(ChecksState::Failing, 5, 5, 1);
    truncated.checks.truncated = true;
    let mut moved = spec_record();
    moved.head_sha = "9f8e7d6aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    vec![
        (
            "checks passed (5/5) on 1a2b3c4",
            line(
                Outcome::Satisfied,
                None,
                Some(&with_checks(ChecksState::Passing, 5, 5, 0)),
                cd,
            ),
        ),
        (
            "checks failed (1 of 5) on 1a2b3c4",
            line(
                Outcome::Satisfied,
                None,
                Some(&with_checks(ChecksState::Failing, 5, 5, 1)),
                cd,
            ),
        ),
        (
            "checks failing (1 failed, 3 of 5 done) on 1a2b3c4",
            line(
                Outcome::Satisfied,
                None,
                Some(&with_checks(ChecksState::Pending, 5, 3, 1)),
                cd,
            ),
        ),
        (
            "no checks reported on 1a2b3c4",
            line(
                Outcome::Satisfied,
                None,
                Some(&with_checks(ChecksState::None, 0, 0, 0)),
                cd,
            ),
        ),
        (
            "merged",
            line(
                Outcome::Satisfied,
                None,
                Some(&spec_record()),
                Until::Merged,
            ),
        ),
        (
            "changed",
            line(
                Outcome::Satisfied,
                None,
                Some(&spec_record()),
                Until::Changed,
            ),
        ),
        (
            "ended: closed",
            line(Outcome::Ended, Some(Reason::Closed), None, cd),
        ),
        (
            "ended: gone",
            line(Outcome::Ended, Some(Reason::Gone), None, cd),
        ),
        (
            "ended: merged",
            line(Outcome::Ended, Some(Reason::Merged), None, cd),
        ),
        (
            "ended: head moved to 9f8e7d6, expected 1a2b3c4",
            line(Outcome::Ended, Some(Reason::HeadMoved), Some(&moved), cd),
        ),
        (
            "checks failed (1 of 5) on 1a2b3c4 (truncated: unread checks may still run)",
            line(Outcome::Satisfied, None, Some(&truncated), cd),
        ),
    ]
}

#[test]
fn cli6_outcome_line_alone() {
    for (want, got) in six_cases() {
        assert_eq!(got, want);
        assert!(!got.ends_with('\n'));
    }
    let st = stale(with_checks(ChecksState::Passing, 5, 5, 0));
    let l = outcome_line(Outcome::Satisfied, None, Some(&st), Until::ChecksDone, None);
    assert_eq!(l, "checks passed (5/5) on 1a2b3c4");
    assert_eq!(
        outcome_line(Outcome::Timeout, None, None, Until::Merged, None),
        ""
    );
}

#[test]
fn cli6_outcome_lines() {
    let mut text = String::new();
    let cd = checks_done_sha();
    let cases: Vec<(Outcome, Option<Reason>, Record)> = vec![
        (
            Outcome::Satisfied,
            None,
            with_checks(ChecksState::Passing, 5, 5, 0),
        ),
        (
            Outcome::Satisfied,
            None,
            with_checks(ChecksState::Failing, 5, 5, 1),
        ),
        (
            Outcome::Satisfied,
            None,
            with_checks(ChecksState::Pending, 5, 3, 1),
        ),
        (
            Outcome::Satisfied,
            None,
            with_checks(ChecksState::None, 0, 0, 0),
        ),
        (Outcome::Ended, Some(Reason::Merged), spec_record()),
        (Outcome::Ended, Some(Reason::Closed), spec_record()),
        (Outcome::Ended, Some(Reason::Gone), spec_record()),
        (
            Outcome::Satisfied,
            None,
            stale(with_checks(ChecksState::Passing, 5, 5, 0)),
        ),
        (Outcome::Satisfied, None, {
            let mut r = stale(with_checks(ChecksState::Failing, 5, 5, 1));
            r.checks.truncated = true;
            r
        }),
    ];
    for (o, reason, r) in cases {
        let mut e = env_of(wait_body(o, reason, Some(r)));
        e.cursor = "k3x9q2mz-418".into();
        text += &wait(&e, &ask(cd.clone()), &ctx());
    }
    let mut cond = Ask::new(Until::Merged);
    cond.since = Some("ignored".into());
    text += &wait(
        &env_of(wait_body(Outcome::Satisfied, None, Some(spec_record()))),
        &ask(cond),
        &ctx(),
    );
    let mut moved = spec_record();
    moved.head_sha = "9f8e7d6aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    text += &wait(
        &env_of(wait_body(
            Outcome::Ended,
            Some(Reason::HeadMoved),
            Some(moved),
        )),
        &ask(cd),
        &ctx(),
    );
    assert!(text.contains("checks passed (5/5) on 1a2b3c4 (stale, 4m old)\n"));
    assert!(text.contains("(truncated: unread checks may still run) (stale, 4m old)\n"));
    assert!(
        text.contains("ended: head moved to 9f8e7d6, expected 1a2b3c4\ncursor: k3x9q2mz-418\n")
    );
    gold("wait-outcome-lines", &text);
    gold(
        "wait-satisfied",
        &wait(&wait_env(), &ask(checks_done_sha()), &ctx()),
    );
}

#[test]
fn cli6_flags() {
    assert_eq!(flags(&Ask::new(Until::Merged)), "--until merged");
    let mut c = Ask::new(Until::ChecksDone);
    c.sha = Some("1A2b3c4".into());
    c.rerun_since = Some("k3x9q2mz-400".into());
    c.fail_fast = true;
    c.since = Some("k3x9q2mz-1".into());
    assert_eq!(
        flags(&c),
        "--until checks-done --sha 1A2b3c4 --rerun-since k3x9q2mz-400 --fail-fast"
    );
    let mut k = Ask::new(Until::Changed);
    k.kind = Some("activity,merged".into());
    assert_eq!(flags(&k), "--until changed --kind activity,merged");
}

#[test]
fn cli6_timeout_block() {
    let mut e = env_of(wait_body(Outcome::Timeout, None, Some(spec_record())));
    e.cursor = "k3x9q2mz-418".into();
    let a = ask(checks_done_sha());
    let text = wait(&e, &a, &ctx());
    assert_eq!(
        text,
        "\
timeout after 100s: yachiko/circitor#12 checks_pending (3/5, 0 failed)
again: circitor wait yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --since k3x9q2mz-418
cursor: k3x9q2mz-418
"
    );
    gold("wait-timeout", &text);

    let mut a2 = a.clone();
    a2.timeout = 300;
    a2.timeout_given = true;
    assert!(wait(&e, &a2, &ctx()).contains("--sha 1a2b3c4 --timeout 300 --since k3x9q2mz-418\n"));
    a2.timeout = 100;
    assert!(wait(&e, &a2, &ctx()).contains("--timeout 100 --since"));
    let mut a3 = a.clone();
    a3.json = true;
    assert!(wait(&e, &a3, &ctx()).contains("--sha 1a2b3c4 --json --since k3x9q2mz-418\n"));
    assert!(!wait(&e, &a, &ctx()).contains("--timeout"));

    let mut a4 = ask(Ask::new(Until::ChecksDone));
    a4.cond.rerun_since = Some("k3x9q2mz-400".into());
    a4.cond.fail_fast = true;
    a4.timeout_given = true;
    a4.json = true;
    let t4 = wait(&e, &a4, &ctx());
    assert!(t4.contains(
        "again: circitor wait yachiko/circitor#12 --until checks-done --rerun-since k3x9q2mz-400 --fail-fast --timeout 100 --json --since k3x9q2mz-418\n"
    ));
    let mut a5 = ask(Ask::new(Until::Changed));
    a5.cond.kind = Some("activity".into());
    assert!(wait(&e, &a5, &ctx()).contains("--until changed --kind activity --since"));
    gold("wait-timeout-all-flags", &t4);
}

#[test]
fn cli6_timeout_other_head() {
    let mut r = stale(spec_record());
    r.head_sha = "9f8e7d6aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    r.fetched_at = t("2026-11-24T21:58:00Z");
    let e = env_of(wait_body(Outcome::Timeout, None, Some(r.clone())));
    let text = wait(&e, &ask(checks_done_sha()), &ctx());
    assert!(text.starts_with(
        "timeout after 100s: yachiko/circitor#12 head 9f8e7d6, not 1a2b3c4 (stale, 2m old)\n"
    ));
    gold("wait-timeout-other-head", &text);
    // The same head, in another case: the status as usual.
    let mut c = checks_done_sha();
    c.sha = Some("9F8E7D6".into());
    assert!(wait(&e, &ask(c), &ctx()).contains("checks_pending (3/5, 0 failed) (stale, 2m old)"));
}

#[test]
fn cli6_no_record() {
    let e: Envelope<WaitBody> = api_json!("wait-timeout-no-record");
    let text = wait(&e, &ask(checks_done_sha()), &ctx());
    assert!(text.starts_with("timeout after 100s: yachiko/circitor#12 no record yet\n"));
    gold("wait-timeout-no-record", &text);
}

#[test]
fn cli6_no_answer() {
    let a = ask(checks_done_sha());
    let sock = Path::new("/tmp/c/circitor.sock");
    let mut out = String::new();
    for kind in [
        NoAnswer::NoTimeLeft,
        NoAnswer::DaemonStopped,
        NoAnswer::NotAnswering,
    ] {
        out += &wait_no_answer(kind, &a, Some("k3x9q2mz-418"), sock);
    }
    assert!(out.starts_with(
        "timeout after 100s: yachiko/circitor#12 (no answer from the daemon)\nagain: circitor wait yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --since k3x9q2mz-418\ncursor: k3x9q2mz-418\ndaemon stopped\n"
    ));
    assert!(out.contains("daemon not answering on /tmp/c/circitor.sock\n"));
    gold("wait-no-answer", &out);
    let none = wait_no_answer(NoAnswer::DaemonStopped, &a, None, sock);
    assert_eq!(
        none,
        "daemon stopped\nagain: circitor wait yachiko/circitor#12 --until checks-done --sha 1a2b3c4\n"
    );
    gold("wait-no-answer-no-cursor", &none);
}

// ---- NTF-11 ----

#[test]
fn ntf11_already() {
    let p = pr("octo/lib#8");
    let cond = Ask::new(Until::ChecksDone);
    let e = already_env();
    let text = already(&e, &p, &cond, None, &ctx());
    assert!(text.contains("already: checks failed (1 of 1) on 3c4d5e6\ncursor: k3x9q2ab-28\n"));
    assert!(text.starts_with("octo/lib#8  status checks_pending -> checks_failing\n"));
    assert!(!text.contains("again:"));
    gold("already", &text);

    let mut r = e.clone();
    r.body.reset = true;
    r.body.records = vec![r.body.record.clone().unwrap()];
    r.daemon.problem = Some(ProblemCode::GithubUnavailable);
    r.daemon.message = Some("GitHub can't be reached.".into());
    let text = already(&r, &p, &cond, None, &ctx());
    assert!(
        text.starts_with(
            "reset: cursor too old, current state follows\nocto/lib#8  checks_failing"
        )
    );
    assert!(text.contains(
        "again: circitor notify octo/lib#8 --until checks-done --since k3x9q2ab-28\nproblem: github_unavailable, GitHub can't be reached.\ncursor: k3x9q2ab-28\n"
    ));
    gold("already-reset", &text);
    let with_life = already(&r, &p, &cond, Some(3600), &ctx());
    assert!(with_life.contains("--until checks-done --for 3600 --since k3x9q2ab-28\n"));

    let mut rep = e;
    rep.body.replaced = true;
    rep.body.more_events = true;
    let text = already(&rep, &p, &cond, None, &ctx());
    assert!(text.contains(
        "already: checks failed (1 of 1) on 3c4d5e6\nreplaced an earlier notify\nmore events: circitor changes octo/lib#8 --since k3x9q2ab-28\ncursor:"
    ));
    gold("already-replaced", &text);
}

#[test]
fn ntf11_registered() {
    let mut e = registered_env();
    e.body.record.as_mut().unwrap().pr = pr("yachiko/circitor#12");
    let p = pr("yachiko/circitor#12");
    let first = "registered: yachiko/circitor#12 until checks-done on 1a2b3c4, for 1d";
    let mut ec = e.clone();
    ec.body.record = Some(spec_record());
    ec.cursor = "k3x9q2mz-418".into();
    let text = registered(&ec, first, &p, &ctx());
    assert_eq!(
        text,
        "\
registered: yachiko/circitor#12 until checks-done on 1a2b3c4, for 1d
yachiko/circitor#12  checks_pending  1a2b3c4  checks 3/5, 0 failed  review none  \"Add the poller\"
a confirmation was posted to this session; if none arrives, notices are held or refused here: run circitor notify yachiko/circitor#12 --cancel, then use circitor wait
cursor: k3x9q2mz-418
"
    );
    gold("registered", &text);

    let mut r = ec;
    r.body.replaced = true;
    r.body.reset = true;
    r.body.records = vec![spec_record()];
    r.body.events = vec![event("yachiko/circitor#12", vec![EventKind::Merged])];
    r.body.more_events = true;
    r.daemon.problem = Some(ProblemCode::Auth);
    r.daemon.message = Some("Run gh auth login.".into());
    let text = registered(&r, first, &p, &ctx());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "reset: cursor too old, current state follows");
    assert!(lines[1].starts_with("yachiko/circitor#12  checks_pending"));
    assert_eq!(lines[2], "yachiko/circitor#12  merged");
    assert_eq!(lines[3], first);
    assert_eq!(lines[6], "replaced an earlier notify");
    assert!(lines[7].starts_with("more events: circitor changes yachiko/circitor#12 --since"));
    assert_eq!(lines[8], "problem: auth, Run gh auth login.");
    assert_eq!(lines[9], "cursor: k3x9q2mz-418");
    gold("registered-everything", &text);
}

// ---- CLI-5 ----

fn error_env(code: &str) -> ApiError {
    let json = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("testdata/api/error-{code}.json")),
    )
    .unwrap();
    serde_json::from_str::<crate::api::ErrorBody>(&json)
        .unwrap()
        .error
}

#[test]
fn cli5_error_lines() {
    let mut out = String::new();
    for code in [
        "bad_request",
        "not_found",
        "unknown_route",
        "auth",
        "starting",
        "github_unavailable",
        "rate_limited",
        "limit",
        "no_session",
    ] {
        let e = error_env(code);
        let l = api_error(&e, false, &ctx_plus1());
        assert!(l.ends_with('\n') && l.matches('\n').count() == 1, "{l}");
        assert!(l.starts_with(&format!("{code}: ")));
        out += &l;
    }
    assert!(
        out.contains("(try again in 42m, until 23:42 local)\n"),
        "{out}"
    );
    assert!(out.contains("(try again in a minute)\n"));
    let bad = error_env("bad_request");
    let unk = error_env("unknown_route");
    out += &api_error(&bad, true, &ctx());
    out += &api_error(&unk, true, &ctx());
    assert!(out.contains("older than this CLI: run circitor stop"));
    assert!(!api_error(&error_env("not_found"), true, &ctx()).contains("older"));
    gold("errors", &out);
}

#[test]
fn cli5_wait_code() {
    let cd = Until::ChecksDone;
    let body = |o, r: Option<Record>| wait_body(o, None, r);
    assert_eq!(
        wait_code(
            &body(
                Outcome::Satisfied,
                Some(with_checks(ChecksState::Passing, 5, 5, 0))
            ),
            cd
        ),
        0
    );
    assert_eq!(
        wait_code(
            &body(
                Outcome::Satisfied,
                Some(with_checks(ChecksState::None, 0, 0, 0))
            ),
            cd
        ),
        0
    );
    assert_eq!(
        wait_code(
            &body(
                Outcome::Satisfied,
                Some(with_checks(ChecksState::Failing, 5, 5, 1))
            ),
            cd
        ),
        8
    );
    assert_eq!(
        wait_code(
            &body(
                Outcome::Satisfied,
                Some(with_checks(ChecksState::Pending, 5, 3, 1))
            ),
            cd
        ),
        8
    );
    assert_eq!(
        wait_code(
            &body(
                Outcome::Satisfied,
                Some(with_checks(ChecksState::Pending, 5, 3, 0))
            ),
            cd
        ),
        0
    );
    // Only `checks-done` has a code 8.
    assert_eq!(
        wait_code(
            &body(
                Outcome::Satisfied,
                Some(with_checks(ChecksState::Failing, 5, 5, 1))
            ),
            Until::Merged
        ),
        0
    );
    assert_eq!(
        wait_code(
            &body(
                Outcome::Satisfied,
                Some(with_checks(ChecksState::Failing, 5, 5, 1))
            ),
            Until::Changed
        ),
        0
    );
    assert_eq!(wait_code(&body(Outcome::Satisfied, None), cd), 0);
    assert_eq!(wait_code(&body(Outcome::Timeout, None), cd), 3);
    assert_eq!(wait_code(&body(Outcome::Ended, None), cd), 4);
}

#[test]
fn cli5_error_code() {
    use ErrorCode::*;
    let rows = [
        (BadRequest, 2, 1),
        (NotFound, 5, 5),
        (UnknownRoute, 1, 1),
        (Auth, 7, 7),
        (Starting, 9, 9),
        (GithubUnavailable, 9, 9),
        (RateLimited, 9, 9),
        (Limit, 10, 10),
        (NoSession, 11, 11),
    ];
    for (code, plain, older) in rows {
        assert_eq!(error_code(code, false), plain, "{code:?}");
        assert_eq!(error_code(code, true), older, "{code:?} older");
    }
}

#[test]
fn cli5_refresh_code() {
    for (o, want) in [
        (PollOutcome::Polled, 0),
        (PollOutcome::Shared, 0),
        (PollOutcome::Auth, 7),
        (PollOutcome::GithubUnavailable, 9),
        (PollOutcome::RateLimited, 9),
    ] {
        assert_eq!(refresh_code(o), want, "{o:?}");
    }
}

#[test]
fn cli5_no_answer_code() {
    assert_eq!(no_answer_code(NoAnswer::NoTimeLeft), 3);
    assert_eq!(no_answer_code(NoAnswer::DaemonStopped), 6);
    assert_eq!(no_answer_code(NoAnswer::NotAnswering), 6);
}
