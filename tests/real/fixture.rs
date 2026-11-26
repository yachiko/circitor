//! The fixture every real-process test uses (TST-9's sub-bullet): one `ShortDir`, a cleared
//! environment, a `config.toml` naming a fake `gh`, the fake GitHub on real time, a 30 s limit, a
//! guard that stops every daemon the directory's lock named, and TST-11's sweep at the end.

use std::collections::BTreeMap;
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::clock::{self, Mono, SharedClock, SystemClock};
use circitor::daemon::instance::{is_held, read_holder};
use circitor::testkit::github::PrId;
use circitor::testkit::server::FakeGitHub;
use circitor::testkit::{
    MARKER_CHECK, MARKER_GITHUB_TEXT, MARKER_SESSION_TOKEN, MARKER_TITLE, MARKER_TOKEN,
    MARKER_TOKEN_2, ShortDir,
};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::process::{Child, Command};

/// TST-9: every test fails after 30 s.
pub const LIMIT: Duration = Duration::from_secs(30);
/// The trap's "poll a condition every 20 ms".
const POLL: Duration = Duration::from_millis(20);
/// The fake `gh`'s hang: an odd number, so the guard knows its own `sleep`.
const HANG_SECONDS: &str = "2999";

/// The built binary, a test build (TST-1).
pub fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_circitor"))
}

/// What the fake `gh` does, chosen by a file beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gh {
    /// As gh does: a non-empty `GH_TOKEN`, else `GITHUB_TOKEN`, else `MARKER_TOKEN` (docs/02 §1).
    Token,
    /// Exit 1 with `no oauth token found for github.com`.
    NoLogin,
    /// Exit 3.
    Exit3,
    /// Never exits; records its PID.
    Hang,
}

/// One finished process: its streams, its exit code (`None`: a signal ended it) and how long it ran.
#[derive(Clone, Debug)]
pub struct Out {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub took: Duration,
}

impl Out {
    pub fn text(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// A process the test started and runs in the background.
pub struct Bg {
    child: Child,
    started: Mono,
    what: String,
}

impl Bg {
    /// Its PID, while it runs.
    pub fn pid(&self) -> u32 {
        self.child.id().expect("a running process has a PID")
    }
}

pub struct Fixture {
    dir: ShortDir,
    pub gh: FakeGitHub,
    /// `o/r#1`, open, the fake login's own.
    pub pr: PrId,
    pub clock: SharedClock,
    env: BTreeMap<String, String>,
    /// Every daemon PID seen, and the fake `gh`'s.
    pids: Mutex<Vec<u32>>,
    /// Every stdout and stderr captured (TST-11).
    outputs: Mutex<Vec<(String, String)>>,
    /// Files a foreground daemon's stderr went to (TST-11).
    fg_logs: Mutex<Vec<PathBuf>>,
}

/// Runs `body` on a new fixture whose `config.toml` holds `config` (and `gh_path`), within 30 s,
/// then TST-11's sweep. The guard runs when the fixture is dropped, on failure too.
pub async fn real<F, Fut>(config: &str, body: F)
where
    F: FnOnce(Arc<Fixture>) -> Fut,
    Fut: Future<Output = ()>,
{
    let fx = Arc::new(Fixture::new(config));
    let clock = fx.clock.clone();
    let inner = fx.clone();
    let run = async move {
        body(inner.clone()).await;
        inner.finish();
    };
    // TST-9: the test fails by itself after 30 s.
    if clock::timeout(&clock, LIMIT, run).await.is_err() {
        panic!("the test ran over {} s", LIMIT.as_secs());
    }
    drop(fx);
}

impl Fixture {
    fn new(config: &str) -> Fixture {
        let dir = ShortDir::new();
        let root = dir.path().to_path_buf();
        let clock: SharedClock = SystemClock::new();
        // TST-9: the fake GitHub in the test process, on real time.
        let gh = FakeGitHub::start_real(clock.clone());
        for sub in ["home", "config/circitor", "gh", "bin"] {
            std::fs::create_dir_all(root.join(sub)).expect("the fixture's directories");
        }
        // The runtime directory as a login session has it: the user's own, 0700.
        let run = root.join("run");
        std::fs::create_dir(&run).expect("the runtime directory");
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700))
            .expect("the runtime directory's mode");

        let mut env = BTreeMap::new();
        for (k, v) in [
            ("HOME", root.join("home")),
            ("XDG_CONFIG_HOME", root.join("config")),
            ("XDG_STATE_HOME", root.join("state")),
            ("XDG_RUNTIME_DIR", run),
            ("GH_CONFIG_DIR", root.join("gh")),
        ] {
            env.insert(k.to_string(), v.to_string_lossy().into_owned());
        }
        env.insert("PATH".into(), "/usr/bin:/bin".into());
        env.insert("CIRCITOR_TEST_GITHUB".into(), gh.url());

        // The world every test names: `o/r#1`, open, the fake login's own.
        let pr = gh.world(|w| {
            let r = w.repo("o", "r");
            w.open_pr(r)
        });
        let fx = Fixture {
            dir,
            gh,
            pr,
            clock,
            env,
            pids: Mutex::new(Vec::new()),
            outputs: Mutex::new(Vec::new()),
            fg_logs: Mutex::new(Vec::new()),
        };
        fx.write_gh();
        fx.set_gh(Gh::Token);
        fx.config(config);
        fx
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    // ---- paths (OPS-3, OPS-4, on Linux) ----

    pub fn state_dir(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.root()
                .join("home/Library/Application Support/circitor")
        } else {
            self.root().join("state/circitor")
        }
    }
    pub fn runtime_dir(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.state_dir()
        } else {
            self.root().join("run/circitor")
        }
    }
    pub fn lock_file(&self) -> PathBuf {
        self.state_dir().join("circitor.lock")
    }
    pub fn log_file(&self) -> PathBuf {
        self.state_dir().join("circitor.log")
    }
    pub fn state_file(&self) -> PathBuf {
        self.state_dir().join("state.json")
    }
    pub fn socket(&self) -> PathBuf {
        self.runtime_dir().join("circitor.sock")
    }
    pub fn gh_path(&self) -> PathBuf {
        self.root().join("bin/gh")
    }
    fn gh_pid_file(&self) -> PathBuf {
        self.root().join("bin/gh.pid")
    }

    // ---- the config and the fake `gh` ----

    /// Replaces `config.toml`: `gh_path` and `body`.
    pub fn config(&self, body: &str) {
        let text = format!("gh_path = \"{}\"\n{body}", self.gh_path().display());
        std::fs::write(self.root().join("config/circitor/config.toml"), text).expect("config.toml");
    }

    /// The fake `gh` (TST-9, TST-11): a `/bin/sh` script, mode 0755, closed before it's run.
    fn write_gh(&self) {
        let bin = self.root().join("bin");
        let script = format!(
            r#"#!/bin/sh
# The fake gh (TST-9). It prints the marker on stderr, as a warning would be (TST-11).
echo "warning: {MARKER_TOKEN}" >&2
mode=$(cat "{bin}/gh-mode" 2>/dev/null)
case "$mode" in
nologin)
    echo "no oauth token found for github.com" >&2
    exit 1;;
exit3)
    exit 3;;
