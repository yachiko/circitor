//! The CLI (`plan/contracts/cli.md`). T02's and final: T11 builds `args`, `naming` and `guide`;
//! T10 `render`; T24 `conn`; T27 `commands`; T48 `notify`; T43 `fake`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::clock::SharedClock;
use crate::config::{Env, Platform};

pub mod args;
pub mod commands;
pub mod conn;
#[cfg(any(test, feature = "test-endpoint"))]
pub mod fake;
pub mod guide;
pub mod naming;
pub mod notify;
pub mod render;

/// CLI-5.
pub mod exit {
    pub const OK: i32 = 0;
    pub const ERROR: i32 = 1;
    pub const USAGE: i32 = 2;
    pub const TIMEOUT: i32 = 3;
    pub const ENDED: i32 = 4;
    pub const NOT_FOUND: i32 = 5;
    pub const NO_DAEMON: i32 = 6;
    pub const AUTH: i32 = 7;
    pub const CHECKS_FAILED: i32 = 8;
    pub const UNAVAILABLE: i32 = 9;
    pub const LIMIT: i32 = 10;
    /// `notify` (NTF-11).
    pub const NO_SESSION: i32 = 11;
    pub const SIGINT: i32 = 130;
    pub const SIGTERM: i32 = 143;
}

/// API-12's default, which the CLI always sends (CLI-6).
pub const DEFAULT_WAIT_TIMEOUT: u32 = 100;

/// Everything a command prints, and its exit code. Nothing is printed before the command ends (CLI-6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// A command's inputs, each through one interface (CLI-13). `CliEnv::real` is T27's (`commands`).
// CLI-13: the clocks, the directory, the environment, the version and the way a daemon starts.
#[derive(Clone)]
pub struct CliEnv {
    /// Durations by `mono()`, ages and local times by `wall()`.
    pub clock: SharedClock,
    /// For "until 14:05 local" (CLI-3); UTC in tests (TST-5).
    pub tz: chrono::FixedOffset,
    pub cwd: PathBuf,
    pub env: Env,
    pub platform: Platform,
    /// The CLI's own (CLI-9).
    pub version: String,
    pub spawner: Arc<dyn Spawner>,
    pub git: Arc<dyn Git>,
    pub interrupt: Interrupt,
    /// CLI-7's 90 s; `CIRCITOR_TEST_CLI_LIMIT` in a test build (TST-1).
    pub request_limit: Duration,
}

/// Runs one command to its end. `args` excludes the program's name. Never `circitor daemon` without
/// `--check-config`: `main` hands that to `daemon::run::main`.
pub async fn run(args: &[String], env: &CliEnv) -> Output {
    commands::run(args, env).await
}

/// The binary's entry: real `CliEnv`, a runtime, `run`, prints, returns the exit code.
pub fn main() -> i32 {
    commands::main()
}

/// How a daemon is started (CLI-7). The real one execs `circitor daemon --spawned`, detached, with `env`
/// less `GH_TOKEN`, `GITHUB_TOKEN` (OPS-5), `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN` (NTF-9).
#[async_trait::async_trait]
pub trait Spawner: Send + Sync + 'static {
    /// `log` is `circitor.log`, already rotated and opened for appending by the caller.
    async fn spawn(&self, log: std::fs::File, env: &Env) -> std::io::Result<Box<dyn Spawned>>;
}

pub trait Spawned: Send {
    /// `Some(status)` once the process has exited; `None` while it runs.
    fn exited(&mut self) -> Option<i32>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Branch {
    Named(String),
    Detached,
    NoRepo,
}

#[async_trait::async_trait]
pub trait Git: Send + Sync + 'static {
    /// `None`: no repository, or no `origin`.
    async fn origin_url(&self, cwd: &Path) -> Option<String>;
    /// The name `git` pushes the current branch to on `origin`: its upstream's branch, or its own name (CLI-2).
    async fn push_branch(&self, cwd: &Path) -> Branch;
}

/// `SIGINT` and `SIGTERM` (CLI-6). Resolves with 130 or 143.
#[derive(Clone)]
pub struct Interrupt(tokio::sync::watch::Receiver<Option<i32>>);

/// Fires an `Interrupt`: tests, and `main`'s signal handlers.
#[derive(Clone)]
pub struct InterruptTrigger(Arc<tokio::sync::watch::Sender<Option<i32>>>);

impl InterruptTrigger {
    /// The first code fired is the one every `wait` resolves with.
    pub fn fire(&self, code: i32) {
        self.0.send_if_modified(|held| {
            if held.is_none() {
                *held = Some(code);
                true
            } else {
                false
            }
        });
    }
}

impl Interrupt {
    /// One that never fires.
    pub fn never() -> Interrupt {
        let (_, rx) = tokio::sync::watch::channel(None);
        Interrupt(rx)
    }

