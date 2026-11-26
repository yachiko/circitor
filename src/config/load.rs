//! Reading and checking `config.toml` (OPS-1, OPS-2).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Config, QuotaConfig};

/// The lowest `exit_after` other than 0, in seconds (OPS-2: 300). A test build lowers it
/// to 1 (TST-1). This is the only place the feature touches the config.
#[cfg(not(feature = "test-endpoint"))]
const EXIT_AFTER_MIN: i64 = 300;
#[cfg(feature = "test-endpoint")]
const EXIT_AFTER_MIN: i64 = 1;

/// An inclusive range, in the unit of its key.
#[derive(Clone, Copy, Debug)]
pub(super) struct Range {
    pub min: i64,
    pub max: i64,
}

/// OPS-2's ranges, all (proposal), in one place. The check takes it as a value.
#[derive(Clone, Debug)]
pub(super) struct Limits {
    pub interval: Range,
    pub hot_interval: Range,
    pub idle_interval: Range,
    pub idle_after: Range,
    pub exit_after: Range,
    pub keep_done: Range,
    pub max_per_hour: Range,
    pub reserve: Range,
}

impl Limits {
    /// OPS-2's table with `exit_after`'s floor as the given value.
    pub(super) fn with_exit_after_min(min: i64) -> Limits {
        let r = |min, max| Range { min, max };
        Limits {
            interval: r(10, 300),
            hot_interval: r(5, 60),
            idle_interval: r(60, 3_600),
            idle_after: r(60, 86_400),
            exit_after: r(min, 604_800),
            keep_done: r(0, 604_800),
            max_per_hour: r(60, 4_000),
            reserve: r(0, 4_000),
        }
    }

    /// The table this build checks against.
    pub(super) fn current() -> Limits {
        Limits::with_exit_after_min(EXIT_AFTER_MIN)
    }
}

/// One line naming the key or the file (OPS-1). `Display` is the message printed on stderr.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists and can't be read.
    #[error("{}: can't be read: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Not valid TOML; the line is where the parser stopped.
    #[error("{}: line {line}: not valid TOML: {message}", path.display())]
    Syntax {
        path: PathBuf,
        line: usize,
        message: String,
    },
    /// One key is wrong: unknown, the wrong type or out of range.
    #[error("{}: {key}: {problem}", path.display())]
    Key {
        path: PathBuf,
        key: String,
        problem: String,
    },
    /// The intervals contradict each other.
    #[error("{}: {problem}", path.display())]
    Intervals { path: PathBuf, problem: String },
}

/// A missing file is `Config::default()`. Anything else wrong is an error (OPS-1).
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    load_with(path, &Limits::current())
}

pub(super) fn load_with(path: &Path, limits: &Limits) -> Result<Config, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // OPS-1: only a missing file means the defaults.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    parse(path, &text, limits)
}

fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

