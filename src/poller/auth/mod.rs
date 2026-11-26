//! The token's life (POL-3, POL-20; T13).

use std::sync::Arc;
use std::time::Duration;

use crate::clock::{Mono, SharedClock};
use crate::log::Logger;
use crate::token::{GhFailure, Token, TokenRead, TokenSource, TokenSourceKind};

#[cfg(test)]
mod tests;

/// POL-3: the token is read again this long after the last read ended.
const READ_EVERY: Duration = Duration::from_secs(10 * 60);
/// POL-20 (a proposal): in `auth` the token is read again this often.
const AUTH_READ_EVERY: Duration = Duration::from_secs(60);
/// POL-20 (a proposal): a demand reads again in `auth` when the last read ended this long ago.
const DEMAND_AFTER: Duration = Duration::from_secs(5);
/// POL-20 (a proposal): the token that was refused gets another try after this long.
const SAME_TOKEN_AGAIN: Duration = Duration::from_secs(10 * 60);

/// API-3's `auth` message.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthProblem {
    pub message: String,
}

pub enum AfterRefusal {
    Retry(Token),
    InAuth(AuthProblem),
}

/// Why the daemon is in `auth`; the message is built from it and the source.
#[derive(Clone, Debug)]
enum Cause {
    NoLogin,
    Failed(GhFailure),
    Refused,
}

/// The token in hand, the token refused, the read times.
pub struct Auth {
    source: Arc<dyn TokenSource>,
    clock: SharedClock,
    logger: Logger,
    token: Option<Token>,
    /// The token the last 401 was about, while in `auth`.
    refused: Option<Token>,
    /// When `refused` was last refused, or a try with a token opened.
    refused_at: Mono,
    first_done: bool,
    last_end: Option<Mono>,
    /// `Some` while in `auth`, the one try included.
    cause: Option<Cause>,
    /// One try with `token` is open.
    trying: bool,
    /// A `token read failed` line has been logged for the current run of failures.
    failed_run: bool,
}

impl Auth {
    /// No `Activity`: the real source counts `gh` itself (clock.md).
    pub fn new(source: Arc<dyn TokenSource>, clock: SharedClock, logger: Logger) -> Auth {
        Auth {
            source,
            clock,
            logger,
            token: None,
            refused: None,
            refused_at: Mono::ZERO,
            first_done: false,
            last_end: None,
            cause: None,
            trying: false,
            failed_run: false,
        }
    }

    /// The token to send with. Runs the first read if none has ended: everything that needs GitHub
    /// waits for it (OPS-5). In `auth`, with `demand`, reads again if the last read is 5 s old (POL-20).
    pub async fn token(&mut self, demand: bool) -> Result<Token, AuthProblem> {
        // OPS-5: the token is read after the socket is bound; this is the first read everything waits for.
        if !self.first_done {
            let read = self.read().await;
            self.apply_first(read);
        } else if demand && self.cause.is_some() && self.last_read_age() >= DEMAND_AFTER {
            // POL-20: in `auth`, a demand reads again when the last read ended 5 s ago.
            let read = self.read().await;
            self.apply_later(read);
        }
        self.current()
    }

    /// A request sent with `refused` got a 401 (POL-20). First time: reads again, `Retry`. When the
    /// repeat (`was_retry`) or a try in `auth` gets one: `InAuth`.
    pub async fn unauthorized(&mut self, refused: &Token, was_retry: bool) -> AfterRefusal {
        // POL-20: a 401 on a try, or on the repeat, reads nothing.
        if self.cause.is_some() || was_retry {
            self.refuse(refused);
            return AfterRefusal::InAuth(self.problem_now());
        }
        match self.read().await {
            TokenRead::Token(t) => {
                self.failed_run = false;
                self.token = Some(t.clone());
                AfterRefusal::Retry(t)
            }
            TokenRead::Failed(f) => {
                self.note_failure(&f);
                // POL-3: a failed read keeps the token in hand.
                let kept = self.token.clone().unwrap_or_else(|| refused.clone());
                AfterRefusal::Retry(kept)
            }
            TokenRead::NoLogin => {
                self.drop_token();
                AfterRefusal::InAuth(self.problem_now())
            }
        }
    }

    /// A request sent with `used` succeeded: `auth` ends.
    pub fn accepted(&mut self, used: &Token) {
        if self.cause.take().is_some() {
            self.trying = false;
            self.refused = None;
            self.token = Some(used.clone());
            self.logger.info("problem end code=auth");
        }
    }

    /// When the next periodic read is due: 10 min after the last, 60 s in `auth` (POL-3, POL-20).
    pub fn due(&self) -> Mono {
        match self.last_end {
            None => Mono::ZERO,
            Some(end) if self.cause.is_some() => end.plus(AUTH_READ_EVERY),
            Some(end) => end.plus(READ_EVERY),
        }
    }

    /// The periodic read. `true` when there's now a token worth one try (another than the one refused,
    /// or the same after 10 minutes).
    pub async fn periodic(&mut self) -> bool {
        let first = !self.first_done;
        let read = self.read().await;
        if first {
            self.apply_first(read);
            false
        } else {
            self.apply_later(read)
        }
    }

    /// `Some` while in `auth`, the one try included, until `accepted`.
    pub fn problem(&self) -> Option<AuthProblem> {
        self.cause.as_ref().map(|_| self.problem_now())
    }

