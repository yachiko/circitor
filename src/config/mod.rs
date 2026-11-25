//! Configuration, environment and paths (`plan/contracts/config.md`). T02 declares, with
//! `Config::default()`, `Env`, `Platform` and `check_socket_path`; T06 builds the rest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod load;
mod paths;

pub use load::{ConfigError, load};
pub use paths::{DirError, ensure_private_dir, resolve};

/// OPS-2, checked. `Default` is OPS-2's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Empty: every owner (POL-5); compared without case.
    pub owners: Vec<String>,
    /// 30 s.
    pub interval: Duration,
    /// 10 s.
    pub hot_interval: Duration,
    /// 300 s.
    pub idle_interval: Duration,
    /// 600 s.
    pub idle_after: Duration,
    /// 3 600 s; `None` for the file's 0: never.
    pub exit_after: Option<Duration>,
    /// 86 400 s.
    pub keep_done: Duration,
    pub quota: QuotaConfig,
    /// "gh".
    pub gh_path: String,
}

/// 600, 500.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotaConfig {
    pub max_per_hour: u32,
    pub reserve: u32,
}

impl Default for QuotaConfig {
    // OPS-2: the defaults.
    fn default() -> QuotaConfig {
        QuotaConfig {
            max_per_hour: 600,
            reserve: 500,
        }
    }
}

impl Default for Config {
    // OPS-2: the defaults.
    fn default() -> Config {
        Config {
            owners: Vec::new(),
            interval: Duration::from_secs(30),
            hot_interval: Duration::from_secs(10),
            idle_interval: Duration::from_secs(300),
            idle_after: Duration::from_secs(600),
            exit_after: Some(Duration::from_secs(3_600)),
            keep_done: Duration::from_secs(86_400),
            quota: QuotaConfig::default(),
            gh_path: "gh".to_string(),
        }
    }
}

/// The process environment and home directory, as one value (CLI-13). Tests build their own.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub vars: BTreeMap<String, String>,
    pub home: Option<PathBuf>,
}

impl Env {
    /// The process's variables (those that are valid UTF-8) and its home directory. The one place
    /// circitor reads either.
    pub fn from_process() -> Env {
        let vars = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        Env {
            vars,
            home: std::env::home_dir().filter(|h| !h.as_os_str().is_empty()),
        }
    }

    /// `None` for unset and for empty (OPS-4: an empty variable counts as unset).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
}

impl Platform {
    /// The platform this binary was built for; anything but macOS is treated as Linux.
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    /// OPS-1.
    pub config_file: PathBuf,
    /// OPS-3.
    pub state_dir: PathBuf,
    /// state_dir/state.json.
    pub state_file: PathBuf,
    /// state_dir/circitor.lock.
    pub lock_file: PathBuf,
    /// state_dir/circitor.log.
    pub log_file: PathBuf,
    /// OPS-4.
    pub socket: PathBuf,
    /// The directory circitor makes for the socket, when it isn't the state directory and the path
    /// didn't come from `CIRCITOR_SOCKET` (which is used as it is, OPS-3).
    pub socket_dir: Option<PathBuf>,
}

/// The longest socket path, in bytes (OPS-4; docs/04 §4: the limit is 104 on macOS).
pub const MAX_SOCKET_PATH: usize = 103;

/// Why the paths can't be worked out (OPS-3, OPS-4). Each names what to fix.
#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("CIRCITOR_SOCKET must be an absolute path: {0}")]
    RelativeSocket(PathBuf),
    #[error("the socket path is over 103 bytes; set CIRCITOR_SOCKET to a shorter one: {0}")]
    TooLong(PathBuf),
    #[error("no home directory: set HOME")]
    NoHome,
}

/// OPS-4: at most 103 bytes. Built by T02 (`transport::connect` needs it before T06 is merged).
pub fn check_socket_path(path: &Path) -> Result<(), PathError> {
    // OPS-4: a path over 103 bytes is refused.
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        Err(PathError::TooLong(path.to_path_buf()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OPS-2: the defaults.
    #[test]
    fn ops2_defaults() {
        let c = Config::default();
        assert_eq!(c.interval, Duration::from_secs(30));
        assert_eq!(c.hot_interval, Duration::from_secs(10));
        assert_eq!(c.idle_interval, Duration::from_secs(300));
        assert_eq!(c.idle_after, Duration::from_secs(600));
        assert_eq!(c.exit_after, Some(Duration::from_secs(3_600)));
        assert_eq!(c.keep_done, Duration::from_secs(86_400));
        assert_eq!(
            c.quota,
            QuotaConfig {
                max_per_hour: 600,
                reserve: 500
            }
        );
        assert_eq!(c.gh_path, "gh");
        assert!(c.owners.is_empty());
    }

    // OPS-4: an empty variable counts as unset.
    #[test]
    fn ops4_an_empty_variable_is_unset() {
        let mut env = Env::default();
        env.vars.insert("XDG_RUNTIME_DIR".into(), String::new());
        env.vars
            .insert("CIRCITOR_SOCKET".into(), "/tmp/c.sock".into());
        assert_eq!(env.get("XDG_RUNTIME_DIR"), None);
        assert_eq!(env.get("NOT_THERE"), None);
        assert_eq!(env.get("CIRCITOR_SOCKET"), Some("/tmp/c.sock"));
    }

    // OPS-4: a socket path over 103 bytes is refused.
    #[test]
    fn ops4_check_socket_path_103_bytes() {
        let path = |n: usize| PathBuf::from(format!("/{}", "a".repeat(n - 1)));
        assert_eq!(path(103).as_os_str().len(), 103);
        assert!(check_socket_path(&path(103)).is_ok());
        assert!(matches!(
            check_socket_path(&path(104)),
            Err(PathError::TooLong(_))
        ));
    }

    #[test]
    fn platform_current_is_this_one() {
        let expected = if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        };
        assert_eq!(Platform::current(), expected);
    }
}
