//! Connecting, spawning, replacing (CLI-7 to CLI-10; T24).
//!
//! Every wait here is on the injected clock (`clock::timeout`, `clock::sleep`), so the tests cost no
//! real time. Every line put in `ConnError.stderr` or `warn` is a whole line; an OS error's text goes
//! last on its line, after `: `.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nix::sys::signal::Signal;

use crate::api::{Request, StatusLite};
use crate::cli::{CliEnv, Spawned, exit};
use crate::clock::{Mono, SharedClock, sleep, timeout};
use crate::config::{Paths, ensure_private_dir};
use crate::daemon::instance::{is_held, read_holder};
use crate::transport::{self, ConnectError, Dropped, Head, Probe};

mod spawn;
#[cfg(test)]
pub(crate) mod stub;
#[cfg(test)]
mod tests;

pub use spawn::RealSpawner;

/// CLI-7: how long the CLI waits for a daemon it started to answer (STA-11's 6 s and start-up).
const START_WAIT: Duration = Duration::from_secs(8);
/// CLI-7: how often the CLI looks at the process it started, and probes.
const WATCH_EVERY: Duration = Duration::from_millis(100);
/// CLI-8, CLI-9: how long the CLI waits for the lock to be released after asking for a stop (OPS-7).
const RELEASE_WAIT: Duration = Duration::from_secs(8);
/// CLI-10: how long a wait that had its cursor waits for a daemon to answer.
const RECONNECT_WAIT: Duration = Duration::from_secs(8);
/// STA-11: how often the lock is tested while waiting for it.
const LOCK_EVERY: Duration = Duration::from_millis(100);
/// CLI-8: `SIGKILL` this long after `SIGTERM`.
const KILL_AFTER: Duration = Duration::from_secs(5);
/// CLI-8 (this brief's value): how long the lock may stay held after `SIGKILL`.
const AFTER_KILL: Duration = Duration::from_secs(2);
/// OPS-9: a log over this many bytes is rotated before a daemon starts.
const ROTATE_OVER: u64 = 5_000_000;
/// CLI-7: at most this many of the new daemon's log lines are shown.
const LOG_TAIL_LINES: usize = 20;

/// Exit 6 and its lines.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnError {
    pub code: i32,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// The socket that answered, the daemon's version and mode.
#[derive(Debug)]
pub struct Link {
    socket: PathBuf,
    version: String,
    mode: String,
    older_unreplaced: bool,
    /// Kept for `request`'s retry (CLI-10), which isn't given them.
    paths: Paths,
    /// A retry may start a daemon: the `Link` came from `ensure` without `no_spawn`.
    may_spawn: bool,
}

fn fail(line: impl AsRef<str>) -> ConnError {
    let mut stderr = line.as_ref().to_string();
    if !stderr.ends_with('\n') {
        stderr.push('\n');
    }
    ConnError {
        code: exit::NO_DAEMON,
        stderr,
    }
}

fn push_line(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}

/// What's left until `deadline`, capped at `limit`.
fn left(clock: &SharedClock, deadline: Option<Mono>, limit: Duration) -> Duration {
    match deadline {
        Some(d) => limit.min(d.since(clock.mono())),
        None => limit,
    }
}

/// Runs `fut` within `deadline`, if there is one. `Err`: the deadline passed.
async fn within<F: Future>(
    clock: &SharedClock,
    deadline: Option<Mono>,
    fut: F,
) -> Result<F::Output, crate::clock::Elapsed> {
    match deadline {
        Some(d) => timeout(clock, d.since(clock.mono()), fut).await,
        None => Ok(fut.await),
    }
}

/// CLI-7: a denied permission names the sandbox settings (docs/03 §2).
fn denied(path: &Path, err: &str) -> ConnError {
    fail(format!(
        "permission denied connecting to {}; if a sandbox runs this command, add circitor to \
         sandbox.excludedCommands, or allow the socket in sandbox.network.allowUnixSockets: {err}",
        path.display()
    ))
}

/// CLI-7: any other error names the path and the system's error.
fn not_answering(path: &Path, err: &str) -> ConnError {
    fail(format!("daemon not answering on {}: {err}", path.display()))
}

fn second_socket_warning(paths: &Paths, second: &Path) -> String {
    format!(
        "warning: no daemon on {}; using the one on {}, named in {}",
        paths.socket.display(),
        second.display(),
        paths.lock_file.display()
    )
}

