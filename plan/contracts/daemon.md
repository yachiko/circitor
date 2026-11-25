# Seam: the daemon

**Purpose.** A daemon the harness can start in-process with every outside thing injected (TST-5), stop, crash and start again on the same directories; the single-instance lock and the socket (STA-11, OPS-4); and the one way anything asks it to stop (OPS-7, API-17).

**Producer:** T02 declares `Mode`, `StopCause`, `StopHandle`, `Deps`, `Running` and the stubs of `start` and `main`. T18 builds `daemon::instance`; T28 builds `daemon::run`. **Consumers:** T22 (`StopHandle`, `Mode`), T24 (`instance::read_holder`, `instance::is_held`), T27 (`run::main`), T29 (`Deps`, `start`, `Running`), T36.

## Declarations (`src/daemon/mod.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Mode { Spawned, Foreground }                            // OPS-6; API-5's `mode`

/// Why the daemon stops (OPS-9 logs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopCause { Signal(i32), Request, Idle, NotOwnFiles, Test }
#[derive(Clone)] pub struct StopHandle(/* a watch sender */);
impl StopHandle {
    pub fn new() -> StopHandle;
    pub fn request(&self, cause: StopCause);                     // the first cause wins; a second call is the "second signal" of OPS-7
    pub fn requested(&self) -> Option<StopCause>;
    pub async fn wait(&self) -> StopCause;
    pub fn times(&self) -> u32;                                  // how often it was asked (OPS-7: a second SIGTERM or SIGINT)
}

/// Everything from outside (TST-5).
pub struct Deps {
    pub clock: SharedClock, pub activity: Activity,
    pub github: Arc<dyn GitHub>, pub token_source: Arc<dyn TokenSource>,
    pub fs: Arc<dyn StateFs>, pub probe: Arc<dyn SocketProbe>,
    pub paths: Paths, pub config: Config,
    pub mode: Mode, pub version: String,
    pub log: Arc<dyn LogSink>, pub level: Level,
    pub seed: u64,
    pub signals: bool,                                           // install handlers for SIGTERM, SIGINT, SIGHUP; `false` in-process
    pub stop_delay: Option<Duration>,                            // `CIRCITOR_TEST_STOP_DELAY` (TST-1)
}
/// Why it didn't start. `Display` is the one line for stderr; `code()` the exit status (0 only for
/// a spawned daemon whose lock's holder answers, STA-11).
#[derive(Debug, thiserror::Error)] pub enum StartError { /* T28 */ }
impl StartError { pub fn code(&self) -> i32; }

pub struct Running { /* … */ }
impl Running {
    pub fn socket(&self) -> &Path;
    pub fn stop(&self, cause: StopCause);
    /// How many client connections are open now (the harness waits on it after a client goes away).
    pub fn open_connections(&self) -> usize;
    /// `Some(status)` once it has stopped by itself or been stopped; never waits.
    pub fn exit_status(&self) -> Option<i32>;
    /// Resolves with the exit status when the daemon has stopped (OPS-7, OPS-8): 0 for every stop.
    pub async fn exited(&mut self) -> i32;
    /// TST-5's crash: everything is dropped at once, without OPS-7's stop. The socket file and
    /// `state.json` stay as they are; the lock is released, as a dead process's is.
    pub async fn crash(self);
}
```

## Declarations (`src/daemon/run/`, T28)

`Running` and `StartError` live here too, in `src/daemon/run/mod.rs`, and `src/daemon/mod.rs` re-exports them: their insides are T28's.

```rust
/// STA-11's order: the directories, the lock, the lock file's line, a stale socket, `state.json`, the
/// socket, and only then the engine (which reads the token). Returns once the socket is bound and serving.
pub async fn start(deps: Deps) -> Result<Running, StartError>;
/// `circitor daemon [--spawned]`: real `Deps` (config, paths, `SystemClock`, `github::client`, `token::gh`,
/// stderr), a runtime, `start`, then waits for the end. Returns the exit status.
pub fn main(spawned: bool) -> i32;
```

## Declarations (`src/daemon/instance/`, T18)

```rust
#[derive(Clone, Debug, PartialEq)] pub struct Holder { pub pid: u32, pub socket: PathBuf }
pub enum Acquire { Held(LockGuard), Busy(Option<Holder>) }
/// STA-11: `flock(LOCK_EX | LOCK_NB)` on `path`, opened without truncating, tried every 100 ms of the
/// clock for up to `patience` (6 s; 15 s for a foreground daemon taking over). `Err`: the file can't be
/// opened or created; that's never retried.
pub async fn acquire(path: &Path, clock: &SharedClock, patience: Duration) -> std::io::Result<Acquire>;
pub const RETRY_EVERY: Duration = Duration::from_millis(100);   // STA-11
pub const PATIENCE: Duration = Duration::from_secs(6);
pub const TAKEOVER_PATIENCE: Duration = Duration::from_secs(15); // OPS-6
pub struct LockGuard { /* the open descriptor */ }               // `Drop` releases the lock
impl LockGuard {
    /// Truncates and writes `<pid> <socket path>\n`.
    pub fn write_holder(&self, pid: u32, socket: &Path) -> std::io::Result<()>;
    /// Whether the path still is this descriptor's file (same device and inode).
    pub fn still_own(&self) -> bool;
}
/// The holder a lock file names: one complete line, or `None`.
pub fn read_holder(path: &Path) -> Option<Holder>;
/// STA-11's test: take `LOCK_EX | LOCK_NB` on a descriptor of our own and let go at once.
pub fn is_held(path: &Path) -> std::io::Result<bool>;

