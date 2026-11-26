//! `POST /v1/track`, `DELETE /v1/track/{owner}/{repo}/{number}` (API-15) and `POST /v1/refresh` (API-16).

use axum::Extension;
use axum::body::Body;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use serde::Deserialize;

use crate::api::{AppState, InFlight, RecordBody, RefreshBody, error, ok, present, refusal};
use crate::model::PrName;

use super::{bad_request, no_params, no_record, path_pr, tier};

/// The most a `track` body may hold: `{"pr": "owner/repo#12"}` is far less.
const MAX_BODY: usize = 64 * 1024;

/// `track`'s body, strict (API-1).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackIn {
    pr: String,
}

async fn track_body(body: Body) -> Result<PrName, crate::api::ApiError> {
    let bytes = axum::body::to_bytes(body, MAX_BODY)
        .await
        .map_err(|_| bad_request("the body can't be read, or is too long"))?;
    // API-1: an unknown key, or a body that isn't JSON, is `bad_request` naming it.
    let input: TrackIn = serde_json::from_slice(&bytes).map_err(|e| {
        bad_request(format!(
            "the body must be {{\"pr\": \"owner/repo#12\"}}: {e}"
        ))
    })?;
    PrName::parse(&input.pr).map_err(|_| {
        bad_request(format!(
            "`pr` must be `owner/repo#number`, not `{}`",
            input.pr
        ))
    })
}

// API-15: track as `manual` through the poller; the file written before the answer (STA-7).
pub(super) async fn track(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    query: RawQuery,
    body: Body,
) -> Response {
    state.poller.client_request();
    if let Err(e) = no_params(&query) {
        return error(&state, e);
    }
    let name = match track_body(body).await {
        Ok(n) => n,
        Err(e) => return error(&state, e),
    };
    // STA-1: named from the moment the ID is known.
    let early = state.store.lookup(&name).map(|t| state.store.name(&t.id));
    // TST-4: the engine counts the fetch, the saver the write.
    flight.park();
    let tracked = state.poller.track(&name).await;
    // STA-7: a loaded PR's saved source is set before its read-through, so the file is written
    // whether it succeeded or was refused. A refusal undoes nothing.
    state.flush.flush().await;
    flight.resume();
    let id = match tracked {
        Ok(id) => id,
        Err(r) => return error(&state, refusal(r)),
    };
    let _named = state.store.name(&id);
    drop(early);
    let view = state.store.view();
    let Some(found) = view.get(&id) else {
        return error(&state, no_record(&name));
    };
    let record = present(&state, found, tier(&state), true);
    ok(
        &state,
        &view,
        RecordBody {
            record: Some(record),
        },
    )
}

// API-15: `untrack` is the store's alone, never the poller's; the file written first when it changed (STA-7).
pub(super) async fn untrack(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    path: Result<Path<(String, String, String)>, PathRejection>,
    query: RawQuery,
) -> Response {
    state.poller.client_request();
    let name = match path_pr(path).and_then(|n| no_params(&query).map(|()| n)) {
        Ok(n) => n,
        Err(e) => return error(&state, e),
    };
    let id = state.store.lookup(&name).map(|t| t.id);
    let _named = id.as_ref().map(|id| state.store.name(id));
    let untracked = state.store.untrack(&name);
    if untracked.file_changed {
        flight.park();
        state.flush.flush().await;
        flight.resume();
    }
    let view = state.store.view();
    // Still tracked: the record of this view. Dropped: its last record, as it was (`stale: false`,
    // its `untracked` event's `rev`). Never tracked, or tracked with no record yet: `null`.
    let record = match id.as_ref().and_then(|id| view.get(id)) {
        Some(found) => Some(present(&state, found, tier(&state), true)),
        None => untracked.record,
    };
    ok(&state, &view, RecordBody { record })
}

// API-16: a poll now; its outcome is HTTP 200 whatever it is.
pub(super) async fn refresh(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    query: RawQuery,
) -> Response {
    state.poller.client_request();
    if let Err(e) = no_params(&query) {
        return error(&state, e);
    }
    flight.park();
    let report = state.poller.poll_now().await;
    flight.resume();
    let view = state.store.view();
    ok(
        &state,
        &view,
        RefreshBody {
            outcome: report.outcome,
            cost: report.cost,
        },
    )
}
