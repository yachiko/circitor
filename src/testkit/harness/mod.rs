//! The harness (TST-5, TST-11, TST-16; T29): the real daemon started in-process on the test clock with
//! the fake GitHub, a fake token source and a fake session socket; stopped, crashed and restarted on
//! the same directories; the CLI's commands run against it as functions; and every test ended with
//! TST-11's sweep. The names are T29's (`plan/contracts/testkit.md` declares none in code).
//!
//! # Starting
//!
//! [`Harness::start`] with [`Opts`]. Unless a test says otherwise (TST-5): a `spawned` daemon with
//! `Config::default()` but `exit_after` `None`, its log in a [`BufferSink`] at `info`, the clock at
//! 2026-11-24T22:00:00Z, the seed [`SEED`] plus the start's number, a [`FakeTokenSource`] giving
//! `MARKER_TOKEN`, a [`FaultFs`] for `state.json`, no signal handlers, and `TEST_VERSION` for the
//! daemon and the CLI. `start` ends with `clock.settle()`; it doesn't advance, so the first poll
//! (and with it API-18's wait) is the test's to let happen.
//!
//! [`Opts`] sets what must hold before the daemon's first request: [`Opts::world`] scripts the world,
//! [`Opts::github`] the fake GitHub otherwise (rate, login, a PR's error), [`Opts::fault`],
//! [`Opts::hold_first`] (taken back with [`Harness::take_hold`]), [`Opts::config`] and its shortcuts
//! ([`Opts::owners`], [`Opts::keep_done`], [`Opts::exit_after`], [`Opts::quota`]), [`Opts::token`]
//! and [`Opts::token_delay`], [`Opts::skew_ahead`] and [`Opts::skew_behind`], [`Opts::version`] and
//! [`Opts::mode`], and [`Opts::no_daemon`]: nothing runs, and the harness's [`spawner`] starts the
//! in-process daemon when a command asks ([`Harness::spawns`] counts).
//!
//! # Paths and the session
//!
//! One [`ShortDir`]. The [`Env`] every command gets ([`Harness::env`]) has `XDG_STATE_HOME` and
//! `XDG_CONFIG_HOME` inside it, no `XDG_RUNTIME_DIR` and no home, and is resolved once with
//! `config::resolve(&env, Platform::Linux)` for the daemon ([`Harness::paths`]). It also holds
//! `CLAUDE_CODE_MESSAGING_SOCKET`, the path of the [`FakeSession`] named `session` bound in the same
//! directory ([`Harness::session`]), and `CLAUDE_CODE_MESSAGING_TOKEN`, `MARKER_SESSION_TOKEN`
//! (TST-16). [`Harness::new_session`] binds another. Sessions live as long as the harness.
//!
//! # The daemon
//!
//! [`Harness::stop`] (the exit status), [`Harness::crash`], [`Harness::restart`],
//! [`Harness::restart_with`] (another config) and [`Harness::restart_as`] (another version or mode),
//! always on the same directories, the same `FaultFs` and the same fake GitHub, with new `Deps` and
//! the next seed; each start, restart and crash calls the fake's `new_daemon()`. [`Harness::exited`]
//! is the daemon's own exit status, once it has one. A daemon the spawner starts takes the slot of
//! the one before it.
//!
//! # Commands
//!
//! [`Harness::cli`] awaits one; [`Harness::cli_bg`] runs one on a task, for a command the test moves
//! the clock under (every `wait`, every spawn, a `list` before the first poll, a `show` that reads
//! through, `stop`). [`Harness::cmd`] gives a [`Cmd`] whose working directory, environment, version,
//! spawner and interrupt can be set first. A [`Bg`] says [`Bg::is_done`], gives its `Output` when
//! awaited, and has [`Bg::cancel`]. While a background command has been woken and not yet polled the
//! clock isn't settled. Spawners: [`Harness::spawner`] (the in-process daemon),
//! [`spawner_bad_config`], [`spawner_exits_ok`] and [`spawner_never_binds`].
//!
//! [`Harness::api_raw`] sends one request the CLI can't; [`Harness::git_repo`] makes a repository.
//!
//! # The sweep
//!
//! [`Harness::finish`], which every test ends with (TST-11); a harness dropped without it panics.
//!
//! ```ignore
//! #[tokio::test]
//! async fn example() {
//!     let h = Harness::start(Opts::default().world(|w| {
//!         let repo = w.repo("o", "r");
//!         w.open_pr(repo);
//!     }))
//!     .await;
//!     h.clock().advance(Duration::from_secs(5)).await;       // the first poll
//!     assert_eq!(h.cli(&["list"]).await.code, 0);
//!     let wait = h.cli_bg(&["wait", "o/r#1", "--until", "merged"]);
//!     h.clock().advance(Duration::from_secs(101)).await;
//!     assert_eq!(wait.await.code, 3);
//!     h.finish().await;
//! }
//! ```

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::api::Request;
use crate::cli::Output;
use crate::cli::fake::FakeSpawner;
use crate::cli::notify::{SOCKET_VAR, TOKEN_VAR};
use crate::clock::{SharedClock, TestClock, Time};
use crate::config::{Config, Env, Paths, Platform, QuotaConfig, resolve};
use crate::daemon::{Deps, Mode, Running, StartError, StopCause, start};
use crate::log::{BufferSink, Level};
use crate::notify::RealPoster;
use crate::state::file::FaultFs;
use crate::testkit::github::World;
use crate::testkit::server::{Aim, FakeGitHub, Fault, Hold, Times};
use crate::testkit::session::FakeSession;
use crate::testkit::{
    MARKER_CHECK, MARKER_GITHUB_TEXT, MARKER_LOGIN, MARKER_SESSION_TOKEN, MARKER_TITLE,
    MARKER_TOKEN, MARKER_TOKEN_2, ShortDir, TEST_VERSION,
};
use crate::token::TokenRead;
use crate::token::fake::FakeTokenSource;
use crate::transport::RealProbe;

