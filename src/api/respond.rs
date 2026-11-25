#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! The envelope, the error body, API-8's window, a record as an answer shows it (T43).

use serde::Serialize;

use crate::model::{Event, NodeId, Record};
use crate::poller::{PollerStatus, Refusal, Tier};
use crate::state::{Cursor, PrView, View};

use super::AppState;
use super::types::{ApiError, DaemonBlock};

/// The daemon block from the poller's status (API-4).
pub fn daemon_block(status: &PollerStatus) -> DaemonBlock {
    unimplemented!("T43")
}

/// A 200 with the envelope: `version`, the cursor of `view`, `daemon`, and `body`'s keys.
pub fn ok<T: Serialize>(state: &AppState, view: &View, body: T) -> axum::response::Response {
    unimplemented!("T43")
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
    unimplemented!("T43")
}

/// The same with another cursor: API-8's `limit` answers with the cursor of the last event returned.
pub fn ok_at<T: Serialize>(state: &AppState, cursor: &Cursor, body: T) -> axum::response::Response {
    unimplemented!("T43")
}

/// The envelope's JSON alone, on one line: what `ok` and `ok_at` send. A wait's late body is this (API-9).
pub fn envelope<T: Serialize>(state: &AppState, cursor: &Cursor, body: T) -> Vec<u8> {
    unimplemented!("T43")
}

/// API-3's body with the code's HTTP status.
pub fn error(state: &AppState, e: ApiError) -> axum::response::Response {
    unimplemented!("T43")
}

/// A `Refusal` from the poller as its API-3 error.
pub fn refusal(r: Refusal) -> ApiError {
    unimplemented!("T43")
}

/// A record as an answer shows it: `stale` computed (MOD-16), `checks.items` kept or left out.
pub fn present(state: &AppState, pr: &PrView, tier: Tier, with_items: bool) -> Record {
    unimplemented!("T43")
}