fn parse(path: &Path, text: &str, limits: &Limits) -> Result<Config, ConfigError> {
    // OPS-1: parsed to a table and walked by hand, so every message is ours and one line.
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        let start = e.span().map(|s| s.start.min(text.len())).unwrap_or(0);
        let line = text.as_bytes()[..start]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
            + 1;
        ConfigError::Syntax {
            path: path.to_path_buf(),
            line,
            message: one_line(e.message()),
        }
    })?;

    let bad = |key: &str, problem: String| ConfigError::Key {
        path: path.to_path_buf(),
        key: one_line(key),
        problem,
    };
    // A whole number inside the range.
    let number = |key: &str, v: &toml::Value, r: Range| -> Result<i64, ConfigError> {
        match v {
            toml::Value::Integer(n) if *n >= r.min && *n <= r.max => Ok(*n),
            toml::Value::Integer(n) => Err(bad(
                key,
                format!("out of range: {n} is not between {} and {}", r.min, r.max),
            )),
            other => Err(bad(
                key,
                format!("must be a whole number, not {}", other.type_str()),
            )),
        }
    };
    let secs = |n: i64| Duration::from_secs(n as u64);

    let mut c = Config::default();
    for (key, value) in &table {
        match key.as_str() {
            "owners" => {
                let err = || bad("owners", "must be a list of strings".to_string());
                let list = value.as_array().ok_or_else(err)?;
                c.owners = list
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).ok_or_else(err))
                    .collect::<Result<_, _>>()?;
            }
            "interval" => c.interval = secs(number(key, value, limits.interval)?),
            "hot_interval" => c.hot_interval = secs(number(key, value, limits.hot_interval)?),
            "idle_interval" => c.idle_interval = secs(number(key, value, limits.idle_interval)?),
            "idle_after" => c.idle_after = secs(number(key, value, limits.idle_after)?),
            "exit_after" => {
                // OPS-2: 0 is never; any other value is in range.
                c.exit_after = match value {
                    toml::Value::Integer(0) => None,
                    _ => Some(secs(number(key, value, limits.exit_after)?)),
                };
            }
            "keep_done" => c.keep_done = secs(number(key, value, limits.keep_done)?),
            "gh_path" => {
                let s = value.as_str().ok_or_else(|| {
                    bad(key, format!("must be a string, not {}", value.type_str()))
                })?;
                // OPS-2: an absolute path, or a name without a slash.
                let ok = if s.contains('/') {
                    Path::new(s).is_absolute()
                } else {
                    !s.is_empty()
                };
                if !ok {
                    return Err(bad(
                        key,
                        "must be an absolute path or a name without '/'".to_string(),
                    ));
                }
                c.gh_path = s.to_string();
            }
            "quota" => {
                let q = value.as_table().ok_or_else(|| {
                    bad(key, format!("must be a table, not {}", value.type_str()))
                })?;
                let mut quota = QuotaConfig::default();
                for (qk, qv) in q {
                    let full = format!("quota.{qk}");
                    match qk.as_str() {
                        "max_per_hour" => {
                            quota.max_per_hour = number(&full, qv, limits.max_per_hour)? as u32
                        }
                        "reserve" => quota.reserve = number(&full, qv, limits.reserve)? as u32,
                        _ => return Err(bad(&full, "unknown key".to_string())),
                    }
                }
                c.quota = quota;
            }
            _ => return Err(bad(key, "unknown key".to_string())),
        }
    }

    // OPS-1: the intervals against each other, on the values in force.
    let intervals = |problem: String| ConfigError::Intervals {
        path: path.to_path_buf(),
        problem,
    };
    if c.hot_interval > c.interval {
        return Err(intervals(format!(
            "hot_interval ({}) must not be more than interval ({})",
            c.hot_interval.as_secs(),
            c.interval.as_secs()
        )));
    }
    if c.interval > c.idle_interval {
        return Err(intervals(format!(
            "interval ({}) must not be more than idle_interval ({})",
            c.interval.as_secs(),
            c.idle_interval.as_secs()
        )));
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::testkit::ShortDir;

    fn data(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/config")
            .join(name)
    }

    fn write(dir: &ShortDir, text: &str) -> PathBuf {
        let p = dir.path().join("config.toml");
        std::fs::write(&p, text).unwrap();
        p
    }

    fn release() -> Limits {
        Limits::with_exit_after_min(300)
    }

    fn run(text: &str, limits: &Limits) -> Result<Config, ConfigError> {
        let dir = ShortDir::new();
        load_with(&write(&dir, text), limits)
    }

    // OPS-1: only a missing file is the defaults.
    #[test]
    fn ops1_missing_file_is_the_defaults() {
        let dir = ShortDir::new();
        let c = load(&dir.path().join("nope.toml")).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(load(&data("defaults.toml")).unwrap(), Config::default());
    }

    // OPS-1, OPS-2: every key of the full file.
    #[test]
    fn ops1_full_file_loads_every_key() {
        let c = load(&data("full.toml")).unwrap();
        let d = Config::default();
        assert_eq!(
            c.owners,
            vec!["Yachiko".to_string(), "some-org".to_string()]
        );
        assert_eq!(c.interval, Duration::from_secs(60));
        assert_eq!(c.hot_interval, Duration::from_secs(20));
        assert_eq!(c.idle_interval, Duration::from_secs(900));
        assert_eq!(c.idle_after, Duration::from_secs(1200));
        assert_eq!(c.exit_after, Some(Duration::from_secs(7200)));
        assert_eq!(c.keep_done, Duration::from_secs(3600));
        assert_eq!(
            c.quota,
            QuotaConfig {
                max_per_hour: 1000,
                reserve: 100
            }
        );
        assert_eq!(c.gh_path, "/opt/homebrew/bin/gh");
        assert_ne!(c.owners, d.owners);
        assert_ne!(c.interval, d.interval);
        assert_ne!(c.hot_interval, d.hot_interval);
        assert_ne!(c.idle_interval, d.idle_interval);
        assert_ne!(c.idle_after, d.idle_after);
        assert_ne!(c.exit_after, d.exit_after);
        assert_ne!(c.keep_done, d.keep_done);
        assert_ne!(c.quota.max_per_hour, d.quota.max_per_hour);
        assert_ne!(c.quota.reserve, d.quota.reserve);
        assert_ne!(c.gh_path, d.gh_path);
    }

    // OPS-2: the defaults.
    #[test]
    fn ops2_defaults() {
        let c = Config::default();
        assert_eq!(c.interval.as_secs(), 30);
        assert_eq!(c.hot_interval.as_secs(), 10);
        assert_eq!(c.idle_interval.as_secs(), 300);
        assert_eq!(c.idle_after.as_secs(), 600);
        assert_eq!(c.exit_after, Some(Duration::from_secs(3_600)));
        assert_eq!(c.keep_done.as_secs(), 86_400);
        assert_eq!((c.quota.max_per_hour, c.quota.reserve), (600, 500));
        assert_eq!(c.gh_path, "gh");
        assert!(c.owners.is_empty());
    }

    // OPS-2: the lowest and highest load; one below and one above are refused, naming the key.
    #[test]
    fn ops2_ranges() {
        let cases: [(&str, i64, i64); 7] = [
            ("interval", 10, 300),
            ("hot_interval", 5, 60),
            ("idle_interval", 60, 3_600),
            ("idle_after", 60, 86_400),
            ("keep_done", 0, 604_800),
            ("quota.max_per_hour", 60, 4_000),
            ("quota.reserve", 0, 4_000),
        ];
        for (key, lo, hi) in cases {
            let with = |v: i64| match key.split_once('.') {
                Some((t, k)) => format!("[{t}]\n{k} = {v}\n"),
                // hot_interval's top needs an interval above it to keep the order.
                None if key == "hot_interval" => format!("interval = 60\n{key} = {v}\n"),
                None => format!("{key} = {v}\n"),
            };
            for ok in [lo, hi] {
                let r = run(&with(ok), &release());
                assert!(r.is_ok(), "{key} = {ok}: {r:?}");
            }
            for bad in [lo - 1, hi + 1] {
                let e = run(&with(bad), &release()).unwrap_err().to_string();
                assert!(e.contains(key), "{key} = {bad}: {e}");
            }
        }
    }

    // OPS-2: exit_after.
    #[test]
    fn ops2_exit_after() {
        let t = |v: i64| run(&format!("exit_after = {v}\n"), &release());
        assert_eq!(t(0).unwrap().exit_after, None);
        assert_eq!(t(300).unwrap().exit_after, Some(Duration::from_secs(300)));
        assert!(t(299).unwrap_err().to_string().contains("exit_after"));
        assert!(t(604_800).is_ok());
        assert!(t(604_801).unwrap_err().to_string().contains("exit_after"));
        // TST-1: the table of a test build has 1 as the floor.
        let dir = ShortDir::new();
        let r = load(&write(&dir, "exit_after = 1\n"));
        if cfg!(feature = "test-endpoint") {
            assert_eq!(r.unwrap().exit_after, Some(Duration::from_secs(1)));
        } else {
            assert!(r.is_err());
        }
        assert!(run("exit_after = 1\n", &Limits::with_exit_after_min(1)).is_ok());
    }

    // OPS-2: gh_path.
    #[test]
    fn ops2_gh_path() {
        for ok in ["gh", "/opt/homebrew/bin/gh"] {
            let c = run(&format!("gh_path = \"{ok}\"\n"), &release()).unwrap();
            assert_eq!(c.gh_path, ok);
        }
        for bad in ["bin/gh", "./gh", ""] {
            let e = run(&format!("gh_path = \"{bad}\"\n"), &release()).unwrap_err();
            assert!(e.to_string().contains("gh_path"), "{bad}: {e}");
        }
        assert!(run("gh_path = 3\n", &release()).is_err());
    }

    // OPS-2: owners is a list of strings.
    #[test]
    fn ops2_owners_is_a_list_of_strings() {
        assert!(run("owners = []\n", &release()).unwrap().owners.is_empty());
        let e = run("owners = [1]\n", &release()).unwrap_err().to_string();
        assert!(e.contains("owners"), "{e}");
        assert!(run("owners = \"a\"\n", &release()).is_err());
    }

    // OPS-1: the intervals against each other.
    #[test]
    fn ops1_intervals_are_checked_against_each_other() {
        let e = run("hot_interval = 60\n", &release())
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("hot_interval") && e.contains("interval ("),
            "{e}"
        );
        let e = run("interval = 120\nidle_interval = 60\n", &release())
            .unwrap_err()
            .to_string();
        assert!(e.contains("interval") && e.contains("idle_interval"), "{e}");
        let c = run(
            "hot_interval = 60\ninterval = 60\nidle_interval = 60\n",
            &release(),
        )
        .unwrap();
        assert_eq!(c.interval, Duration::from_secs(60));
    }

    // OPS-1: every error is one line naming the file and, where one is at fault, the key.
    #[test]
    fn ops1_every_error_is_one_line_naming_it() {
        let cases = [
            ("unknown_key.toml", Some("speed")),
            ("unknown_quota_key.toml", Some("quota.burst")),
            ("unknown_table.toml", Some("cache")),
            ("out_of_range.toml", Some("interval")),
            ("out_of_range_quota.toml", Some("quota.reserve")),
            ("wrong_type.toml", Some("interval")),
            ("wrong_type_quota.toml", Some("quota.reserve")),
            ("fraction.toml", Some("interval")),
            ("not_toml.toml", None),
            ("intervals_out_of_order.toml", Some("hot_interval")),
        ];
        for (name, key) in cases {
            let p = data(name);
            let e = load(&p).unwrap_err().to_string();
            assert!(!e.contains('\n'), "{name}: {e}");
            assert!(e.contains(&p.display().to_string()), "{name}: {e}");
            if let Some(k) = key {
                assert!(e.contains(k), "{name}: {e}");
            }
        }
        // A syntax error gives the line.
        let e = load(&data("not_toml.toml")).unwrap_err().to_string();
        assert!(e.contains("line 2"), "{e}");

        let dir = ShortDir::new();
        // A directory in the file's place.
        let e = load(dir.path()).unwrap_err().to_string();
        assert!(
            !e.contains('\n') && e.contains(&dir.path().display().to_string()),
            "{e}"
        );
        // A file nobody can read.
        if !nix::unistd::geteuid().is_root() {
            let p = write(&dir, "interval = 30\n");
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
            let e = load(&p).unwrap_err().to_string();
            assert!(
                !e.contains('\n') && e.contains(&p.display().to_string()),
                "{e}"
            );
        }
    }
}
