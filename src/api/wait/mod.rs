#![allow(unused_variables, dead_code)] // stub: T23 removes this
//! The `wait` route and its waiters (T23).

use crate::api::AppState;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
}
