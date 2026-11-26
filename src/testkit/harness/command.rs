//! Commands as functions (CLI-13, TST-5): [`Cmd`], awaited or in the background ([`Bg`]), and the
//! probe that keeps the test clock from moving while a background command has an answer to read.

use std::future::{Future, IntoFuture};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::task::JoinHandle;

use crate::cli::commands::RealGit;
use crate::cli::{CliEnv, Interrupt, Output, Spawner};
use crate::clock::SettleProbe;
use crate::config::{Env, Platform};

use super::{Shared, lock};

/// CLI-7's limit on any request but a wait.
const REQUEST_LIMIT: Duration = Duration::from_secs(90);
/// How many yields `Bg::cancel` gives the daemon to see the connection close.
const CLOSE_YIELDS: usize = 100_000;

// TST-5: each command takes its inputs (CLI-13): the test clock, UTC, a directory and environment,
// a version and a spawner.
/// TST-4, the harness's share: unsettled while a background command has been woken and not yet
/// polled, so an answer the daemon sent is read before time moves on. A command parked on a timer
/// or on its socket counts as settled.
#[derive(Default)]
pub(super) struct BgProbe {
    woken: AtomicUsize,
}

impl SettleProbe for BgProbe {
    fn settled(&self) -> bool {
        self.woken.load(Ordering::SeqCst) == 0
    }
}

/// The waker a tracked command is polled with: marks it woken, then wakes its task.
struct Flag {
    probe: Arc<BgProbe>,
    woken: AtomicBool,
    outer: Mutex<Option<Waker>>,
}

impl Flag {
    fn set(&self) {
        if !self.woken.swap(true, Ordering::SeqCst) {
            self.probe.woken.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn clear(&self) {
        if self.woken.swap(false, Ordering::SeqCst) {
            self.probe.woken.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.set();
        let outer = lock(&self.outer).clone();
        if let Some(w) = outer {
            w.wake();
        }
    }
}

/// A command's future, polled through a [`Flag`].
struct Tracked<F> {
    fut: Pin<Box<F>>,
    flag: Arc<Flag>,
    waker: Waker,
}

impl<F: Future> Tracked<F> {
    fn new(fut: F, probe: Arc<BgProbe>) -> Tracked<F> {
        let flag = Arc::new(Flag {
            probe,
            woken: AtomicBool::new(false),
            outer: Mutex::new(None),
        });
        let waker = Waker::from(flag.clone());
        Tracked {
            fut: Box::pin(fut),
            flag,
            waker,
        }
    }
}

impl<F: Future> Future for Tracked<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        *lock(&self.flag.outer) = Some(cx.waker().clone());
        self.flag.clear();
        let waker = self.waker.clone();
        let mut inner = Context::from_waker(&waker);
        let polled = self.fut.as_mut().poll(&mut inner);
        if polled.is_ready() {
            self.flag.clear();
        }
        polled
    }
}

impl<F> Drop for Tracked<F> {
    fn drop(&mut self) {
        // A cancelled command is no longer anything to wait for.
        self.flag.clear();
        *lock(&self.flag.outer) = None;
    }
}

/// One background command as the sweep sees it: `finish` fails while one is neither done nor cancelled.
pub(super) struct BgEntry {
    pub args: Vec<String>,
    pub done: Arc<AtomicBool>,
}

/// A command to run: [`Harness::cmd`](super::Harness::cmd) with the harness's defaults, each of which
/// can be changed before [`Cmd::run`] or [`Cmd::bg`].
pub struct Cmd {
    shared: Arc<Shared>,
    args: Vec<String>,
    env: CliEnv,
}

impl Cmd {
    pub(super) fn new(
        shared: Arc<Shared>,
        args: &[&str],
        env: Env,
        spawner: Arc<dyn Spawner>,
    ) -> Cmd {
        let env = CliEnv {
            clock: shared.clock.clone(),
            // TST-5: a fixed time zone, UTC.
            tz: chrono::FixedOffset::east_opt(0).expect("UTC is an offset"),
            cwd: shared.dir.clone(),
            env,
            platform: Platform::Linux,
            version: crate::testkit::TEST_VERSION.to_string(),
            spawner,
            git: Arc::new(RealGit),
            interrupt: Interrupt::never(),
            request_limit: REQUEST_LIMIT,
        };
        Cmd {
            shared,
            args: args.iter().map(|a| (*a).to_string()).collect(),
            env,
        }
    }

