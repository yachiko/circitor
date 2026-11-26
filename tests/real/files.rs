//! Files on a real file system: modes, what's written, the socket path, the log (OPS-3, OPS-4,
//! STA-7, STA-12, OPS-9).

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::process::Stdio;

use nix::sys::signal::Signal;

use crate::fixture::{alive, real, signal};
use crate::{CONFIG_STAY, s};

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .permissions()
        .mode()
        & 0o7777
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

// TST-9:
// OPS-3, OPS-4, STA-7: the state directory and, on Linux, the runtime directory are 0700; an
// existing 0755 state directory becomes 0700; the socket and `state.json` are 0600.
// STA-12: the state directory holds nothing but the files it lists.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops3_modes_and_files() {
    real(CONFIG_STAY, |fx| async move {
        let state_dir = fx.state_dir();
        std::fs::create_dir_all(&state_dir).expect("the state directory");
        std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o755)).expect("0755");
        assert_eq!(mode(&state_dir), 0o755);

        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let pid = fx.daemon_pid();
        assert_eq!(mode(&state_dir), 0o700, "the state directory");
        if !cfg!(target_os = "macos") {
            assert_eq!(mode(&fx.runtime_dir()), 0o700, "the runtime directory");
            // OPS-4: the runtime directory holds the socket and nothing else.
            assert_eq!(names(&fx.runtime_dir()), ["circitor.sock"]);
        }
        let socket = std::fs::symlink_metadata(fx.socket()).expect("the socket");
        assert!(socket.file_type().is_socket());
        assert_eq!(socket.permissions().mode() & 0o7777, 0o600, "the socket");

        // `state.json` is written at the latest at the stop (STA-7).
        let out = fx.cli(&["stop"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        fx.gone(pid).await;
        assert_eq!(mode(&fx.state_file()), 0o600, "state.json");

        // STA-12.
        let mut allowed = vec![
            "circitor.lock",
            "circitor.log",
            "circitor.log.1",
            "state.json",
            "state.json.bad",
            "state.json.tmp",
        ];
        if cfg!(target_os = "macos") {
            allowed.push("circitor.sock");
        }
        for name in names(&state_dir) {
            assert!(
                allowed.contains(&name.as_str()),
                "STA-12: {name} in the state directory"
            );
        }
        if !cfg!(target_os = "macos") {
            assert!(names(&fx.runtime_dir()).is_empty());
        }
        let root_names = names(fx.root());
        for name in root_names {
            assert!(
                ["bin", "config", "gh", "home", "run", "state"].contains(&name.as_str()),
                "STA-12: {name} in the test's directory"
            );
        }
    })
    .await;
}

// TST-9:
// OPS-4: a socket path over 103 bytes is refused by the CLI and by the daemon, naming
// `CIRCITOR_SOCKET`.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops4_long_socket_path_refused() {
    real(CONFIG_STAY, |fx| async move {
        let long = fx.root().join(format!("{}.sock", "s".repeat(100)));
        let long = long.to_string_lossy().into_owned();
        assert!(long.len() > 103);
        let env = [("CIRCITOR_SOCKET", long.as_str())];
        let out = fx.cli_env(&["list"], &env).await;
        assert_ne!(out.code, Some(0), "{out:?}");
        assert!(out.stderr.contains("CIRCITOR_SOCKET"), "{out:?}");

        let mut daemon = fx.command(&["daemon"], &env);
        let out = fx.run_to_end(&mut daemon, "a daemon on a long path").await;
        assert_ne!(out.code, Some(0), "{out:?}");
        assert!(out.stderr.contains("CIRCITOR_SOCKET"), "{out:?}");
        assert!(!fx.lock_held());
        assert!(!Path::new(&long).exists());
    })
    .await;
}

// TST-9:
// OPS-4: a regular file at the socket path stops the daemon, naming the path.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops4_regular_file_at_socket_path() {
    real(CONFIG_STAY, |fx| async move {
        let dir = fx.runtime_dir();
        std::fs::create_dir_all(&dir).expect("the socket's directory");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("0700");
        std::fs::write(fx.socket(), "not a socket").expect("a regular file");

        let mut daemon = fx.command(&["daemon"], &[]);
        let out = fx.run_to_end(&mut daemon, "a daemon over a file").await;
        assert_ne!(out.code, Some(0), "{out:?}");
        let path = fx.socket().to_string_lossy().into_owned();
        assert!(out.stderr.contains(&path), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(fx.socket()).expect("the file is left"),
            "not a socket"
        );
        assert!(!fx.lock_held());
    })
    .await;
}

// TST-9:
// OPS-9, API-5: a foreground daemon whose stderr is a pipe with its reading end closed keeps
// serving, and `status --json` holds `log_unwritable`.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops9_log_unwritable() {
    real(CONFIG_STAY, |fx| async move {
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let mut fg = fx.daemon_fg_to(&[], Stdio::from(writer));
        let pid = fg.id().expect("its PID");
        fx.until("the foreground daemon's socket", s(10), || {
            fx.socket().exists()
        })
        .await;
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let start = fx.mono();
        loop {
            let st = fx.status().await;
            let raised = st["warnings"]
                .as_array()
                .is_some_and(|w| w.iter().any(|w| w["code"] == "log_unwritable"));
            if raised {
                break;
            }
            assert!(fx.mono().since(start) < s(5), "no log_unwritable: {st}");
            fx.sleep(crate::ms(20)).await;
        }
        assert!(alive(pid));
        signal(pid, Signal::SIGTERM);
        let status = circitor::clock::timeout(&fx.clock, s(6), fg.wait())
            .await
            .expect("it exits within 6 s")
            .expect("its status");
        assert_eq!(status.code(), Some(0));
    })
    .await;
}
