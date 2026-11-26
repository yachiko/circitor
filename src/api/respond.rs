//! The envelope, the error body, API-8's window, a record as an answer shows it (T43).

use axum::http::{StatusCode, header};
use serde::Serialize;

use crate::model::{Event, NodeId, Record};
use crate::poller::{PollerStatus, Refusal, RefusalCode, Tier, cadence};
use crate::state::{Cursor, PrView, View};

use super::AppState;
use super::types::{ApiError, DaemonBlock, Envelope, ErrorBody, ErrorCode};

/// The daemon block from the poller's status (API-4).
pub fn daemon_block(status: &PollerStatus) -> DaemonBlock {
    DaemonBlock {
        tier: status.tier,
        last_poll_at: status.last_poll_at,
        problem: status.problem.as_ref().map(|p| p.code),
        message: status.problem.as_ref().map(|p| p.message.clone()),
        until: status.problem.as_ref().and_then(|p| p.until),
    }
}

/// One line of JSON, no newline (API-2): `version`, `cursor`, `daemon`, then the body's keys.
fn envelope_with<T: Serialize>(
    version: &str,
    daemon: DaemonBlock,
    cursor: &Cursor,
    body: T,
) -> Vec<u8> {
    let envelope = Envelope {
        version: version.to_string(),
        cursor: cursor.to_string(),
        daemon,
        body,
    };
    // The answer types hold strings, numbers and lists with string keys only: serialising can't fail.
    serde_json::to_vec(&envelope).expect("an answer always serialises")
}

/// API-3's body, and its status.
fn error_with(version: &str, e: ApiError) -> (StatusCode, Vec<u8>) {
    let status = e.code.http();
    let body = ErrorBody {
        version: version.to_string(),
        error: e,
    };
    // As above: can't fail.
    (
        status,
        serde_json::to_vec(&body).expect("an error always serialises"),
    )
}