mod command;
mod raw;
mod spawn;

pub use command::{Bg, Cmd};

use command::{BgEntry, BgProbe};

/// The fixed seed (TST-5); each start adds its number, so a restart has another epoch (STA-9).
pub const SEED: u64 = 0x7329;
/// TST-5: the test clock starts here.
pub const START: &str = "2026-11-24T22:00:00Z";
/// How far `finish` and `stop` move the clock in one step when the daemon waits on it.
const DRIVE_STEP: Duration = Duration::from_millis(100);
/// At most this many steps (60 s of the clock).
const DRIVE_STEPS: usize = 600;
/// OPS-1's message the bad-config spawner writes to the log.
pub const BAD_CONFIG: &str = "circitor: config.toml: unknown key `colour` (OPS-1)";

type WorldFn = Box<dyn FnOnce(&mut World) + Send>;
type GitHubFn = Box<dyn FnOnce(&FakeGitHub) + Send>;

/// A std lock, taken and released inside one function; a poisoned one still holds its value.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What holds before the daemon's first request (`plan/contracts/testkit.md`).
pub struct Opts {
    world: Vec<WorldFn>,
    github: Vec<GitHubFn>,
    config: Config,
    token: Option<TokenRead>,
    token_delay: Option<Duration>,
    hold_first: bool,
    version: String,
    mode: Mode,
    no_daemon: bool,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            world: Vec::new(),
            github: Vec::new(),
            // TST-5: OPS-2's defaults but `exit_after` 0, never.
            config: Config {
                exit_after: None,
                ..Config::default()
            },
            token: None,
            token_delay: None,
            hold_first: false,
            version: TEST_VERSION.to_string(),
            mode: Mode::Spawned,
            no_daemon: false,
        }
    }
}

impl Opts {
    /// Scripts the world before anything runs (several calls run in order).
    pub fn world(mut self, f: impl FnOnce(&mut World) + Send + 'static) -> Opts {
        self.world.push(Box::new(f));
        self
    }

    /// Anything else on the fake GitHub, after the world is scripted: its rate, its login, a PR's error.
    pub fn github(mut self, f: impl FnOnce(&FakeGitHub) + Send + 'static) -> Opts {
        self.github.push(Box::new(f));
        self
    }

    /// A fault in force from the first request.
    pub fn fault(self, fault: Fault, aim: Aim, times: Times) -> Opts {
        self.github(move |gh| gh.fault_aimed(fault, aim, times))
    }

