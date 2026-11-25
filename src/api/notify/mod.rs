#![allow(unused_variables, dead_code)] // stub: T47 removes this
//! The two notify routes (NTF-2, NTF-3; T47).

use crate::api::AppState;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
}
