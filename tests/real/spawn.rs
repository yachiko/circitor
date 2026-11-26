//! The CLI starts a daemon that outlives it (CLI-7, STA-11, OPS-4, OPS-5, NTF-9, OPS-1).

use std::process::Stdio;

use circitor::cli::notify::{SOCKET_VAR, TOKEN_VAR};
use circitor::testkit::{MARKER_SESSION_TOKEN, MARKER_TOKEN, MARKER_TOKEN_2};
use nix::sys::signal::Signal;
use tokio::io::AsyncReadExt;

use crate::fixture::{Out, alive, real, signal};
use crate::{CONFIG, CONFIG_STAY, ms, s};

// TST-9:
// CLI-7: `list` with no daemon exits 0, and the daemon it started lives on: the PID in the lock
// file is alive, and the lock is held.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli7_list_starts_a_daemon_that_lives_on() {
    real(CONFIG, |fx| async move {
        assert!(!fx.lock_held());
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        assert!(out.stdout.contains("o/r#1"), "{out:?}");
        let pid = fx.daemon_pid();
        assert!(alive(pid), "the daemon {pid} is gone");
        assert!(fx.lock_held());
        // Still there a moment after the CLI is gone.
        fx.sleep(ms(500)).await;
        assert!(alive(pid) && fx.lock_held());
        assert!(fx.socket().exists());
    })
    .await;
}

// TST-9:
// CLI-7: the CLI's stdout, read to its end, closes when the CLI exits while the daemon lives on.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli7_stdout_closes_when_the_cli_exits() {
    real(CONFIG, |fx| async move {
        let mut cmd = fx.command(&["list"], &[]);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let started = fx.mono();
        let mut child = cmd.spawn().expect("the CLI starts");
        let mut stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");
        let clock = fx.clock.clone();
        let reader = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf).await;
            (buf, clock.mono())
        });
        let errs = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            buf
        });
        let status = child.wait().await.expect("the CLI's status");
        let exited = fx.mono();
        let (buf, eof) = circitor::clock::timeout(&fx.clock, s(2), reader)
            .await
            .expect("stdout reached its end within 2 s of the exit")
            .expect("the reader");
        let err = errs.await.expect("stderr");
        let out = Out {
            stdout: String::from_utf8_lossy(&buf).into_owned(),
            stderr: String::from_utf8_lossy(&err).into_owned(),
            code: status.code(),
            took: exited.since(started),
        };
        fx.record(&out);
        assert_eq!(out.code, Some(0), "{out:?}");
        assert!(
            eof.since(exited) < s(1),
            "EOF {:?} after the exit",
            eof.since(exited)
        );
        let pid = fx.daemon_pid();
        assert!(alive(pid) && fx.lock_held(), "the daemon lives on");
    })
    .await;
}

// TST-9:
// CLI-7, STA-11: two CLIs at once leave one daemon; one more `list` connects to it.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_two_clis_one_daemon() {
    real(CONFIG, |fx| async move {
        let a = fx.cli_bg(&["list"], &[]);
        let b = fx.cli_bg(&["list"], &[]);
        let a = fx.finish_bg(a, s(20)).await;
        let b = fx.finish_bg(b, s(20)).await;
        assert_eq!(a.code, Some(0), "{a:?}");
        assert_eq!(b.code, Some(0), "{b:?}");
        let pid = fx.daemon_pid();
        assert!(alive(pid) && fx.lock_held());
        let c = fx.cli(&["list"]).await;
        assert_eq!(c.code, Some(0), "{c:?}");
        assert_eq!(
            fx.daemon_pid(),
            pid,
            "the third list reached the same daemon"
        );
        assert!(alive(pid));
    })
    .await;
}

// TST-9:
// OPS-4: a stale socket file is replaced. After a `SIGKILL` the socket file stays; the next `list`
// exits 0 on a new daemon.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops4_stale_socket_is_replaced() {
    real(CONFIG, |fx| async move {
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let old = fx.daemon_pid();
        signal(old, Signal::SIGKILL);
        fx.until("the killed daemon is gone", s(5), || !alive(old))
            .await;
        assert!(fx.socket().exists(), "the socket file stays after SIGKILL");
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let new = fx.daemon_pid();
        assert_ne!(new, old);
        assert!(alive(new) && fx.lock_held());
    })
    .await;
}

// TST-9:
// OPS-5, CLI-7: a spawned daemon uses gh's login although `GH_TOKEN` was set to a second marker,
// which the fake GitHub never receives. NTF-9, CLI-7: the daemon's own environment holds neither
// `CLAUDE_CODE_MESSAGING_SOCKET` nor `CLAUDE_CODE_MESSAGING_TOKEN`.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops5_spawned_daemon_uses_gh_login() {
    real(CONFIG, |fx| async move {
        let session = fx.root().join("session.sock");
        let session = session.to_string_lossy().into_owned();
        let out = fx
            .cli_env(
                &["list"],
                &[
                    ("GH_TOKEN", MARKER_TOKEN_2),
                    (SOCKET_VAR, &session),
                    (TOKEN_VAR, MARKER_SESSION_TOKEN),
                ],
            )
            .await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let requests = fx.gh.requests();
        assert!(!requests.is_empty());
        for r in &requests {
            let auth = r.authorization.as_deref().unwrap_or("");
            assert!(
                auth.contains(MARKER_TOKEN) && !auth.contains(MARKER_TOKEN_2),
                "request {} wasn't sent with gh's login",
                r.index
            );
        }
        let pid = fx.daemon_pid();
        if cfg!(target_os = "macos") {
            eprintln!("TST-9: no /proc on macOS: the daemon's environment isn't checked");
        } else {
            let environ = std::fs::read(format!("/proc/{pid}/environ")).expect("the environ");
            let names: Vec<String> = environ
                .split(|b| *b == 0)
                .filter_map(|kv| {
                    let kv = String::from_utf8_lossy(kv);
                    kv.split_once('=').map(|(k, _)| k.to_string())
                })
                .collect();
            assert!(names.iter().any(|n| n == "HOME"), "{names:?}");
            for gone in [SOCKET_VAR, TOKEN_VAR, "GH_TOKEN", "GITHUB_TOKEN"] {
                assert!(
                    !names.iter().any(|n| n == gone),
                    "the daemon's environment holds {gone}"
                );
            }
        }
    })
    .await;
}

// TST-9:
// CLI-7, OPS-1: a bad config exits 6 within 1 s, and stderr holds the OPS-1 line naming the key.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli7_bad_config_exits_6_within_1_s() {
    real(CONFIG_STAY, |fx| async move {
        fx.config("exit_after = 60\nno_such_key = 1\n");
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(6), "{out:?}");
        assert!(out.stderr.contains("no_such_key"), "{out:?}");
        assert!(out.took < s(1), "it took {:?}", out.took);
        assert!(!fx.lock_held());
    })
    .await;
}
