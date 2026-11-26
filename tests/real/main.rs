#![cfg(feature = "test-endpoint")]
//! T36, M4: the built binary as real processes, on real time (TST-9). Each test has its own
//! `ShortDir`, environment, fake `gh` and fake GitHub (`fixture`); nothing is shared but the binary.
//! The only suite that waits on the wall clock (TST-2).

mod fixture;

mod files;
mod perf;
mod signals;
mod spawn;
mod stop;
mod token;
mod upgrade;

use std::time::Duration;

/// Whole seconds.
pub fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Milliseconds.
pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The config most tests use: a few seconds of idleness before a spawned daemon exits.
pub const CONFIG: &str = "exit_after = 5\n";
/// For tests whose daemon must stay through the test.
pub const CONFIG_STAY: &str = "exit_after = 60\n";
