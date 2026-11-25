#![allow(unused_variables, dead_code)] // stub: T12 removes this
//! The real token source: runs `gh auth token` (T12).

use std::sync::Arc;

use crate::clock::{Activity, SharedClock};
use crate::config::Env;

use super::{TokenRead, TokenSource, TokenSourceKind};

pub struct GhTokenSource {
    gh_path: String,
    env: Env,
    clock: SharedClock,
    activity: Activity,
}

impl GhTokenSource {
    /// `gh_path` as configured (OPS-2); `env` is the daemon's own: it gives `PATH`, decides `kind()`, and is the child's environment.
    pub fn new(
        gh_path: &str,
        env: Env,
        clock: SharedClock,
        activity: Activity,
    ) -> Arc<GhTokenSource> {
        unimplemented!("T12")
    }
}

#[async_trait::async_trait]
impl TokenSource for GhTokenSource {
    async fn read(&self) -> TokenRead {
        unimplemented!("T12")
    }

    fn kind(&self) -> TokenSourceKind {
        unimplemented!("T12")
    }

    fn gh_path(&self) -> String {
        unimplemented!("T12")
    }
}
