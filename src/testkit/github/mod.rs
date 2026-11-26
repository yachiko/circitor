//! The fake GitHub's world and GraphQL engine (TST-3; T14). It shares no type with `src/github/`
//! or `src/model/`: it builds JSON from the schema and docs/02.
//!
//! - [`World`] is the scripted world: repositories, PRs, their commits and checks, comments,
//!   reviews and threads, and changes scheduled at times on GitHub's own clock. Its building
//!   methods are documented one by one in `world.rs`; they're what the scenario tasks script.
//! - [`execute`] answers one POST body from the world as it is now: parsed, checked against
//!   GitHub's public schema ([`schema`]), only the fields asked, under the keys asked, costed by
//!   GitHub's formula ([`cost_of`]).
//! - [`describe`] says what a request asks, for T20's request log.
//!
//! T20 wraps all of this in an HTTP server (`testkit::server`) with the headers, the transport's
//! faults and the rules that fail a test.

use std::sync::OnceLock;

use apollo_compiler::Schema;
use apollo_compiler::validation::Valid;

mod cost;
mod exec;
mod query;
mod world;

#[cfg(test)]
mod tests;

pub use cost::{cost_of, describe};
pub use exec::execute;
pub use world::{PrId, RepoId, RunId, World};

/// GitHub's public schema as T41 fetched it.
const SCHEMA_TEXT: &str = include_str!("../../../testdata/github/schema.graphql");

fn valid_schema() -> &'static Valid<Schema> {
    static SCHEMA: OnceLock<Valid<Schema>> = OnceLock::new();
    SCHEMA.get_or_init(
        || match Schema::parse_and_validate(SCHEMA_TEXT, "schema.graphql") {
            Ok(valid) => valid,
            // GitHub's own schema is what GitHub validates against: a rule apollo-compiler is stricter
            // on doesn't make a query less valid there. Parsing must still have worked.
            Err(with_errors) => Valid::assume_valid(with_errors.partial),
        },
    )
}

/// GitHub's public schema, parsed once per process from `testdata/github/schema.graphql`.
pub fn schema() -> &'static Schema {
    valid_schema()
}

/// One request's answer.
#[derive(Debug)]
pub struct Executed {
    /// `{"data": …}` with `errors` when there are any.
    pub body: serde_json::Value,
    /// GitHub's formula (docs/02 §2).
    pub cost: u32,
    /// The IDs `nodes(ids:)` (and `node(id:)`) asked for, in the order asked.
    pub node_ids: Vec<String>,
}

/// Why a request wasn't answered. Each of these fails the test (TST-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejected {
    /// The body isn't JSON with a `query` string, or the query doesn't parse.
    Unparsable(String),
    /// The schema refuses the query: the text names the field (`PullRequest.nope`) or the rule.
    NotInSchema(String),
    /// In the schema, not in the fake: `Type.field`, so a new query field fails loudly.
    NoResolver(String),
    /// A mutation (POL-1).
    Mutation,
}

/// What a request asks, for T20's request log, which never parses GraphQL itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    /// `Some`: it holds a finding (`viewer.pullRequests`); inside, that page's cursor (`after`).
    pub finding: Option<Option<String>>,
    /// Each node ID (`nodes(ids:)`, `node(id:)`) with what was asked of it, in the order asked.
    pub nodes: Vec<(String, AskedOf)>,
    /// A by-name lookup: owner, repository, number.
    pub lookup: Option<(String, String, u64)>,
}

/// What a request asked of one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskedOf {
    /// The record (POL-6): the PR's own fields.
    Record,
    /// The record and POL-12's items (a first refresh, POL-4).
    RecordAndItems,
    /// POL-12's items only: `comments` or `reviews` with `first`/`last`, no record field.
    Items,
    /// A further page of the last commit's checks (POL-8): `contexts(after:)`, no record field.
    ChecksPage,
}