#[derive(Debug, PartialEq)] pub enum SocketFile { Absent, RemovedStale, Live(StatusLite), NotAnswering /* also `Probe::Denied` */, NotASocket }
/// OPS-4, with the lock held: removes the file only if it's a socket that refuses a connection.
pub async fn clear_socket(path: &Path, probe: &dyn SocketProbe) -> std::io::Result<SocketFile>;
pub struct Bound { pub listener: tokio::net::UnixListener, /* device and inode */ }
/// Binds so that the socket's mode is 0600 (OPS-4): under a umask of **0177**. OPS-4's text says 077,
/// which gives 0700 on Linux (checked 2026-11-25; README spec issue 13). Checks the path's length first.
pub fn bind(path: &Path) -> std::io::Result<Bound>;
impl Bound {
    pub fn still_own(&self) -> bool;
    pub fn remove_if_own(&self);                                  // OPS-7, STA-11: only a socket it made
}
```

## Semantics

- **`start` owns the order and every exit.** A step that fails returns `StartError`; nothing is left bound or locked. T28's brief has the order line by line.
- **`Running::exited` is the only place an exit status is decided.** In-process it's returned; `main` passes it to the process.
- **One way to stop.** Signals, `POST /v1/shutdown` (T22 calls `state.stop.request(StopCause::Request)` after its answer is written), the idle exit and the minute's files check all call `StopHandle::request`. T28's stop sequence runs once; a second request during it ends it at once (OPS-7).
- **`clear_socket` looks before it connects:** `lstat` first, since connecting to a regular file is refused just like a dead socket. Only a socket is ever removed.
- **The umask is the process's.** `bind` holds a process-wide mutex around setting the umask, binding and restoring it, because tests start several daemons in one process.
- **A lock held by this same process** (a test restarting in-process) behaves like any holder: `flock` locks belong to the open file description, so a second descriptor in one process is refused too.
- **`main` builds its own runtime.** `cli::main` calls it for `circitor daemon` before any runtime exists.
- **`crash`** aborts every task and drops the listener without removing the socket file, so the next `start` meets a stale socket and an unsaved state (TST-8 `crash`).

## The fake

There's no fake daemon. The harness (T29) starts the real one through `start` with the test clock, the fake GitHub's client address, `FakeTokenSource`, a `BufferSink`, directories under `/tmp`, `signals: false`. Tasks that need a server before T28 (T17, T24, T27) write a few lines of stub server on a socket.

## Don't assume

- That `start` returning means the first poll ran, or that the token was read: only that the socket answers (OPS-5).
- That an in-process daemon exits the process: nothing in `src/daemon/` calls `std::process::exit` except `main`.
