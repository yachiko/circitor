//! The real GitHub client (T07): builds the one GraphQL query a `Query` describes, posts it, and
//! decodes the answer (`plan/contracts/github.md`).

use http::HeaderValue;
use http::header::{AUTHORIZATION, CONTENT_TYPE};

use crate::config::Env;
use crate::token::Token;

use super::{Answer, Failure, GitHub, Query, Unavailable};

mod decode;
mod query;
#[cfg(test)]
mod tests;

/// POL-1, OPS-16: the one endpoint a release build talks to.
pub const ENDPOINT: &str = "https://api.github.com/graphql";

/// TST-1: the variable a test build takes its endpoint from. Not in a release
/// build, whose binary holds no `CIRCITOR_TEST_` name.
#[cfg(any(test, feature = "test-endpoint"))]
pub const TEST_GITHUB_VAR: &str = "CIRCITOR_TEST_GITHUB";

/// Why the endpoint can't be used: names the variable or the host. Never holds a token.
#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    /// TST-1: a test build without `CIRCITOR_TEST_GITHUB`.
    #[cfg(any(test, feature = "test-endpoint"))]
    #[error("{TEST_GITHUB_VAR} isn't set; a test build needs http://127.0.0.1:<port>/graphql")]
    Unset,
    /// TST-1: anything but `http://127.0.0.1:<port>/graphql`.
    #[cfg(any(test, feature = "test-endpoint"))]
    #[error("{TEST_GITHUB_VAR} must be http://127.0.0.1:<port>/graphql, not one on host {host}")]
    Refused { host: String },
    /// The HTTP client couldn't be built (its TLS setup).
    #[error("the HTTP client couldn't be built")]
    Build,
}

/// A reqwest client and the endpoint.
pub struct Client {
    http: reqwest::Client,
    endpoint: String,
}

impl Client {
    /// A release build: `https://api.github.com/graphql`, and `env` isn't read. A test build (TST-1):
    /// `CIRCITOR_TEST_GITHUB` from `env`, loopback only; `Err` names the variable or the host.
    pub fn from_env(env: &Env, version: &str) -> Result<Client, EndpointError> {
        #[cfg(feature = "test-endpoint")]
        {
            // TST-1: a test build reaches the loopback only, and through no proxy.
            let url = env.get(TEST_GITHUB_VAR).ok_or(EndpointError::Unset)?;
            let endpoint = loopback(url)?;
            let http = builder(version)
                .no_proxy()
                .build()
                .map_err(|_| EndpointError::Build)?;
            Ok(Client { http, endpoint })
        }
        #[cfg(not(feature = "test-endpoint"))]
        {
            // POL-1, OPS-16: api.github.com only; a release build doesn't read `env`.
            let _ = env;
            let http = builder(version).build().map_err(|_| EndpointError::Build)?;
            Ok(Client {
                http,
                endpoint: ENDPOINT.to_string(),
            })
        }
    }

    /// The fake GitHub's `client()`. Panics on any host but the loopback (TST-1).
    #[cfg(any(test, feature = "test-endpoint"))]
    pub fn for_tests(url: &str) -> Client {
        // TST-1: no client of a test build can reach another host.
        let endpoint = match loopback(url) {
            Ok(endpoint) => endpoint,
            Err(e) => panic!("Client::for_tests: {e}"),
        };
        let http = match builder(&crate::version::version()).no_proxy().build() {
            Ok(http) => http,
            Err(_) => panic!("Client::for_tests: the HTTP client couldn't be built"),
        };
        Client { http, endpoint }
    }
}

/// OPS-16: a JSON POST with circitor's `User-Agent`, never a redirect followed, and no timeout or
/// retry of its own (the gate's, POL-2).
fn builder(version: &str) -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder()
        .user_agent(format!("circitor/{version}"))
        .redirect(reqwest::redirect::Policy::none());
    #[cfg(feature = "test-endpoint")]
    let builder = {
        // TST-1: a test build's client sends its process ID; the fake GitHub tells processes apart by it.
        let mut headers = http::HeaderMap::new();
        headers.insert("x-circitor-test-pid", HeaderValue::from(std::process::id()));
        builder.default_headers(headers)
    };
    builder
}

/// TST-1: `http://127.0.0.1:<port>/graphql`, and nothing else.
#[cfg(any(test, feature = "test-endpoint"))]
fn loopback(url: &str) -> Result<String, EndpointError> {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return Err(EndpointError::Refused {
            host: "(not a URL)".to_string(),
        });
    };
    let host = parsed.host_str().unwrap_or("(none)").to_string();
    let ok = parsed.scheme() == "http"
        && host == "127.0.0.1"
        && parsed.port().is_some()
        && parsed.path() == "/graphql"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none();
    if ok {
        Ok(parsed.to_string())
    } else {
        Err(EndpointError::Refused { host })
    }
}

#[async_trait::async_trait]
impl GitHub for Client {
    async fn send(&self, token: &Token, query: &Query) -> Result<Answer, Failure> {
        // OPS-5: the token goes into the `Authorization` header, marked sensitive, and nowhere else.
        let Ok(mut auth) = HeaderValue::from_str(&format!("bearer {}", token.expose())) else {
            // A token that can't be a header value can't be sent: as good as refused (POL-20).
            return Err(Failure::Unauthorized);
        };
        auth.set_sensitive(true);
        let body = request_body(query).to_string().into_bytes();
        let response = self
            .http
            .post(&self.endpoint)
            .header(AUTHORIZATION, auth)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| Failure::Unavailable {
                kind: Unavailable::Network,
                status: None,
            })?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        // A body cut before its `Content-Length`, or a connection dropped mid-body, can't be read.
        let body = response.bytes().await.ok();
        decode::decode(query, status, &headers, body.as_deref())
    }
}

/// The POST body `send` would send for `query`: `{"query": …, "variables": …}`. T37 records with these.
pub fn request_body(query: &Query) -> serde_json::Value {
    query::body(&query::plan(query))
}