/// What connecting by CLI-7's rules found.
enum Found {
    Answers(PathBuf, StatusLite),
    NoDaemon,
    Fails(ConnError),
}

fn probe_failure(path: &Path, p: Probe) -> Found {
    match p {
        Probe::Answers(s) => Found::Answers(path.to_path_buf(), s),
        Probe::NoDaemon => Found::NoDaemon,
        Probe::Denied(e) => Found::Fails(denied(path, &e)),
        Probe::NotAnswering(e) => Found::Fails(not_answering(path, &e)),
    }
}

/// CLI-7, OPS-4: the CLI's socket path, then the one the lock file names, if it's another path. The
/// second is tried only when the first is no daemon; when it answers, one warning names both.
async fn find(clock: &SharedClock, paths: &Paths, warn: &mut String) -> Found {
    // CLI-7: connecting tries the CLI's socket path first.
    let first = transport::probe(&paths.socket, clock).await;
    if !matches!(first, Probe::NoDaemon) {
        return probe_failure(&paths.socket, first);
    }
    // OPS-4: the CLI also tries the socket path in `circitor.lock`.
    let Some(holder) = read_holder(&paths.lock_file) else {
        return Found::NoDaemon;
    };
    if holder.socket == paths.socket {
        return Found::NoDaemon;
    }
    let second = transport::probe(&holder.socket, clock).await;
    let found = probe_failure(&holder.socket, second);
    if matches!(found, Found::Answers(..)) {
        push_line(warn, second_socket_warning(paths, &holder.socket));
    }
    found
}

/// CLI-9: the daemon's version against the CLI's, by semver precedence (build metadata ignored).
/// `None`: one of them doesn't parse.
fn compare(daemon: &str, cli: &str) -> Option<Ordering> {
    let d = semver::Version::parse(daemon).ok()?;
    let c = semver::Version::parse(cli).ok()?;
    // CLI-9: `Ord` would compare build metadata; precedence doesn't.
    Some(d.cmp_precedence(&c))
}

fn is_older(daemon: &str, cli: &str) -> bool {
    compare(daemon, cli) == Some(Ordering::Less)
}

/// The rotated log's name: `circitor.log.1`.
fn rotated(log: &Path) -> PathBuf {
    let mut name = OsString::from(log.as_os_str());
    name.push(".1");
    PathBuf::from(name)
}

/// CLI-7, OPS-9: what comes before a spawn. Returns the log, opened for appending, and its size.
fn prepare(paths: &Paths) -> Result<(File, u64), ConnError> {
    // CLI-7: the state directory first (OPS-3, mode 0700), and the socket's own one.
    ensure_private_dir(&paths.state_dir)
        .map_err(|e| fail(format!("can't start the daemon: {e}")))?;
    if let Some(dir) = &paths.socket_dir {
        ensure_private_dir(dir).map_err(|e| fail(format!("can't start the daemon: {e}")))?;
    }
    // OPS-9: a log over 5 000 000 bytes is renamed over `circitor.log.1`.
    match std::fs::metadata(&paths.log_file) {
        Ok(meta) if meta.len() > ROTATE_OVER => {
            let old = rotated(&paths.log_file);
            std::fs::rename(&paths.log_file, &old).map_err(|e| {
                fail(format!(
                    "can't start the daemon: can't rename {} to {}: {e}",
                    paths.log_file.display(),
                    old.display()
                ))
            })?;
        }
        _ => {}
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&paths.log_file)
        .map_err(|e| {
            fail(format!(
                "can't start the daemon: can't open {}: {e}",
                paths.log_file.display()
            ))
        })?;
    let size = log
        .metadata()
        .map_err(|e| {
            fail(format!(
                "can't start the daemon: can't read {}: {e}",
                paths.log_file.display()
            ))
        })?
        .len();
    Ok((log, size))
}

/// CLI-7: up to 20 lines the new daemon wrote, from the size the log had when the CLI opened it.
fn log_tail(log: &Path, from: u64) -> Vec<String> {
    let mut bytes = Vec::new();
    let read = File::open(log).and_then(|mut f| {
        f.seek(SeekFrom::Start(from))?;
        f.read_to_end(&mut bytes)
    });
    if read.is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(LOG_TAIL_LINES);
    lines[skip..].iter().map(|l| l.to_string()).collect()
}

