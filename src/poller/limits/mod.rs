#![allow(unused_variables, dead_code)] // stub: T08 removes this
//! Quota and backoff (POL-17 to POL-19, POL-23, POL-25; T08). Pure: the caller passes the time.

use std::time::Duration;

use crate::clock::{Mono, Time};
use crate::config::{Config, QuotaConfig};
use crate::poller::{Policy, QuotaState, Tier};

pub struct Quota {
    cfg: QuotaConfig,
}

impl Quota {
    pub fn new(cfg: &QuotaConfig) -> Quota {
        unimplemented!("T08")
    }

    /// POL-17.
    pub fn spent(&mut self, cost: u32, at: Time) {
        unimplemented!("T08")
    }

    /// Every answer's `rateLimit` block (POL-18). `remaining == 0` is exhausted from then on (POL-25).
    pub fn observed(&mut self, remaining: u32, reset_at: Option<Time>, at: Time) {
        unimplemented!("T08")
    }

    /// `Failure::Exhausted`.
    pub fn exhausted(&mut self, reset_at: Option<Time>, at: Time) {
        unimplemented!("T08")
    }

    pub fn secondary(
        &mut self,
        retry_after: Option<Duration>,
        at: Mono,
        at_wall: Time,
        idle_interval: Duration,
    ) {
        unimplemented!("T08")
    }

    /// Any request that succeeds: the run of secondary limits ends.
    pub fn answered(&mut self) {
        unimplemented!("T08")
    }

    pub fn state(&self, wall: Time, mono: Mono) -> QuotaState {
        unimplemented!("T08")
    }

    pub fn policy(&self, wall: Time, mono: Mono, tier: Tier, cfg: &Config) -> Policy {
        unimplemented!("T08")
    }

    pub fn cost_last_hour(&self, wall: Time) -> u32 {
        unimplemented!("T08")
    }

    /// `quota_capped`'s own condition.
    pub fn capped(&self, wall: Time) -> bool {
        unimplemented!("T08")
    }

    /// `quota_reserve`'s own condition.
    pub fn in_reserve(&self) -> bool {
        unimplemented!("T08")
    }

    pub fn remaining(&self) -> Option<u32> {
        unimplemented!("T08")
    }

    pub fn reset_at(&self) -> Option<Time> {
        unimplemented!("T08")
    }

    /// When sending resumes, while exhausted or under a secondary limit (API-4's `until`).
    pub fn resume_at(&self, wall: Time, mono: Mono) -> Option<Time> {
        unimplemented!("T08")
    }
}

/// POL-19.
pub struct Backoff {
    next: Option<Mono>,
}

impl Backoff {
    pub fn new() -> Backoff {
        unimplemented!("T08")
    }

    /// 5 s, doubling to 5 min.
    pub fn failed(&mut self, at: Mono, at_wall: Time) {
        unimplemented!("T08")
    }

    pub fn succeeded(&mut self) {
        unimplemented!("T08")
    }

    /// POL-22: starts again at 5 s, if it was running.
    pub fn woke(&mut self, at: Mono, at_wall: Time) {
        unimplemented!("T08")
    }

    pub fn active(&self) -> bool {
        unimplemented!("T08")
    }

    pub fn next_try(&self) -> Option<Mono> {
        unimplemented!("T08")
    }

    /// API-4's `until`.
    pub fn next_try_wall(&self) -> Option<Time> {
        unimplemented!("T08")
    }

    /// POL-16: one per step.
    pub fn take_read_through(&mut self) -> bool {
        unimplemented!("T08")
    }
}

impl Default for Backoff {
    fn default() -> Backoff {
        Backoff::new()
    }
}
