//! Where circitor's files are (OPS-1, OPS-3, OPS-4) and the private directories (OPS-3).

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::{Env, PathError, Paths, Platform, check_socket_path};

/// Why a directory can't be made private (OPS-3). Each names the path.
#[derive(Debug, thiserror::Error)]
pub enum DirError {
    #[error("{}: is not a directory", .0.display())]
    NotADirectory(PathBuf),
    #[error("{}: is owned by another user; circitor won't use it", .0.display())]
    NotOwned(PathBuf),
    #[error("{}: can't {action}: {source}", path.display())]
    Io {
        path: PathBuf,
        action: &'static str,
        source: std::io::Error,
    },
}

/// A variable that is set, non-empty and absolute; anything else counts as unset (OPS-4).
fn absolute_var<'a>(env: &'a Env, name: &str) -> Option<&'a Path> {
    env.get(name).map(Path::new).filter(|p| p.is_absolute())
}

/// OPS-3, OPS-4. Touches no file: it only computes.
pub fn resolve(env: &Env, platform: Platform) -> Result<Paths, PathError> {
    // A missing home is an error only when a path needs it.
    let home = || env.home.as_deref().ok_or(PathError::NoHome);

    // OPS-1: the same on both systems.
    let config_file = match absolute_var(env, "XDG_CONFIG_HOME") {
        Some(dir) => dir.to_path_buf(),
        None => home()?.join(".config"),
    }
    .join("circitor")
    .join("config.toml");

    // OPS-3.
    let state_dir = match platform {
        Platform::Linux => match absolute_var(env, "XDG_STATE_HOME") {
            Some(dir) => dir.to_path_buf(),
            None => home()?.join(".local").join("state"),
        }
        .join("circitor"),
        Platform::MacOs => home()?
            .join("Library")
            .join("Application Support")
            .join("circitor"),
    };

    // OPS-4.
    let (socket, socket_dir) = if let Some(s) = env.get("CIRCITOR_SOCKET") {
        // OPS-4: CIRCITOR_SOCKET overrides the path, must be absolute, and its directory is
        // used as it is.
        let p = Path::new(s);
        if !p.is_absolute() {
            return Err(PathError::RelativeSocket(p.to_path_buf()));
        }
        (p.to_path_buf(), None)
    } else {
        match (platform, absolute_var(env, "XDG_RUNTIME_DIR")) {
            (Platform::Linux, Some(run)) => {
                let dir = run.join("circitor");
                (dir.join("circitor.sock"), Some(dir))
            }
            _ => (state_dir.join("circitor.sock"), None),
        }
    };
    // OPS-4: a path over 103 bytes is refused, wherever it came from.
    check_socket_path(&socket)?;

    Ok(Paths {
        state_file: state_dir.join("state.json"),
        lock_file: state_dir.join("circitor.lock"),
        log_file: state_dir.join("circitor.log"),
        config_file,
        state_dir,
        socket,
        socket_dir,
    })
}

