//! Stops: `stop`, `SIGTERM`, the idle exit, the 5 s deadline, a stopped daemon, and waiters that
//! start none (OPS-7, OPS-8, CLI-8, CLI-10).

use std::sync::Arc;

use nix::sys::signal::Signal;

use crate::fixture::{Fixture, alive, real, signal};
use crate::{CONFIG_STAY, ms, s};

/// OPS-7: no socket, a valid state file, the lock free.
fn stopped_cleanly(fx: &Fixture) {
    assert!(!fx.socket().exists(), "the socket is still there");
    let text = std::fs::read_to_string(fx.state_file()).expect("state.json is there");
    let json: serde_json::Value = serde_json::from_str(&text).expect("state.json parses");
    assert_eq!(json["version"], 1, "state.json's version");
    assert!(!fx.lock_held(), "the lock is still held");
}

/// `list`, which must start a daemon; its PID.
async fn started(fx: &Fixture) -> u32 {
    let out = fx.cli(&["list"]).await;
    assert_eq!(out.code, Some(0), "{out:?}");
    assert!(out.stdout.contains("o/r#1"), "{out:?}");
    fx.daemon_pid()
}

// TST-9:
// OPS-7, CLI-8: `stop` leaves no socket and a valid state file, and the lock free.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops7_stop_leaves_no_socket() {
    real(CONFIG_STAY, |fx| async move {
        let pid = started(&fx).await;
        let out = fx.cli(&["stop"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        fx.gone(pid).await;
        stopped_cleanly(&fx);
        assert!(fx.log().contains(" stop cause=request"), "{}", fx.log());
    })
    .await;
}

// TST-9:
// OPS-7: `SIGTERM` leaves no socket and a valid state file, and the lock free.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops7_sigterm_leaves_no_socket() {
    real(CONFIG_STAY, |fx| async move {
        let pid = started(&fx).await;
        signal(pid, Signal::SIGTERM);
        fx.until("the daemon exits on SIGTERM", s(6), || !alive(pid))
            .await;
        stopped_cleanly(&fx);
        assert!(fx.log().contains(" stop cause=signal"), "{}", fx.log());
    })
    .await;
}

// TST-9:
// OPS-7: a foreground daemon the test started exits 0 on `SIGTERM`, leaving no socket.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops7_foreground_exits_0_on_sigterm() {
    real(CONFIG_STAY, |fx| async move {
        let mut fg = fx.daemon_fg(&[]);
        let pid = fg.id().expect("its PID");
        fx.until("the foreground daemon's socket", s(10), || {
            fx.socket().exists()
        })
        .await;
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        assert_eq!(fx.daemon_pid(), pid);
        signal(pid, Signal::SIGTERM);
        let status = circitor::clock::timeout(&fx.clock, s(6), fg.wait())
            .await
            .expect("it exits within 6 s")
            .expect("its status");
        assert_eq!(status.code(), Some(0), "{status:?}");
        stopped_cleanly(&fx);
    })
    .await;
}

// TST-9:
// OPS-8: a spawned daemon with `exit_after = 2` is gone within 10 s of its last request.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops8_idle_exit() {
    real("exit_after = 2\n", |fx| async move {
        // The request arrives after the CLI starts and is answered before it exits: `exit_after` is
        // counted from within that span.
        let asked = fx.mono();
        let pid = started(&fx).await;
        let answered = fx.mono();
        fx.until("the idle daemon exits", s(10), || !alive(pid))
            .await;
        let after = fx.mono().since(asked);
        assert!(
            after >= s(2),
            "it exited {after:?} after its last request began"
        );
        assert!(fx.mono().since(answered) <= s(10));
        assert!(fx.log().contains(" stop cause=idle"), "{}", fx.log());
        stopped_cleanly(&fx);
    })
    .await;
}

// TST-9:
// OPS-8, OPS-6: a foreground daemon with `exit_after = 2` doesn't exit for idleness.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops8_foreground_never_idles() {
    real("exit_after = 2\n", |fx| async move {
        let mut fg = fx.daemon_fg(&[]);
        let pid = fg.id().expect("its PID");
        fx.until("the foreground daemon's socket", s(10), || {
            fx.socket().exists()
        })
        .await;
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        // A negative check: polled, it must stay alive for 5 s, over twice `exit_after`.
        let start = fx.mono();
        while fx.mono().since(start) < s(5) {
            assert!(alive(pid), "the foreground daemon exited");
            fx.sleep(ms(20)).await;
        }
        signal(pid, Signal::SIGTERM);
        let status = circitor::clock::timeout(&fx.clock, s(6), fg.wait())
            .await
            .expect("it exits within 6 s")
            .expect("its status");
        assert_eq!(status.code(), Some(0));
    })
    .await;
}

