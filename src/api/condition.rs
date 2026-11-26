//! A condition, checked and evaluated (API-10, API-11, NTF-4; T44). Pure: no clock, no store, no I/O.

use std::time::Duration;

use crate::clock::Mono;
use crate::model::{ChecksState, NodeId, PrState, Record};
use crate::state::{Cursor, View};

use super::types::{ApiError, Ask, ErrorCode, Outcome, Reason, Until};

/// MOD-15's kinds a `kind` may name (API-10): all but `tracked` and `untracked`, in MOD-15's order.
pub const KIND_WORDS: [&str; 12] = [
    "status",
    "head",
    "checks",
    "merged",
    "closed",
    "reopened",
    "gone",
    "draft",
    "mergeable",
    "review",
    "activity",
    "edited",
];

/// A condition, checked (API-10). `ask` is kept as given, for the texts.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub ask: Ask,
    pub since: Option<Cursor>,
    pub rerun_since: Option<Cursor>,
    /// Of `KIND_WORDS`, in the order given.
    pub kinds: Option<Vec<&'static str>>,
}

impl Condition {
    /// API-10's checks, once: `sha` is 7 to 40 hex characters; `sha`, `rerun_since` and `fail_fast` go
    /// with `checks-done` only and `kind` with `changed` only; every word of `kind` is one of
    /// `KIND_WORDS`, and there's at least one; the two cursors are in STA-9's form. `Err` is
    /// `bad_request` naming the parameter (API-1).
    pub fn check(ask: Ask) -> Result<Condition, ApiError> {
        // API-10: `sha`, `rerun_since` and `fail_fast` with `checks-done` only, `kind` with `changed` only.
        if ask.until != Until::ChecksDone {
            if ask.sha.is_some() {
                return Err(refuse("sha is taken by until=checks-done only"));
            }
            if ask.rerun_since.is_some() {
                return Err(refuse("rerun_since is taken by until=checks-done only"));
            }
            if ask.fail_fast {
                return Err(refuse("fail_fast is taken by until=checks-done only"));
            }
        }
        if ask.until != Until::Changed && ask.kind.is_some() {
            return Err(refuse("kind is taken by until=changed only"));
        }
        // API-10: `sha` is 7 to 40 hex characters, kept as given.
        if let Some(sha) = &ask.sha {
            let ok = (7..=40).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit());
            if !ok {
                return Err(refuse("sha must be 7 to 40 hex characters"));
            }
        }
        // API-10, D8: `kind` is a comma-separated list of MOD-15's kinds but `tracked` and `untracked`.
        let kinds = match &ask.kind {
            None => None,
            Some(list) => {
                let mut words = Vec::new();
                for word in list.split(',') {
                    match KIND_WORDS.iter().find(|k| **k == word) {
                        Some(k) => words.push(*k),
                        None => {
                            return Err(refuse(
                                "kind must be a comma-separated list of event kinds",
                            ));
                        }
                    }
                }
                Some(words)
            }
        };
        // STA-9: both cursors in `<epoch>-<rev>` form.
        let since = match &ask.since {
            None => None,
            Some(s) => Some(
                s.parse::<Cursor>()
                    .map_err(|_| refuse("since is not a cursor"))?,
            ),
        };
        let rerun_since = match &ask.rerun_since {
            None => None,
            Some(s) => Some(
                s.parse::<Cursor>()
                    .map_err(|_| refuse("rerun_since is not a cursor"))?,
            ),
        };
        Ok(Condition {
            ask,
            since,
            rerun_since,
            kinds,
        })
    }
}

/// API-1's `bad_request`, built here: `api::bad_request` is T43's.
fn refuse(message: &str) -> ApiError {
    ApiError {
        code: ErrorCode::BadRequest,
        message: message.to_string(),
        until: None,
    }
}

/// What one wait or one subscription remembers between evaluations.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// The request's arrival: `head_moved`'s 60 s run from it (API-11, NTF-4).
    pub began: Mono,
    /// `changed` without `since`: the record's `rev` at the first evaluation that saw one.
    pub baseline: Option<u64>,
    /// `sha` matched `head_sha` once.
    pub sha_matched: bool,
    /// The view's `rev` at the last evaluation: events up to it have been looked at.
    pub looked: Option<u64>,
    /// The point was too old at the first evaluation (STA-9): `changed` holds.
    pub held_at_once: bool,
    /// An event of a named kind was recorded after the point.
    pub kind_seen: bool,
    /// A `checks` event was recorded after `rerun_since`.
    pub rerun_seen: bool,
    /// `rerun_since` was too old at the first evaluation: that part is dropped.
    pub rerun_dropped: bool,
}

