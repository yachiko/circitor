use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::stub::{Act, Stub};
use super::*;
use crate::cli::fake::{FakeGit, FakeSpawner};
use crate::cli::{Branch, Interrupt, Spawner};
use crate::clock::{Clock, TestClock, Time};
use crate::config::{Env, Platform};
use crate::testkit::ShortDir;

const CLI_VERSION: &str = "0.2.0+bbb";
/// A process ID that isn't this one: signals to it are only recorded.
const OTHER_PID: u32 = 4_000_001;

struct Fix {
    dir: ShortDir,
    paths: Paths,
    clock: Arc<TestClock>,
}

fn fix() -> Fix {
    let dir = ShortDir::new();
    let state_dir = dir.path().join("st");
    std::fs::create_dir(&state_dir).unwrap();
    let paths = Paths {
        config_file: dir.path().join("config.toml"),
        state_file: state_dir.join("state.json"),
        lock_file: state_dir.join("circitor.lock"),
        log_file: state_dir.join("circitor.log"),
        state_dir,
        socket: dir.path().join("s.sock"),
        socket_dir: None,
    };
    let clock = TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap());
    Fix { dir, paths, clock }
}

impl Fix {
    fn env(&self, spawner: Arc<dyn Spawner>) -> CliEnv {
        CliEnv {
            clock: self.clock.clone(),
            tz: chrono::FixedOffset::east_opt(0).unwrap(),
            cwd: self.dir.path().to_path_buf(),
            env: Env::default(),
            platform: Platform::Linux,
            version: CLI_VERSION.into(),
            spawner,
            git: FakeGit::new(None, Branch::NoRepo),
            interrupt: Interrupt::never(),
            request_limit: Duration::from_secs(90),
        }
    }

    fn stub(&self, path: &Path, version: &str, mode: &str) -> Stub {
        let s = Stub::start(path, version, mode);
        self.clock.add_probe(s.probe());
        s
    }

    /// A stub on the CLI's own socket that holds the lock as another process.
    fn daemon(&self, version: &str, mode: &str) -> Stub {
        let s = self.stub(&self.paths.socket, version, mode);
        s.hold_lock(&self.paths.lock_file, OTHER_PID);
        s
    }

    fn secs(&self) -> f64 {
        self.clock.mono().as_duration().as_secs_f64()
    }

    /// Advances in 100 ms steps until `h` is done, or `max` has passed.
    async fn until_done<T>(&self, h: &tokio::task::JoinHandle<T>, max: Duration) {
        let end = self.clock.mono().plus(max);
        while !h.is_finished() && self.clock.mono() < end {
            self.clock.advance(Duration::from_millis(100)).await;
        }
    }

    /// Advances in 100 ms steps to `secs` since the start.
    async fn to(&self, secs: f64) {
        let target = Duration::from_secs_f64(secs);
        while self.clock.mono().as_duration() + Duration::from_millis(100) <= target {
            self.clock.advance(Duration::from_millis(100)).await;
        }
        let rest = target.saturating_sub(self.clock.mono().as_duration());
        if !rest.is_zero() {
            self.clock.advance(rest).await;
        }
    }
}

/// A child that runs and never exits.
struct Running;
impl Spawned for Running {
    fn exited(&mut self) -> Option<i32> {
        None
    }
}

struct Exited(i32);
impl Spawned for Exited {
    fn exited(&mut self) -> Option<i32> {
        Some(self.0)
    }
}

type Slot = Arc<Mutex<Option<Stub>>>;

/// A spawner whose child binds a stub of `version` on `path` at once.
fn binds(f: &Fix, version: &'static str) -> (Arc<FakeSpawner>, Slot) {
    let slot: Slot = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    let path = f.paths.socket.clone();
    let clock = f.clock.clone();
    let spawner = FakeSpawner::starts(move |_log, _env| {
        let stub = Stub::start(&path, version, "spawned");
        clock.add_probe(stub.probe());
        *s2.lock().unwrap() = Some(stub);
        Box::new(Running)
    });
    (spawner, slot)
}

type Ensured = (Result<Link, ConnError>, String);