fn json_response(status: StatusCode, body: Vec<u8>) -> axum::response::Response {
    let mut response = axum::response::Response::new(axum::body::Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

/// A 200 with the envelope: `version`, the cursor of `view`, `daemon`, and `body`'s keys.
pub fn ok<T: Serialize>(state: &AppState, view: &View, body: T) -> axum::response::Response {
    ok_at(state, &view.cursor(), body)
}

/// API-8's rule, once, for `changes` and for `wait`: the events after `since`, for every PR or for one,
/// cut at `limit`; `reset` for a too-old cursor, which gets no events unless it's from another epoch and
/// the history still holds every event of this one; without `since`, no events. `cursor` is the view's,
/// or the last returned event's when `limit` cut the list.
pub struct Window {
    pub events: Vec<Event>,
    pub more_events: bool,
    pub reset: bool,
    pub cursor: Cursor,
}

pub fn window(
    view: &View,
    since: Option<&Cursor>,
    pr: Option<&NodeId>,
    limit: Option<u32>,
) -> Window {
    // API-8: without `since`, no events, just the cursor.
    let (mut events, reset): (Vec<Event>, bool) = match since {
        None => (Vec::new(), false),
        Some(c) if !view.too_old(c) => (
            view.events_after(c.rev, pr).into_iter().cloned().collect(),
            false,
        ),
        Some(c) => {
            // API-8: a cursor from another epoch gets every event of this one while the history
            // still holds them all (it starts at revision 1, or nothing was ever recorded).
            let all_held = view.rev == 0 || view.events.first().is_some_and(|e| e.rev == 1);
            if c.epoch != view.epoch && all_held {
                (
                    view.events_after(0, pr).into_iter().cloned().collect(),
                    true,
                )
            } else {
                (Vec::new(), true)
            }
        }
    };
    let mut more_events = false;
    let mut cursor = view.cursor();
    if let Some(limit) = limit {
        let limit = limit as usize;
        if events.len() > limit {
            events.truncate(limit);
            more_events = true;
            if let Some(last) = events.last() {
                cursor = Cursor {
                    epoch: view.epoch.clone(),
                    rev: last.rev,
                };
            }
        }
    }
    Window {
        events,
        more_events,
        reset,
        cursor,
    }
}

/// The same with another cursor: API-8's `limit` answers with the cursor of the last event returned.
pub fn ok_at<T: Serialize>(state: &AppState, cursor: &Cursor, body: T) -> axum::response::Response {
    json_response(StatusCode::OK, envelope(state, cursor, body))
}

/// The envelope's JSON alone, on one line: what `ok` and `ok_at` send. A wait's late body is this (API-9).
pub fn envelope<T: Serialize>(state: &AppState, cursor: &Cursor, body: T) -> Vec<u8> {
    let daemon = daemon_block(&state.poller.status());
    envelope_with(&state.info.version, daemon, cursor, body)
}

/// API-3's body with the code's HTTP status.
pub fn error(state: &AppState, e: ApiError) -> axum::response::Response {
    error_response(&state.info.version, e)
}

/// `error` for a caller with only the version: the router's fallbacks.
pub(super) fn error_response(version: &str, e: ApiError) -> axum::response::Response {
    let (status, body) = error_with(version, e);
    json_response(status, body)
}

/// A `Refusal` from the poller as its API-3 error.
pub fn refusal(r: Refusal) -> ApiError {
    let code = match r.code {
        RefusalCode::NotFound => ErrorCode::NotFound,
        RefusalCode::Auth => ErrorCode::Auth,
        RefusalCode::GithubUnavailable => ErrorCode::GithubUnavailable,
        RefusalCode::RateLimited => ErrorCode::RateLimited,
        RefusalCode::Limit => ErrorCode::Limit,
    };
    ApiError {
        code,
        message: r.message,
        until: r.until,
    }
}

/// A record as an answer shows it: `stale` computed (MOD-16), `checks.items` kept or left out.
pub fn present(state: &AppState, pr: &PrView, tier: Tier, with_items: bool) -> Record {
    let mut record = (*pr.record).clone();
    record.stale = cadence::stale(&record, &pr.timing, tier, &state.config, state.clock.mono());
    if !with_items {
        record.checks.items = None;
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::{ListBody, Until};
    use crate::clock::Time;
    use crate::model::{EventKind, PrName, Source};
    use crate::poller::{DaemonProblem, ProblemCode};
    use std::sync::Arc;

    fn status(problem: Option<DaemonProblem>) -> PollerStatus {
        PollerStatus {
            tier: Tier::Active,
            last_poll_at: None,
            last_poll: None,
            problem,
            problem_since: None,
            cost_last_hour: 0,
            remaining: None,
            reset_at: None,
            login: None,
            token_source: crate::token::TokenSourceKind::Gh,
            gh_path: "gh".into(),
        }
    }

    fn cursor(rev: u64) -> Cursor {
        Cursor {
            epoch: "k3x9a2bq".into(),
            rev,
        }
    }

    // API-3: each code gives its status and the body's shape.
    #[test]
    fn api3_error_body() {
        let all = [
            (ErrorCode::BadRequest, 400, "bad_request"),
            (ErrorCode::NotFound, 404, "not_found"),
            (ErrorCode::UnknownRoute, 404, "unknown_route"),
            (ErrorCode::Auth, 503, "auth"),
            (ErrorCode::Starting, 503, "starting"),
            (ErrorCode::GithubUnavailable, 503, "github_unavailable"),
            (ErrorCode::RateLimited, 503, "rate_limited"),
            (ErrorCode::Limit, 409, "limit"),
            (ErrorCode::NoSession, 409, "no_session"),
        ];
        for (code, status, word) in all {
            let (s, body) = error_with(
                "1.2.3+test",
                ApiError {
                    code,
                    message: "m".into(),
                    until: None,
                },
            );
            assert_eq!(s.as_u16(), status);
            assert_eq!(
                String::from_utf8(body).unwrap(),
                format!(
                    r#"{{"version":"1.2.3+test","error":{{"code":"{word}","message":"m","until":null}}}}"#
                )
            );
        }
        let until = Time::parse("2026-11-24T22:00:00Z").unwrap();
        let (_, body) = error_with(
            "v",
            ApiError {
                code: ErrorCode::RateLimited,
                message: "m".into(),
                until: Some(until),
            },
        );
        assert!(
            String::from_utf8(body)
                .unwrap()
                .ends_with(r#""until":"2026-11-24T22:00:00Z"}}"#)
        );
        let response = error_response("v", bad_request_for_test());
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    }

    fn bad_request_for_test() -> ApiError {
        crate::api::bad_request("x")
    }

    // API-3: five refusals, five codes.
    #[test]
    fn api3_refusal_codes() {
        let until = Some(Time::parse("2026-11-24T22:30:00Z").unwrap());
        for (code, want) in [
            (RefusalCode::NotFound, ErrorCode::NotFound),
            (RefusalCode::Auth, ErrorCode::Auth),
            (RefusalCode::GithubUnavailable, ErrorCode::GithubUnavailable),
            (RefusalCode::RateLimited, ErrorCode::RateLimited),
            (RefusalCode::Limit, ErrorCode::Limit),
        ] {
            let e = refusal(Refusal {
                code,
                message: "words".into(),
                until,
            });
            assert_eq!(e.code, want);
            assert_eq!(e.message, "words");
            assert_eq!(e.until, until);
        }
    }

    // API-2, API-4: key order, one line, the daemon block.
    #[test]
    fn api2_api4_envelope() {
        let quiet = daemon_block(&status(None));
        assert_eq!(
            (quiet.problem, quiet.message.clone(), quiet.until),
            (None, None, None)
        );
        let until = Time::parse("2026-11-24T23:00:00Z").unwrap();
        let limited = daemon_block(&status(Some(DaemonProblem {
            code: ProblemCode::RateLimited,
            message: "wait".into(),
            until: Some(until),
        })));
        assert_eq!(limited.problem, Some(ProblemCode::RateLimited));
        assert_eq!(limited.message.as_deref(), Some("wait"));
        assert_eq!(limited.until, Some(until));

        let body = ListBody {
            records: Vec::new(),
            untracked: Vec::new(),
            pending: vec![PrName::parse("o/r#1").unwrap()],
            reset: false,
        };
        let bytes = envelope_with("1.2.3+test", quiet, &cursor(7), &body);
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains('\n'));
        assert_eq!(
            text,
            r#"{"version":"1.2.3+test","cursor":"k3x9a2bq-7","daemon":{"tier":"active","last_poll_at":null,"problem":null,"message":null,"until":null},"records":[],"untracked":[],"pending":["o/r#1"],"reset":false}"#
        );
        let text = String::from_utf8(envelope_with("v", limited, &cursor(1), &body)).unwrap();
        assert!(text.contains(
            r#""problem":"rate_limited","message":"wait","until":"2026-11-24T23:00:00Z""#
        ));
        let _ = Until::Merged;
    }

    fn event(rev: u64, id: &str, pr: &str) -> Event {
        Event {
            rev,
            pr: PrName::parse(pr).unwrap(),
            at: Time::from_unix_millis(0),
            kinds: vec![EventKind::Tracked {
                source: Source::Authored,
            }],
            id: NodeId(id.into()),
        }
    }

    fn view(rev: u64, events: Vec<Event>) -> View {
        View {
            epoch: "k3x9a2bq".into(),
            rev,
            prs: Vec::new(),
            events: Arc::from(events),
            pending: Vec::new(),
            tracked: [0; 3],
            waiters: 0,
            subscriptions: 0,
        }
    }

    fn revs(w: &Window) -> Vec<u64> {
        w.events.iter().map(|e| e.rev).collect()
    }

    // API-8: the window's rules, over hand-built views.
    #[test]
    fn api8_window() {
        let events = |from: u64| {
            (from..=5)
                .map(|r| {
                    if r % 2 == 1 {
                        event(r, "A", "o/r#1")
                    } else {
                        event(r, "B", "o/r#2")
                    }
                })
                .collect::<Vec<_>>()
        };
        let full = view(5, events(1));
        let w = window(&full, None, None, None);
        assert!(w.events.is_empty() && !w.reset && !w.more_events);
        assert_eq!(w.cursor, cursor(5));
        let w = window(&full, Some(&cursor(2)), None, None);
        assert_eq!(revs(&w), [3, 4, 5]);
        assert!(!w.reset);
        assert_eq!(w.cursor, cursor(5));
        let w = window(&full, Some(&cursor(2)), None, Some(2));
        assert_eq!(revs(&w), [3, 4]);
        assert!(w.more_events && !w.reset);
        assert_eq!(w.cursor, cursor(4));
        // A limit that doesn't cut.
        let w = window(&full, Some(&cursor(2)), None, Some(3));
        assert!(!w.more_events);
        assert_eq!(w.cursor, cursor(5));
        let w = window(&full, Some(&cursor(5)), None, None);
        assert!(w.events.is_empty() && !w.reset);
        let w = window(&full, Some(&cursor(6)), None, None);
        assert!(w.events.is_empty() && w.reset);
        assert_eq!(w.cursor, cursor(5));

        let cut = view(5, events(3));
        let w = window(&cut, Some(&cursor(1)), None, None);
        assert!(w.events.is_empty() && w.reset);
        let w = window(&cut, Some(&cursor(2)), None, None);
        assert_eq!(revs(&w), [3, 4, 5]);
        assert!(!w.reset);

        let other = Cursor {
            epoch: "zzzzzzzz".into(),
            rev: 9,
        };
        let w = window(&full, Some(&other), None, None);
        assert_eq!(revs(&w), [1, 2, 3, 4, 5]);
        assert!(w.reset);
        let w = window(&cut, Some(&other), None, None);
        assert!(w.events.is_empty() && w.reset);

        // `pr` filters by `id`, after a rename.
        let mut renamed = events(1);
        renamed[4] = event(5, "A", "o/renamed#1");
        let view = view(5, renamed);
        let a = NodeId("A".into());
        let w = window(&view, Some(&cursor(0)), Some(&a), None);
        assert_eq!(revs(&w), [1, 3, 5]);
        let w = window(&view, Some(&other), Some(&a), None);
        assert_eq!(revs(&w), [1, 3, 5]);
    }
}
