//! The status the API reports (API-4, API-5): published before every `.await`, so `status()` never waits.

use crate::clock::Time;
use crate::poller::{DaemonProblem, PollerStatus, ProblemCode, QuotaState};
use crate::token::TokenSourceKind;

use super::lock;
use super::run::Run;

impl Run {
    /// API-4's `problem`: the first that holds of `auth`, `rate_limited`, `github_unavailable`, `quota_low`.
    fn problem_now(&self) -> Option<DaemonProblem> {
        let (wall, mono) = (self.wall(), self.now());
        if let Some(p) = self.auth.problem() {
            return Some(DaemonProblem {
                code: ProblemCode::Auth,
                message: p.message,
                until: None,
            });
        }
        let state = self.quota.state(wall, mono);
        if matches!(state, QuotaState::Exhausted | QuotaState::Secondary) {
            let until = self.quota.resume_at(wall, mono);
            return Some(DaemonProblem {
                code: ProblemCode::RateLimited,
                message: rate_limited_message(until),
                until,
            });
        }
        if self.backoff.active() {
            let until = self.backoff.next_try_wall();
            return Some(DaemonProblem {
                code: ProblemCode::GithubUnavailable,
                message: unavailable_message(
                    self.unavailable.as_deref(),
                    self.d.token_source.kind(),
                    until,
                ),
                until,
            });
        }
        let cfg = &self.d.config;
        match state {
            QuotaState::Capped => Some(DaemonProblem {
                code: ProblemCode::QuotaLow,
                message: format!(
                    "circitor has spent its quota.max_per_hour of {} points in the last hour: it polls again when the count drops, and still serves forced refreshes.",
                    cfg.quota.max_per_hour
                ),
                until: None,
            }),
            QuotaState::Reserve => Some(DaemonProblem {
                code: ProblemCode::QuotaLow,
                message: format!(
                    "GitHub reports {} points left, under quota.reserve ({}): circitor polls every {} s until the limit resets.",
                    self.quota.remaining().unwrap_or(0),
                    cfg.quota.reserve,
                    cfg.idle_interval.as_secs()
                ),
                until: None,
            }),
            _ => None,
        }
    }

    /// Publishes the status, and what `client_request` reads of `Auth` (POL-20).
    pub(super) fn publish(&mut self) {
        let (wall, mono) = (self.wall(), self.now());
        let problem = self.problem_now();
        let code = problem.as_ref().map(|p| p.code);
        // NTF-4 reads it: the time the code last changed, not the time of each try.
        if code != self.problem.map(|(c, _)| c) {
            self.problem = code.map(|c| (c, mono));
        }
        let status = PollerStatus {
            tier: self.tier,
            last_poll_at: self.last_poll_at,
            last_poll: self.last_poll,
            problem,
            problem_since: self.problem.map(|(_, at)| at),
            cost_last_hour: self.quota.cost_last_hour(wall),
            remaining: self.quota.remaining(),
            reset_at: self.quota.reset_at(),
            login: self.login.clone(),
            token_source: self.d.token_source.kind(),
            gh_path: self.d.token_source.gh_path(),
        };
        *lock(&self.shared.status) = status;
        let mut hand = lock(&self.shared.hand);
        hand.in_auth = self.auth.problem().is_some();
        hand.last_read_ended = self.auth.last_read_ended();
    }
}

/// API-3's `rate_limited` message: when circitor will ask again.
pub(super) fn rate_limited_message(until: Option<Time>) -> String {
    match until {
        Some(t) => format!("GitHub's rate limit is reached: circitor will ask again at {t}."),
        None => {
            "GitHub's rate limit is reached: circitor will ask again when it lifts.".to_string()
        }
    }
}

/// API-3's `github_unavailable` message: what failed and the token source in use, never GitHub's text.
pub(super) fn unavailable_message(
    what: Option<&str>,
    source: TokenSourceKind,
    until: Option<Time>,
) -> String {
    let what = what.unwrap_or("an error");
    let next = match until {
        Some(t) => format!("circitor will try again at {t}"),
        None => "circitor will try again".to_string(),
    };
    format!(
        "GitHub couldn't be reached ({what}, with the token from {}): {next}.",
        source.as_str()
    )
}
