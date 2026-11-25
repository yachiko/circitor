//! circitor: a local daemon that watches GitHub pull requests for coding agents.
//!
//! The module tree is fixed (plan/conventions.md, "Layout"); each leaf belongs to one task.

pub mod api;
pub mod cli;
pub mod clock;
pub mod config;
pub mod daemon;
pub mod github;
pub mod log;
pub mod model;
pub mod notify;
pub mod poller;
pub mod state;
pub mod status;
pub mod token;
pub mod transport;
pub mod version;

#[cfg(any(test, feature = "test-endpoint"))]
pub mod testkit;
