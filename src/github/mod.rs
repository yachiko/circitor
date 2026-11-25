//! The one door to GitHub (`plan/contracts/github.md`). T02's and final: T07 builds `client`,
//! T43 `fake`.

use std::time::Duration;

use crate::clock::Time;
use crate::model::{ChecksRead, ItemsRead, NodeId, PrName, PrRead};
use crate::token::Token;

pub mod client;
#[cfg(any(test, feature = "test-endpoint"))]
pub mod fake;

pub use client::{Client, EndpointError, request_body};

/// One HTTP request to GitHub's GraphQL endpoint. A poll's request holds `find` and `nodes` together (POL-4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// Finding (POL-4): one page of `viewer.pullRequests(states: OPEN, first: 100, orderBy: UPDATED_AT desc)`
    /// with `id`, `number`, `updatedAt` and the repository's owner and name, and `viewer { login }`.
    pub find: Option<Find>,
    /// By node ID, through `nodes(ids:)` and shared fragments (POL-4, POL-7). At most 40.
    pub nodes: Vec<NodeQuery>,
    /// Reading through (POL-16): one PR by owner, repository and number, as `Want::RecordAndItems`.
    pub lookup: Option<PrName>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Find {
    pub after: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeQuery {
    pub id: NodeId,
    pub want: Want,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Want {
    /// POL-6: the full record, checks `contexts(first: 100)`.
    Record,
    /// A first refresh (POL-4): the record plus POL-12's fields.
    RecordAndItems,
    /// POL-12: the last 20 comments and the last 20 reviews, nothing else.
    Items,
    /// POL-8: the next page of the last commit's checks, nothing else.
    ChecksPage { after: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// The `rateLimit` block; `None` if GitHub left it out.
    pub rate: Option<Rate>,
    /// The `Date` header (MOD-6).
    pub date: Option<Time>,
    /// `Some` iff the query had `find` and that part came back.
    pub found: Option<Found>,
    /// One per `Query::nodes`, same order.
    pub nodes: Vec<NodeAnswer>,
    pub lookup: Option<LookupAnswer>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rate {
    pub cost: u32,
    pub remaining: u32,
    pub reset_at: Option<Time>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub login: String,
    pub prs: Vec<FoundPr>,
    pub next: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FoundPr {
    pub id: NodeId,
    pub pr: PrName,
    pub updated_at: Option<Time>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeAnswer {
    /// `items` iff `RecordAndItems`.
    Record {
        read: Box<PrRead>,
        items: Option<ItemsRead>,
    },
    Items(ItemsRead),
    ChecksPage(ChecksRead),
    /// The node is `null` with a `NOT_FOUND` error on its path (POL-21).
    NotFound,
    /// Any other error whose path points at or inside the node, or a node of another type (POL-21, POL-24).
    /// `type_` is GitHub's `type`, as written, or a word of the client's own (`UNLISTED_STATE`, `NOT_A_PULL_REQUEST`).
    Error {
        type_: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum LookupAnswer {
    Found {
        read: Box<PrRead>,
        items: ItemsRead,
    },
    /// `NOT_FOUND` for the repository or the PR (POL-16).
    NotFound,
    Error {
        type_: String,
    },
}

/// A request that failed as a whole. Classified in this order (POL-25 before POL-19 and POL-24).
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// HTTP 401 (POL-20).
    Unauthorized,
    /// POL-25: no `data`, and `x-ratelimit-remaining: 0`.
    Exhausted { reset_at: Option<Time> },
    /// POL-25: a 403, or a 200 with an error, naming a secondary rate limit.
    Secondary { retry_after: Option<Duration> },
    /// POL-19, POL-24.
    Unavailable {
        kind: Unavailable,
        status: Option<u16>,
    },
}

/// A timeout is the caller's (the gate, POL-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    Network,
    Http,
    NoData,
    BadBody,
}

#[async_trait::async_trait]
pub trait GitHub: Send + Sync + 'static {
    /// Sends one request and decodes its answer. No retry, no sleep, no timeout of its own.
    /// Dropping the future abandons the request.
    async fn send(&self, token: &Token, query: &Query) -> Result<Answer, Failure>;
}