    /// The first request is held until [`Harness::take_hold`]'s handle is released.
    pub fn hold_first(mut self) -> Opts {
        self.hold_first = true;
        self
    }

    /// GitHub's clock runs ahead of the test clock's wall time.
    pub fn skew_ahead(self, d: Duration) -> Opts {
        self.github(move |gh| gh.set_skew(d))
    }

    /// GitHub's clock runs behind.
    pub fn skew_behind(self, d: Duration) -> Opts {
        self.github(move |gh| gh.set_skew_behind(d))
    }

    /// Changes the config (the intervals, anything else). Built by hand, never loaded: a test build's
    /// range allows `exit_after` of 1 s (TST-1).
    pub fn config(mut self, f: impl FnOnce(&mut Config)) -> Opts {
        f(&mut self.config);
        self
    }

    pub fn owners(self, owners: &[&str]) -> Opts {
        self.config(|c| c.owners = owners.iter().map(|o| (*o).to_string()).collect())
    }

    pub fn keep_done(self, d: Duration) -> Opts {
        self.config(|c| c.keep_done = d)
    }

    pub fn exit_after(self, d: Option<Duration>) -> Opts {
        self.config(|c| c.exit_after = d)
    }

    pub fn quota(self, max_per_hour: u32, reserve: u32) -> Opts {
        self.config(|c| {
            c.quota = QuotaConfig {
                max_per_hour,
                reserve,
            }
        })
    }

    /// The token source's first answer.
    pub fn token(mut self, read: TokenRead) -> Opts {
        self.token = Some(read);
        self
    }

    /// Each read of the token takes this much of the clock.
    pub fn token_delay(mut self, d: Duration) -> Opts {
        self.token_delay = Some(d);
        self
    }

    /// The first daemon's version (CLI-9).
    pub fn version(mut self, version: &str) -> Opts {
        self.version = version.to_string();
        self
    }

    /// The first daemon's mode (OPS-6).
    pub fn mode(mut self, mode: Mode) -> Opts {
        self.mode = mode;
        self
    }

    /// No daemon runs yet: the spawner starts one when a command asks.
    pub fn no_daemon(mut self) -> Opts {
        self.no_daemon = true;
        self
    }
}

/// What the harness, its spawner and its commands share.
struct Shared {
    pub clock: Arc<TestClock>,
    pub dir: PathBuf,
    pub github: FakeGitHub,
    pub token: Arc<FakeTokenSource>,
    pub fs: Arc<FaultFs>,
    pub log: BufferSink,
    pub paths: Paths,
    pub config: Mutex<Config>,
    pub version: Mutex<String>,
    pub mode: Mutex<Mode>,
    /// Starts so far: each one's number, and its seed's offset.
    pub starts: AtomicU64,
    /// The daemon now, with its start's number. An exited one stays until the next start.
    pub daemon: Mutex<Option<(u64, Running)>>,
    pub outputs: Mutex<Vec<Output>>,
    pub raw: Mutex<Vec<String>>,
    pub bg: Mutex<Vec<BgEntry>>,
    pub bg_probe: Arc<BgProbe>,
}

impl Shared {
    /// Starts a daemon on the harness's directories with new `Deps` and the next seed; it takes the slot.
    pub async fn launch(&self, version: String, mode: Mode) -> Result<u64, StartError> {
        let n = self.starts.fetch_add(1, Ordering::SeqCst);
        self.github.new_daemon();
        let clock: SharedClock = self.clock.clone();
        let deps = Deps {
            clock: clock.clone(),
            activity: self.clock.activity(),
            github: Arc::new(self.github.client()),
            token_source: self.token.clone(),
            fs: self.fs.clone(),
            probe: Arc::new(RealProbe {
                clock: clock.clone(),
            }),
            poster: RealPoster::new(),
            paths: self.paths.clone(),
            config: lock(&self.config).clone(),
            mode,
            version,
            log: Arc::new(self.log.clone()),
            level: Level::Info,
            // STA-9: the epoch comes from the seed alone.
            seed: SEED + n,
            signals: false,
            stop_delay: None,
        };
        let running = start(deps).await?;
        let old = lock(&self.daemon).replace((n, running));
        drop(old);
        Ok(n)
    }