hang)
    echo $$ > "{bin}/gh.pid"
    exec sleep {HANG_SECONDS};;
esac
if [ -n "$GH_TOKEN" ]; then
    echo "$GH_TOKEN"
elif [ -n "$GITHUB_TOKEN" ]; then
    echo "$GITHUB_TOKEN"
else
    echo "{MARKER_TOKEN}"
fi
"#,
            bin = bin.display()
        );
        let path = self.gh_path();
        std::fs::write(&path, script).expect("the fake gh");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the fake gh's mode");
    }

    pub fn set_gh(&self, mode: Gh) {
        let word = match mode {
            Gh::Token => "token",
            Gh::NoLogin => "nologin",
            Gh::Exit3 => "exit3",
            Gh::Hang => "hang",
        };
        std::fs::write(self.root().join("bin/gh-mode"), word).expect("the fake gh's mode file");
    }

    /// The PID the hanging fake `gh` recorded.
    pub fn gh_pid(&self) -> Option<u32> {
        let text = std::fs::read_to_string(self.gh_pid_file()).ok()?;
        let pid = text.trim().parse().ok()?;
        self.remember(pid);
        Some(pid)
    }

    // ---- processes ----

    /// TST-9: every child of the test starts here, from a cleared environment: the fixture's
    /// variables, then `extra`.
    pub fn command(&self, args: &[&str], extra: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new(binary());
        cmd.args(args);
        cmd.env_clear();
        cmd.envs(&self.env);
        cmd.envs(extra.iter().copied());
        cmd.current_dir(self.root());
        cmd.stdin(Stdio::null());
        cmd.kill_on_drop(true);
        cmd
    }

    /// Runs the CLI to its end.
    pub async fn cli(&self, args: &[&str]) -> Out {
        self.cli_env(args, &[]).await
    }

    pub async fn cli_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Out {
        let mut cmd = self.command(args, extra);
        self.run_to_end(&mut cmd, &args.join(" ")).await
    }

    /// Runs `cmd` to its end with both streams captured.
    pub async fn run_to_end(&self, cmd: &mut Command, what: &str) -> Out {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let started = self.clock.mono();
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("{what}: can't start: {e}"));
        let out = child
            .wait_with_output()
            .await
            .unwrap_or_else(|e| panic!("{what}: can't wait: {e}"));
        let out = Out {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            code: out.status.code(),
            took: self.clock.mono().since(started),
        };
        self.record(&out);
        self.holder_pid();
        out
    }

    /// Starts the CLI in the background, both streams captured.
    pub fn cli_bg(&self, args: &[&str], extra: &[(&str, &str)]) -> Bg {
        let mut cmd = self.command(args, extra);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let started = self.clock.mono();
        let child = cmd.spawn().expect("the CLI starts");
        Bg {
            child,
            started,
            what: args.join(" "),
        }
    }

    /// Waits for a background process to end, at most `limit`.
    pub async fn finish_bg(&self, bg: Bg, limit: Duration) -> Out {
        let Bg {
            child,
            started,
            what,
        } = bg;
        let out = clock::timeout(&self.clock, limit, child.wait_with_output())
            .await
            .unwrap_or_else(|_| panic!("`{what}` didn't end within {limit:?}"))
            .unwrap_or_else(|e| panic!("{what}: can't wait: {e}"));
        let out = Out {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            code: out.status.code(),
            took: self.clock.mono().since(started),
        };
        self.record(&out);
        out
    }

    /// A foreground daemon (`circitor daemon`), its stderr to a file of the fixture's (swept).
    pub fn daemon_fg(&self, extra: &[(&str, &str)]) -> Child {
        let n = self.fg_logs.lock().unwrap().len();
        let path = self.root().join(format!("fg{n}.log"));
        let file = std::fs::File::create(&path).expect("the foreground daemon's log");
        self.fg_logs.lock().unwrap().push(path);
        self.daemon_fg_to(extra, Stdio::from(file))
    }

    /// A foreground daemon whose stderr is `stderr`; stdout goes nowhere.
    pub fn daemon_fg_to(&self, extra: &[(&str, &str)], stderr: Stdio) -> Child {
        let mut cmd = self.command(&["daemon"], extra);
        cmd.stdout(Stdio::null()).stderr(stderr);
        let child = cmd.spawn().expect("the foreground daemon starts");
        if let Some(pid) = child.id() {
            self.remember(pid);
        }
        child
    }

    /// What the `n`th foreground daemon wrote on stderr so far.
    pub fn fg_log(&self, n: usize) -> String {
        let path = self.fg_logs.lock().unwrap()[n].clone();
        std::fs::read_to_string(path).unwrap_or_default()
    }

    pub fn record(&self, out: &Out) {
        self.outputs
            .lock()
            .unwrap()
            .push((out.stdout.clone(), out.stderr.clone()));
    }

    fn remember(&self, pid: u32) {
        let mut pids = self.pids.lock().unwrap();
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }

    // ---- the daemon, from outside ----

    /// The PID `circitor.lock` names, if a complete line is there.
    pub fn holder_pid(&self) -> Option<u32> {
        let pid = read_holder(&self.lock_file())?.pid;
        self.remember(pid);
        Some(pid)
    }

    /// The holder's PID, which must be there.
    pub fn daemon_pid(&self) -> u32 {
        self.holder_pid()
            .unwrap_or_else(|| panic!("no holder in {}", self.lock_file().display()))
    }

    /// STA-11's test: taken means released.
    pub fn lock_held(&self) -> bool {
        is_held(&self.lock_file()).unwrap_or(false)
    }

    /// Polls `cond` every 20 ms until it holds, for at most `limit`; fails naming `what`.
    pub async fn until(&self, what: &str, limit: Duration, mut cond: impl FnMut() -> bool) {
        let start = self.clock.mono();
        loop {
            if cond() {
                return;
            }
            if self.clock.mono().since(start) >= limit {
                panic!("{what}: not within {limit:?}");
            }
            clock::sleep(&self.clock, POLL).await;
        }
    }

    /// After the lock was seen released (`stop`'s return, CLI-8): the process ends with it, and is
    /// reaped within 1 s.
    pub async fn gone(&self, pid: u32) {
        self.until(
            &format!("the daemon {pid} is gone"),
            Duration::from_secs(1),
            || !alive(pid),
        )
        .await;
    }

    /// Sleeps on real time; only for a negative check ("still alive after…").
    pub async fn sleep(&self, d: Duration) {
        clock::sleep(&self.clock, d).await;
    }

    pub fn mono(&self) -> Mono {
        self.clock.mono()
    }

    /// `status --json`, which must answer.
    pub async fn status(&self) -> serde_json::Value {
        let out = self.cli(&["status", "--json"]).await;
        assert_eq!(out.code, Some(0), "status: {out:?}");
        serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("status --json: {e}: {out:?}"))
    }

    /// Polls `status --json` until `waiters` is `n`.
    pub async fn until_waiters(&self, n: u64) {
        let start = self.clock.mono();
        loop {
            let st = self.status().await;
            if st["waiters"].as_u64() == Some(n) {
                return;
            }
            assert!(
                self.clock.mono().since(start) < Duration::from_secs(10),
                "waiters never reached {n}: {st}"
            );
            clock::sleep(&self.clock, POLL).await;
        }
    }

    /// Reads the log file and its old copy.
    pub fn log(&self) -> String {
        let mut text = std::fs::read_to_string(self.log_file()).unwrap_or_default();
        text.push_str(
            &std::fs::read_to_string(self.state_dir().join("circitor.log.1")).unwrap_or_default(),
        );
        text
    }

    // ---- the end ----

    /// TST-11's sweep, then the fake GitHub's rules.
    fn finish(&self) {
        let mut logs = vec![self.log()];
        for path in self.fg_logs.lock().unwrap().iter() {
            logs.push(std::fs::read_to_string(path).unwrap_or_default());
        }
        let state = std::fs::read_to_string(self.state_file()).unwrap_or_default();
        let secrets = [MARKER_TOKEN, MARKER_TOKEN_2, MARKER_SESSION_TOKEN];
        // TST-11: the log and the state file hold no token, and neither GitHub marker (nor a check's name).
        for (name, text) in logs
            .iter()
            .map(|l| ("the log", l))
            .chain([("state.json", &state)])
        {
            for marker in secrets
                .iter()
                .chain(&[MARKER_TITLE, MARKER_GITHUB_TEXT, MARKER_CHECK])
            {
                assert!(
                    !text.contains(marker),
                    "TST-11: {name} holds the marker {marker}"
                );
            }
        }
        // TST-11: no stdout or stderr the test captured holds a token.
        for (stdout, stderr) in self.outputs.lock().unwrap().iter() {
            for marker in secrets {
                assert!(
                    !stdout.contains(marker) && !stderr.contains(marker),
                    "TST-11: a captured output holds a token marker"
                );
            }
        }
        self.gh.check();
    }
}

