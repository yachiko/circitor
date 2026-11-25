#![allow(unused_variables, dead_code)] // stub: T22 removes this
//! The routes but `wait` and the notify routes (T22).

use crate::api::AppState;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
}
