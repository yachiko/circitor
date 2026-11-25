#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! Strict query parameters (API-1; T43).

use crate::model::PrName;
use crate::state::Cursor;

use super::types::ApiError;

/// Strict query parameters (API-1).
pub struct Params {
    pairs: Vec<(String, String)>,
}

impl Params {
    /// A name given twice is `bad_request`.
    pub fn parse(query: Option<&str>) -> Result<Params, ApiError> {
        unimplemented!("T43")
    }

    pub fn take(&mut self, name: &str) -> Option<String> {
        unimplemented!("T43")
    }

    /// `1` or `0`; absent is `false`.
    pub fn flag(&mut self, name: &str) -> Result<bool, ApiError> {
        unimplemented!("T43")
    }

    /// Never clamped.
    pub fn number(&mut self, name: &str, min: u32, max: u32) -> Result<Option<u32>, ApiError> {
        unimplemented!("T43")
    }

    /// STA-9's form.
    pub fn cursor(&mut self, name: &str) -> Result<Option<Cursor>, ApiError> {
        unimplemented!("T43")
    }

    /// Anything left: `bad_request` naming it.
    pub fn finish(self) -> Result<(), ApiError> {
        unimplemented!("T43")
    }
}

pub fn bad_request(message: impl Into<String>) -> ApiError {
    unimplemented!("T43")
}

/// `{owner}/{repo}/{number}` from a path; a number that isn't one is `bad_request`.
pub fn pr_from_path(owner: &str, repo: &str, number: &str) -> Result<PrName, ApiError> {
    unimplemented!("T43")
}
