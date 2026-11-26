//! `GET /v1/status` (API-5) and `POST /v1/shutdown` (API-17). Neither counts as a client request,
//! and neither waits for the first poll (API-18).

use std::convert::Infallible;

use axum::extract::{RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};

use crate::api::{
    AppState, GithubInfo, LastPoll, ShutdownBody, StatusBody, TrackedCounts, envelope, error, ok,
};
use crate::daemon::{StopCause, StopHandle};
use crate::status::{Warning, WarningCode};

use super::no_params;

// API-5: health and counts; always at once, and no client activity (POL-13).
pub(super) async fn status(State(state): State<AppState>, query: RawQuery) -> Response {
    if let Err(e) = no_params(&query) {
        return error(&state, e);
    }
    let poller = state.poller.status();
    let view = state.store.view();
    let mut warnings = state.warnings.list();
    // OPS-9, API-5: `log_unwritable` while the logger fails.
    if let Some(since) = state.logger.failing_since()
        && !warnings
            .iter()
            .any(|w| w.code == WarningCode::LogUnwritable)
    {
        warnings.push(Warning {
            code: WarningCode::LogUnwritable,
            message: "log lines can't be written; the daemon keeps running".to_string(),
            since,
        });
        // `Warnings::list`'s order: by `since` (stable, so one raised at the same time stays first).
        warnings.sort_by_key(|w| w.since);
    }
    let [authored, manual, asked] = view.tracked;
    let body = StatusBody {
        mode: state.info.mode,
        started_at: state.info.started_at,
        last_poll: poller
            .last_poll
            .map(|(at, outcome)| LastPoll { at, outcome }),
        cost_last_hour: poller.cost_last_hour,
        github: GithubInfo {
            remaining: poller.remaining,
            reset_at: poller.reset_at,
            login: poller.login.clone(),
        },
        tracked: TrackedCounts {
            authored,
            manual,
            asked,
        },
        waiters: view.waiters,
        // NTF-9: the number of open subscriptions, and nothing else about one.
        subscriptions: view.subscriptions,
        owners: state.config.owners.clone(),
        token_source: poller.token_source.as_str().to_string(),
        gh_path: poller.gh_path.clone(),
        paths: state.info.paths.clone(),
        warnings,
    };
    ok(&state, &view, body)
}

/// Asks for the stop when dropped: after the server has taken the whole answer (API-17).
struct StopOnDrop(StopHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.request(StopCause::Request);
    }
}

// API-17: answers, then stops as a `SIGTERM` would. The stop itself is T28's.
pub(super) async fn shutdown(State(state): State<AppState>, query: RawQuery) -> Response {
    if let Err(e) = no_params(&query) {
        return error(&state, e);
    }
    let view = state.store.view();
    let bytes = envelope(&state, &view.cursor(), ShutdownBody {});
    let stop = StopOnDrop(state.stop.clone());
    // The closure owns the guard: it's dropped with the body, once the server has taken it all.
    let body = Full::new(Bytes::from(bytes)).map_err(move |never: Infallible| {
        let _held = &stop;
        match never {}
    });
    let mut response = Response::new(axum::body::Body::new(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}
