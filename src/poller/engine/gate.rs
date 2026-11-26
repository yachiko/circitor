//! The gate (POL-2): every request to GitHub goes through `Run::request`. The token from `Auth`, the 1 s
//! since the last request ended, a request number and send times, 20 s of the clock around
//! `GitHub::send`, and what each result feeds (POL-17 to POL-20, POL-25).

use std::time::Duration;

use crate::clock::{Mono, timeout};
use crate::github::{Answer, Failure, Query, Unavailable};
use crate::model::ReadMeta;
use crate::poller::QuotaState;
use crate::poller::auth::AfterRefusal;
use crate::token::Token;

use super::run::Run;

/// POL-2: between the end of one request and the start of the next.
const GAP: Duration = Duration::from_secs(1);
/// POL-2: a request times out after this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Why a request didn't answer.
#[derive(Clone, Debug)]
pub(super) enum Fail {
    /// POL-20: no token, or the repeat got a 401 too (`Auth::problem` says why).
    Auth,
    /// POL-25.
    RateLimited,
    /// POL-19, POL-24: unreachable, a timeout, another status, no data.
    Unavailable,
}

/// What a request came to.
pub(super) struct Sent {
    pub result: Result<Answer, Fail>,
    /// The send times of the request that answered (or of the last one sent); `None`: nothing sent.
    pub meta: Option<ReadMeta>,
    /// The points it cost (POL-17): every request sent, a repeat's included.
    pub cost: u32,
}

/// One send's raw result.
enum Raw {
    Done(Result<Answer, Failure>),
    TimedOut,
}

impl Run {
    /// POL-2's one function. `mark_at` is the poll's send time for a poll's later requests (P10); `None`
    /// marks the PRs with this request's own.
    pub(super) async fn request(
        &mut self,
        query: &Query,
        kind: &'static str,
        mark_at: Option<Mono>,
    ) -> Sent {
        let sent = self.request_once(query, kind, mark_at).await;
        if matches!(sent.result, Err(Fail::Auth)) {
            // POL-20: a 401 on a try returns to `auth` at once; the try is over.
            self.try_open = false;
        }
        sent
    }

    async fn request_once(
        &mut self,
        query: &Query,
        kind: &'static str,
        mark_at: Option<Mono>,
    ) -> Sent {
        let in_auth = |meta: Option<ReadMeta>| Sent {
            result: Err(Fail::Auth),
            meta,
            cost: 0,
        };
        // OPS-5: the first read is waited for; with no token nothing is sent (POL-20).
        self.park();
        let token = self.auth.token(false).await;
        self.wake();
        let Ok(token) = token else {
            return in_auth(None);
        };
        let (raw, meta) = self.send_once(&token, query, kind, mark_at).await;
        if !matches!(raw, Raw::Done(Err(Failure::Unauthorized))) {
            return self.feed(raw, meta, &token);
        }
        // POL-20: the token is read again and the request repeated once.
        self.park();
        let after = self.auth.unauthorized(&token, false).await;
        self.wake();
        let AfterRefusal::Retry(retry) = after else {
            return in_auth(Some(meta));
        };
        let (raw, meta) = self.send_once(&retry, query, kind, mark_at).await;
        if !matches!(raw, Raw::Done(Err(Failure::Unauthorized))) {
            return self.feed(raw, meta, &retry);
        }
        // POL-20: the repeat got a 401 too: the `auth` state.
        self.park();
        let _ = self.auth.unauthorized(&retry, true).await;
        self.wake();
        in_auth(Some(meta))
    }

    /// The gap, the number, the send times, `mark_sent`, the 20 s, and the `debug` line (OPS-9).
    async fn send_once(
        &mut self,
        token: &Token,
        query: &Query,
        kind: &'static str,
        mark_at: Option<Mono>,
    ) -> (Raw, ReadMeta) {
        if let Some(end) = self.last_end {
            let at = end.plus(GAP);
            if self.now() < at {
                // POL-2: waiting for the gap is parking (clock.md).
                self.park();
                self.d.clock.sleep_until(at).await;
                self.wake();
            }
        }
        // MOD-2: the gate numbers from 1.
        self.seq += 1;
        let meta = ReadMeta {
            seq: self.seq,
            sent_wall: self.wall(),
            sent_mono: self.now(),
            github_date: None,
        };
        // P10: a poll's PRs count as sent when the poll was; `ReadMeta` keeps this request's own.
        let ids = Run::ids(&query.nodes);
        if !ids.is_empty() {
            self.d
                .store
                .mark_sent(&ids, mark_at.unwrap_or(meta.sent_mono));
        }
        // API-4: published before the await; the guard is kept across the request (clock.md).
        self.publish();
        let github = self.d.github.clone();
        let clock = self.d.clock.clone();
        let raw = match timeout(&clock, REQUEST_TIMEOUT, github.send(token, query)).await {
            Ok(r) => Raw::Done(r),
            Err(_) => Raw::TimedOut,
        };
        let end = self.now();
        self.last_end = Some(end);
        if self.d.logger.enabled(crate::log::Level::Debug) {
            let (cost, result) = match &raw {
                Raw::Done(Ok(a)) => (a.rate.as_ref().map_or(1, |r| r.cost), "ok".to_string()),
                Raw::Done(Err(f)) => (0, failure_word(f)),
                Raw::TimedOut => (0, "timeout".to_string()),
            };
            // OPS-9: the kind, the cost and the duration; never a title or GitHub's text.
            self.d.logger.debug(format_args!(
                "request kind={kind} seq={} cost={cost} duration_ms={} result={result}",
                meta.seq,
                end.since(meta.sent_mono).as_millis()
            ));
        }
        (raw, meta)
    }