    /// The exit status of start `n`'s daemon, while it holds the slot.
    pub fn exit_of(&self, n: u64) -> Option<i32> {
        match &*lock(&self.daemon) {
            Some((m, r)) if *m == n => r.exit_status(),
            _ => None,
        }
    }

    pub fn open_connections(&self) -> usize {
        lock(&self.daemon)
            .as_ref()
            .map_or(0, |(_, r)| r.open_connections())
    }

    /// Whether a daemon runs now.
    fn running(&self) -> bool {
        lock(&self.daemon)
            .as_ref()
            .is_some_and(|(_, r)| r.exit_status().is_none())
    }

    /// Waits for the daemon in the slot to end by itself, moving the clock as it needs; false if
    /// there's none.
    async fn exit_by_itself(&self) -> bool {
        let Some((n, mut r)) = lock(&self.daemon).take() else {
            return false;
        };
        drive(&self.clock, "a daemon's own exit", r.exited()).await;
        let mut slot = lock(&self.daemon);
        if slot.is_none() {
            *slot = Some((n, r));
        }
        true
    }

    /// Stops the daemon in the slot if it runs, and returns its status; it stays in the slot.
    async fn stop(&self) -> Option<i32> {
        let (n, mut r) = lock(&self.daemon).take()?;
        let status = match r.exit_status() {
            Some(s) => s,
            None => {
                r.stop(StopCause::Test);
                drive(&self.clock, "stop", r.exited()).await
            }
        };
        let mut slot = lock(&self.daemon);
        if slot.is_none() {
            *slot = Some((n, r));
        }
        Some(status)
    }
}

/// Awaits `fut`, moving the clock in 100 ms steps whenever it's settled and `fut` isn't done: for a
/// daemon whose stop, or whose answer, waits on the clock. Panics after 60 s of the clock.
async fn drive<T>(clock: &TestClock, what: &str, fut: impl Future<Output = T>) -> T {
    let mut fut = std::pin::pin!(fut);
    for _ in 0..DRIVE_STEPS {
        tokio::select! {
            biased;
            v = &mut fut => return v,
            _ = clock.settle() => {}
        }
        tokio::select! {
            biased;
            v = &mut fut => return v,
            _ = clock.advance(DRIVE_STEP) => {}
        }
    }
    panic!("{what}: not done after 60 s of the clock");
}

/// A spawner whose child writes OPS-1's message ([`BAD_CONFIG`]) to the log and exits 1.
pub fn spawner_bad_config() -> Arc<FakeSpawner> {
    FakeSpawner::exits(1, Some(BAD_CONFIG))
}

/// A spawner whose child exits 0 at once, as one that found the lock held does (STA-11).
pub fn spawner_exits_ok() -> Arc<FakeSpawner> {
    FakeSpawner::exits(0, None)
}

/// A spawner whose child runs and binds nothing.
pub fn spawner_never_binds() -> Arc<FakeSpawner> {
    FakeSpawner::never_binds()
}

/// The harness. See the module's documentation.
pub struct Harness {
    shared: Arc<Shared>,
    spawner: Arc<FakeSpawner>,
    env: Env,
    sessions: Mutex<Vec<Arc<FakeSession>>>,
    hold: Mutex<Option<Hold>>,
    repos: AtomicUsize,
    finished: bool,
    /// Last: removed once everything above is gone.
    dir: ShortDir,
}