fn with_tail(first: String, paths: &Paths, from: u64) -> ConnError {
    let tail = log_tail(&paths.log_file, from);
    let mut out = String::new();
    if tail.is_empty() {
        push_line(
            &mut out,
            format!("{first}; it wrote nothing to {}", paths.log_file.display()),
        );
    } else {
        push_line(
            &mut out,
            format!("{first}; its last lines in {}:", paths.log_file.display()),
        );
        for line in tail {
            push_line(&mut out, line);
        }
    }
    ConnError {
        code: exit::NO_DAEMON,
        stderr: out,
    }
}

// U9: the CLI starts the daemon when needed (its idle exit is the daemon's, T28).
/// CLI-7: starts a daemon and watches it come up. Returns the socket that answered and its status.
async fn start(
    env: &CliEnv,
    paths: &Paths,
    deadline: Option<Mono>,
    warn: &mut String,
) -> Result<(PathBuf, StatusLite), ConnError> {
    let (log, size) = prepare(paths)?;
    let child = env
        .spawner
        .spawn(log, &env.env)
        .await
        .map_err(|e| fail(format!("can't start the daemon: {e}")))?;
    let limit = left(&env.clock, deadline, START_WAIT);
    let watched = timeout(&env.clock, limit, watch(&env.clock, paths, child)).await;
    match watched {
        Ok(Ok((socket, status))) => {
            if socket != paths.socket {
                push_line(warn, second_socket_warning(paths, &socket));
            }
            Ok((socket, status))
        }
        Ok(Err(code)) => Err(with_tail(
            format!(
                "the daemon exited with status {code} before answering on {}",
                paths.socket.display()
            ),
            paths,
            size,
        )),
        Err(_) => {
            let how = if limit < START_WAIT {
                "in the time left".to_string()
            } else {
                format!("within {}s", START_WAIT.as_secs())
            };
            Err(with_tail(
                format!(
                    "the daemon didn't answer on {} {how}",
                    paths.socket.display()
                ),
                paths,
                size,
            ))
        }
    }
}

/// CLI-7: every 100 ms, asks whether the process has exited and probes. A non-zero exit ends it at once
/// (`Err` with the status); an exit 0 means another daemon holds the lock (STA-11): from then on the
/// socket the lock file names is probed. Runs until a socket answers; the caller bounds it.
async fn watch(
    clock: &SharedClock,
    paths: &Paths,
    mut child: Box<dyn Spawned>,
) -> Result<(PathBuf, StatusLite), i32> {
    let mut exited_ok = false;
    loop {
        if !exited_ok {
            match child.exited() {
                Some(0) => exited_ok = true,
                Some(code) => return Err(code),
                None => {}
            }
        }
        let target = if exited_ok {
            read_holder(&paths.lock_file).map(|h| h.socket)
        } else {
            Some(paths.socket.clone())
        };
        if let Some(socket) = target
            && let Probe::Answers(status) = transport::probe(&socket, clock).await
        {
            return Ok((socket, status));
        }
        sleep(clock, WATCH_EVERY).await;
    }
}

/// Waits up to `limit` for the lock to be released. `Ok(true)`: released.
async fn released(clock: &SharedClock, lock: &Path, limit: Duration) -> std::io::Result<bool> {
    let waited = timeout(clock, limit, async {
        loop {
            // STA-11's test, on a descriptor of our own.
            if !is_held(lock)? {
                return Ok::<(), std::io::Error>(());
            }
            sleep(clock, LOCK_EVERY).await;
        }
    })
    .await;
    match waited {
        Ok(Ok(())) => Ok(true),
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(false),
    }
}

/// One request on its own connection: connect, send, read the body. A connect that fails is `Dropped`.
async fn once(socket: &Path, req: &Request) -> Result<Reply, Dropped> {
    let conn = transport::connect(socket).await.map_err(|_| Dropped)?;
    let head = conn.send(req).await?;
    let status = head.status();
    let body = head.body().await?;
    Ok(Reply { status, body })
}

/// What CLI-9 decided about the daemon that answered.
enum Verdict {
    Keep { older: bool },
    Replace,
}

