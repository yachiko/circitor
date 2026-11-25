#![allow(unused_variables, dead_code)] // stub: T07 removes this
//! The real GitHub client (T07).

use crate::config::Env;
use crate::token::Token;

use super::{Answer, Failure, GitHub, Query};

/// Why the endpoint can't be used: names the variable or the host. T07 fills it.
#[derive(Debug, thiserror::Error)]
pub enum EndpointError {}

/// A reqwest client and the endpoint.
pub struct Client {
    http: reqwest::Client,
    endpoint: String,
}

impl Client {
    /// A release build: `https://api.github.com/graphql`, and `env` isn't read. A test build (TST-1):
    /// `CIRCITOR_TEST_GITHUB` from `env`, loopback only; `Err` names the variable or the host.
    pub fn from_env(env: &Env, version: &str) -> Result<Client, EndpointError> {
        unimplemented!("T07")
    }

    /// The fake GitHub's `client()`. Panics on any host but the loopback (TST-1).
    #[cfg(any(test, feature = "test-endpoint"))]
    pub fn for_tests(url: &str) -> Client {
        unimplemented!("T07")
    }
}

#[async_trait::async_trait]
impl GitHub for Client {
    async fn send(&self, token: &Token, query: &Query) -> Result<Answer, Failure> {
        unimplemented!("T07")
    }
}

/// The POST body `send` would send for `query`: `{"query": …, "variables": …}`. T37 records with these.
pub fn request_body(query: &Query) -> serde_json::Value {
    unimplemented!("T07")
}
