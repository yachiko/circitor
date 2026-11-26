//! The real token source: runs `gh auth token` (T12, OPS-5).
//!
//! `gh`'s output goes into a `Token` and nowhere else: this type has no logger, and no error or
//! `Debug` output of it holds a byte of either stream.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared, WeakShared};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::clock::{Activity, SharedClock, timeout};
use crate::config::Env;

use super::{GhFailure, Token, TokenRead, TokenSource, TokenSourceKind};

/// OPS-5: a read's limit, the lookup included.
const READ_LIMIT: Duration = Duration::from_secs(10);

/// OPS-5 (proposal): Homebrew's `bin` directories, tried in this order after `PATH`.
const FIXED_DIRS: [&str; 3] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/home/linuxbrew/.linuxbrew/bin",
];

/// docs/02 §1, gh 2.101.0: what stderr holds when there is no login.
const NO_LOGIN: &[u8] = b"no oauth token found";

type RunFuture = BoxFuture<'static, TokenRead>;

pub struct GhTokenSource {
    configured: String,
    env: Env,
    clock: SharedClock,
    activity: Activity,
    fixed_dirs: Vec<PathBuf>,
    kind: TokenSourceKind,
    /// The path last run.
    last_path: Arc<Mutex<Option<String>>>,
    /// The run in progress. Weak: the run belongs to its callers (OPS-7).
    running: Mutex<Option<WeakShared<RunFuture>>>,
}

impl GhTokenSource {
    /// `gh_path` as configured (OPS-2); `env` is the daemon's own: it gives `PATH`, decides `kind()`, and is the child's environment.
    pub fn new(
        gh_path: &str,
        env: Env,
        clock: SharedClock,
        activity: Activity,
    ) -> Arc<GhTokenSource> {
        let dirs = FIXED_DIRS.iter().map(PathBuf::from).collect();
        Arc::new(Self::build(gh_path, env, clock, activity, dirs))
    }

    /// For tests: their own list in place of Homebrew's directories.
    #[cfg(test)]
    pub(crate) fn with_fixed_dirs(
        gh_path: &str,
        env: Env,
        clock: SharedClock,
        activity: Activity,
        dirs: Vec<PathBuf>,
    ) -> Arc<GhTokenSource> {
        Arc::new(Self::build(gh_path, env, clock, activity, dirs))
    }

    fn build(
        gh_path: &str,
        env: Env,
        clock: SharedClock,
        activity: Activity,
        fixed_dirs: Vec<PathBuf>,
    ) -> GhTokenSource {
        // docs/02 §1: an empty variable is ignored (`Env::get` says `None` for it).
        let kind = if env.get("GH_TOKEN").is_some() {
            TokenSourceKind::GhTokenVar
        } else if env.get("GITHUB_TOKEN").is_some() {
            TokenSourceKind::GithubTokenVar
        } else {
            TokenSourceKind::Gh
        };
        GhTokenSource {
            configured: gh_path.to_string(),
            env,
            clock,
            activity,
            fixed_dirs,
            kind,
            last_path: Arc::new(Mutex::new(None)),
            running: Mutex::new(None),
        }
    }

    fn new_run(&self) -> Shared<RunFuture> {
        let job = Job {
            configured: self.configured.clone(),
            env: self.env.clone(),
            fixed_dirs: self.fixed_dirs.clone(),
            last_path: self.last_path.clone(),
        };
        let clock = self.clock.clone();
        let activity = self.activity.clone();
        let run: RunFuture = async move {
            // TST-4: a guard while `gh` runs (the clock contract: real I/O that ends without the clock moving).
            let _busy = activity.begin();
            let mut slot: Option<Child> = None;
            match timeout(&clock, READ_LIMIT, job.run(&mut slot)).await {
                Ok(read) => read,
                Err(_) => {
                    // OPS-5: kill it, and wait until it is reaped, then answer.
                    if let Some(child) = slot.as_mut() {
                        let _ = child.kill().await;
                    }
                    TokenRead::Failed(GhFailure::Timeout)
                }
            }
        }
        .boxed();
        run.shared()
    }
}