fn spawn_ensure(
    env: &CliEnv,
    paths: &Paths,
    no_spawn: bool,
    budget: Option<Duration>,
) -> tokio::task::JoinHandle<Ensured> {
    let (env, paths) = (env.clone(), paths.clone());
    tokio::spawn(async move {
        let mut warn = String::new();
        let r = ensure(&env, &paths, no_spawn, budget, &mut warn).await;
        (r, warn)
    })
}

fn err(r: Result<Link, ConnError>) -> ConnError {
    match r {
        Ok(l) => panic!("expected an error, got a link to {}", l.socket().display()),
        Err(e) => e,
    }
}

// CLI-7: the CLI's own socket path answers: no warning.
#[tokio::test]
async fn cli7_first_path_answers() {
    let f = fix();
    let _d = f.daemon(CLI_VERSION, "spawned");
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert_eq!(link.socket(), f.paths.socket);
    assert!(!link.older_unreplaced());
    assert_eq!(warn, "");
    assert_eq!(spawner.spawns(), 0);
}

// OPS-4: the CLI also tries the socket path in `circitor.lock`, with a warning naming both.
#[tokio::test]
async fn ops4_lock_file_socket_answers_with_a_warning() {
    let f = fix();
    let other = f.dir.path().join("o s.sock");
    let d = f.stub(&other, CLI_VERSION, "spawned");
    d.hold_lock(&f.paths.lock_file, OTHER_PID);
    let env = f.env(FakeSpawner::never_binds());
    let mut warn = String::new();
    let link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert_eq!(link.socket(), d.path());
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert!(
        warn.contains(&f.paths.socket.display().to_string()),
        "{warn}"
    );
    assert!(warn.contains(&other.display().to_string()), "{warn}");
    // `existing` connects the same way.
    let mut warn = String::new();
    let link = existing(&env, &f.paths, &mut warn).await.unwrap().unwrap();
    assert_eq!(link.socket(), other);
    assert!(warn.contains(&other.display().to_string()), "{warn}");
}

// U9, CLI-7: no daemon on both paths starts one, once.
#[tokio::test]
async fn cli7_no_daemon_spawns_once() {
    let f = fix();
    let (spawner, slot) = binds(&f, CLI_VERSION);
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let mut link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert_eq!(spawner.spawns(), 1);
    assert_eq!(link.socket(), f.paths.socket);
    assert_eq!(warn, "");
    link.request(&env, &Request::Refresh).await.unwrap();
    assert_eq!(spawner.spawns(), 1);
    let stub = slot.lock().unwrap().take().unwrap();
    assert_eq!(stub.count("POST /v1/refresh"), 1);
}

// CLI-7: `--no-spawn` with no daemon is exit 6, and nothing starts.
#[tokio::test]
async fn cli7_no_spawn_is_exit_6() {
    let f = fix();
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let e = err(ensure(&env, &f.paths, true, None, &mut warn).await);
    assert_eq!(e.code, 6);
    assert!(e.stderr.contains("--no-spawn"), "{}", e.stderr);
    assert!(e.stderr.ends_with('\n'));
    assert_eq!(spawner.spawns(), 0);
}

// CLI-7: only a missing socket or a refused connection is "no daemon": one that never answers starts nothing.
#[tokio::test]
async fn cli7_not_answering_starts_nothing() {
    let f = fix();
    let d = f.daemon(CLI_VERSION, "spawned");
    d.otherwise(Act::Stall);
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let h = spawn_ensure(&env, &f.paths, false, None);
    f.clock.settle().await;
    f.to(1.9).await;
    assert!(!h.is_finished(), "ended before OPS-4's 2 s");
    f.to(2.0).await;
    let (r, _) = h.await.unwrap();
    let e = err(r);
    assert_eq!(e.code, 6);
    assert!(
        e.stderr.contains(&f.paths.socket.display().to_string()),
        "{}",
        e.stderr
    );
    assert!(
        e.stderr.contains("daemon not answering on "),
        "{}",
        e.stderr
    );
    assert_eq!(spawner.spawns(), 0);
}