impl Harness {
    /// Starts the fakes and, unless `no_daemon`, the daemon; settles.
    // TST-5: the daemon in-process, on the test clock, with the fakes and directories under `/tmp`.
    pub async fn start(opts: Opts) -> Harness {
        let dir = ShortDir::new();
        let start_at = Time::parse(START).expect("START is RFC 3339");
        let clock = TestClock::new(start_at);
        let github = FakeGitHub::start(clock.clone());
        for f in opts.world {
            github.world(f);
        }
        for f in opts.github {
            f(&github);
        }
        let hold = opts.hold_first.then(|| github.hold_next());
        let shared_clock: SharedClock = clock.clone();
        let token = FakeTokenSource::new(shared_clock);
        if let Some(read) = opts.token {
            token.set(read);
        }
        if let Some(d) = opts.token_delay {
            token.set_delay(d);
        }

        // Paths: XDG variables inside the directory, resolved once for the daemon and the CLI.
        let state_home = dir.path().join("state");
        let config_home = dir.path().join("config");
        for d in [&state_home, &config_home] {
            std::fs::create_dir(d).expect("a directory in the harness's ShortDir");
        }
        let mut env = Env::default();
        env.vars
            .insert("XDG_STATE_HOME".into(), state_home.display().to_string());
        env.vars
            .insert("XDG_CONFIG_HOME".into(), config_home.display().to_string());
        let paths = resolve(&env, Platform::Linux).expect("the harness's paths resolve");

        // TST-16: a command is given the socket's path and a marker token in its environment.
        let session = FakeSession::bind(dir.path(), "session", clock.clone());
        env.vars
            .insert(SOCKET_VAR.into(), session.path().display().to_string());
        env.vars
            .insert(TOKEN_VAR.into(), MARKER_SESSION_TOKEN.to_string());

        let bg_probe = Arc::new(BgProbe::default());
        clock.add_probe(bg_probe.clone());
        let shared = Arc::new(Shared {
            clock: clock.clone(),
            dir: dir.path().to_path_buf(),
            github,
            token,
            fs: Arc::new(FaultFs::new()),
            log: BufferSink::default(),
            paths,
            config: Mutex::new(opts.config),
            version: Mutex::new(opts.version),
            mode: Mutex::new(opts.mode),
            starts: AtomicU64::new(0),
            daemon: Mutex::new(None),
            outputs: Mutex::new(Vec::new()),
            raw: Mutex::new(Vec::new()),
            bg: Mutex::new(Vec::new()),
            bg_probe,
        });
        let spawner = spawn::starter(&shared);
        let h = Harness {
            shared,
            spawner,
            env,
            sessions: Mutex::new(vec![session]),
            hold: Mutex::new(hold),
            repos: AtomicUsize::new(0),
            finished: false,
            dir,
        };
        if !opts.no_daemon {
            h.launch().await;
        }
        h.shared.clock.settle().await;
        h
    }

    /// Starts a daemon with the harness's version and mode; panics, with the log, if it can't.
    async fn launch(&self) {
        let version = lock(&self.shared.version).clone();
        let mode = *lock(&self.shared.mode);
        if let Err(e) = self.shared.launch(version, mode).await {
            panic!(
                "the harness's daemon didn't start: {e}\n{}",
                self.shared.log.contents()
            );
        }
    }

    // ---- accessors ----

    pub fn clock(&self) -> &Arc<TestClock> {
        &self.shared.clock
    }

    pub fn github(&self) -> &FakeGitHub {
        &self.shared.github
    }

    pub fn token(&self) -> &Arc<FakeTokenSource> {
        &self.shared.token
    }

    pub fn fs(&self) -> &Arc<FaultFs> {
        &self.shared.fs
    }

    /// The daemon's log, every start's.
    pub fn log(&self) -> &BufferSink {
        &self.shared.log
    }