impl Progress {
    /// Nothing seen yet.
    pub fn new(began: Mono) -> Progress {
        Progress {
            began,
            baseline: None,
            sha_matched: false,
            looked: None,
            held_at_once: false,
            kind_seen: false,
            rerun_seen: false,
            rerun_dropped: false,
        }
    }
}

/// API-11.
pub const HEAD_MOVED_AFTER: Duration = Duration::from_secs(60);

/// API-10 and API-11 on one view, for the PR `id`. `None`: neither the condition nor an end holds.
/// The condition is checked before the ends. Never `Outcome::Timeout`: a deadline is its caller's.
pub fn evaluate(
    cond: &Condition,
    view: &View,
    id: &NodeId,
    progress: &mut Progress,
) -> Option<(Outcome, Option<Reason>)> {
    // No record, no answer, and nothing remembered (STA-8).
    let pr = view.get(id)?;
    let record: &Record = &pr.record;
    let until = cond.ask.until;

    // NTF-4: a subscription searches the event history once. The first evaluation that sees a record
    // reads from the point; every later one only from `looked`.
    let mut after = progress.looked;
    if progress.looked.is_none() {
        // The cursors are judged on this view, once (STA-9).
        if until == Until::Changed {
            match &cond.since {
                Some(since) if view.too_old(since) => progress.held_at_once = true,
                Some(since) => after = Some(since.rev),
                None => {
                    // API-10: without `since`, `changed` waits for the change after this one.
                    progress.baseline = Some(record.rev);
                    progress.looked = Some(view.rev);
                    return None;
                }
            }
        }
        if let Some(rerun) = &cond.rerun_since {
            if view.too_old(rerun) {
                // API-10: a too-old `rerun_since` drops that part; the caller says `reset`.
                progress.rerun_dropped = true;
            } else {
                after = Some(rerun.rev);
            }
        }
    }
    progress.looked = Some(view.rev);

    // D8, API-11: an event after the point that ends the PR, of a kind `kind` doesn't name.
    let mut unnamed_end: Option<Reason> = None;
    let wants_events = (until == Until::Changed && cond.kinds.is_some())
        || (until == Until::ChecksDone && cond.rerun_since.is_some() && !progress.rerun_dropped);
    if let (true, Some(after)) = (wants_events, after) {
        // Events are this PR's by `id`, never by name: a rename changes the name.
        for event in view.events_after(after, Some(id)) {
            for kind in &event.kinds {
                let word = kind.word();
                if until == Until::ChecksDone {
                    // API-10: a `checks` event after `rerun_since`.
                    if word == "checks" {
                        progress.rerun_seen = true;
                    }
                } else if let Some(kinds) = &cond.kinds {
                    if kinds.contains(&word) {
                        progress.kind_seen = true;
                    } else if unnamed_end.is_none() {
                        unnamed_end = end_reason(word);
                    }
                }
            }
        }
    }

    // API-11: the condition first, then the ends.
    match until {
        Until::Merged => {
            if record.state == PrState::Merged {
                return Some((Outcome::Satisfied, None));
            }
            state_end(record.state, false).map(|r| (Outcome::Ended, Some(r)))
        }
        Until::Changed => {
            // API-10, STA-9: a too-old `since` holds at once, with or without `kind`.
            if progress.held_at_once {
                return Some((Outcome::Satisfied, None));
            }
            if cond.kinds.is_some() {
                // D8: an event of a named kind after the point.
                if progress.kind_seen {
                    return Some((Outcome::Satisfied, None));
                }
                // API-11: a merge, a close or `gone` whose kind isn't named ends the wait.
                return unnamed_end.map(|r| (Outcome::Ended, Some(r)));
            }
            // API-10: the record's `rev` is after the point.
            let point = cond.since.as_ref().map(|c| c.rev).or(progress.baseline)?;
            (record.rev > point).then_some((Outcome::Satisfied, None))
        }
        Until::ChecksDone => checks_done(cond, pr.timing.fetched, record, progress),
    }
}

