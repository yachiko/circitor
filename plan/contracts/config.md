# Seam: configuration, environment and paths

**Purpose.** The one struct every component reads its settings from (OPS-1, OPS-2), the one interface to the environment (CLI-13), and the one place that knows where circitor's files are (OPS-3, OPS-4).

**Producer:** T02 declares, with `Config::default()`; T06 builds loading, checking and path resolution. **Consumers:** T08, T09, T15, T21 (read `Config`), T18, T24, T27, T28, T29 (paths and loading).

## Declarations (`src/config/`)

```rust
/// OPS-2, checked. `Default` is OPS-2's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub owners: Vec<String>,               // empty: every owner (POL-5); compared without case
    pub interval: Duration,                // 30 s
    pub hot_interval: Duration,            // 10 s
    pub idle_interval: Duration,           // 300 s
    pub idle_after: Duration,              // 600 s
    pub exit_after: Option<Duration>,      // 3 600 s; `None` for the file's 0: never
    pub keep_done: Duration,               // 86 400 s
    pub quota: QuotaConfig,
    pub gh_path: String,                   // "gh"
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuotaConfig { pub max_per_hour: u32, pub reserve: u32 }   // 600, 500

/// One line naming the key or the file (OPS-1). `Display` is the message printed on stderr.
#[derive(Debug, thiserror::Error)] pub enum ConfigError { /* T06 */ }
/// A missing file is `Config::default()`. Anything else wrong is an error (OPS-1).
pub fn load(path: &Path) -> Result<Config, ConfigError>;

/// The process environment and home directory, as one value (CLI-13). Tests build their own.
#[derive(Clone, Debug, Default)]
pub struct Env { pub vars: BTreeMap<String, String>, pub home: Option<PathBuf> }
impl Env {
    pub fn from_process() -> Env;
    /// `None` for unset and for empty (OPS-4: an empty variable counts as unset).
    pub fn get(&self, name: &str) -> Option<&str>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Platform { Linux, MacOs }
impl Platform { pub fn current() -> Platform; }

#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    pub config_file: PathBuf,              // OPS-1
    pub state_dir: PathBuf,                // OPS-3
    pub state_file: PathBuf,               // state_dir/state.json
    pub lock_file: PathBuf,                // state_dir/circitor.lock
    pub log_file: PathBuf,                 // state_dir/circitor.log
    pub socket: PathBuf,                   // OPS-4
    /// The directory circitor makes for the socket, when it isn't the state directory and the path
    /// didn't come from `CIRCITOR_SOCKET` (which is used as it is, OPS-3).
    pub socket_dir: Option<PathBuf>,
}
#[derive(Debug, thiserror::Error)] pub enum PathError { /* relative CIRCITOR_SOCKET, over 103 bytes, no home; T06 */ }
#[derive(Debug, thiserror::Error)] pub enum DirError { /* owned by someone else, can't be created or changed; names the path; T06 */ }
pub fn resolve(env: &Env, platform: Platform) -> Result<Paths, PathError>;
/// OPS-3: creates it with mode 0700, or sets an existing one of the user's to 0700; one owned by anyone else is an error naming it.
pub fn ensure_private_dir(path: &Path) -> Result<(), DirError>;
/// OPS-4: at most 103 bytes. Built by T02 (three lines; `transport::connect` needs it before T06 is merged).
pub fn check_socket_path(path: &Path) -> Result<(), PathError>;
```

## Semantics

- **Read once.** The daemon loads the config at start, before anything else (OPS-1), and passes `Arc<Config>` down. Nothing re-reads it. The CLI loads it only to check it before replacing a daemon (CLI-9) and for `daemon --check-config`.
- **`resolve` touches no file.** It only computes. `ensure_private_dir` is called by the daemon at start (T28) and by the CLI before it spawns (CLI-7, T24), for `state_dir` and for `socket_dir` when it's `Some`.
- **Platform is a value.** `resolve(env, Platform::MacOs)` runs and is tested on Linux.
- **Test builds (TST-1)** lower `exit_after`'s minimum to 1 s. That's a `cfg(feature = "test-endpoint")` constant inside T06's range check, nowhere else.
- **Durations are whole seconds** in the file. `Config` holds them as `Duration`.

## The fake

`Config::default()` and a hand-built `Env` are the fakes. `testdata/config/` is T06's: `full.toml` (every key, non-default values), `defaults.toml` (empty) and the bad ones.

## Don't assume

- That `XDG_RUNTIME_DIR` is set on Linux, or that anything like it exists on macOS (docs/04 §4).
- That `paths.socket` is inside `state_dir`.
