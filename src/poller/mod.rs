//! The poller, outside and inside (`plan/contracts/poller.md`). T02's and final: T08 builds
//! `limits`, T09 `cadence`, T13 `auth`, T21 and T25 `engine`, T43 `fake`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::clock::{Mono, Time};
use crate::model::{NodeId, PrName};
use crate::token::TokenSourceKind;

pub mod auth;
pub mod cadence;
pub mod engine;
#[cfg(any(test, feature = "test-endpoint"))]
pub mod fake;
pub mod limits;

pub use engine::{Engine, EngineDeps};

/// POL-13; hot isn't a tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Active,
    Idle,
}

/// `daemon.problem` (API-4), in its order of precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemCode {
    Auth,
    RateLimited,
    GithubUnavailable,
    QuotaLow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DaemonProblem {
    pub code: ProblemCode,
    pub message: String,
    pub until: Option<Time>,
}

/// Why something that needs GitHub wasn't done. Each is one API-3 error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalCode {
    NotFound,
    Auth,
    GithubUnavailable,
    RateLimited,
    Limit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
    pub until: Option<Time>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refreshed {
    /// Sent, answered and applied.
    Applied,
    /// Sent, and it failed.
    Failed,
    /// POL-15's window, POL-19's backoff, `auth`, or a quota state that sends none (API-9: failed at once).
    NotSent,
}

/// API-16's `outcome`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollOutcome {
    Polled,
    Shared,
    Auth,
    GithubUnavailable,
    RateLimited,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollReport {
    pub outcome: PollOutcome,
    pub cost: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FirstPoll {
    Running,
    /// API-18: its finding and every first refresh have been applied or have failed.
    Finished,
    /// Finished, but none of its requests was answered (POL-19): the routes answer this error until one is.
    Unanswered(Refusal),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollerStatus {
    pub tier: Tier,
    /// The last poll applied (API-4).
    pub last_poll_at: Option<Time>,
    /// API-5's `last_poll`.
    pub last_poll: Option<(Time, PollOutcome)>,
    pub problem: Option<DaemonProblem>,
    /// Monotonic time `problem`'s code last changed to what it is now; `None` with no problem. A try with a
    /// new token in `auth` doesn't move it: `problem` stays `auth` through the try (POL-20). NTF-4's `problem`
    /// end counts its 10 minutes from it (`notify.md`).
    pub problem_since: Option<Mono>,
    pub cost_last_hour: u32,
    /// API-5's `github`.
    pub remaining: Option<u32>,
    pub reset_at: Option<Time>,
    pub login: Option<String>,
    pub token_source: TokenSourceKind,
    pub gh_path: String,
}

#[async_trait::async_trait]
pub trait PollerHandle: Send + Sync + 'static {
    /// A client request arrived (POL-13). Every route but `status` and `shutdown` calls it first, the two
    /// notify routes included. Moves the tier to active, starts a poll if one is due, and runs POL-22's check. Never waits.
    fn client_request(&self);
    /// Monotonic time of the last `client_request`, or of the start (OPS-8). Waiters and subscriptions
    /// aren't in it: T28 asks the store for those.
    fn idle_since(&self) -> Mono;
    /// POL-16: fetch an untracked PR by name, track it as `asked`, apply its first refresh. `Ok` holds its ID.
    /// Also for a loaded PR with no record (STA-8), which keeps its saved source.
    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal>;
    /// API-15 `track`: source `manual`, and a read-through when there's no record. A loaded PR's saved
    /// source is set first, so it's kept when the read-through fails. `Ok` holds its ID.
    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal>;
    /// POL-15: refresh one tracked PR by ID now (`fresh`, a wait's arrival), sharing one in flight.
    async fn refresh_pr(&self, id: &NodeId) -> Refreshed;
    /// API-16: a whole poll now. Answers when it's done; never waits out a backoff or a reset.
    async fn poll_now(&self) -> PollReport;
    /// The hot set may have changed (a waiter came or went): look again. Never called for a subscription.
    fn nudge(&self);
    fn first_poll(&self) -> tokio::sync::watch::Receiver<FirstPoll>;
    fn status(&self) -> PollerStatus;
}

/// POL-23. Declared here so T09 can use it beside T08.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaState {
    Normal,
    Slowed,
    Reserve,
    Capped,
    Exhausted,
    Secondary,
}

/// POL-23's row, worked out for a tier.
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    pub state: QuotaState,
    /// `None`: no polls.
    pub poll_every: Option<Duration>,
    /// `None`: no hot refreshes.
    pub hot_every: Option<Duration>,
    /// Forced refreshes, read-throughs and `track` are sent.
    pub demand: bool,
    /// POL-15: 5 s, or 30 s.
    pub pr_window: Duration,
    /// POL-15: 10 s, or 30 s.
    pub poll_window: Duration,
    /// POL-11's request is sent.
    pub follow_up: bool,
}