// CLI-7: a denied permission names the two sandbox settings and starts nothing.
#[tokio::test]
async fn cli7_denied_names_the_sandbox_keys() {
    if nix::unistd::geteuid().is_root() {
        return;
    }
    let f = fix();
    let closed = f.dir.path().join("c");
    std::fs::create_dir(&closed).unwrap();
    let mut paths = f.paths.clone();
    paths.socket = closed.join("s.sock");
    let _d = Stub::start(&paths.socket, CLI_VERSION, "spawned");
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).unwrap();
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let r = ensure(&env, &paths, false, None, &mut warn).await;
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o700)).unwrap();
    let e = err(r);
    assert_eq!(e.code, 6);
    assert!(
        e.stderr.contains("sandbox.excludedCommands"),
        "{}",
        e.stderr
    );
    assert!(
        e.stderr.contains("sandbox.network.allowUnixSockets"),
        "{}",
        e.stderr
    );
    assert_eq!(spawner.spawns(), 0);
}

// CLI-7: before starting a daemon the state directory, and the socket's own, are made 0700.
#[tokio::test]
async fn cli7_state_dir_is_0700() {
    let f = fix();
    let mut paths = f.paths.clone();
    let state_dir = f.dir.path().join("new-state");
    paths.state_dir = state_dir.clone();
    paths.lock_file = state_dir.join("circitor.lock");
    paths.log_file = state_dir.join("circitor.log");
    paths.state_file = state_dir.join("state.json");
    paths.socket_dir = Some(f.dir.path().join("run"));
    paths.socket = f.dir.path().join("run").join("s.sock");
    let slot: Slot = Arc::new(Mutex::new(None));
    let (s2, sock, clock) = (slot.clone(), paths.socket.clone(), f.clock.clone());
    let spawner = FakeSpawner::starts(move |_log, _env| {
        let stub = Stub::start(&sock, CLI_VERSION, "spawned");
        clock.add_probe(stub.probe());
        *s2.lock().unwrap() = Some(stub);
        Box::new(Running)
    });
    let env = f.env(spawner);
    let mut warn = String::new();
    ensure(&env, &paths, false, None, &mut warn).await.unwrap();
    for dir in [&state_dir, &f.dir.path().join("run")] {
        let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{}", dir.display());
    }
    assert!(paths.log_file.exists());

    // A state directory that can't be made is exit 6 at once, naming it.
    let mut bad = f.paths.clone();
    std::fs::write(f.dir.path().join("file"), "x").unwrap();
    bad.state_dir = f.dir.path().join("file").join("st");
    bad.socket = f.dir.path().join("none.sock");
    bad.lock_file = bad.state_dir.join("circitor.lock");
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let e = err(ensure(&env, &bad, false, None, &mut warn).await);
    assert_eq!(e.code, 6);
    assert!(
        e.stderr.contains(&bad.state_dir.display().to_string()),
        "{}",
        e.stderr
    );
    assert_eq!(spawner.spawns(), 0);
}

// OPS-9: a log over 5 000 000 bytes is renamed over `circitor.log.1`; 5 000 000 stays.
#[tokio::test]
async fn ops9_rotates_over_5_000_000_bytes() {
    let f = fix();
    let old = f.paths.state_dir.join("circitor.log.1");
    std::fs::write(&old, "older\n").unwrap();
    std::fs::write(&f.paths.log_file, vec![b'x'; 5_000_001]).unwrap();
    let (spawner, slot) = binds(&f, CLI_VERSION);
    let env = f.env(spawner);
    let mut warn = String::new();
    ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert_eq!(std::fs::metadata(&old).unwrap().len(), 5_000_001);
    assert_eq!(std::fs::metadata(&f.paths.log_file).unwrap().len(), 0);
    drop(slot.lock().unwrap().take());

    let f = fix();
    std::fs::write(&f.paths.log_file, vec![b'x'; 5_000_000]).unwrap();
    let (spawner, _slot) = binds(&f, CLI_VERSION);
    let env = f.env(spawner);
    ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&f.paths.log_file).unwrap().len(),
        5_000_000
    );
    assert!(!f.paths.state_dir.join("circitor.log.1").exists());
}