/// A daemon with `o/r#1` listed, and a `wait` held on it.
async fn held_wait(fx: &Arc<Fixture>) -> (u32, crate::fixture::Bg) {
    let pid = started(fx).await;
    let bg = fx.cli_bg(&["wait", "o/r#1", "--until", "merged"], &[]);
    fx.until_waiters(1).await;
    (pid, bg)
}

/// CLI-10: the waiter ends `daemon stopped`, exit 6, having started none.
async fn ends_daemon_stopped(fx: &Fixture, bg: crate::fixture::Bg) {
    let out = fx.finish_bg(bg, s(15)).await;
    assert_eq!(out.code, Some(6), "{out:?}");
    assert!(out.text().contains("daemon stopped"), "{out:?}");
    assert!(!fx.lock_held(), "a daemon holds the lock");
    if let Some(pid) = fx.holder_pid() {
        assert!(!alive(pid), "the holder {pid} is alive");
    }
}

// TST-9:
// CLI-10, OPS-7: after a `stop`, a waiting CLI starts none: it ends `daemon stopped`, exit 6.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli10_waiter_after_stop_starts_none() {
    real(CONFIG_STAY, |fx| async move {
        let (pid, bg) = held_wait(&fx).await;
        let out = fx.cli(&["stop"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        fx.gone(pid).await;
        ends_daemon_stopped(&fx, bg).await;
    })
    .await;
}

// TST-9:
// CLI-10: a daemon killed with `SIGKILL` while a CLI waits: the CLI starts none and ends
// `daemon stopped`, exit 6.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli10_waiter_after_sigkill_starts_none() {
    real(CONFIG_STAY, |fx| async move {
        let (pid, bg) = held_wait(&fx).await;
        signal(pid, Signal::SIGKILL);
        fx.until("the killed daemon is gone", s(5), || !alive(pid))
            .await;
        ends_daemon_stopped(&fx, bg).await;
    })
    .await;
}

// TST-9:
// OPS-7: the stop's 5 s deadline. With `CIRCITOR_TEST_STOP_DELAY=30` the socket is gone at once,
// the process 5 s to 7 s after `stop` was sent, and the log holds a `stop deadline` line.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops7_stop_deadline() {
    real(CONFIG_STAY, |fx| async move {
        let out = fx
            .cli_env(&["list"], &[("CIRCITOR_TEST_STOP_DELAY", "30")])
            .await;
        assert_eq!(out.code, Some(0), "{out:?}");
        let pid = fx.daemon_pid();
        let sent = fx.mono();
        let stop = fx.cli_bg(&["stop"], &[]);
        fx.until("the socket is removed at once", s(1), || {
            !fx.socket().exists()
        })
        .await;
        fx.until("the daemon exits by the deadline", s(8), || !alive(pid))
            .await;
        let gone = fx.mono().since(sent);
        // OPS-7's 5 s is (proposal): a second of slack above, none below.
        assert!(gone >= s(5) && gone <= s(7) + s(1), "gone after {gone:?}");
        let out = fx.finish_bg(stop, s(10)).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        assert!(fx.log().contains(" stop deadline"), "{}", fx.log());
        assert!(!fx.lock_held());
    })
    .await;
}

// TST-9:
// CLI-8: `stop` on a daemon sent `SIGSTOP`, with the CLI's limit at 2 s: no answer, `SIGTERM`,
// `SIGKILL` 5 s later; the process is dead and `stop` exits 0.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli8_stop_on_a_stopped_daemon() {
    real(CONFIG_STAY, |fx| async move {
        let pid = started(&fx).await;
        signal(pid, Signal::SIGSTOP);
        let out = fx
            .cli_env(&["stop"], &[("CIRCITOR_TEST_CLI_LIMIT", "2")])
            .await;
        assert_eq!(out.code, Some(0), "{out:?}");
        fx.gone(pid).await;
        // 2 s without an answer, then 5 s between `SIGTERM` and `SIGKILL` (CLI-8; a second of slack).
        assert!(
            out.took >= s(7) && out.took <= s(8) + s(1),
            "stop took {:?}",
            out.took
        );
        assert!(!fx.lock_held());
    })
    .await;
}