/// API-10's `checks-done` row, then API-11's ends for it.
fn checks_done(
    cond: &Condition,
    fetched: Option<Mono>,
    record: &Record,
    progress: &mut Progress,
) -> Option<(Outcome, Option<Reason>)> {
    let checks = &record.checks;
    // API-10: `passing`, `failing` or `none`, and settled (MOD-5, MOD-6). MOD-7: a truncated
    // `failing` holds like any other.
    let done = matches!(
        checks.state,
        ChecksState::Passing | ChecksState::Failing | ChecksState::None
    );
    // API-10, D8, D9: with `fail_fast`, one failure stands in for the first part and for `settled`.
    let first = (done && checks.settled) || (cond.ask.fail_fast && checks.failed > 0);
    // API-10: `head_sha` starts with `sha`, compared without case.
    let sha_ok = match &cond.ask.sha {
        None => true,
        Some(sha) => starts_with_no_case(&record.head_sha, sha),
    };
    if cond.ask.sha.is_some() && sha_ok {
        progress.sha_matched = true;
    }
    // API-10: a `checks` event after `rerun_since`, or that part dropped as too old.
    let rerun_ok = cond.rerun_since.is_none() || progress.rerun_seen || progress.rerun_dropped;
    if first && sha_ok && rerun_ok {
        return Some((Outcome::Satisfied, None));
    }
    // API-11: a state's reason comes before `head_moved`.
    if let Some(reason) = state_end(record.state, true) {
        return Some((Outcome::Ended, Some(reason)));
    }
    // API-11: `head_moved`: matched once and then not, or never matched on a refresh sent at least
    // `HEAD_MOVED_AFTER` after the wait began. The view's send time, never the clock.
    if cond.ask.sha.is_some() && !sha_ok {
        let late = fetched.is_some_and(|f| f.since(progress.began) >= HEAD_MOVED_AFTER);
        if progress.sha_matched || late {
            return Some((Outcome::Ended, Some(Reason::HeadMoved)));
        }
    }
    None
}

/// API-11: `closed` and `gone` end `merged` and `checks-done`; `merged` ends `checks-done` too.
fn state_end(state: PrState, merged_ends: bool) -> Option<Reason> {
    match state {
        PrState::Closed => Some(Reason::Closed),
        PrState::Gone => Some(Reason::Gone),
        PrState::Merged if merged_ends => Some(Reason::Merged),
        _ => None,
    }
}

/// The reason an event's kind gives when it ends a `changed` with `kind` (API-11).
fn end_reason(word: &str) -> Option<Reason> {
    match word {
        "merged" => Some(Reason::Merged),
        "closed" => Some(Reason::Closed),
        "gone" => Some(Reason::Gone),
        _ => None,
    }
}

