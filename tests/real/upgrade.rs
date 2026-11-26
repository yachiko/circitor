//! One daemon replacing another: an older one by the CLI (CLI-9, CLI-10), a spawned one by a
//! foreground one (OPS-6).

use nix::sys::signal::Signal;

use crate::fixture::{alive, real, signal};
use crate::{CONFIG_STAY, s};

const OLD: [(&str, &str); 1] = [("CIRCITOR_TEST_VERSION", "0.1.0")];
const NEW: [(&str, &str); 1] = [("CIRCITOR_TEST_VERSION", "0.2.0")];

// TST-9:
// CLI-9, CLI-10: an older daemon is replaced, and a waiting CLI reconnects. Daemon and waiter are
// 0.1.0; a `list` of 0.2.0 replaces the daemon; the waiter starts none, reaches the new one, and
// exits 0 when the PR merges in the fake's world. `interval = 10`: a poll sees the merge in time.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli9_older_daemon_replaced_waiter_reconnects() {
    real("exit_after = 60\ninterval = 10\n", |fx| async move {
        let out = fx.cli_env(&["list"], &OLD).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        assert!(out.stdout.contains("o/r#1"), "{out:?}");
        let old = fx.daemon_pid();
        let waiter = fx.cli_bg(&["wait", "o/r#1", "--until", "merged"], &OLD);
        fx.until_waiters(1).await;

        let out = fx.cli_env(&["list"], &NEW).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let new = fx.daemon_pid();
        assert_ne!(new, old, "the old daemon wasn't replaced: {out:?}");
        assert!(!alive(old), "the old daemon is alive");
        let st = fx.status().await;
        assert_eq!(st["version"], "0.2.0", "{st}");

        let pr = fx.pr;
        fx.gh.world(|w| w.merge(pr));
        let out = fx.finish_bg(waiter, s(20)).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        assert!(out.stdout.contains("merged"), "{out:?}");
        // The waiter started none: the 0.2.0 daemon still holds the lock.
        assert_eq!(fx.daemon_pid(), new);
        assert!(alive(new) && fx.lock_held());
    })
    .await;
}

// TST-9:
// OPS-6: a foreground daemon takes over a spawned one: within 15 s `status` reads
// `mode: foreground` and the spawned process is gone. A second foreground daemon exits non-zero,
// naming the holder's socket.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops6_foreground_takes_over() {
    real(CONFIG_STAY, |fx| async move {
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let spawned = fx.daemon_pid();
        let mut fg = fx.daemon_fg(&[]);
        let fg_pid = fg.id().expect("its PID");
        let start = fx.mono();
        loop {
            // OPS-6's 15 s is STA-11's (proposal): a second of slack above.
            assert!(
                fx.mono().since(start) < s(16),
                "no takeover within 15 s: {}",
                fx.fg_log(0)
            );
            if !alive(spawned) && fx.holder_pid() == Some(fg_pid) && fx.socket().exists() {
                let st = fx.status().await;
                if st["mode"] == "foreground" {
                    break;
                }
            }
            fx.sleep(crate::ms(20)).await;
        }

        let mut second = fx.command(&["daemon"], &[]);
        let out = fx
            .run_to_end(&mut second, "a second foreground daemon")
            .await;
        assert_ne!(out.code, Some(0), "{out:?}");
        let socket = fx.socket().to_string_lossy().into_owned();
        assert!(out.stderr.contains(&socket), "{out:?}");
        assert_eq!(fx.daemon_pid(), fg_pid);

        signal(fg_pid, Signal::SIGTERM);
        let status = circitor::clock::timeout(&fx.clock, s(6), fg.wait())
            .await
            .expect("it exits within 6 s")
            .expect("its status");
        assert_eq!(status.code(), Some(0));
    })
    .await;
}