// CLI-7: a child that exits with an error ends the wait at once, with the lines it wrote.
#[tokio::test]
async fn cli7_child_exits_1() {
    let f = fix();
    let spawner = FakeSpawner::exits(1, Some("config.toml: interval: must be at least 10"));
    let env = f.env(spawner.clone());
    let h = spawn_ensure(&env, &f.paths, false, None);
    f.until_done(&h, Duration::from_secs(1)).await;
    assert!(h.is_finished());
    assert!(f.secs() < 1.0);
    let e = err(h.await.unwrap().0);
    assert_eq!(e.code, 6);
    assert!(e.stderr.contains("status 1"), "{}", e.stderr);
    assert!(
        e.stderr
            .lines()
            .any(|l| l == "config.toml: interval: must be at least 10"),
        "{}",
        e.stderr
    );

    // 25 lines after the spawn: the last 20, and none from before.
    let f = fix();
    std::fs::write(&f.paths.log_file, "before 1\nbefore 2\n").unwrap();
    let spawner = FakeSpawner::starts(|mut log, _env| {
        use std::io::Write;
        for i in 1..=25 {
            writeln!(log, "line {i}").unwrap();
        }
        Box::new(Exited(1))
    });
    let env = f.env(spawner);
    let mut warn = String::new();
    let e = err(ensure(&env, &f.paths, false, None, &mut warn).await);
    let lines: Vec<&str> = e.stderr.lines().collect();
    assert_eq!(lines.len(), 21, "{}", e.stderr);
    let want: Vec<String> = (6..=25).map(|i| format!("line {i}")).collect();
    assert_eq!(
        &lines[1..],
        want.iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(!e.stderr.contains("before 1") && !e.stderr.contains("before 2"));
}

// CLI-7: a child that exits 0 lost the lock to another daemon: the CLI waits for the lock file's socket.
#[tokio::test]
async fn cli7_child_exits_0_waits_for_the_holder() {
    let f = fix();
    let spawner = FakeSpawner::exits(0, None);
    let env = f.env(spawner.clone());
    let h = spawn_ensure(&env, &f.paths, false, None);
    f.clock.settle().await;
    f.to(1.0).await;
    assert!(!h.is_finished());
    let other = f.dir.path().join("h.sock");
    let d = f.stub(&other, CLI_VERSION, "spawned");
    d.hold_lock(&f.paths.lock_file, OTHER_PID);
    f.until_done(&h, Duration::from_secs(2)).await;
    let (r, warn) = h.await.unwrap();
    let link = r.unwrap();
    assert_eq!(link.socket(), other);
    assert!(f.secs() < 1.5);
    assert!(warn.contains(&other.display().to_string()), "{warn}");
    assert_eq!(spawner.spawns(), 1);
}

// CLI-7: a child that never binds: still waiting at 7.9 s, exit 6 at 8 s.
#[tokio::test]
async fn cli7_never_binds() {
    let f = fix();
    let env = f.env(FakeSpawner::never_binds());
    let h = spawn_ensure(&env, &f.paths, false, None);
    f.clock.settle().await;
    f.to(7.9).await;
    assert!(!h.is_finished());
    f.to(8.0).await;
    let e = err(h.await.unwrap().0);
    assert_eq!(e.code, 6);
    assert!(e.stderr.contains("within 8s"), "{}", e.stderr);
}

// CLI-6: a wait's budget caps the wait for a start.
#[tokio::test]
async fn cli6_budget_caps_the_wait() {
    let f = fix();
    let env = f.env(FakeSpawner::never_binds());
    let h = spawn_ensure(&env, &f.paths, false, Some(Duration::from_secs(3)));
    f.clock.settle().await;
    f.to(2.9).await;
    assert!(!h.is_finished());
    f.to(3.0).await;
    let e = err(h.await.unwrap().0);
    assert_eq!(e.code, 6);
}

// CLI-9: an older spawned daemon is shut down and replaced; the next request goes to the new one.
#[tokio::test]
async fn cli9_older_spawned_is_replaced() {
    let f = fix();
    let old = f.daemon("0.1.0+aaa", "spawned");
    let (spawner, slot) = binds(&f, CLI_VERSION);
    let env = f.env(spawner.clone());
    let h = spawn_ensure(&env, &f.paths, false, None);
    f.until_done(&h, Duration::from_secs(10)).await;
    let (r, warn) = h.await.unwrap();
    let mut link = r.unwrap();
    assert_eq!(warn, "");
    assert!(!link.older_unreplaced());
    assert_eq!(old.count("POST /v1/shutdown"), 1);
    assert_eq!(old.count("GET /v1/status"), 1);
    assert_eq!(old.requests().len(), 2);
    assert_eq!(spawner.spawns(), 1);
    link.request(&env, &Request::Refresh).await.unwrap();
    let new = slot.lock().unwrap().take().unwrap();
    assert_eq!(new.count("POST /v1/refresh"), 1);
    assert_eq!(old.count("POST /v1/refresh"), 0);
}

// CLI-9: versions compare by semver precedence: build metadata is ignored.
#[tokio::test]
async fn cli9_build_metadata_is_ignored() {
    let f = fix();
    let old = f.daemon("0.2.0+aaa", "spawned");
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert_eq!(warn, "");
    assert!(!link.older_unreplaced());
    assert_eq!(old.count("POST /v1/shutdown"), 0);
    assert_eq!(spawner.spawns(), 0);
}

async fn kept(version: &str, mode: &str) -> (Fix, Stub, Link, String, usize) {
    let f = fix();
    let d = f.daemon(version, mode);
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert_eq!(d.count("POST /v1/shutdown"), 0);
    let spawns = spawner.spawns();
    (f, d, link, warn, spawns)
}

// CLI-9: a newer daemon is never replaced: one warning.
#[tokio::test]
async fn cli9_newer_is_kept() {
    let (_f, _d, link, warn, spawns) = kept("0.3.0+ccc", "spawned").await;
    assert!(!link.older_unreplaced());
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert!(warn.contains("newer"), "{warn}");
    assert_eq!(spawns, 0);
}

// CLI-9: a foreground daemon is never replaced; an older one is flagged for CLI-9's hint.
#[tokio::test]
async fn cli9_foreground_is_kept() {
    let (_f, _d, link, warn, spawns) = kept("0.1.0+aaa", "foreground").await;
    assert!(link.older_unreplaced());
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert!(warn.contains("foreground"), "{warn}");
    assert_eq!(spawns, 0);
    // A mode that is neither word is kept too.
    let (_f, _d, link, warn, _) = kept("0.1.0+aaa", "other").await;
    assert!(link.older_unreplaced());
    assert_eq!(warn.lines().count(), 1, "{warn}");
}

// CLI-9: a version that doesn't parse is never replaced, with a warning.
#[tokio::test]
async fn cli9_unparsable_is_kept() {
    let (_f, _d, link, warn, spawns) = kept("not-a-version", "spawned").await;
    assert!(!link.older_unreplaced());
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert!(warn.contains("not-a-version"), "{warn}");
    assert_eq!(spawns, 0);
}

// CLI-9: a config that fails keeps the old daemon, naming the key; no shutdown is sent.
#[tokio::test]
async fn cli9_bad_config_keeps_it() {
    let f = fix();
    std::fs::write(&f.paths.config_file, "bogus_key = 1\n").unwrap();
    let d = f.daemon("0.1.0+aaa", "spawned");
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    assert!(link.older_unreplaced());
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert!(warn.contains("bogus_key"), "{warn}");
    assert_eq!(d.count("POST /v1/shutdown"), 0);
    assert_eq!(spawner.spawns(), 0);
}

// CLI-9: a lock still held 8 s after the shutdown keeps the old daemon, with a warning.
#[tokio::test]
async fn cli9_lock_held_after_8s_keeps_it() {
    let f = fix();
    let d = f.daemon("0.1.0+aaa", "spawned");
    d.keep_lock_on_shutdown();
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let h = spawn_ensure(&env, &f.paths, false, None);
    f.clock.settle().await;
    f.to(7.9).await;
    assert!(!h.is_finished());
    f.to(8.0).await;
    let (r, warn) = h.await.unwrap();
    let link = r.unwrap();
    assert!(link.older_unreplaced());
    assert_eq!(d.count("POST /v1/shutdown"), 1);
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert!(warn.contains("still held"), "{warn}");
    assert_eq!(spawner.spawns(), 0);
}

// CLI-9: at most one replacement per command: an older daemon after it is kept.
#[tokio::test]
async fn cli9_replaces_once() {
    let f = fix();
    let old = f.daemon("0.1.0+aaa", "spawned");
    let (spawner, slot) = binds(&f, "0.1.0+ccc");
    let env = f.env(spawner.clone());
    let h = spawn_ensure(&env, &f.paths, false, None);
    f.until_done(&h, Duration::from_secs(10)).await;
    let (r, warn) = h.await.unwrap();
    let link = r.unwrap();
    assert!(link.older_unreplaced());
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert!(warn.contains("already replaced"), "{warn}");
    assert_eq!(spawner.spawns(), 1);
    assert_eq!(old.count("POST /v1/shutdown"), 1);
    let new = slot.lock().unwrap().take().unwrap();
    assert_eq!(new.count("POST /v1/shutdown"), 0);
}

// CLI-7: `--no-spawn` uses an older daemon, with a warning, and replaces nothing.
#[tokio::test]
async fn cli7_no_spawn_keeps_an_older_daemon() {
    let f = fix();
    let d = f.daemon("0.1.0+aaa", "spawned");
    let env = f.env(FakeSpawner::never_binds());
    let mut warn = String::new();
    let link = ensure(&env, &f.paths, true, None, &mut warn).await.unwrap();
    assert!(link.older_unreplaced());
    assert_eq!(warn.lines().count(), 1, "{warn}");
    assert_eq!(d.count("POST /v1/shutdown"), 0);
}

// CLI-7: any request but a wait has a limit: past it, exit 6 naming the socket.
#[tokio::test]
async fn cli7_request_limit_90s() {
    let f = fix();
    let d = f.daemon(CLI_VERSION, "spawned");
    let env = f.env(FakeSpawner::never_binds());
    let mut warn = String::new();
    let mut link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    d.otherwise(Act::Stall);
    let env2 = env.clone();
    let h = tokio::spawn(async move { link.request(&env2, &Request::Refresh).await });
    f.clock.settle().await;
    f.clock.advance(Duration::from_millis(89_900)).await;
    assert!(!h.is_finished());
    f.clock.advance(Duration::from_millis(100)).await;
    let e = h.await.unwrap().unwrap_err();
    assert_eq!(e.code, 6);
    assert_eq!(
        e.stderr,
        format!("daemon not answering on {}\n", f.paths.socket.display())
    );
}

// CLI-10: a drop is tried once more as a new command; a second drop is exit 6.
#[tokio::test]
async fn cli10_one_retry() {
    let f = fix();
    let d = f.daemon(CLI_VERSION, "spawned");
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let mut link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    // The refresh drops; the retry's status probe and refresh are answered.
    d.script([Act::Close]);
    let reply = link.request(&env, &Request::Refresh).await.unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(d.count("POST /v1/refresh"), 2);

    // Two drops.
    d.script([Act::Close, Act::Answer, Act::Close]);
    let e = link.request(&env, &Request::Refresh).await.unwrap_err();
    assert_eq!(e.code, 6);
    assert_eq!(d.count("POST /v1/refresh"), 4);
    assert_eq!(spawner.spawns(), 0);
}

// CLI-10: a retry with no daemon left starts one, when the command may.
#[tokio::test]
async fn cli10_retry_may_start_a_daemon() {
    let f = fix();
    let d = f.daemon(CLI_VERSION, "spawned");
    let (spawner, slot) = binds(&f, CLI_VERSION);
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    let mut link = ensure(&env, &f.paths, false, None, &mut warn)
        .await
        .unwrap();
    d.script([Act::Close]);
    d.close();
    d.release_lock();
    link.request(&env, &Request::Refresh).await.unwrap();
    assert_eq!(spawner.spawns(), 1);
    let new = slot.lock().unwrap().take().unwrap();
    assert_eq!(new.count("POST /v1/refresh"), 1);
}

// CLI-10: a wait that had its cursor waits up to 8 s for a daemon, within its budget, and starts none.
#[tokio::test]
async fn cli10_reconnect() {
    let f = fix();
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let (e2, p2) = (env.clone(), f.paths.clone());
    let h = tokio::spawn(async move { reconnect(&e2, &p2, Duration::from_secs(30)).await });
    f.clock.settle().await;
    f.to(3.0).await;
    assert!(!h.is_finished());
    let _d = f.daemon(CLI_VERSION, "spawned");
    f.until_done(&h, Duration::from_secs(1)).await;
    let link = h.await.unwrap().unwrap();
    assert_eq!(link.socket(), f.paths.socket);
    assert!(f.secs() <= 3.2, "{}", f.secs());

    for (budget, end) in [(30.0, 8.0), (2.0, 2.0)] {
        let f = fix();
        let env = f.env(spawner.clone());
        let p2 = f.paths.clone();
        let h = tokio::spawn(
            async move { reconnect(&env, &p2, Duration::from_secs_f64(budget)).await },
        );
        f.clock.settle().await;
        f.to(end - 0.1).await;
        assert!(!h.is_finished());
        f.to(end).await;
        let e = h.await.unwrap().unwrap_err();
        assert_eq!(e.code, 6);
    }
    assert_eq!(spawner.spawns(), 0);
}

// CLI-8: `existing` connects only.
#[tokio::test]
async fn cli8_existing_none() {
    let f = fix();
    let spawner = FakeSpawner::never_binds();
    let env = f.env(spawner.clone());
    let mut warn = String::new();
    assert!(existing(&env, &f.paths, &mut warn).await.unwrap().is_none());
    assert_eq!(spawner.spawns(), 0);
    // An older daemon is used as it is.
    let d = f.daemon("0.1.0", "spawned");
    let link = existing(&env, &f.paths, &mut warn).await.unwrap().unwrap();
    assert!(link.older_unreplaced());
    assert_eq!(d.count("POST /v1/shutdown"), 0);
    assert_eq!(warn, "");
}

type Signals = Arc<Mutex<Vec<(f64, i32, Signal)>>>;

fn spawn_stop(
    f: &Fix,
    env: &CliEnv,
) -> (tokio::task::JoinHandle<Result<bool, ConnError>>, Signals) {
    let signals: Signals = Arc::new(Mutex::new(Vec::new()));
    let (s2, clock) = (signals.clone(), f.clock.clone());
    let (env, paths) = (env.clone(), f.paths.clone());
    let h = tokio::spawn(async move {
        let record = move |pid: i32, sig: Signal| {
            let at = clock.mono().as_duration().as_secs_f64();
            s2.lock().unwrap().push((at, pid, sig));
        };
        let mut warn = String::new();
        stop_with(&env, &paths, &mut warn, &record).await
    });
    (h, signals)
}

// CLI-8: no daemon on both paths: `Ok(false)`, nothing signalled.
#[tokio::test]
async fn cli8_stop_not_running() {
    let f = fix();
    let env = f.env(FakeSpawner::never_binds());
    let (h, signals) = spawn_stop(&f, &env);
    assert!(!h.await.unwrap().unwrap());
    assert!(signals.lock().unwrap().is_empty());
}

// CLI-8: `stop` returns once the lock is released, with no signal.
#[tokio::test]
async fn cli8_stop_waits_for_the_lock() {
    let f = fix();
    let d = f.daemon(CLI_VERSION, "spawned");
    d.keep_lock_on_shutdown();
    let env = f.env(FakeSpawner::never_binds());
    let (h, signals) = spawn_stop(&f, &env);
    f.clock.settle().await;
    f.to(3.0).await;
    assert!(!h.is_finished());
    d.release_lock();
    f.until_done(&h, Duration::from_secs(1)).await;
    assert!(h.await.unwrap().unwrap());
    assert!(f.secs() <= 3.2);
    assert!(signals.lock().unwrap().is_empty());
    // API-2: the shutdown is the first request; no status is read before it.
    assert_eq!(d.requests(), vec!["POST /v1/shutdown".to_string()]);
}

// CLI-8: a daemon that doesn't answer gets `SIGTERM`, then `SIGKILL` 5 s later, only while the lock is held.
#[tokio::test]
async fn cli8_stop_signals() {
    let f = fix();
    let d = f.daemon(CLI_VERSION, "spawned");
    d.otherwise(Act::Stall);
    let mut env = f.env(FakeSpawner::never_binds());
    env.request_limit = Duration::from_secs(2);
    let (h, signals) = spawn_stop(&f, &env);
    f.clock.settle().await;
    f.to(1.9).await;
    assert!(signals.lock().unwrap().is_empty());
    f.to(2.0).await;
    assert_eq!(
        *signals.lock().unwrap(),
        vec![(2.0, OTHER_PID as i32, Signal::SIGTERM)]
    );
    f.to(6.9).await;
    assert_eq!(signals.lock().unwrap().len(), 1);
    f.to(7.0).await;
    assert_eq!(
        signals.lock().unwrap()[1],
        (7.0, OTHER_PID as i32, Signal::SIGKILL)
    );
    // Still held 2 s after `SIGKILL`: exit 6, naming the process.
    f.to(9.0).await;
    let e = h.await.unwrap().unwrap_err();
    assert_eq!(e.code, 6);
    assert!(
        e.stderr.contains(&format!("process {OTHER_PID}")),
        "{}",
        e.stderr
    );

    // Released between the two: no `SIGKILL`.
    let f = fix();
    let d = f.daemon(CLI_VERSION, "spawned");
    d.otherwise(Act::Stall);
    let mut env = f.env(FakeSpawner::never_binds());
    env.request_limit = Duration::from_secs(2);
    let (h, signals) = spawn_stop(&f, &env);
    f.clock.settle().await;
    f.to(4.0).await;
    d.release_lock();
    f.until_done(&h, Duration::from_secs(6)).await;
    assert!(h.await.unwrap().unwrap());
    let sent = signals.lock().unwrap().clone();
    assert_eq!(sent, vec![(2.0, OTHER_PID as i32, Signal::SIGTERM)]);

    // Never to this process.
    let f = fix();
    let d = f.stub(&f.paths.socket, CLI_VERSION, "spawned");
    d.hold_lock(&f.paths.lock_file, std::process::id());
    d.otherwise(Act::Stall);
    let mut env = f.env(FakeSpawner::never_binds());
    env.request_limit = Duration::from_secs(2);
    let (h, signals) = spawn_stop(&f, &env);
    f.clock.settle().await;
    f.until_done(&h, Duration::from_secs(12)).await;
    assert_eq!(h.await.unwrap().unwrap_err().code, 6);
    assert!(signals.lock().unwrap().is_empty());
}

// OPS-5, NTF-9: the spawned daemon's environment has no token and no session; gh's settings stay.
#[test]
fn ops5_spawned_env() {
    let mut env = Env::default();
    for (k, v) in [
        ("GH_TOKEN", crate::testkit::MARKER_TOKEN),
        ("GITHUB_TOKEN", crate::testkit::MARKER_TOKEN_2),
        ("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/session.sock"),
        (
            "CLAUDE_CODE_MESSAGING_TOKEN",
            crate::testkit::MARKER_SESSION_TOKEN,
        ),
        ("GH_CONFIG_DIR", "/home/u/.config/gh"),
        ("XDG_CONFIG_HOME", "/home/u/.config"),
        ("PATH", "/usr/bin"),
    ] {
        env.vars.insert(k.into(), v.into());
    }
    let log = tempfile::tempfile().unwrap();
    let cmd = spawn::daemon_command(Path::new("/opt/circitor"), &env, &log).unwrap();
    let std = cmd.as_std();
    assert_eq!(std.get_program(), "/opt/circitor");
    let args: Vec<_> = std.get_args().collect();
    assert_eq!(args, ["daemon", "--spawned"]);
    assert_eq!(std.get_current_dir(), Some(Path::new("/")));
    let envs: std::collections::BTreeMap<String, Option<String>> = std
        .get_envs()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();
    for gone in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "CLAUDE_CODE_MESSAGING_SOCKET",
        "CLAUDE_CODE_MESSAGING_TOKEN",
    ] {
        assert!(!envs.contains_key(gone), "{gone} is passed on");
    }
    assert_eq!(
        envs.get("GH_CONFIG_DIR"),
        Some(&Some("/home/u/.config/gh".to_string()))
    );
    assert_eq!(
        envs.get("XDG_CONFIG_HOME"),
        Some(&Some("/home/u/.config".to_string()))
    );
    assert_eq!(envs.get("PATH"), Some(&Some("/usr/bin".to_string())));
}