struct Job {
    configured: String,
    env: Env,
    fixed_dirs: Vec<PathBuf>,
    last_path: Arc<Mutex<Option<String>>>,
}

impl Job {
    /// Finds `gh` and runs it. The child is put in `slot` so the caller can kill and reap it.
    async fn run(&self, slot: &mut Option<Child>) -> TokenRead {
        let Some(path) = self.find().await else {
            self.remember(self.configured.clone());
            return TokenRead::Failed(GhFailure::CantRun);
        };
        self.remember(path.to_string_lossy().into_owned());
        // U11, U13: gh's login for github.com, those four arguments, the given environment only.
        let spawned = Command::new(&path)
            .args(["auth", "token", "--hostname", "github.com"])
            .env_clear()
            .envs(&self.env.vars)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let Ok(child) = spawned else {
            return TokenRead::Failed(GhFailure::CantRun);
        };
        let child = slot.insert(child);
        let (Some(mut out), Some(mut err)) = (child.stdout.take(), child.stderr.take()) else {
            return TokenRead::Failed(GhFailure::CantRun);
        };
        let mut out_bytes = Vec::new();
        let mut err_bytes = Vec::new();
        // Both pipes are read together with the wait: a full pipe never blocks the child.
        let (status, _, _) = tokio::join!(
            child.wait(),
            out.read_to_end(&mut out_bytes),
            err.read_to_end(&mut err_bytes)
        );
        let Ok(status) = status else {
            return TokenRead::Failed(GhFailure::CantRun);
        };
        interpret(status.code(), out_bytes, &err_bytes)
    }

    fn remember(&self, path: String) {
        *self
            .last_path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
    }

    /// OPS-5: a path with a `/` is the only one tried; a name is looked for in `PATH`, then in the fixed places.
    async fn find(&self) -> Option<PathBuf> {
        if self.configured.contains('/') {
            let path = PathBuf::from(&self.configured);
            return is_runnable(&path).await.then_some(path);
        }
        let from_path = self
            .env
            .vars
            .get("PATH")
            .map(String::as_str)
            .unwrap_or("")
            .split(':')
            .filter(|entry| !entry.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        for dir in from_path.iter().chain(self.fixed_dirs.iter()) {
            let candidate = dir.join(&self.configured);
            if is_runnable(&candidate).await {
                return Some(candidate);
            }
        }
        None
    }
}

/// A regular file with an execute bit.
async fn is_runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match tokio::fs::metadata(path).await {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

/// OPS-5: the three outcomes. `code` is `None` when a signal ended the process.
fn interpret(code: Option<i32>, stdout: Vec<u8>, stderr: &[u8]) -> TokenRead {
    match code {
        None => TokenRead::Failed(GhFailure::Killed),
        Some(0) => {
            // Converted once, strictly: bytes that aren't UTF-8 are dropped.
            let Ok(mut text) = String::from_utf8(stdout) else {
                return TokenRead::Failed(GhFailure::NoOutput);
            };
            if text.ends_with('\n') {
                text.pop();
            }
            if text.trim().is_empty() || text.contains('\n') {
                return TokenRead::Failed(GhFailure::NoOutput);
            }
            TokenRead::Token(Token::new(text))
        }
        Some(code) => {
            if stderr.windows(NO_LOGIN.len()).any(|w| w == NO_LOGIN) {
                TokenRead::NoLogin
            } else {
                TokenRead::Failed(GhFailure::Exit(code))
            }
        }
    }
}

#[async_trait::async_trait]
impl TokenSource for GhTokenSource {
    async fn read(&self) -> TokenRead {
        // OPS-5: concurrent reads share one run; the run is held weakly, so when every caller is gone it is dropped.
        let run = {
            let mut running = self
                .running
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let live = running
                .as_ref()
                .and_then(WeakShared::upgrade)
                .filter(|run| run.peek().is_none());
            match live {
                Some(run) => run,
                None => {
                    let run = self.new_run();
                    *running = run.downgrade();
                    run
                }
            }
        };
        run.await
    }

    fn kind(&self) -> TokenSourceKind {
        self.kind
    }

    fn gh_path(&self) -> String {
        self.last_path
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_else(|| self.configured.clone())
    }
}

#[cfg(test)]
mod tests;
