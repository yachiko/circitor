#![allow(unused_variables, dead_code)] // stub: T43 removes this
//! `FakeSpawner` and `FakeGit` (T43).

use std::path::Path;
use std::sync::Arc;

use crate::cli::{Branch, Git, Spawned, Spawner};
use crate::config::Env;

type Starter = Box<dyn Fn(std::fs::File, &Env) -> Box<dyn Spawned> + Send + Sync + 'static>;

pub struct FakeSpawner {
    starter: Option<Starter>,
}

impl FakeSpawner {
    /// A child that first appends `log_line` to the log file it was given, then has exited with `code`
    /// (OPS-1's message for a bad config).
    pub fn exits(code: i32, log_line: Option<&str>) -> Arc<FakeSpawner> {
        unimplemented!("T43")
    }

    /// A child that runs and binds nothing.
    pub fn never_binds() -> Arc<FakeSpawner> {
        unimplemented!("T43")
    }

    /// For the harness, whose closure starts the in-process daemon on a task and returns a `Spawned`
    /// that follows it.
    pub fn starts(
        f: impl Fn(std::fs::File, &Env) -> Box<dyn Spawned> + Send + Sync + 'static,
    ) -> Arc<FakeSpawner> {
        unimplemented!("T43")
    }

    /// How many children were spawned.
    pub fn spawns(&self) -> usize {
        unimplemented!("T43")
    }
}

#[async_trait::async_trait]
impl Spawner for FakeSpawner {
    async fn spawn(&self, log: std::fs::File, env: &Env) -> std::io::Result<Box<dyn Spawned>> {
        unimplemented!("T43")
    }
}

pub struct FakeGit {
    origin: Option<String>,
    branch: Branch,
}

impl FakeGit {
    pub fn new(origin: Option<&str>, branch: Branch) -> Arc<FakeGit> {
        unimplemented!("T43")
    }
}

#[async_trait::async_trait]
impl Git for FakeGit {
    async fn origin_url(&self, cwd: &Path) -> Option<String> {
        unimplemented!("T43")
    }

    async fn push_branch(&self, cwd: &Path) -> Branch {
        unimplemented!("T43")
    }
}
