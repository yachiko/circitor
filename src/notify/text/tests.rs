//! The notices' words (NTF-7, NTF-8; TST-7's share): golden files in `tests/golden/notify/`.

use super::*;
use crate::api::Until;
use crate::cli::render::stale_mark;
use crate::clock::Time;
use crate::model::tests::sample_record;
use crate::model::{Checks, ChecksState, FailedItem, PrState};
use crate::testkit::golden;
use crate::testkit::{MARKER_CHECK, MARKER_LOGIN, MARKER_TITLE};

const DAY: Duration = Duration::from_secs(86_400);
const SHA: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

fn pr() -> PrName {
    PrName::parse("yachiko/circitor#12").unwrap()
}

fn cursor() -> Cursor {
    "k3x9q2mz-418".parse().unwrap()
}

fn told(cond: Ask) -> Told {
    Told {
        pr: pr(),
        cond,
        life: DAY,
        life_given: false,
        cursor: cursor(),
    }
}

fn checks_done(sha: Option<&str>) -> Ask {
    let mut a = Ask::new(Until::ChecksDone);
    a.sha = sha.map(str::to_string);
    a
}

/// An open record on `SHA` whose title, author and failed checks are markers (NTF-8).
fn record(state: ChecksState, total: u32, completed: u32, failed: u32) -> Record {
    let mut r = sample_record();
    r.state = PrState::Open;
    r.head_sha = SHA.into();
    r.title = MARKER_TITLE.into();
    r.author = Some(MARKER_LOGIN.into());
    r.checks = Checks {
        state,
        settled: true,
        truncated: false,
        total,
        completed,
        failed,
        failed_items: (0..failed)
            .map(|_| FailedItem {
                name: MARKER_CHECK.into(),
                url: Some("https://x/1".into()),
            })
            .collect(),
        items: None,
    };
    r
}

// TST-7: every notice of NTF-7 and NTF-8 as a golden file, one notice a file, with a newline after it.
fn gold(name: &str, text: &str) {
    golden::check(
        &format!("tests/golden/notify/{name}.txt"),
        &format!("{text}\n"),
    );
}

// NTF-7: the confirmation, and NTF-7's one rule for the condition's words.
#[test]
fn ntf7_ntf8_texts_confirmation() {
    let first = confirmation(&told(checks_done(Some("1a2b3c4"))));
    // NTF-7's first example, byte for byte.
    assert_eq!(
        first,
        "circitor: watching yachiko/circitor#12 until checks-done on 1a2b3c4, for 1d. If you can read this, notices reach this session."
    );
    gold("confirmation_sha", &first);

    let plain = confirmation(&told(Ask::new(Until::Merged)));
    assert_eq!(
        plain,
        "circitor: watching yachiko/circitor#12 until merged, for 1d. If you can read this, notices reach this session."
    );
    gold("confirmation_plain", &plain);

    // NTF-7's second example, byte for byte.
    let mut changed = Ask::new(Until::Changed);
    changed.kind = Some("review".into());
    changed.since = Some("k3x9q2mz-418".into());
    let mut t = told(changed);
    t.life = Duration::from_secs(7_200);
    let second = confirmation(&t);
    assert_eq!(
        second,
        "circitor: watching yachiko/circitor#12 until changed kind review since k3x9q2mz-418, for 2h. If you can read this, notices reach this session."
    );
    gold("confirmation_kind_since", &second);

    // Every parameter, in NTF-7's order; the SHA's first 7, lower-cased from upper case.
    let mut all = checks_done(Some("1A2B3C4D5E"));
    all.rerun_since = Some("k3x9q2mz-400".into());
    all.fail_fast = true;
    all.kind = Some("checks,review".into());
    all.since = Some("k3x9q2mz-418".into());
    let every = confirmation(&told(all.clone()));
    assert_eq!(
        every,
        "circitor: watching yachiko/circitor#12 until checks-done on 1a2b3c4 after k3x9q2mz-400 fail-fast kind checks,review since k3x9q2mz-418, for 1d. If you can read this, notices reach this session."
    );
    gold("confirmation_every_parameter", &every);
    assert_eq!(
        watching(&pr(), &all, Duration::from_secs(90)),
        "yachiko/circitor#12 until checks-done on 1a2b3c4 after k3x9q2mz-400 fail-fast kind checks,review since k3x9q2mz-418, for 1m30s"
    );
}