    /// What each result feeds: `Quota`, `Backoff`, `Auth`, and the problem lines (OPS-9).
    fn feed(&mut self, raw: Raw, mut meta: ReadMeta, token: &Token) -> Sent {
        let (wall, mono) = (self.wall(), self.now());
        let failure = match raw {
            Raw::Done(Ok(answer)) => {
                meta.github_date = answer.date;
                // POL-17: an answer without `rateLimit` counts 1 point, the formula's least.
                let cost = answer.rate.as_ref().map_or(1, |r| r.cost);
                self.quota.spent(cost, wall);
                if let Some(rate) = &answer.rate {
                    // POL-18, POL-25: `remaining == 0` is exhausted from then on.
                    self.quota.observed(rate.remaining, rate.reset_at, wall);
                }
                self.quota.answered();
                if self.backoff.active() {
                    // POL-19: any request that succeeds ends it, and a poll follows at once.
                    self.backoff.succeeded();
                    self.unavailable = None;
                    self.d.logger.info("problem end code=github_unavailable");
                    if !self.in_poll {
                        self.poll_asap = true;
                    }
                }
                let limited = matches!(
                    self.quota.state(wall, mono),
                    QuotaState::Exhausted | QuotaState::Secondary
                );
                if limited {
                    self.rate_begin();
                } else if self.rate_logged {
                    self.rate_logged = false;
                    self.d.logger.info("problem end code=rate_limited");
                }
                self.auth.accepted(token);
                self.try_open = false;
                return Sent {
                    result: Ok(answer),
                    meta: Some(meta),
                    cost,
                };
            }
            Raw::Done(Err(Failure::Exhausted { reset_at })) => {
                // POL-25: nothing is sent until the reset.
                self.quota.exhausted(reset_at, wall);
                self.rate_begin();
                Fail::RateLimited
            }
            Raw::Done(Err(Failure::Secondary { retry_after })) => {
                self.quota
                    .secondary(retry_after, mono, wall, self.d.config.idle_interval);
                self.rate_begin();
                Fail::RateLimited
            }
            Raw::Done(Err(Failure::Unavailable { kind, status })) => {
                match status {
                    Some(s) => self.unreachable(format!("status {s}"), format!("status={s}")),
                    None => self.unreachable(
                        unavailable_what(kind).to_string(),
                        format!("kind={}", unavailable_word(kind)),
                    ),
                }
                Fail::Unavailable
            }
            Raw::TimedOut => {
                self.unreachable("a timeout".to_string(), "kind=timeout".to_string());
                Fail::Unavailable
            }
            Raw::Done(Err(Failure::Unauthorized)) => {
                // Handled by `request` before it gets here.
                Fail::Auth
            }
        };
        Sent {
            result: Err(failure),
            meta: Some(meta),
            cost: 0,
        }
    }

    /// POL-19: the backoff; the first failure is logged, not each retry (OPS-9).
    /// `what` is for API-3's message, `pair` for the log line.
    fn unreachable(&mut self, what: String, pair: String) {
        if !self.backoff.active() {
            self.d
                .logger
                .warn(format_args!("problem begin code=github_unavailable {pair}"));
            self.unavailable = Some(what);
        }
        let (mono, wall) = (self.now(), self.wall());
        self.backoff.failed(mono, wall);
    }

    fn rate_begin(&mut self) {
        if !self.rate_logged {
            self.rate_logged = true;
            self.d.logger.warn("problem begin code=rate_limited");
        }
    }
}

fn unavailable_word(kind: Unavailable) -> &'static str {
    match kind {
        Unavailable::Network => "network",
        Unavailable::Http => "http",
        Unavailable::NoData => "no_data",
        Unavailable::BadBody => "bad_body",
    }
}

/// For API-3's message: what GitHub answered, never its text.
fn unavailable_what(kind: Unavailable) -> &'static str {
    match kind {
        Unavailable::Network => "a network error",
        Unavailable::Http => "an HTTP error",
        Unavailable::NoData => "an answer without data",
        Unavailable::BadBody => "an answer that couldn't be read",
    }
}

fn failure_word(f: &Failure) -> String {
    match f {
        Failure::Unauthorized => "unauthorized".to_string(),
        Failure::Exhausted { .. } => "exhausted".to_string(),
        Failure::Secondary { .. } => "secondary".to_string(),
        Failure::Unavailable {
            status: Some(s), ..
        } => format!("status_{s}"),
        Failure::Unavailable { kind, .. } => unavailable_word(*kind).to_string(),
    }
}