/// CLI-9: whether to replace the daemon that answered, with a warning for each one kept.
fn judge(
    env: &CliEnv,
    paths: &Paths,
    socket: &Path,
    status: &StatusLite,
    may_replace: bool,
    no_spawn: bool,
    warn: &mut String,
) -> Verdict {
    let (dv, cv) = (&status.version, &env.version);
    let at = socket.display();
    match compare(dv, cv) {
        // CLI-9: a version that doesn't parse is never replaced, with a warning.
        None => {
            push_line(
                warn,
                format!(
                    "warning: the daemon on {at} reports version {dv:?}, which can't be compared with this CLI's {cv}; using it"
                ),
            );
            Verdict::Keep { older: false }
        }
        Some(Ordering::Equal) => Verdict::Keep { older: false },
        // CLI-9: a newer daemon is never replaced.
        Some(Ordering::Greater) => {
            push_line(
                warn,
                format!(
                    "warning: the daemon on {at} is version {dv}, newer than this CLI's {cv}; using it"
                ),
            );
            Verdict::Keep { older: false }
        }
        Some(Ordering::Less) => {
            let older =
                format!("warning: the daemon on {at} is version {dv}, older than this CLI's {cv}");
            // CLI-9: one whose `mode` isn't `spawned` (a foreground one) is never replaced.
            if status.mode != "spawned" {
                push_line(
                    warn,
                    format!(
                        "{older}; it runs in mode {:?}, so it isn't replaced: restart it to update it",
                        status.mode
                    ),
                );
                return Verdict::Keep { older: true };
            }
            // CLI-9: at most one replacement per command.
            if !may_replace {
                push_line(
                    warn,
                    format!(
                        "{older}; one daemon was already replaced in this command, so it's used"
                    ),
                );
                return Verdict::Keep { older: true };
            }
            if no_spawn {
                push_line(
                    warn,
                    format!("{older}; --no-spawn keeps it: run circitor stop to replace it"),
                );
                return Verdict::Keep { older: true };
            }
            // CLI-9: before replacing, the config is checked in this process (OPS-1).
            if let Err(e) = crate::config::load(&paths.config_file) {
                push_line(
                    warn,
                    format!("{older}; it isn't replaced, as the config doesn't load: {e}"),
                );
                return Verdict::Keep { older: true };
            }
            Verdict::Replace
        }
    }
}

/// CLI-7 and CLI-9: connect; with no daemon, start one; replace an older one. `budget` is what's left of a
/// wait's deadline, `None` for other commands. Warnings (a second socket, a newer daemon) go to `warn`.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn ensure(
    env: &CliEnv,
    paths: &Paths,
    no_spawn: bool,
    budget: Option<Duration>,
    warn: &mut String,
) -> Result<Link, ConnError> {
    let clock = &env.clock;
    // CLI-6: with a budget, no wait here outlasts it.
    let deadline = budget.map(|b| clock.mono().plus(b));
    let out_of_time = || {
        fail(format!(
            "no daemon answered on {} in the time left",
            paths.socket.display()
        ))
    };
    let found = within(clock, deadline, find(clock, paths, warn))
        .await
        .map_err(|_| out_of_time())?;
    // CLI-9: the status is read before any other request: the probe is that read.
    let (mut socket, mut status) = match found {
        Found::Answers(socket, status) => (socket, status),
        Found::Fails(e) => return Err(e),
        // CLI-7: only a missing socket file or a refused connection on both paths is "no daemon".
        Found::NoDaemon => {
            if no_spawn {
                // CLI-7: `--no-spawn` turns the start off.
                return Err(fail(format!(
                    "no daemon running on {}, and --no-spawn starts none",
                    paths.socket.display()
                )));
            }
            start(env, paths, deadline, warn).await?
        }
    };
    let mut replaced = false;
    loop {
        match judge(env, paths, &socket, &status, !replaced, no_spawn, warn) {
            Verdict::Keep { older } => {
                return Ok(Link {
                    socket,
                    version: status.version,
                    mode: status.mode,
                    older_unreplaced: older,
                    paths: paths.clone(),
                    may_spawn: !no_spawn,
                });
            }
            Verdict::Replace => {
                // CLI-9: at most one replacement per command.
                replaced = true;
                // CLI-9: ask it to shut down (API-2: any daemon takes this), then wait for its lock.
                let limit = left(clock, deadline, env.request_limit);
                let _asked = timeout(clock, limit, once(&socket, &Request::Shutdown)).await;
                let limit = left(clock, deadline, RELEASE_WAIT);
                let free = released(clock, &paths.lock_file, limit)
                    .await
                    .unwrap_or(false);
                if !free {
                    let budget_out = limit < RELEASE_WAIT;
                    if budget_out {
                        return Err(out_of_time());
                    }
                    push_line(
                        warn,
                        format!(
                            "warning: the daemon on {} is version {}, older than this CLI's {}; it was asked to stop, but {} is still held after {}s, so it's used",
                            socket.display(),
                            status.version,
                            env.version,
                            paths.lock_file.display(),
                            RELEASE_WAIT.as_secs()
                        ),
                    );
                    return Ok(Link {
                        socket,
                        version: status.version,
                        mode: status.mode,
                        older_unreplaced: true,
                        paths: paths.clone(),
                        may_spawn: !no_spawn,
                    });
                }
                (socket, status) = start(env, paths, deadline, warn).await?;
            }
        }
    }
}