// NTF-8: every outcome notice, through T10's outcome line.
#[test]
fn ntf7_ntf8_texts_outcomes() {
    let cd = told(checks_done(None));
    let merged_ask = told(Ask::new(Until::Merged));
    let passing = record(ChecksState::Passing, 5, 5, 0);
    let running = record(ChecksState::Pending, 5, 3, 0);
    let mut ff = checks_done(None);
    ff.fail_fast = true;
    let sat = (Outcome::Satisfied, None);
    let ended = |r: Reason| (Outcome::Ended, Some(r));
    let cases: Vec<(&str, String)> = vec![
        (
            "outcome_checks_passed",
            outcome(&cd, sat, Some(&passing), true),
        ),
        (
            "outcome_checks_failed",
            outcome(&cd, sat, Some(&record(ChecksState::Failing, 5, 5, 1)), true),
        ),
        (
            "outcome_checks_failing",
            outcome(
                &told(ff),
                sat,
                Some(&record(ChecksState::Pending, 5, 3, 1)),
                true,
            ),
        ),
        (
            "outcome_no_checks",
            outcome(&cd, sat, Some(&record(ChecksState::None, 0, 0, 0)), true),
        ),
        (
            "outcome_merged",
            outcome(&merged_ask, sat, Some(&passing), true),
        ),
        (
            "outcome_changed",
            outcome(&told(Ask::new(Until::Changed)), sat, Some(&passing), true),
        ),
        (
            "outcome_ended_closed",
            outcome(&merged_ask, ended(Reason::Closed), Some(&passing), true),
        ),
        (
            "outcome_ended_gone",
            outcome(&cd, ended(Reason::Gone), Some(&running), true),
        ),
        (
            "outcome_ended_merged",
            outcome(&cd, ended(Reason::Merged), Some(&running), true),
        ),
        (
            "outcome_ended_head_moved",
            outcome(
                &told(checks_done(Some("9F8E7D6"))),
                ended(Reason::HeadMoved),
                Some(&running),
                true,
            ),
        ),
        (
            "outcome_history_too_old",
            outcome(&merged_ask, sat, Some(&passing), false),
        ),
    ];
    for (name, text) in &cases {
        gold(name, text);
        // NTF-8: nothing a GitHub user wrote but the PR's name.
        for marker in [MARKER_TITLE, MARKER_LOGIN, MARKER_CHECK] {
            assert!(!text.contains(marker), "{name}: {text}");
        }
        assert!(!text.contains('\n'), "{name}");
    }
    let get = |n: &str| cases.iter().find(|c| c.0 == n).unwrap().1.clone();
    // NTF-8's examples, byte for byte.
    assert_eq!(
        get("outcome_checks_failed"),
        "circitor: yachiko/circitor#12 checks failed (1 of 5) on 1a2b3c4. Details: circitor changes yachiko/circitor#12 --since k3x9q2mz-418"
    );
    assert_eq!(
        get("outcome_merged"),
        "circitor: yachiko/circitor#12 merged. Details: circitor changes yachiko/circitor#12 --since k3x9q2mz-418"
    );
    assert_eq!(
        get("outcome_ended_closed"),
        "circitor: yachiko/circitor#12 ended: closed. Details: circitor changes yachiko/circitor#12 --since k3x9q2mz-418"
    );
    assert_eq!(
        get("outcome_history_too_old"),
        "circitor: yachiko/circitor#12 merged. History too old; details: circitor show yachiko/circitor#12"
    );
    assert!(
        get("outcome_checks_failing").contains("checks failing (1 failed, 3 of 5 done) on 1a2b3c4")
    );
}