/// Whether `pid` is a live process (a zombie is dead).
pub fn alive(pid: u32) -> bool {
    if kill(Pid::from_raw(pid as i32), None).is_err() {
        return false;
    }
    if cfg!(target_os = "macos") {
        // `ps` marks a zombie `Z`; no `/proc` there (docs/04 §4).
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output();
        match out {
            Ok(o) => {
                let s = String::from_utf8_lossy(&o.stdout);
                !s.trim().is_empty() && !s.trim_start().starts_with('Z')
            }
            Err(_) => true,
        }
    } else {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // The state is the first field after the command's closing parenthesis.
            Ok(stat) => stat
                .rsplit_once(')')
                .map(|(_, rest)| !rest.trim_start().starts_with('Z'))
                .unwrap_or(true),
            Err(_) => false,
        }
    }
}

/// Sends `sig` to `pid`; a process already gone is fine.
pub fn signal(pid: u32, sig: Signal) {
    let _ = kill(Pid::from_raw(pid as i32), sig);
}

/// The command line of `pid`, to tell our own processes from a reused PID.
fn command_line(pid: u32) -> String {
    if cfg!(target_os = "macos") {
        std::process::Command::new("/bin/ps")
            .args(["-o", "command=", "-p", &pid.to_string()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    } else {
        std::fs::read(format!("/proc/{pid}/cmdline"))
            .map(|b| String::from_utf8_lossy(&b).replace('\0', " "))
            .unwrap_or_default()
    }
}

/// Ours: this build's daemon, or the fake `gh`'s `sleep`.
fn ours(pid: u32) -> bool {
    let cmd = command_line(pid);
    (cmd.contains(&*binary().to_string_lossy()) && cmd.contains("daemon"))
        || cmd.contains(&format!("sleep {HANG_SECONDS}"))
}

// TST-9: the guard. On success and on failure alike, every daemon the lock named (and every one the
// test saw) is stopped with `SIGKILL` (a `SIGSTOP`ped one too), and the test asserts it's dead.
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(h) = read_holder(&self.lock_file()) {
            self.remember(h.pid);
        }
        let _ = self.gh_pid();
        let pids = self.pids.lock().unwrap().clone();
        let mine: Vec<u32> = pids.into_iter().filter(|p| alive(*p) && ours(*p)).collect();
        for pid in &mine {
            signal(*pid, Signal::SIGKILL);
        }
        // The guard runs outside any `.await` (a drop): real time, polled, at most 5 s.
        let mut still: Vec<u32> = mine;
        for _ in 0..250 {
            still.retain(|p| alive(*p));
            if still.is_empty() {
                break;
            }
            std::thread::sleep(POLL);
        }
        if !std::thread::panicking() {
            assert!(
                still.is_empty(),
                "TST-9: processes still alive after the guard: {still:?}"
            );
        }
    }
}