    /// The working directory (the harness's directory unless set).
    pub fn cwd(mut self, cwd: &Path) -> Cmd {
        self.env.cwd = cwd.to_path_buf();
        self
    }

    /// The whole environment, in place of [`Harness::env`](super::Harness::env).
    pub fn env(mut self, env: Env) -> Cmd {
        self.env.env = env;
        self
    }

    /// Sets one variable of the environment.
    pub fn var(mut self, name: &str, value: &str) -> Cmd {
        self.env
            .env
            .vars
            .insert(name.to_string(), value.to_string());
        self
    }

    /// Takes one variable out of the environment.
    pub fn without(mut self, name: &str) -> Cmd {
        self.env.env.vars.remove(name);
        self
    }

    /// The CLI's own version (CLI-9); `TEST_VERSION` unless set.
    pub fn version(mut self, version: &str) -> Cmd {
        self.env.version = version.to_string();
        self
    }

    /// The way it starts a daemon; the harness's [`spawner`](super::Harness::spawner) unless set.
    pub fn spawner(mut self, spawner: Arc<dyn Spawner>) -> Cmd {
        self.env.spawner = spawner;
        self
    }

    /// `SIGINT` and `SIGTERM` (CLI-6); never unless set.
    pub fn interrupt(mut self, interrupt: Interrupt) -> Cmd {
        self.env.interrupt = interrupt;
        self
    }

    /// Runs it to its end, awaited. It hangs if the command waits on the clock: use [`Cmd::bg`].
    pub async fn run(self) -> Output {
        let out = crate::cli::run(&self.args, &self.env).await;
        lock(&self.shared.outputs).push(out.clone());
        out
    }

    /// Runs it on a task of its own, for a command the test advances time under.
    pub fn bg(self) -> Bg {
        let done = Arc::new(AtomicBool::new(false));
        lock(&self.shared.bg).push(BgEntry {
            args: self.args.clone(),
            done: done.clone(),
        });
        let before = self.shared.open_connections();
        let shared = self.shared.clone();
        let finished = done.clone();
        let (args, env) = (self.args, self.env);
        let fut = async move {
            let out = crate::cli::run(&args, &env).await;
            lock(&shared.outputs).push(out.clone());
            finished.store(true, Ordering::SeqCst);
            out
        };
        let task = tokio::spawn(Tracked::new(fut, self.shared.bg_probe.clone()));
        Bg {
            shared: self.shared,
            task: Some(task),
            done,
            before,
        }
    }
}

/// A background command. `is_done` without awaiting; `.await` gives its [`Output`]; [`Bg::cancel`]
/// drops it.
pub struct Bg {
    shared: Arc<Shared>,
    task: Option<JoinHandle<Output>>,
    done: Arc<AtomicBool>,
    /// The daemon's open connections when it began.
    before: usize,
}

impl Bg {
    /// Whether the command has ended.
    pub fn is_done(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// The command's future is dropped, so its connection closes. Returns once the daemon has seen
    /// the close (`Running::open_connections` back to what it was when the command began).
    pub async fn cancel(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        self.done.store(true, Ordering::SeqCst);
        for _ in 0..CLOSE_YIELDS {
            if self.shared.open_connections() <= self.before {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "Bg::cancel: the daemon still has {} connections open, {} before the command",
            self.shared.open_connections(),
            self.before
        );
    }
}

impl IntoFuture for Bg {
    type Output = Output;
    type IntoFuture = BoxFuture<'static, Output>;

    fn into_future(mut self) -> Self::IntoFuture {
        let task = self.task.take();
        Box::pin(async move {
            match task {
                Some(task) => match task.await {
                    Ok(out) => out,
                    Err(e) => std::panic::resume_unwind(match e.try_into_panic() {
                        Ok(p) => p,
                        Err(e) => Box::new(format!("the background command didn't end: {e}")),
                    }),
                },
                None => panic!("the background command was already taken"),
            }
        })
    }
}

/// The directory a new git repository goes in.
pub(super) fn repo_dir(dir: &Path, n: usize) -> PathBuf {
    dir.join(format!("repo{n}"))
}