// NTF-8, NTF-10: expired, problem, stopped and `Ask again`.
#[test]
fn ntf7_ntf8_texts_expired_problem_stopped() {
    let t = told(checks_done(Some("1a2b3c4")));
    let now = Time::parse("2026-11-26T12:00:00Z").unwrap();
    let mut r = record(ChecksState::Pending, 5, 3, 0);
    r.stale = true;
    r.error = None;
    r.fetched_at = now.minus(Duration::from_secs(2 * 86_400 + 5 * 3_600));
    let mark = stale_mark(&r, now);
    assert_eq!(mark, " (stale, 2d5h old)");

    let stale = expired(&t, Status::ChecksPending, &mark);
    // NTF-8's example, byte for byte.
    assert_eq!(
        stale,
        "circitor: notify for yachiko/circitor#12 expired after 1d; it's checks_pending (stale, 2d5h old). Ask again: circitor notify yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --since k3x9q2mz-418"
    );
    gold("expired_stale", &stale);
    let fresh = expired(&t, Status::ChecksPending, "");
    assert!(!fresh.contains("stale"));
    gold("expired", &fresh);

    let auth = problem(&t, ProblemWhy::Auth);
    assert_eq!(
        auth,
        "circitor: can't read yachiko/circitor#12 from GitHub (auth); notify is cancelled. Details: circitor status"
    );
    gold("problem_auth", &auth);
    let failing = problem(&t, ProblemWhy::PrFailing);
    assert_eq!(
        failing,
        "circitor: can't read yachiko/circitor#12 from GitHub (pr_failing); notify is cancelled. Details: circitor status"
    );
    gold("problem_pr_failing", &failing);

    let asked = stopped(&t, true);
    assert_eq!(
        asked,
        "circitor: the daemon is stopping; notify for yachiko/circitor#12 is cancelled. Ask again: circitor notify yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --since k3x9q2mz-418 --no-spawn"
    );
    gold("stopped_no_spawn", &asked);
    let chosen = stopped(&t, false);
    assert!(chosen.ends_with("--since k3x9q2mz-418"));
    gold("stopped", &chosen);

    // `Ask again` with `--fail-fast`, `--kind`, and `--for` in whole seconds before `--since` (P15).
    let mut ff = checks_done(Some("1a2b3c4"));
    ff.fail_fast = true;
    let mut a = told(ff);
    a.life = Duration::from_secs(3_600);
    a.life_given = true;
    let again = ask_again(&a);
    assert_eq!(
        again,
        "circitor notify yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --fail-fast --for 3600 --since k3x9q2mz-418"
    );
    gold("ask_again_fail_fast_for", &again);
    let mut k = Ask::new(Until::Changed);
    k.kind = Some("review,checks".into());
    let mut kt = told(k);
    kt.life = Duration::from_secs(3_600);
    kt.life_given = true;
    let again = ask_again(&kt);
    assert_eq!(
        again,
        "circitor notify yachiko/circitor#12 --until changed --kind review,checks --for 3600 --since k3x9q2mz-418"
    );
    gold("ask_again_kind_for", &again);
    gold("expired_for_given", &expired(&kt, Status::Ready, ""));
}

// NTF-8: a record whose title, author and failed check are markers: no notice built for it holds any.
#[test]
fn ntf8_no_github_text() {
    let r = record(ChecksState::Failing, 3, 3, 2);
    let mut t = told(checks_done(Some("1a2b3c4")));
    t.cond.fail_fast = true;
    let texts = [
        confirmation(&t),
        outcome(&t, (Outcome::Satisfied, None), Some(&r), true),
        outcome(
            &t,
            (Outcome::Ended, Some(Reason::HeadMoved)),
            Some(&r),
            false,
        ),
        expired(&t, r.status, " (stale, 1m old)"),
        problem(&t, ProblemWhy::Auth),
        problem(&t, ProblemWhy::PrFailing),
        stopped(&t, true),
        ask_again(&t),
    ];
    for text in &texts {
        for marker in [MARKER_TITLE, MARKER_LOGIN, MARKER_CHECK] {
            assert!(!text.contains(marker), "{text}");
        }
        assert!(!text.contains("https://"), "{text}");
    }
}