/// `full` starts with `prefix`, ASCII case ignored. Neither is changed or printed.
fn starts_with_no_case(full: &str, prefix: &str) -> bool {
    full.len() >= prefix.len()
        && full.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::clock::Time;
    use crate::model::tests::sample_record;
    use crate::model::{Checks, Counts, Event, EventKind, PrName, Review, Status};
    use crate::state::{PrView, Timing};

    const EPOCH: &str = "k3x9a2bq";
    const HEAD: &str = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";

    fn id() -> NodeId {
        NodeId("PR_1".into())
    }

    fn other() -> NodeId {
        NodeId("PR_2".into())
    }

    fn began() -> Mono {
        Mono::ZERO.plus(Duration::from_secs(100))
    }

    fn checks(state: ChecksState, settled: bool, failed: u32) -> Checks {
        Checks {
            state,
            settled,
            truncated: false,
            total: 3,
            completed: 2,
            failed,
            failed_items: Vec::new(),
            items: Some(Vec::new()),
        }
    }

    /// An open record at `rev`, its checks pending and unsettled.
    fn rec(rev: u64) -> Record {
        let mut r = sample_record();
        r.rev = rev;
        r.state = PrState::Open;
        r.head_sha = HEAD.into();
        r.checks = checks(ChecksState::Pending, false, 0);
        r.error = None;
        r.stale = false;
        r
    }

    fn with_state(mut r: Record, state: PrState) -> Record {
        r.state = state;
        r
    }

    fn with_checks(mut r: Record, c: Checks) -> Record {
        r.checks = c;
        r
    }

    fn ev(rev: u64, of: NodeId, kind: EventKind) -> Event {
        Event {
            rev,
            pr: PrName::parse("yachiko/circitor#12").unwrap(),
            at: Time::from_unix_millis(0),
            kinds: vec![kind],
            id: of,
        }
    }

    fn checks_kind() -> EventKind {
        EventKind::Checks {
            state: ChecksState::Passing,
            total: 1,
            completed: 1,
            failed: 0,
            failed_items: Vec::new(),
        }
    }

    fn activity() -> EventKind {
        EventKind::Activity {
            counts: Counts::default(),
            items: Vec::new(),
            more: false,
        }
    }

    fn review() -> EventKind {
        EventKind::Review {
            review: Review::Approved,
        }
    }

    fn status() -> EventKind {
        EventKind::Status {
            from: Status::ChecksPending,
            to: Status::Ready,
        }
    }

    /// Events `from..=to`, all `status` events of the other PR, with `extra` put in their place.
    fn history(from: u64, to: u64, extra: Vec<Event>) -> Vec<Event> {
        (from..=to)
            .map(|rev| {
                extra
                    .iter()
                    .find(|e| e.rev == rev)
                    .cloned()
                    .unwrap_or_else(|| ev(rev, other(), status()))
            })
            .collect()
    }

    fn view_fetched(
        rev: u64,
        record: Option<Record>,
        events: Vec<Event>,
        fetched: Option<Mono>,
    ) -> View {
        View {
            epoch: EPOCH.into(),
            rev,
            prs: record
                .into_iter()
                .map(|r| PrView {
                    id: id(),
                    record: Arc::new(r),
                    timing: Timing {
                        fetched,
                        ..Timing::default()
                    },
                })
                .collect(),
            events: events.into(),
            pending: Vec::new(),
            tracked: [0; 3],
            waiters: 0,
            subscriptions: 0,
        }
    }

    /// The view at `rev` holding events 1 to `rev`, the record fetched as the wait began.
    fn view(rev: u64, record: Record, extra: Vec<Event>) -> View {
        view_fetched(rev, Some(record), history(1, rev, extra), Some(began()))
    }

    fn at(rev: u64) -> String {
        format!("{EPOCH}-{rev}")
    }

    fn cond(ask: Ask) -> Condition {
        Condition::check(ask).unwrap()
    }

    fn ask(until: Until) -> Ask {
        Ask::new(until)
    }

    /// One evaluation, from nothing seen.
    fn once(c: &Condition, v: &View) -> Option<(Outcome, Option<Reason>)> {
        evaluate(c, v, &id(), &mut Progress::new(began()))
    }

    const SAT: Option<(Outcome, Option<Reason>)> = Some((Outcome::Satisfied, None));

    fn ended(r: Reason) -> Option<(Outcome, Option<Reason>)> {
        Some((Outcome::Ended, Some(r)))
    }

    fn refused(a: Ask, name: &str) {
        let e = Condition::check(a.clone()).expect_err(&format!("{a:?}"));
        assert_eq!(e.code, ErrorCode::BadRequest, "{a:?}");
        assert!(e.message.starts_with(name), "{a:?}: {}", e.message);
        assert_eq!(e.until, None);
    }

    // API-10: the parameters, checked once.
    #[test]
    fn api10_check() {
        let sha = |s: &str| Ask {
            sha: Some(s.into()),
            ..ask(Until::ChecksDone)
        };
        refused(sha("1a2b3c"), "sha");
        refused(sha(&"a".repeat(41)), "sha");
        refused(sha("1a2b3cz"), "sha");
        assert_eq!(
            cond(sha("1A2b3c4")).ask.sha.as_deref(),
            Some("1A2b3c4"),
            "kept as given"
        );
        assert!(Condition::check(sha(&"a".repeat(40))).is_ok());

        // Each parameter with a condition that doesn't take it.
        refused(
            Ask {
                sha: Some("1a2b3c4".into()),
                ..ask(Until::Merged)
            },
            "sha",
        );
        refused(
            Ask {
                rerun_since: Some(at(1)),
                ..ask(Until::Merged)
            },
            "rerun_since",
        );
        refused(
            Ask {
                fail_fast: true,
                ..ask(Until::Merged)
            },
            "fail_fast",
        );
        refused(
            Ask {
                kind: Some("activity".into()),
                ..ask(Until::ChecksDone)
            },
            "kind",
        );

        let kind = |s: &str| Ask {
            kind: Some(s.into()),
            ..ask(Until::Changed)
        };
        assert_eq!(
            cond(kind("activity,merged")).kinds,
            Some(vec!["activity", "merged"])
        );
        for bad in ["tracked", "untracked", "foo", "", "activity,", "Activity"] {
            refused(kind(bad), "kind");
        }

        refused(
            Ask {
                since: Some("abc".into()),
                ..ask(Until::Merged)
            },
            "since",
        );
        refused(
            Ask {
                rerun_since: Some("abc".into()),
                ..ask(Until::ChecksDone)
            },
            "rerun_since",
        );
        let c = cond(Ask {
            since: Some(at(4)),
            rerun_since: Some(at(3)),
            ..ask(Until::ChecksDone)
        });
        assert_eq!(c.since.map(|c| c.rev), Some(4));
        assert_eq!(c.rerun_since.map(|c| c.rev), Some(3));
    }

    // API-10, API-11: one case per row.
    #[test]
    fn api10_api11_table() {
        // `merged`.
        let merged = cond(ask(Until::Merged));
        assert_eq!(
            once(
                &merged,
                &view(5, with_state(rec(5), PrState::Merged), vec![])
            ),
            SAT
        );
        assert_eq!(once(&merged, &view(5, rec(5), vec![])), None);
        assert_eq!(
            once(
                &merged,
                &view(5, with_state(rec(5), PrState::Closed), vec![])
            ),
            ended(Reason::Closed)
        );
        assert_eq!(
            once(&merged, &view(5, with_state(rec(5), PrState::Gone), vec![])),
            ended(Reason::Gone)
        );

        // `checks-done`: passing, failing and none, settled and not; pending never.
        let done = cond(ask(Until::ChecksDone));
        for state in [
            ChecksState::Passing,
            ChecksState::Failing,
            ChecksState::None,
        ] {
            let failed = u32::from(state == ChecksState::Failing);
            let settled = with_checks(rec(5), checks(state, true, failed));
            assert_eq!(once(&done, &view(5, settled, vec![])), SAT, "{state:?}");
            let unsettled = with_checks(rec(5), checks(state, false, failed));
            assert_eq!(once(&done, &view(5, unsettled, vec![])), None, "{state:?}");
        }
        let pending = with_checks(rec(5), checks(ChecksState::Pending, true, 0));
        assert_eq!(once(&done, &view(5, pending, vec![])), None);

        // MOD-7: a truncated `failing` holds.
        let mut truncated = checks(ChecksState::Failing, true, 1);
        truncated.truncated = true;
        assert_eq!(
            once(&done, &view(5, with_checks(rec(5), truncated), vec![])),
            SAT
        );

        // `sha`: a prefix, without case.
        let passing = with_checks(rec(5), checks(ChecksState::Passing, true, 0));
        for (sha, answer) in [
            ("1A2B3C4", SAT),
            ("1a2b3c4d5e6f", SAT),
            (HEAD, SAT),
            ("fffffff", None),
            ("2b3c4d5", None),
        ] {
            let c = cond(Ask {
                sha: Some(sha.into()),
                ..ask(Until::ChecksDone)
            });
            assert_eq!(once(&c, &view(5, passing.clone(), vec![])), answer, "{sha}");
        }

        // Merged: with checks done `satisfied`, with them pending `ended`, `merged`.
        assert_eq!(
            once(
                &done,
                &view(5, with_state(passing.clone(), PrState::Merged), vec![])
            ),
            SAT
        );
        assert_eq!(
            once(&done, &view(5, with_state(rec(5), PrState::Merged), vec![])),
            ended(Reason::Merged)
        );
        // Closed and gone end `checks-done`.
        assert_eq!(
            once(&done, &view(5, with_state(rec(5), PrState::Closed), vec![])),
            ended(Reason::Closed)
        );
        assert_eq!(
            once(&done, &view(5, with_state(rec(5), PrState::Gone), vec![])),
            ended(Reason::Gone)
        );

        // A merge, a close or `gone` satisfies `changed` like any change.
        let changed = cond(Ask {
            since: Some(at(4)),
            ..ask(Until::Changed)
        });
        for (state, kind) in [
            (PrState::Merged, EventKind::Merged),
            (PrState::Closed, EventKind::Closed),
            (PrState::Gone, EventKind::Gone),
        ] {
            let v = view(5, with_state(rec(5), state), vec![ev(5, id(), kind)]);
            assert_eq!(once(&changed, &v), SAT, "{state:?}");
        }
    }

    // API-10: `changed` without `since` waits for the change after the first evaluation.
    #[test]
    fn api10_changed_baseline() {
        let c = cond(ask(Until::Changed));
        let mut p = Progress::new(began());
        assert_eq!(evaluate(&c, &view(5, rec(5), vec![]), &id(), &mut p), None);
        assert_eq!(p.baseline, Some(5));
        // Another PR's change moves the counter, not this record.
        assert_eq!(evaluate(&c, &view(6, rec(5), vec![]), &id(), &mut p), None);
        assert_eq!(evaluate(&c, &view(7, rec(7), vec![]), &id(), &mut p), SAT);
        assert_eq!(p.baseline, Some(5));

        // With `since`, a record above it answers at the first call.
        let since = |rev| {
            cond(Ask {
                since: Some(at(rev)),
                ..ask(Until::Changed)
            })
        };
        let mut p = Progress::new(began());
        assert_eq!(
            evaluate(&since(4), &view(5, rec(5), vec![]), &id(), &mut p),
            SAT
        );
        assert_eq!(p.baseline, None);
        assert_eq!(once(&since(5), &view(5, rec(5), vec![])), None);
    }

    // API-10, API-11, D8: `kind` narrows `changed`; an unnamed end ends it.
    #[test]
    fn api10_kind() {
        let kinds = |k: &str| {
            cond(Ask {
                since: Some(at(2)),
                kind: Some(k.into()),
                ..ask(Until::Changed)
            })
        };
        let activity_only = kinds("activity");
        assert_eq!(
            once(
                &activity_only,
                &view(3, rec(3), vec![ev(3, id(), checks_kind())])
            ),
            None
        );
        assert_eq!(
            once(
                &activity_only,
                &view(3, rec(3), vec![ev(3, id(), activity())])
            ),
            SAT
        );
        // By `id`: another PR's activity, even under this name, doesn't count.
        assert_eq!(
            once(
                &activity_only,
                &view(3, rec(3), vec![ev(3, other(), activity())])
            ),
            None
        );
        // At or before the point doesn't count.
        assert_eq!(
            once(
                &activity_only,
                &view(3, rec(3), vec![ev(2, id(), activity())])
            ),
            None
        );

        for (state, kind, word, reason) in [
            (PrState::Merged, EventKind::Merged, "merged", Reason::Merged),
            (PrState::Closed, EventKind::Closed, "closed", Reason::Closed),
            (PrState::Gone, EventKind::Gone, "gone", Reason::Gone),
        ] {
            let v = view(3, with_state(rec(3), state), vec![ev(3, id(), kind)]);
            assert_eq!(once(&activity_only, &v), ended(reason), "{word}");
            assert_eq!(once(&kinds(&format!("activity,{word}")), &v), SAT, "{word}");
        }

        // A named kind seen with an unnamed end: the condition comes first.
        let v = view(
            4,
            with_state(rec(4), PrState::Merged),
            vec![ev(3, id(), activity()), ev(4, id(), EventKind::Merged)],
        );
        assert_eq!(once(&activity_only, &v), SAT);
    }

    // API-10, D8, D9: `fail_fast` answers at the first failure, `settled` or not.
    #[test]
    fn api10_fail_fast() {
        let fast = cond(Ask {
            fail_fast: true,
            ..ask(Until::ChecksDone)
        });
        let plain = cond(ask(Until::ChecksDone));
        let one_failed = with_checks(rec(5), checks(ChecksState::Pending, true, 1));
        assert_eq!(once(&fast, &view(5, one_failed.clone(), vec![])), SAT);
        assert_eq!(once(&plain, &view(5, one_failed, vec![])), None);
        let unsettled = with_checks(rec(5), checks(ChecksState::Pending, false, 1));
        assert_eq!(once(&fast, &view(5, unsettled.clone(), vec![])), SAT);
        assert_eq!(once(&plain, &view(5, unsettled.clone(), vec![])), None);

        // `sha` still has to hold.
        let fast_sha = cond(Ask {
            fail_fast: true,
            sha: Some("fffffff".into()),
            ..ask(Until::ChecksDone)
        });
        assert_eq!(once(&fast_sha, &view(5, unsettled, vec![])), None);

        // No failure: `checks-done` as ever.
        let none_failed = with_checks(rec(5), checks(ChecksState::Pending, true, 0));
        assert_eq!(once(&fast, &view(5, none_failed, vec![])), None);
        let passing = with_checks(rec(5), checks(ChecksState::Passing, true, 0));
        assert_eq!(once(&fast, &view(5, passing, vec![])), SAT);
        let passing_unsettled = with_checks(rec(5), checks(ChecksState::Passing, false, 0));
        assert_eq!(once(&fast, &view(5, passing_unsettled, vec![])), None);
    }

    // API-10: `rerun_since` waits for a `checks` event after it.
    #[test]
    fn api10_rerun_since() {
        let c = cond(Ask {
            rerun_since: Some(at(3)),
            ..ask(Until::ChecksDone)
        });
        let passing = with_checks(rec(5), checks(ChecksState::Passing, true, 0));
        // A `checks` event at the point, not after it, and another PR's after it.
        let before = vec![
            ev(3, id(), checks_kind()),
            ev(4, id(), activity()),
            ev(5, other(), checks_kind()),
        ];
        let mut p = Progress::new(began());
        assert_eq!(
            evaluate(&c, &view(5, passing.clone(), before), &id(), &mut p),
            None
        );
        assert!(!p.rerun_seen && !p.rerun_dropped);
        // One after it, on a later view.
        let mut later = passing.clone();
        later.rev = 6;
        assert_eq!(
            evaluate(
                &c,
                &view(6, later, vec![ev(6, id(), checks_kind())]),
                &id(),
                &mut p
            ),
            SAT
        );
        assert!(p.rerun_seen);
        // One after it, at the first call.
        assert_eq!(
            once(&c, &view(5, passing, vec![ev(4, id(), checks_kind())])),
            SAT
        );
    }

    // API-11: `head_moved`, and a state's reason before it.
    #[test]
    fn api11_head_moved() {
        let c = cond(Ask {
            sha: Some("1a2b3c4".into()),
            ..ask(Until::ChecksDone)
        });
        let moved = |mut r: Record| {
            r.head_sha = "ffffffffffffffffffffffffffffffffffffffff".into();
            r
        };

        // Matched, then another head.
        let mut p = Progress::new(began());
        assert_eq!(evaluate(&c, &view(5, rec(5), vec![]), &id(), &mut p), None);
        assert!(p.sha_matched);
        assert_eq!(
            evaluate(&c, &view(6, moved(rec(6)), vec![]), &id(), &mut p),
            ended(Reason::HeadMoved)
        );

        // Never matched: by the send time of the read, 59 s then 60 s after the wait began.
        let after = |s| {
            view_fetched(
                5,
                Some(moved(rec(5))),
                history(1, 5, vec![]),
                Some(began().plus(Duration::from_secs(s))),
            )
        };
        let mut p = Progress::new(began());
        assert_eq!(evaluate(&c, &after(59), &id(), &mut p), None);
        assert_eq!(
            evaluate(&c, &after(60), &id(), &mut p),
            ended(Reason::HeadMoved)
        );
        // Never fetched: never late.
        let unfetched = view_fetched(5, Some(moved(rec(5))), history(1, 5, vec![]), None);
        assert_eq!(once(&c, &unfetched), None);

        // A closed PR whose head also moved: `closed`.
        let mut p = Progress::new(began());
        assert_eq!(evaluate(&c, &view(5, rec(5), vec![]), &id(), &mut p), None);
        let closed = with_state(moved(rec(6)), PrState::Closed);
        assert_eq!(
            evaluate(&c, &view(6, closed, vec![]), &id(), &mut p),
            ended(Reason::Closed)
        );
    }

    // NTF-4: a subscription searches the event history once; what it saw is remembered.
    #[test]
    fn ntf4_memory() {
        let kinds = |k: &str| {
            cond(Ask {
                since: Some(at(0)),
                kind: Some(k.into()),
                ..ask(Until::Changed)
            })
        };
        // The second view's history starts at event 401: the cursor `…-0` is now too old.
        let later = |extra| view(500, rec(500), extra);
        let later = |extra| {
            let v = later(extra);
            View {
                events: v.events[400..].to_vec().into(),
                ..v
            }
        };
        assert!(later(vec![]).too_old(&Cursor {
            epoch: EPOCH.into(),
            rev: 0
        }));

        // A kind seen at event 3 still holds.
        let seen = kinds("activity");
        let mut p = Progress::new(began());
        let first = view(5, rec(5), vec![ev(3, id(), activity())]);
        assert_eq!(evaluate(&seen, &first, &id(), &mut p), SAT);
        assert_eq!(evaluate(&seen, &later(vec![]), &id(), &mut p), SAT);
        assert!(!p.held_at_once);

        // One not seen doesn't start holding.
        let unseen = kinds("review");
        let mut p = Progress::new(began());
        assert_eq!(evaluate(&unseen, &first, &id(), &mut p), None);
        assert_eq!(p.looked, Some(5));
        assert_eq!(evaluate(&unseen, &later(vec![]), &id(), &mut p), None);
        assert!(!p.held_at_once && !p.kind_seen);
        assert_eq!(p.looked, Some(500));
        // A later event of a named kind, above `looked`, is found.
        let v = view(501, rec(501), vec![ev(501, id(), review())]);
        let v = View {
            events: v.events[401..].to_vec().into(),
            ..v
        };
        assert_eq!(evaluate(&unseen, &v, &id(), &mut p), SAT);

        // `rerun_seen` stays once the cursor is too old.
        let rerun = cond(Ask {
            rerun_since: Some(at(0)),
            ..ask(Until::ChecksDone)
        });
        let mut p = Progress::new(began());
        assert_eq!(
            evaluate(
                &rerun,
                &view(5, rec(5), vec![ev(3, id(), checks_kind())]),
                &id(),
                &mut p
            ),
            None
        );
        assert!(p.rerun_seen);
        let mut done = later(vec![]);
        done.prs[0].record = Arc::new(with_checks(rec(500), checks(ChecksState::Passing, true, 0)));
        assert_eq!(evaluate(&rerun, &done, &id(), &mut p), SAT);
        assert!(!p.rerun_dropped);
    }

    // STA-9: a cursor too old at the first evaluation.
    #[test]
    fn sta9_too_old_at_first() {
        let other_epoch = "zzzzzzzz-3".to_string();
        for kind in [None, Some("review".to_string())] {
            let c = cond(Ask {
                since: Some(other_epoch.clone()),
                kind: kind.clone(),
                ..ask(Until::Changed)
            });
            let mut p = Progress::new(began());
            assert_eq!(
                evaluate(&c, &view(5, rec(5), vec![]), &id(), &mut p),
                SAT,
                "{kind:?}"
            );
            assert!(p.held_at_once);
        }

        let c = cond(Ask {
            rerun_since: Some(other_epoch),
            ..ask(Until::ChecksDone)
        });
        let mut p = Progress::new(began());
        assert_eq!(evaluate(&c, &view(5, rec(5), vec![]), &id(), &mut p), None);
        assert!(p.rerun_dropped);
        let passing = with_checks(rec(6), checks(ChecksState::Passing, true, 0));
        assert_eq!(evaluate(&c, &view(6, passing, vec![]), &id(), &mut p), SAT);
    }

    // STA-8: no record, no answer, nothing remembered.
    #[test]
    fn no_record_is_none() {
        let v = view_fetched(5, None, history(1, 5, vec![]), Some(began()));
        for a in [
            ask(Until::Changed),
            Ask {
                since: Some(at(1)),
                kind: Some("activity".into()),
                ..ask(Until::Changed)
            },
            ask(Until::Merged),
            Ask {
                sha: Some("1a2b3c4".into()),
                rerun_since: Some(at(1)),
                ..ask(Until::ChecksDone)
            },
        ] {
            let c = cond(a);
            let mut p = Progress::new(began());
            assert_eq!(evaluate(&c, &v, &id(), &mut p), None);
            assert_eq!(p, Progress::new(began()));
        }
    }
}
