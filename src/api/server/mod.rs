//! The routes but `wait` and the notify routes (T22): `list`, `show`, `changes`, `track`, `untrack`,
//! `refresh`, `status` and `shutdown`, and API-18's wait for the first poll.
//!
//! Every handler: `poller.client_request()` first (not `status`, API-5; not `shutdown`); its parameters
//! through `Params`, ended with `finish()` (API-1); the store changed, if at all; then one `store.view()`
//! (MOD-13), from which the records, events and cursor all come; `respond::ok`, `ok_at` or `error`.

use std::time::Duration;

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, RawQuery};
use axum::routing::{delete, get, post};

use crate::clock::timeout;
use crate::model::{NodeId, PrName, PrState, Record};
use crate::poller::{FirstPoll, Tier};
use crate::state::store::{Class, Named};

use super::{
    ApiError, AppState, ErrorCode, InFlight, Params, bad_request, pr_from_path, present, refusal,
};

mod changing;
mod reading;
mod status;

#[cfg(test)]
mod tests;

/// API-18: how long `list` and `/v1/changes` wait for the first poll, by the clock, before `starting`.
const FIRST_POLL_WAIT: Duration = Duration::from_secs(10);

/// The eight routes of this task (API-5 to API-8, API-15 to API-17). `router` adds the fallbacks and the layer.
// API-2, API-4: every answer goes out through `respond::ok`, `ok_at`, `envelope` or `error`, which
// add `version`, and on a 200 the view's `cursor` and `daemon`; API-3: a refusal through `respond::refusal`.
pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/v1/prs", get(reading::list))
        .route("/v1/prs/{owner}/{repo}/{number}", get(reading::show))
        .route("/v1/changes", get(reading::changes))
        .route(
            "/v1/prs/{owner}/{repo}/{number}/changes",
            get(reading::pr_changes),
        )
        .route("/v1/track", post(changing::track))
        .route(
            "/v1/track/{owner}/{repo}/{number}",
            delete(changing::untrack),
        )
        .route("/v1/refresh", post(changing::refresh))
        .route("/v1/status", get(status::status))
        .route("/v1/shutdown", post(status::shutdown))
}

/// The query's parameters (API-1): decoded by `Params`, a name given twice is `bad_request`.
fn params(query: &RawQuery) -> Result<Params, ApiError> {
    Params::parse(query.0.as_deref())
}

/// A query with no parameter of its own: anything in it is `bad_request` naming it (API-1).
fn no_params(query: &RawQuery) -> Result<(), ApiError> {
    params(query)?.finish()
}

/// `{owner}/{repo}/{number}` of a route's path (API-1): a number that isn't one is `bad_request`.
fn path_pr(
    path: Result<Path<(String, String, String)>, PathRejection>,
) -> Result<PrName, ApiError> {
    let Path((owner, repo, number)) =
        path.map_err(|_| bad_request("the path isn't `{owner}/{repo}/{number}`"))?;
    pr_from_path(&owner, &repo, &number)
}

/// The tier records are presented for (MOD-16's `stale`).
fn tier(state: &AppState) -> Tier {
    state.poller.status().tier
}

/// An error for a PR the view holds no record of after the poller said it read it: not a state
/// the engine leaves, but answered rather than assumed away.
fn no_record(name: &PrName) -> ApiError {
    ApiError {
        code: ErrorCode::GithubUnavailable,
        message: format!("{name} has no record after it was read; try again"),
        until: None,
    }
}

/// API-18: waits for the first poll, up to 10 s of the clock, parked (TST-4). `Running` at the end is
/// `starting`; `Unanswered` is its refusal's error at once; `Finished` lets the route answer.
async fn first_poll(state: &AppState, flight: &InFlight) -> Result<(), ApiError> {
    let mut rx = state.poller.first_poll();
    let now = rx.borrow_and_update().clone();
    match now {
        FirstPoll::Finished => return Ok(()),
        FirstPoll::Unanswered(r) => return Err(refusal(r)),
        FirstPoll::Running => {}
    }
    let waiting = async {
        loop {
            if rx.changed().await.is_err() {
                // The engine is gone: nothing will finish it.
                return None;
            }
            match rx.borrow_and_update().clone() {
                FirstPoll::Running => continue,
                other => return Some(other),
            }
        }
    };
    // TST-4: waiting on the clock and the poller isn't work in flight.
    flight.park();
    let ended = timeout(&state.clock, FIRST_POLL_WAIT, waiting).await;
    flight.resume();
    match ended {
        Ok(Some(FirstPoll::Finished)) => Ok(()),
        Ok(Some(FirstPoll::Unanswered(r))) => Err(refusal(r)),
        Ok(Some(FirstPoll::Running)) | Ok(None) | Err(_) => Err(starting()),
    }
}

fn starting() -> ApiError {
    ApiError {
        code: ErrorCode::Starting,
        message: format!(
            "the first poll hasn't finished after {} s; try again in a few seconds",
            FIRST_POLL_WAIT.as_secs()
        ),
        until: None,
    }
}

/// A PR named in a route's path, with its name guard (STA-1), held until the route answers.
struct Resolved {
    id: NodeId,
    /// The record was there before this request: a `fresh` refresh is worth asking for.
    had_record: bool,
    _named: Named,
}

/// The PR of a route's path, read through when it has no record (API-7, API-18, POL-16).
/// `store.name` is taken as soon as the ID is known (STA-1).
async fn resolve(state: &AppState, flight: &InFlight, name: &PrName) -> Result<Resolved, ApiError> {
    let mut early = None;
    if let Some(tracked) = state.store.lookup(name) {
        let named = state.store.name(&tracked.id);
        if matches!(tracked.class, Class::Open | Class::Ended) {
            return Ok(Resolved {
                id: tracked.id,
                had_record: true,
                _named: named,
            });
        }
        // `Loaded` or `First`: no record yet (STA-8); read it through, still named.
        early = Some(named);
    }
    // TST-4: the engine counts the read-through itself.
    flight.park();
    let read = state.poller.read_through(name).await;
    flight.resume();
    let id = read.map_err(refusal)?;
    let named = state.store.name(&id);
    drop(early);
    Ok(Resolved {
        id,
        had_record: false,
        _named: named,
    })
}

/// API-6's order: open first, then `updated_at` newest first (none last), then `pr`.
fn sort_records(records: &mut [Record]) {
    records.sort_by(|a, b| {
        let open = |r: &Record| r.state != PrState::Open;
        open(a)
            .cmp(&open(b))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.pr.cmp(&b.pr))
    });
}

/// Every record of `view` as an answer shows it, in API-6's order.
fn all_records(
    state: &AppState,
    view: &crate::state::store::View,
    keep: impl Fn(&Record) -> bool,
    with_items: bool,
) -> Vec<Record> {
    let tier = tier(state);
    let mut records: Vec<Record> = view
        .prs
        .iter()
        .filter(|p| keep(&p.record))
        .map(|p| present(state, p, tier, with_items))
        .collect();
    sort_records(&mut records);
    records
}
