#![allow(unused_variables, dead_code)] // stub: T14 removes this
//! The fake GitHub's world and GraphQL engine (TST-3; T14). It shares no type with `src/github/`
//! or `src/model/`: it builds JSON from the schema and docs/02.

use crate::clock::Time;

/// GitHub's public schema, parsed once per process from `testdata/github/schema.graphql`.
pub fn schema() -> &'static apollo_compiler::Schema {
    unimplemented!("T14")
}

/// The scripted world (TST-3): PRs with their fields, checks, comments and reviews, and changes
/// scheduled at times on GitHub's own clock. T14 designs its building methods.
pub struct World {
    now: Time,
}

impl World {
    pub fn new(login: &str, start: Time) -> World {
        unimplemented!("T14")
    }

    /// The `rateLimit` block's `remaining` and `resetAt`; `execute` answers them, T20 sets and spends them.
    pub fn set_rate(&mut self, remaining: u32, reset_at: Time) {
        unimplemented!("T14")
    }

    /// GitHub's clock: only ever forward.
    pub fn now(&self) -> Time {
        unimplemented!("T14")
    }

    /// Applies the scheduled changes due by `t`, in order.
    pub fn advance_to(&mut self, t: Time) {
        unimplemented!("T14")
    }
}

pub struct Executed {
    /// `{"data": …}` with `errors` when there are any.
    pub body: serde_json::Value,
    /// GitHub's formula (docs/02 §2).
    pub cost: u32,
    /// The IDs `nodes(ids:)` asked for.
    pub node_ids: Vec<String>,
}

#[derive(Debug)]
pub enum Rejected {
    Unparsable(String),
    NotInSchema(String),
    /// In the schema, not in the fake.
    NoResolver(String),
    Mutation,
}

/// Parses `request` (the POST body: `query`, `variables`), checks it against the schema, answers only
/// the fields asked from the world as it is now, and costs it. Paging is by cursor over the world as it
/// is at this call.
pub fn execute(world: &World, request: &[u8]) -> Result<Executed, Rejected> {
    unimplemented!("T14")
}

/// What a request asks, for T20's request log, which never parses GraphQL itself.
pub struct Asked {
    /// `Some`: it holds a finding; inside, that page's cursor.
    pub finding: Option<Option<String>>,
    /// Each node ID with what was asked of it.
    pub nodes: Vec<(String, AskedOf)>,
    /// Owner, repository, number.
    pub lookup: Option<(String, String, u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskedOf {
    Record,
    RecordAndItems,
    Items,
    ChecksPage,
}

pub fn describe(request: &[u8]) -> Result<Asked, Rejected> {
    unimplemented!("T14")
}