    /// When the last read ended; `None` before the first has. The engine publishes it for `client_request` (POL-20's 5 s).
    pub fn last_read_ended(&self) -> Option<Mono> {
        self.last_end
    }

    pub fn first_read_done(&self) -> bool {
        self.first_done
    }

    async fn read(&mut self) -> TokenRead {
        let read = self.source.read().await;
        self.first_done = true;
        self.last_end = Some(self.clock.mono());
        read
    }

    fn last_read_age(&self) -> Duration {
        match self.last_end {
            Some(end) => self.clock.mono().since(end),
            None => Duration::MAX,
        }
    }

    fn current(&self) -> Result<Token, AuthProblem> {
        match (&self.cause, &self.token) {
            (None, Some(t)) => Ok(t.clone()),
            (Some(_), Some(t)) if self.trying => Ok(t.clone()),
            _ => Err(self.problem_now()),
        }
    }

    fn enter_auth(&mut self, cause: Cause) {
        if self.cause.is_none() {
            // OPS-9: once.
            self.logger.warn("problem begin code=auth");
        }
        self.cause = Some(cause);
        self.trying = false;
    }

    /// POL-3: no login drops the token and enters `auth`.
    fn drop_token(&mut self) {
        self.failed_run = false;
        self.token = None;
        self.refused = None;
        self.enter_auth(Cause::NoLogin);
    }

    /// A 401 about `refused`: into (or back to) `auth`, no read.
    fn refuse(&mut self, refused: &Token) {
        self.refused = Some(refused.clone());
        self.refused_at = self.clock.mono();
        self.enter_auth(Cause::Refused);
    }

    fn note_failure(&mut self, failure: &GhFailure) {
        // POL-3: one `warn` for a run of failures; the cause only, never `gh`'s output.
        if !self.failed_run {
            self.failed_run = true;
            self.logger.warn(format_args!(
                "token read failed cause={}",
                failure_word(failure)
            ));
        }
    }

    /// The first read's outcome (OPS-5): a token, or `auth` at once.
    fn apply_first(&mut self, read: TokenRead) {
        match read {
            TokenRead::Token(t) => self.token = Some(t),
            TokenRead::NoLogin => self.drop_token(),
            TokenRead::Failed(f) => self.enter_auth(Cause::Failed(f)),
        }
    }

    /// A later read's outcome. `true` when it opened a try.
    fn apply_later(&mut self, read: TokenRead) -> bool {
        match read {
            TokenRead::Token(t) => {
                self.failed_run = false;
                if self.cause.is_none() {
                    // POL-3: replaced silently.
                    self.token = Some(t);
                    return false;
                }
                self.consider_in_auth(t)
            }
            TokenRead::NoLogin => {
                self.drop_token();
                false
            }
            TokenRead::Failed(f) => {
                self.note_failure(&f);
                // POL-3: the token in hand is kept. With none in hand, the failure is the cause.
                if self.token.is_none() {
                    self.cause = Some(Cause::Failed(f));
                }
                false
            }
        }
    }

    /// In `auth`, a read gave `t`: does it open a try?
    fn consider_in_auth(&mut self, t: Token) -> bool {
        if self.trying && self.token.as_ref().is_some_and(|h| h.same_as(&t)) {
            return false;
        }
        let worth = match (&self.token, &self.refused) {
            (None, _) | (_, None) => true,
            (Some(_), Some(r)) => {
                !t.same_as(r) || self.clock.mono().since(self.refused_at) >= SAME_TOKEN_AGAIN
            }
        };
        if worth {
            self.token = Some(t);
            self.trying = true;
            // The same token is tried again at most every 10 minutes.
            self.refused_at = self.clock.mono();
        }
        worth
    }

    fn problem_now(&self) -> AuthProblem {
        let cause = self.cause.clone().unwrap_or(Cause::Refused);
        AuthProblem {
            message: message(&cause, self.source.kind(), &self.source.gh_path()),
        }
    }
}

fn failure_word(f: &GhFailure) -> String {
    match f {
        GhFailure::CantRun => "cant_run".to_string(),
        GhFailure::Exit(n) => format!("exit status={n}"),
        GhFailure::Killed => "killed".to_string(),
        GhFailure::Timeout => "timeout".to_string(),
        GhFailure::NoOutput => "no_output".to_string(),
    }
}

/// API-3's `auth` row: one sentence, by cause and by source. Never `gh`'s output.
fn message(cause: &Cause, kind: TokenSourceKind, gh_path: &str) -> String {
    match cause {
        Cause::NoLogin => {
            "There is no GitHub login: run `gh auth login`, and circitor will read the token again."
                .to_string()
        }
        Cause::Failed(f) => {
            let what = match f {
                GhFailure::CantRun => "could not be run".to_string(),
                GhFailure::Exit(n) => format!("failed with exit status {n}"),
                GhFailure::Killed => "was killed".to_string(),
                GhFailure::Timeout => "timed out after 10 s".to_string(),
                GhFailure::NoOutput => "printed no token".to_string(),
            };
            format!(
                "Reading the token with `gh auth token` ({gh_path}) {what}; circitor will try again."
            )
        }
        Cause::Refused => match kind {
            TokenSourceKind::Gh => {
                "GitHub refused the token from gh's login: run `gh auth login`.".to_string()
            }
            TokenSourceKind::GhTokenVar | TokenSourceKind::GithubTokenVar => format!(
                "GitHub refused the token in {}: fix or unset it, then restart the daemon.",
                kind.as_str()
            ),
        },
    }
}