    pub fn pair() -> (InterruptTrigger, Interrupt) {
        let (tx, rx) = tokio::sync::watch::channel(None);
        (InterruptTrigger(Arc::new(tx)), Interrupt(rx))
    }

    /// The code fired, once it is; pends for ever when nothing can fire it.
    pub async fn wait(&mut self) -> i32 {
        loop {
            if let Some(code) = *self.0.borrow_and_update() {
                return code;
            }
            if self.0.changed().await.is_err() {
                // Every trigger is gone without firing: nothing ever will.
                return futures::future::pending().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, Mono, Time};
    use futures::FutureExt;
    use futures::future::BoxFuture;

    struct StillClock;
    impl Clock for StillClock {
        fn wall(&self) -> Time {
            Time::parse("2026-11-24T22:00:00Z").unwrap()
        }
        fn mono(&self) -> Mono {
            Mono::ZERO
        }
        fn sleep_until(&self, _at: Mono) -> BoxFuture<'static, ()> {
            futures::future::pending().boxed()
        }
    }

    struct NoSpawn;
    #[async_trait::async_trait]
    impl Spawner for NoSpawn {
        async fn spawn(
            &self,
            _log: std::fs::File,
            _env: &Env,
        ) -> std::io::Result<Box<dyn Spawned>> {
            Err(std::io::Error::other("no spawning here"))
        }
    }

    struct NoGit;
    #[async_trait::async_trait]
    impl Git for NoGit {
        async fn origin_url(&self, _cwd: &Path) -> Option<String> {
            None
        }
        async fn push_branch(&self, _cwd: &Path) -> Branch {
            Branch::NoRepo
        }
    }

    // CLI-13: every input of a command is a value the test hands over; nothing is the process's.
    #[tokio::test]
    async fn cli13_an_env_is_built_from_values() {
        let mut vars = Env::default();
        vars.vars
            .insert("CIRCITOR_SOCKET".into(), "/tmp/x.sock".into());
        let (trigger, interrupt) = Interrupt::pair();
        let env = CliEnv {
            clock: Arc::new(StillClock),
            tz: chrono::FixedOffset::east_opt(0).unwrap(),
            cwd: PathBuf::from("/nowhere"),
            env: vars,
            platform: Platform::MacOs,
            version: "1.2.3+test".into(),
            spawner: Arc::new(NoSpawn),
            git: Arc::new(NoGit),
            interrupt,
            request_limit: Duration::from_secs(90),
        };
        let copy = env.clone();
        assert_eq!(copy.env.get("CIRCITOR_SOCKET"), Some("/tmp/x.sock"));
        assert_eq!(copy.clock.wall().rfc3339(), "2026-11-24T22:00:00Z");
        assert_eq!(copy.git.push_branch(&copy.cwd).await, Branch::NoRepo);
        assert!(copy.git.origin_url(&copy.cwd).await.is_none());
        let log = tempfile::tempfile().unwrap();
        assert!(copy.spawner.spawn(log, &copy.env).await.is_err());
        trigger.fire(exit::SIGINT);
        assert_eq!(copy.interrupt.clone().wait().await, exit::SIGINT);
    }

    #[tokio::test]
    async fn interrupt_never_pends_and_a_pair_fires() {
        let mut never = Interrupt::never();
        assert!(never.wait().now_or_never().is_none());

        for code in [exit::SIGINT, exit::SIGTERM] {
            let (trigger, mut interrupt) = Interrupt::pair();
            assert!(interrupt.wait().now_or_never().is_none());
            let mut other = interrupt.clone();
            let waiter = tokio::spawn(async move { other.wait().await });
            trigger.fire(code);
            trigger.fire(1);
            assert_eq!(interrupt.wait().await, code);
            assert_eq!(waiter.await.unwrap(), code);
            // A clone made after the fire sees it too.
            assert_eq!(interrupt.clone().wait().await, code);
        }

        let (trigger, mut interrupt) = Interrupt::pair();
        drop(trigger);
        assert!(interrupt.wait().now_or_never().is_none());
    }

    #[test]
    fn exit_codes() {
        assert_eq!(
            [
                exit::OK,
                exit::ERROR,
                exit::USAGE,
                exit::TIMEOUT,
                exit::ENDED,
                exit::NOT_FOUND,
                exit::NO_DAEMON,
                exit::AUTH,
                exit::CHECKS_FAILED,
                exit::UNAVAILABLE,
                exit::LIMIT,
                exit::NO_SESSION
            ],
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        );
        assert_eq!((exit::SIGINT, exit::SIGTERM), (130, 143));
        assert_eq!(notify::SOCKET_VAR, "CLAUDE_CODE_MESSAGING_SOCKET");
        assert_eq!(notify::TOKEN_VAR, "CLAUDE_CODE_MESSAGING_TOKEN");
    }
}
