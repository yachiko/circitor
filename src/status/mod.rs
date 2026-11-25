#![allow(unused_variables, dead_code)] // stub: T04 removes this
//! The `Warnings` registry behind `status`'s `warnings` (API-5; `plan/contracts/log.md`).
//! T02 declares; T04 builds.

use serde::{Deserialize, Serialize};

use crate::clock::{SharedClock, Time};
use crate::log::Logger;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    QuotaCapped,
    QuotaReserve,
    PrLimit,
    PrFailing,
    StateFileSetAside,
    StateFileNewer,
    StateFileUnwritable,
    /// OPS-9, API-5: a log line couldn't be written.
    LogUnwritable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub code: WarningCode,
    pub message: String,
    pub since: Time,
}

#[derive(Clone)]
pub struct Warnings {
    clock: SharedClock,
    logger: Logger,
}

impl Warnings {
    pub fn new(clock: SharedClock, logger: Logger) -> Warnings {
        unimplemented!("T04")
    }

    /// Raises `code` for `key`, or keeps it: `since` stays that of the first raise, `message` is replaced.
    /// `key` tells apart several warnings of one code (`pr_failing`: the PR's node ID); "" otherwise.
    pub fn raise(&self, code: WarningCode, key: &str, message: String) {
        unimplemented!("T04")
    }

    /// Nothing when it isn't raised.
    pub fn clear(&self, code: WarningCode, key: &str) {
        unimplemented!("T04")
    }

    /// By `since`, then code, then key.
    pub fn list(&self) -> Vec<Warning> {
        unimplemented!("T04")
    }

    /// Since when `code` has been raised for `key`; `None` when it isn't (the notifier's `pr_failing`, NTF-4).
    pub fn since(&self, code: WarningCode, key: &str) -> Option<Time> {
        unimplemented!("T04")
    }
}