/// OPS-3: creates it with mode 0700, or sets an existing one of the user's to 0700; one owned by anyone else is an error naming it.
pub fn ensure_private_dir(path: &Path) -> Result<(), DirError> {
    let io = |action: &'static str| {
        move |source| DirError::Io {
            path: path.to_path_buf(),
            action,
            source,
        }
    };
    match std::fs::metadata(path) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(DirError::NotADirectory(path.to_path_buf()));
            }
            // The owner is checked before the mode is changed, against the effective user.
            if meta.uid() != nix::unistd::geteuid().as_raw() {
                return Err(DirError::NotOwned(path.to_path_buf()));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path).map_err(io("create it"))?;
        }
        Err(e) => return Err(io("look at it")(e)),
    }
    // Set explicitly, so the umask doesn't matter.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(io("set its mode to 0700"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MAX_SOCKET_PATH;
    use crate::testkit::ShortDir;

    fn env(home: Option<&str>, vars: &[(&str, &str)]) -> Env {
        Env {
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            home: home.map(PathBuf::from),
        }
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    // OPS-1, OPS-3, OPS-4: Linux.
    #[test]
    fn ops3_ops4_linux_paths() {
        let r = resolve(&env(Some("/home/u"), &[]), Platform::Linux).unwrap();
        assert_eq!(r.config_file, p("/home/u/.config/circitor/config.toml"));
        let state = p("/home/u/.local/state/circitor");
        assert_eq!(r.state_dir, state);
        assert_eq!(r.state_file, state.join("state.json"));
        assert_eq!(r.lock_file, state.join("circitor.lock"));
        assert_eq!(r.log_file, state.join("circitor.log"));
        assert_eq!(r.socket, state.join("circitor.sock"));
        assert_eq!(r.socket_dir, None);

        let r = resolve(
            &env(Some("/home/u"), &[("XDG_RUNTIME_DIR", "/run/user/1000")]),
            Platform::Linux,
        )
        .unwrap();
        assert_eq!(r.socket, p("/run/user/1000/circitor/circitor.sock"));
        assert_eq!(r.socket_dir, Some(p("/run/user/1000/circitor")));
        assert_eq!(r.state_dir, state);

        let r = resolve(
            &env(
                Some("/home/u"),
                &[
                    ("XDG_CONFIG_HOME", "/x/cfg"),
                    ("XDG_STATE_HOME", "/x/state"),
                    ("XDG_RUNTIME_DIR", "/x/run"),
                ],
            ),
            Platform::Linux,
        )
        .unwrap();
        assert_eq!(r.config_file, p("/x/cfg/circitor/config.toml"));
        assert_eq!(r.state_dir, p("/x/state/circitor"));
        assert_eq!(r.log_file, p("/x/state/circitor/circitor.log"));
        assert_eq!(r.socket, p("/x/run/circitor/circitor.sock"));
        assert_eq!(r.socket_dir, Some(p("/x/run/circitor")));
    }

    // OPS-3, OPS-4: macOS.
    #[test]
    fn ops3_ops4_macos_paths() {
        let r = resolve(&env(Some("/Users/u"), &[]), Platform::MacOs).unwrap();
        let state = p("/Users/u/Library/Application Support/circitor");
        assert_eq!(r.config_file, p("/Users/u/.config/circitor/config.toml"));
        assert_eq!(r.state_dir, state);
        assert_eq!(r.state_file, state.join("state.json"));
        assert_eq!(r.socket, state.join("circitor.sock"));
        assert_eq!(r.socket_dir, None);

        let r = resolve(
            &env(
                Some("/Users/u"),
                &[
                    ("XDG_RUNTIME_DIR", "/x/run"),
                    ("XDG_STATE_HOME", "/x/state"),
                    ("XDG_CONFIG_HOME", "/x/cfg"),
                ],
            ),
            Platform::MacOs,
        )
        .unwrap();
        assert_eq!(r.state_dir, state);
        assert_eq!(r.socket, state.join("circitor.sock"));
        assert_eq!(r.socket_dir, None);
        assert_eq!(r.config_file, p("/x/cfg/circitor/config.toml"));
    }

    // OPS-4: an empty or relative XDG variable counts as unset.
    #[test]
    fn ops4_empty_or_relative_variables_count_as_unset() {
        let plain = resolve(&env(Some("/home/u"), &[]), Platform::Linux).unwrap();
        for name in ["XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR"] {
            for value in ["", "state"] {
                let r = resolve(&env(Some("/home/u"), &[(name, value)]), Platform::Linux).unwrap();
                assert_eq!(r, plain, "{name}={value:?}");
            }
        }
    }

    // OPS-4: CIRCITOR_SOCKET.
    #[test]
    fn ops4_circitor_socket() {
        for platform in [Platform::Linux, Platform::MacOs] {
            let e = env(
                Some("/home/u"),
                &[
                    ("CIRCITOR_SOCKET", "/tmp/c/x.sock"),
                    ("XDG_RUNTIME_DIR", "/run/user/1"),
                ],
            );
            let r = resolve(&e, platform).unwrap();
            assert_eq!(r.socket, p("/tmp/c/x.sock"));
            assert_eq!(r.socket_dir, None);
            for bad in ["sock", "./x.sock"] {
                let e = env(Some("/home/u"), &[("CIRCITOR_SOCKET", bad)]);
                let err = resolve(&e, platform).unwrap_err();
                assert!(matches!(err, PathError::RelativeSocket(_)));
                assert!(err.to_string().contains("CIRCITOR_SOCKET"), "{err}");
            }
            let e = env(Some("/home/u"), &[("CIRCITOR_SOCKET", "")]);
            let r = resolve(&e, platform).unwrap();
            assert!(r.socket.ends_with("circitor.sock"));
        }
    }

    // OPS-4: 103 bytes.
    #[test]
    fn ops4_103_bytes() {
        let sock = |n: usize| format!("/{}", "a".repeat(n - 1));
        let ok = env(Some("/h"), &[("CIRCITOR_SOCKET", &sock(103))]);
        assert!(resolve(&ok, Platform::Linux).is_ok());
        let long = env(Some("/h"), &[("CIRCITOR_SOCKET", &sock(104))]);
        let err = resolve(&long, Platform::Linux).unwrap_err();
        assert!(err.to_string().contains("CIRCITOR_SOCKET"), "{err}");

        // The default macOS socket is home + this tail.
        let tail = "/Library/Application Support/circitor/circitor.sock";
        let home = |total: usize| format!("/{}", "h".repeat(total - tail.len() - 1));
        let h103 = home(103);
        assert!(resolve(&env(Some(&h103), &[]), Platform::MacOs).is_ok());
        let h104 = home(104);
        let err = resolve(&env(Some(&h104), &[]), Platform::MacOs).unwrap_err();
        assert!(matches!(err, PathError::TooLong(_)));
        assert!(err.to_string().contains("CIRCITOR_SOCKET"), "{err}");

        // Bytes, not characters: 52 two-byte characters are 52 characters and 104 bytes.
        let multi = format!("/{}", "é".repeat(51));
        assert_eq!(multi.chars().count(), 52);
        assert_eq!(multi.len(), 103);
        assert!(
            resolve(
                &env(Some("/h"), &[("CIRCITOR_SOCKET", &multi)]),
                Platform::Linux
            )
            .is_ok()
        );
        let multi = format!("/{}a", "é".repeat(51));
        assert_eq!(multi.len(), 104);
        assert!(
            resolve(
                &env(Some("/h"), &[("CIRCITOR_SOCKET", &multi)]),
                Platform::Linux
            )
            .is_err()
        );
        assert_eq!(MAX_SOCKET_PATH, 103);
    }

    // OPS-3: a home is needed only when a path needs it.
    #[test]
    fn ops3_no_home() {
        let err = resolve(&env(None, &[]), Platform::Linux).unwrap_err();
        assert!(matches!(err, PathError::NoHome));
        assert!(resolve(&env(None, &[]), Platform::MacOs).is_err());
        let all = env(
            None,
            &[
                ("XDG_CONFIG_HOME", "/x/cfg"),
                ("XDG_STATE_HOME", "/x/state"),
                ("XDG_RUNTIME_DIR", "/x/run"),
            ],
        );
        assert!(resolve(&all, Platform::Linux).is_ok());
        // Config alone still needs the home for the state directory.
        let some = env(None, &[("XDG_CONFIG_HOME", "/x/cfg")]);
        assert!(resolve(&some, Platform::Linux).is_err());
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    // OPS-3: private directories.
    #[test]
    fn ops3_private_dir() {
        let dir = ShortDir::new();
        let deep = dir.path().join("a/b/c");
        ensure_private_dir(&deep).unwrap();
        assert_eq!(mode(&deep), 0o700);

        let old = dir.path().join("old");
        std::fs::create_dir(&old).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&old).unwrap();
        assert_eq!(mode(&old), 0o700);

        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let err = ensure_private_dir(&file).unwrap_err();
        assert!(matches!(err, DirError::NotADirectory(_)));
        assert!(err.to_string().contains(&file.display().to_string()));

        if !nix::unistd::geteuid().is_root() {
            let usr = Path::new("/usr");
            let before = mode(usr);
            let err = ensure_private_dir(usr).unwrap_err();
            assert!(matches!(err, DirError::NotOwned(_)));
            assert!(err.to_string().contains("/usr"), "{err}");
            assert_eq!(mode(usr), before);
        }
    }
}
