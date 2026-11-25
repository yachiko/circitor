#![allow(unused_variables, dead_code)] // stub: T27 removes this
//! The commands (T27).

use std::path::Path;

use crate::cli::conn::Link;
use crate::cli::{Branch, CliEnv, Git, Output};
use crate::model::PrName;

impl CliEnv {
    /// The process's own clock, directory, environment, platform, version, spawner, `git` and signals.
    pub fn real() -> CliEnv {
        unimplemented!("T27")
    }
}

/// `cli::run`'s body.
pub async fn run(args: &[String], env: &CliEnv) -> Output {
    unimplemented!("T27")
}

/// `cli::main`'s body.
pub fn main() -> i32 {
    unimplemented!("T27")
}

/// The real `Git`: runs `git` with tokio::process.
pub struct RealGit;

#[async_trait::async_trait]
impl Git for RealGit {
    async fn origin_url(&self, cwd: &Path) -> Option<String> {
        unimplemented!("T27")
    }

    async fn push_branch(&self, cwd: &Path) -> Branch {
        unimplemented!("T27")
    }
}

/// CLI-2 before any request: `Full` as it is, `Number` with `origin`'s repository. `Ok(None)` is `Bare`,
/// to be found with `find_bare`; `Err` is the whole `Output` (exit 2 and its line).
pub async fn name_offline(env: &CliEnv, arg: Option<&str>) -> Result<Option<PrName>, Output> {
    unimplemented!("T27")
}

/// CLI-2's flow for no argument, against a daemon: the list, the login, the poll, the list again, the
/// choice. `note` gets the line for an ended match. `Err` is the whole `Output`.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn find_bare(env: &CliEnv, link: &mut Link, note: &mut String) -> Result<PrName, Output> {
    unimplemented!("T27")
}
