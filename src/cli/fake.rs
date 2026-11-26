//! `FakeSpawner` and `FakeGit` (T43).

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cli::{Branch, Git, Spawned, Spawner};
use crate::config::Env;

type Starter = Box<dyn Fn(std::fs::File, &Env) -> Box<dyn Spawned> + Send + Sync + 'static>;

/// A child that has exited with a status.
struct Exited(i32);

impl Spawned for Exited {
    fn exited(&mut self) -> Option<i32> {
        Some(self.0)
    }
}

/// A child that runs and never exits.
struct Running;

impl Spawned for Running {
    fn exited(&mut self) -> Option<i32> {
        None
    }
}

// CLI-13: a command can be driven without the process's `exec`.
pub struct FakeSpawner {
    /// `None`: runs and binds nothing.
    starter: Option<Starter>,
    spawns: AtomicUsize,
}

impl FakeSpawner {
    /// A child that first appends `log_line` to the log file it was given, then has exited with `code`
    /// (OPS-1's message for a bad config).
    pub fn exits(code: i32, log_line: Option<&str>) -> Arc<FakeSpawner> {
        let line = log_line.map(str::to_string);
        FakeSpawner::with(Some(Box::new(move |mut log, _env| {
            if let Some(line) = &line {
                // A fake: a log that can't be written leaves the test's own check to fail.
                let _ = writeln!(log, "{line}");
            }
            Box::new(Exited(code))
        })))
    }

    /// A child that runs and binds nothing.
    pub fn never_binds() -> Arc<FakeSpawner> {
        FakeSpawner::with(None)
    }

    /// For the harness, whose closure starts the in-process daemon on a task and returns a `Spawned`
    /// that follows it.
    pub fn starts(
        f: impl Fn(std::fs::File, &Env) -> Box<dyn Spawned> + Send + Sync + 'static,
    ) -> Arc<FakeSpawner> {
        FakeSpawner::with(Some(Box::new(f)))
    }

    fn with(starter: Option<Starter>) -> Arc<FakeSpawner> {
        Arc::new(FakeSpawner {
            starter,
            spawns: AtomicUsize::new(0),
        })
    }

    /// How many children were spawned.
    pub fn spawns(&self) -> usize {
        self.spawns.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl Spawner for FakeSpawner {
    async fn spawn(&self, log: std::fs::File, env: &Env) -> std::io::Result<Box<dyn Spawned>> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        Ok(match &self.starter {
            Some(start) => start(log, env),
            None => Box::new(Running),
        })
    }
}

// CLI-13: and without `git`.
pub struct FakeGit {
    origin: Option<String>,
    branch: Branch,
}

impl FakeGit {
    pub fn new(origin: Option<&str>, branch: Branch) -> Arc<FakeGit> {
        Arc::new(FakeGit {
            origin: origin.map(str::to_string),
            branch,
        })
    }
}

#[async_trait::async_trait]
impl Git for FakeGit {
    async fn origin_url(&self, _cwd: &Path) -> Option<String> {
        self.origin.clone()
    }

    async fn push_branch(&self, _cwd: &Path) -> Branch {
        self.branch.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // CLI-13: the spawner in its three forms.
    #[tokio::test]
    async fn fake_spawner_three_forms() {
        let dir = crate::testkit::ShortDir::new();
        let path = dir.path().join("circitor.log");
        let open = || {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .unwrap()
        };
        let env = Env::default();

        let exits = FakeSpawner::exits(78, Some("bad config"));
        let mut child = exits.spawn(open(), &env).await.unwrap();
        assert_eq!(child.exited(), Some(78));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "bad config\n");
        assert_eq!(exits.spawns(), 1);
        let silent = FakeSpawner::exits(1, None);
        assert_eq!(silent.spawn(open(), &env).await.unwrap().exited(), Some(1));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "bad config\n");

        let never = FakeSpawner::never_binds();
        let mut child = never.spawn(open(), &env).await.unwrap();
        assert_eq!(child.exited(), None);
        assert_eq!(child.exited(), None);
        assert_eq!(never.spawns(), 1);

        let ran = Arc::new(AtomicUsize::new(0));
        let seen = ran.clone();
        let starts = FakeSpawner::starts(move |_log, _env| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::new(Exited(0))
        });
        assert_eq!(starts.spawn(open(), &env).await.unwrap().exited(), Some(0));
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert_eq!(starts.spawns(), 1);
    }

    #[tokio::test]
    async fn fake_git() {
        let git = FakeGit::new(
            Some("https://github.com/o/r.git"),
            Branch::Named("feature".into()),
        );
        for cwd in ["/a", "/b/c"] {
            assert_eq!(
                git.origin_url(Path::new(cwd)).await.as_deref(),
                Some("https://github.com/o/r.git")
            );
            assert_eq!(
                git.push_branch(Path::new(cwd)).await,
                Branch::Named("feature".into())
            );
        }
        let none = FakeGit::new(None, Branch::NoRepo);
        assert_eq!(none.origin_url(Path::new("/a")).await, None);
        assert_eq!(none.push_branch(Path::new("/a")).await, Branch::NoRepo);
    }
}
