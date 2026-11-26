//! `GET /v1/prs` (API-6), `GET /v1/prs/{owner}/{repo}/{number}` (API-7), and the two `changes` routes (API-8).

use axum::Extension;
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery, State};
use axum::response::Response;

use crate::api::{
    AppState, ChangesBody, InFlight, ListBody, RecordBody, error, ok, ok_at, present, window,
};
use crate::model::{EventKind, PrName, PrState};
use crate::poller::Refreshed;
use crate::state::MAX_EVENTS;

use super::{all_records, bad_request, first_poll, no_record, params, path_pr, resolve, tier};

/// `limit`'s range (API-8): 1 to STA-10's history.
const MAX_LIMIT: u32 = MAX_EVENTS as u32;

/// API-6: `state=open` (the default) or `all`.
fn all_states(value: Option<String>) -> Result<bool, crate::api::ApiError> {
    match value.as_deref() {
        None | Some("open") => Ok(false),
        Some("all") => Ok(true),
        Some(_) => Err(bad_request("parameter `state` must be `open` or `all`")),
    }
}

// API-6: the tracked PRs, open ones first; `since`, `state` and `checks`.
pub(super) async fn list(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    query: RawQuery,
) -> Response {
    state.poller.client_request();
    let parsed = (|| {
        let mut p = params(&query)?;
        let all = all_states(p.take("state"))?;
        let since = p.cursor("since")?;
        let checks = p.flag("checks")?;
        p.finish()?;
        Ok((all, since, checks))
    })();
    let (all, since, checks) = match parsed {
        Ok(v) => v,
        Err(e) => return error(&state, e),
    };
    // API-18: wait for the first poll.
    if let Err(e) = first_poll(&state, &flight).await {
        return error(&state, e);
    }
    // MOD-13: one view for the whole answer.
    let view = state.store.view();
    let by_state = |r: &crate::model::Record| all || r.state == PrState::Open;
    let (records, untracked, reset) = match &since {
        None => (
            all_records(&state, &view, by_state, checks),
            Vec::new(),
            false,
        ),
        // API-6: a too-old cursor gives the records `state` asks for, and nothing untracked.
        Some(c) if view.too_old(c) => (
            all_records(&state, &view, by_state, checks),
            Vec::new(),
            true,
        ),
        Some(c) => {
            // API-6: records changed after the cursor, whatever their `state`.
            let records = all_records(&state, &view, |r| r.rev > c.rev, checks);
            // API-6: PRs dropped after it and not tracked now, by the event's `id` (MOD-1).
            let mut untracked: Vec<PrName> = view
                .events_after(c.rev, None)
                .into_iter()
                .filter(|e| {
                    e.kinds
                        .iter()
                        .any(|k| matches!(k, EventKind::Untracked { .. }))
                })
                .filter(|e| view.get(&e.id).is_none() && !view.pending.contains(&e.pr))
                .map(|e| e.pr.clone())
                .collect();
            untracked.sort();
            untracked.dedup();
            (records, untracked, false)
        }
    };
    let body = ListBody {
        records,
        untracked,
        pending: view.pending.clone(),
        reset,
    };
    ok(&state, &view, body)
}

// API-7: one record, with `checks.items`; read through when it has none, refreshed first with `fresh=1`.
pub(super) async fn show(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    path: Result<Path<(String, String, String)>, PathRejection>,
    query: RawQuery,
) -> Response {
    state.poller.client_request();
    let parsed = (|| {
        let name = path_pr(path)?;
        let mut p = params(&query)?;
        let fresh = p.flag("fresh")?;
        p.finish()?;
        Ok((name, fresh))
    })();
    let (name, fresh) = match parsed {
        Ok(v) => v,
        Err(e) => return error(&state, e),
    };
    let pr = match resolve(&state, &flight, &name).await {
        Ok(pr) => pr,
        Err(e) => return error(&state, e),
    };
    if fresh && pr.had_record {
        // POL-15: the answer is from memory whatever the refresh returned (API-3).
        flight.park();
        let _: Refreshed = state.poller.refresh_pr(&pr.id).await;
        flight.resume();
    }
    let view = state.store.view();
    let Some(found) = view.get(&pr.id) else {
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

/// `since` and `limit` of a `changes` route (API-8).
fn changes_params(
    query: &RawQuery,
) -> Result<(Option<crate::state::Cursor>, Option<u32>), crate::api::ApiError> {
    let mut p = params(query)?;
    let since = p.cursor("since")?;
    let limit = p.number("limit", 1, MAX_LIMIT)?;
    p.finish()?;
    Ok((since, limit))
}

// API-8: the events after `since`, over every PR; on a reset, every record, without `checks.items`.
pub(super) async fn changes(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    query: RawQuery,
) -> Response {
    state.poller.client_request();
    let (since, limit) = match changes_params(&query) {
        Ok(v) => v,
        Err(e) => return error(&state, e),
    };
    // API-18: wait for the first poll.
    if let Err(e) = first_poll(&state, &flight).await {
        return error(&state, e);
    }
    let view = state.store.view();
    // API-8: the window's rule is `respond::window`'s.
    let w = window(&view, since.as_ref(), None, limit);
    // API-19: a reset's records leave `checks.items` out.
    let records = if w.reset {
        all_records(&state, &view, |_| true, false)
    } else {
        Vec::new()
    };
    let body = ChangesBody {
        events: w.events,
        records,
        pending: view.pending.clone(),
        more_events: w.more_events,
        reset: w.reset,
    };
    ok_at(&state, &w.cursor, body)
}

// API-8: the same for one PR, read through when it has no record (API-18); `pending` is `[]`.
pub(super) async fn pr_changes(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    path: Result<Path<(String, String, String)>, PathRejection>,
    query: RawQuery,
) -> Response {
    state.poller.client_request();
    let parsed = (|| {
        let name = path_pr(path)?;
        let (since, limit) = changes_params(&query)?;
        Ok((name, since, limit))
    })();
    let (name, since, limit) = match parsed {
        Ok(v) => v,
        Err(e) => return error(&state, e),
    };
    let pr = match resolve(&state, &flight, &name).await {
        Ok(pr) => pr,
        Err(e) => return error(&state, e),
    };
    let view = state.store.view();
    let w = window(&view, since.as_ref(), Some(&pr.id), limit);
    let records = match (w.reset, view.get(&pr.id)) {
        (true, Some(found)) => vec![present(&state, found, tier(&state), false)],
        _ => Vec::new(),
    };
    let body = ChangesBody {
        events: w.events,
        records,
        pending: Vec::new(),
        more_events: w.more_events,
        reset: w.reset,
    };
    ok_at(&state, &w.cursor, body)
}