    /// `state.json` as a read would give it now.
    pub fn state_file(&self) -> Option<String> {
        self.shared
            .fs
            .file(&self.shared.paths.state_file)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    pub fn paths(&self) -> &Paths {
        &self.shared.paths
    }

    /// The harness's `ShortDir`, every command's working directory unless set.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// The `Env` every command gets by default.
    pub fn env(&self) -> Env {
        self.env.clone()
    }

    /// The fake session socket every command's environment names (TST-16).
    pub fn session(&self) -> Arc<FakeSession> {
        lock(&self.sessions)[0].clone()
    }

    /// Binds another session socket, `<name>.sock`, in the same directory; swept like the first.
    pub fn new_session(&self, name: &str) -> Arc<FakeSession> {
        let s = FakeSession::bind(self.dir.path(), name, self.shared.clock.clone());
        lock(&self.sessions).push(s.clone());
        s
    }

    /// The `hold_first` handle, once.
    pub fn take_hold(&self) -> Option<Hold> {
        lock(&self.hold).take()
    }

    /// The default spawner: it starts the in-process daemon (TST-5).
    pub fn spawner(&self) -> Arc<FakeSpawner> {
        self.spawner.clone()
    }

    /// How many daemons the default spawner has started.
    pub fn spawns(&self) -> usize {
        self.spawner.spawns()
    }

    // ---- the daemon ----

    /// The daemon's exit status, once it has stopped by itself or been stopped; `None` while it
    /// runs, after a crash, and with none started.
    pub fn exited(&self) -> Option<i32> {
        lock(&self.shared.daemon)
            .as_ref()
            .and_then(|(_, r)| r.exit_status())
    }

    /// Stops the daemon (OPS-7) and returns its exit status. Moves the clock only if its stop waits on it.
    pub async fn stop(&self) -> i32 {
        match self.shared.stop().await {
            Some(status) => status,
            None => panic!("Harness::stop: no daemon"),
        }
    }

    /// TST-5's crash: dropped without OPS-7's stop. The socket file and `state.json` stay.
    pub async fn crash(&self) {
        let taken = lock(&self.shared.daemon).take();
        let Some((_, r)) = taken else {
            panic!("Harness::crash: no daemon");
        };
        r.crash().await;
        self.shared.github.new_daemon();
        self.shared.clock.settle().await;
    }

    /// Stops the daemon if it runs, and starts another on the same directories.
    pub async fn restart(&self) {
        self.shared.stop().await;
        // The old lock goes with the old `Running`.
        let old = lock(&self.shared.daemon).take();
        drop(old);
        self.launch().await;
        self.shared.clock.settle().await;
    }

    /// The same with another config, kept for later restarts and spawns.
    pub async fn restart_with(&self, config: Config) {
        *lock(&self.shared.config) = config;
        self.restart().await;
    }

    /// The same with another version or mode, kept for later restarts.
    pub async fn restart_as(&self, version: &str, mode: Mode) {
        *lock(&self.shared.version) = version.to_string();
        *lock(&self.shared.mode) = mode;
        self.restart().await;
    }

    // ---- commands ----

    /// A command with the harness's defaults, to change before running it.
    pub fn cmd(&self, args: &[&str]) -> Cmd {
        Cmd::new(
            self.shared.clone(),
            args,
            self.env.clone(),
            self.spawner.clone(),
        )
    }

    /// Runs a command to its end, awaited.
    pub async fn cli(&self, args: &[&str]) -> Output {
        self.cmd(args).run().await
    }

    /// Runs a command in the background.
    pub fn cli_bg(&self, args: &[&str]) -> Bg {
        self.cmd(args).bg()
    }

    /// One request over the daemon's socket (TST-7): its status and body. The answer is swept.
    /// Awaited: a request the daemon holds on the clock hangs.
    pub async fn api_raw(
        &self,
        method: &str,
        path_and_query: &str,
        body: Option<&str>,
    ) -> (u16, String) {
        let answer = raw::request(&self.shared.paths.socket, method, path_and_query, body).await;
        match answer {
            Ok((status, body)) => {
                lock(&self.shared.raw).push(body.clone());
                (status, body)
            }
            Err(e) => panic!("Harness::api_raw {method} {path_and_query}: {e}"),
        }
    }

    /// A git repository in a new directory under the `ShortDir`: `branch` checked out, `origin`
    /// set (TST-5). `git` runs with no global or system config.
    pub async fn git_repo(&self, origin: &str, branch: &str) -> PathBuf {
        let n = self.repos.fetch_add(1, Ordering::SeqCst);
        let path = command::repo_dir(self.dir.path(), n);
        std::fs::create_dir(&path).expect("a directory for the repository");
        for args in [
            vec!["init", "-q"],
            vec!["checkout", "-q", "-b", branch],
            vec!["remote", "add", "origin", origin],
        ] {
            let status = tokio::process::Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(&args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            assert!(
                status.is_ok_and(|s| s.success()),
                "git {args:?} failed in {}",
                path.display()
            );
        }
        path
    }

    // ---- the sweep ----

    /// TST-11: every test ends here. Fails if a background command still runs. With a daemon running,
    /// fetches a `status`, a `list` (`state=all`, `checks=1`) and a `changes`, then stops it. Searches
    /// for the tokens and the session's token in the log, the state file, every `Output`, every
    /// `api_raw` answer and those three; for the GitHub markers and `MARKER_CHECK` in the log and the
    /// state file; reads every session socket's notices (a malformed line fails) and searches them for
    /// `MARKER_TITLE`, `MARKER_GITHUB_TEXT`, `MARKER_CHECK` and `MARKER_LOGIN` (NTF-8). Then the fake
    /// GitHub's `check()`.
    // TST-11: the sweep.
    pub async fn finish(mut self) {
        self.finished = true;
        let still: Vec<String> = lock(&self.shared.bg)
            .iter()
            .filter(|e| !e.done.load(Ordering::SeqCst))
            .map(|e| e.args.join(" "))
            .collect();
        if !still.is_empty() {
            panic!("finish: background commands still running: {still:?}");
        }

        let mut answers: Vec<(String, String)> = Vec::new();
        if self.shared.running() {
            let reqs = [
                Request::Status,
                Request::List {
                    all: true,
                    since: None,
                    checks: true,
                },
                Request::Changes {
                    pr: None,
                    since: None,
                    limit: None,
                },
            ];
            for req in reqs {
                let path = req.path_and_query();
                let body = req.body();
                let method = req.method();
                let fut = raw::request(
                    &self.shared.paths.socket,
                    method.as_str(),
                    &path,
                    body.as_deref(),
                );
                match drive(&self.shared.clock, "finish", fut).await {
                    Ok((_, body)) => answers.push((format!("the answer to {path}"), body)),
                    // OPS-8: a daemon may end by itself while the clock moves for an answer; one
                    // that drops a connection and runs on fails here, after 60 s of the clock.
                    Err(e) => {
                        if !self.shared.exit_by_itself().await {
                            panic!("finish: {path}: {e}");
                        }
                        break;
                    }
                }
            }
            self.shared.stop().await;
        }

        // TST-11: where the tokens may never be.
        let log = self.shared.log.contents();
        let state = self.state_file().unwrap_or_default();
        let mut everywhere = vec![
            ("the log".to_string(), log.clone()),
            ("the state file".to_string(), state.clone()),
        ];
        if let Ok(file) = std::fs::read_to_string(&self.shared.paths.log_file) {
            everywhere.push(("circitor.log".to_string(), file));
        }
        for (i, out) in lock(&self.shared.outputs).iter().enumerate() {
            everywhere.push((format!("output #{i}'s stdout"), out.stdout.clone()));
            everywhere.push((format!("output #{i}'s stderr"), out.stderr.clone()));
        }
        for (i, body) in lock(&self.shared.raw).iter().enumerate() {
            everywhere.push((format!("api_raw answer #{i}"), body.clone()));
        }
        everywhere.extend(answers);

        // NTF-8: no marker of what a GitHub user wrote is in a notice; a malformed line panics here.
        let mut notices = Vec::new();
        for s in lock(&self.sessions).iter() {
            for (i, n) in s.notices().into_iter().enumerate() {
                notices.push((format!("notice #{i} on {}", s.path().display()), n.text));
            }
        }

        let mut found = Vec::new();
        found.extend(search(
            &everywhere,
            &[MARKER_TOKEN, MARKER_TOKEN_2, MARKER_SESSION_TOKEN],
        ));
        // OPS-9, STA-6: a login may be in the log and the state file; nothing else GitHub wrote.
        found.extend(search(
            &everywhere[..2],
            &[MARKER_TITLE, MARKER_GITHUB_TEXT, MARKER_CHECK],
        ));
        found.extend(search(
            &notices,
            &[
                MARKER_TOKEN,
                MARKER_TOKEN_2,
                MARKER_SESSION_TOKEN,
                MARKER_TITLE,
                MARKER_GITHUB_TEXT,
                MARKER_CHECK,
                MARKER_LOGIN,
            ],
        ));
        if !found.is_empty() {
            panic!("TST-11: the sweep found markers:\n{}", found.join("\n"));
        }
        self.shared.github.check();
    }
}

/// Each marker found in each place, as one line naming both.
fn search(places: &[(String, String)], markers: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for (what, text) in places {
        for m in markers {
            if text.contains(m) {
                found.push(format!("{m} in {what}"));
            }
        }
    }
    found
}

impl Drop for Harness {
    fn drop(&mut self) {
        // TST-11: the sweep is never skipped, but a test that already fails isn't failed twice.
        if !self.finished && !std::thread::panicking() {
            panic!("a Harness was dropped without finish()");
        }
    }
}