impl Link {
    fn from_found(socket: PathBuf, status: StatusLite, cli: &str, paths: &Paths) -> Link {
        Link {
            older_unreplaced: is_older(&status.version, cli),
            socket,
            version: status.version,
            mode: status.mode,
            paths: paths.clone(),
            may_spawn: false,
        }
    }
}

/// CLI-8: connect only. `Ok(None)`: not running.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn existing(
    env: &CliEnv,
    paths: &Paths,
    warn: &mut String,
) -> Result<Option<Link>, ConnError> {
    // CLI-8: nothing is started or replaced.
    match find(&env.clock, paths, warn).await {
        Found::Answers(socket, status) => {
            Ok(Some(Link::from_found(socket, status, &env.version, paths)))
        }
        Found::NoDaemon => Ok(None),
        Found::Fails(e) => Err(e),
    }
}

/// CLI-10, for a wait that had its cursor: wait up to 8 s (within `budget`) for a daemon to answer; start none.
pub async fn reconnect(env: &CliEnv, paths: &Paths, budget: Duration) -> Result<Link, ConnError> {
    let clock = &env.clock;
    let limit = RECONNECT_WAIT.min(budget);
    // CLI-10: connecting as CLI-7 does, every 100 ms; nothing is started or replaced.
    let found = timeout(clock, limit, async {
        loop {
            let mut warn = String::new();
            if let Found::Answers(socket, status) = find(clock, paths, &mut warn).await {
                return (socket, status);
            }
            sleep(clock, WATCH_EVERY).await;
        }
    })
    .await;
    match found {
        Ok((socket, status)) => Ok(Link::from_found(socket, status, &env.version, paths)),
        Err(_) => Err(fail(format!(
            "no daemon answered on {} within {}s",
            paths.socket.display(),
            limit.as_secs_f64()
        ))),
    }
}

impl Link {
    /// The one that answered (CLI-7): for CLI-6's lines.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// CLI-9: an older daemon that was kept.
    pub fn older_unreplaced(&self) -> bool {
        self.older_unreplaced
    }

    /// The version the daemon reported.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The `mode` the daemon reported (API-5).
    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// Any request but a wait: CLI-7's limit, and CLI-10's one retry as a new command.
    pub async fn request(&mut self, env: &CliEnv, req: &Request) -> Result<Reply, ConnError> {
        let clock = env.clock.clone();
        // CLI-7: every request has a limit on the CLI's side; the retry is inside the same one.
        let answer = timeout(&clock, env.request_limit, self.try_twice(env, req)).await;
        match answer {
            Ok(r) => r,
            Err(_) => Err(fail(format!(
                "daemon not answering on {}",
                self.socket.display()
            ))),
        }
    }

    async fn try_twice(&mut self, env: &CliEnv, req: &Request) -> Result<Reply, ConnError> {
        if let Ok(reply) = once(&self.socket, req).await {
            return Ok(reply);
        }
        // CLI-10: a drop, or a connect that fails, is tried once more as a new command: connect again,
        // start a daemon if this command may, replace none.
        let mut warn = String::new();
        let (socket, status) = match find(&env.clock, &self.paths, &mut warn).await {
            Found::Answers(socket, status) => (socket, status),
            Found::Fails(e) => return Err(e),
            Found::NoDaemon if self.may_spawn => start(env, &self.paths, None, &mut warn).await?,
            Found::NoDaemon => {
                return Err(fail(format!(
                    "the connection to {} dropped, and no daemon is running now",
                    self.socket.display()
                )));
            }
        };
        self.older_unreplaced = is_older(&status.version, &env.version);
        self.socket = socket;
        self.version = status.version;
        self.mode = status.mode;
        // CLI-10: a second drop is exit 6.
        once(&self.socket, req).await.map_err(|_| {
            fail(format!(
                "the connection to {} dropped twice without an answer",
                self.socket.display()
            ))
        })
    }

