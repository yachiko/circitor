#![allow(unused_variables, dead_code)] // stub: T13 removes this
//! The token's life (POL-3, POL-20; T13).

use std::sync::Arc;

use crate::clock::{Mono, SharedClock};
use crate::log::Logger;
use crate::token::{Token, TokenSource};

/// API-3's `auth` message.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthProblem {
    pub message: String,
}

pub enum AfterRefusal {
    Retry(Token),
    InAuth(AuthProblem),
}

/// The token in hand, the token refused, the read times.
pub struct Auth {
    source: Arc<dyn TokenSource>,
    clock: SharedClock,
    logger: Logger,
}

impl Auth {
    /// No `Activity`: the real source counts `gh` itself (clock.md).
    pub fn new(source: Arc<dyn TokenSource>, clock: SharedClock, logger: Logger) -> Auth {
        unimplemented!("T13")
    }

    /// The token to send with. Runs the first read if none has ended: everything that needs GitHub
    /// waits for it (OPS-5). In `auth`, with `demand`, reads again if the last read is 5 s old (POL-20).
    pub async fn token(&mut self, demand: bool) -> Result<Token, AuthProblem> {
        unimplemented!("T13")
    }

    /// A request sent with `refused` got a 401 (POL-20). First time: reads again, `Retry`. When the
    /// repeat (`was_retry`) or a try in `auth` gets one: `InAuth`.
    pub async fn unauthorized(&mut self, refused: &Token, was_retry: bool) -> AfterRefusal {
        unimplemented!("T13")
    }

    /// A request sent with `used` succeeded: `auth` ends.
    pub fn accepted(&mut self, used: &Token) {
        unimplemented!("T13")
    }

    /// When the next periodic read is due: 10 min after the last, 60 s in `auth` (POL-3, POL-20).
    pub fn due(&self) -> Mono {
        unimplemented!("T13")
    }

    /// The periodic read. `true` when there's now a token worth one try (another than the one refused,
    /// or the same after 10 minutes).
    pub async fn periodic(&mut self) -> bool {
        unimplemented!("T13")
    }

    /// `Some` while in `auth`, the one try included, until `accepted`.
    pub fn problem(&self) -> Option<AuthProblem> {
        unimplemented!("T13")
    }

    /// When the last read ended; `None` before the first has. The engine publishes it for `client_request` (POL-20's 5 s).
    pub fn last_read_ended(&self) -> Option<Mono> {
        unimplemented!("T13")
    }

    pub fn first_read_done(&self) -> bool {
        unimplemented!("T13")
    }
}