    /// A wait: sends and returns the head. No limit, no retry: T27 owns CLI-6's deadline and CLI-10's rule.
    pub async fn open(&self, req: &Request) -> Result<Head, Dropped> {
        let conn = transport::connect(&self.socket)
            .await
            .map_err(|_| Dropped)?;
        conn.send(req).await
    }
}

/// CLI-8's `stop`, whole: ask, wait for the lock, `SIGTERM`, `SIGKILL`. `Ok(false)`: nothing was running.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn stop(env: &CliEnv, paths: &Paths, warn: &mut String) -> Result<bool, ConnError> {
    stop_with(env, paths, warn, &send_signal).await
}

/// The real signal: `kill(2)`.
fn send_signal(pid: i32, signal: Signal) {
    // The outcome shows in the lock test that follows.
    let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal);
}

/// Connects by CLI-7's rules without reading the status (API-2: `stop` sends `Shutdown` first).
/// `Ok(None)`: no daemon on both paths. `Err(Some)`: a refusal that ends the command.
async fn connect_for_stop(
    paths: &Paths,
    warn: &mut String,
) -> Result<Option<(PathBuf, transport::Conn)>, ConnError> {
    let to_err = |path: &Path, e: ConnectError| match e {
        ConnectError::Denied(e) => denied(path, &e.to_string()),
        other => not_answering(path, &other.to_string()),
    };
    match transport::connect(&paths.socket).await {
        Ok(conn) => return Ok(Some((paths.socket.clone(), conn))),
        Err(ConnectError::NoDaemon) => {}
        Err(e) => return Err(to_err(&paths.socket, e)),
    }
    let Some(holder) = read_holder(&paths.lock_file) else {
        return Ok(None);
    };
    if holder.socket == paths.socket {
        return Ok(None);
    }
    match transport::connect(&holder.socket).await {
        Ok(conn) => {
            push_line(warn, second_socket_warning(paths, &holder.socket));
            Ok(Some((holder.socket, conn)))
        }
        Err(ConnectError::NoDaemon) => Ok(None),
        Err(e) => Err(to_err(&holder.socket, e)),
    }
}

/// `stop`, with the signal sent through `signal` so a test can record it.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
async fn stop_with(
    env: &CliEnv,
    paths: &Paths,
    warn: &mut String,
    signal: &(dyn Fn(i32, Signal) + Sync),
) -> Result<bool, ConnError> {
    let clock = &env.clock;
    // CLI-8: only a missing socket file or a refused connection on both paths is "not running".
    let Some((socket, conn)) = connect_for_stop(paths, warn).await? else {
        return Ok(false);
    };
    let lock_err = |e: std::io::Error| {
        fail(format!(
            "can't test the lock in {}: {e}",
            paths.lock_file.display()
        ))
    };
    // CLI-8: the shutdown, under CLI-7's limit; no status read first (API-2).
    let asked = timeout(clock, env.request_limit, async {
        let head = conn.send(&Request::Shutdown).await?;
        head.body().await
    })
    .await;
    // CLI-8: answered or dropped, the lock gets 8 s; no answer in time goes straight to the signals.
    if asked.is_ok()
        && released(clock, &paths.lock_file, RELEASE_WAIT)
            .await
            .map_err(lock_err)?
    {
        return Ok(true);
    }
    // CLI-8: `SIGTERM`, then `SIGKILL` 5 s later, each only while the lock is still held.
    for (sig, then_wait) in [(Signal::SIGTERM, KILL_AFTER), (Signal::SIGKILL, AFTER_KILL)] {
        if !is_held(&paths.lock_file).map_err(lock_err)? {
            return Ok(true);
        }
        if let Some(pid) = signallable(paths) {
            signal(pid, sig);
        }
        if released(clock, &paths.lock_file, then_wait)
            .await
            .map_err(lock_err)?
        {
            return Ok(true);
        }
    }
    let who = match read_holder(&paths.lock_file) {
        Some(h) => format!("process {}", h.pid),
        None => "a process the lock file doesn't name".to_string(),
    };
    Err(fail(format!(
        "the daemon on {} didn't stop: {who} still holds {}",
        socket.display(),
        paths.lock_file.display()
    )))
}

/// The process the lock file names, if it may be signalled: never 0, 1 or this process.
fn signallable(paths: &Paths) -> Option<i32> {
    let holder = read_holder(&paths.lock_file)?;
    // Never signal blindly: no holder, a process ID of 0 or 1, or this process's own, gets nothing.
    if holder.pid <= 1 || holder.pid == std::process::id() {
        return None;
    }
    i32::try_from(holder.pid).ok()
}
