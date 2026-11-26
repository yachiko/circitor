//! The token reader against the fake `gh`'s outcomes (OPS-5): no login, a failure, a hang.

use crate::fixture::{Fixture, Gh, Out, alive, real};
use crate::{CONFIG_STAY, s};

/// `list` until it answers with the token's problem: exit 7 (a first `list` may meet API-18's
/// 10 s and answer `starting`, exit 9, while `gh` still runs).
async fn auth_answer(fx: &Fixture) -> Out {
    for _ in 0..3 {
        let out = fx.cli(&["list"]).await;
        match out.code {
            Some(7) => return out,
            Some(9) => continue,
            _ => panic!("list: {out:?}"),
        }
    }
    panic!("list never answered auth");
}

// TST-9:
// OPS-5: no login gives exit 7, naming `gh auth login`.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops5_gh_no_login() {
    real(CONFIG_STAY, |fx| async move {
        fx.set_gh(Gh::NoLogin);
        let out = auth_answer(&fx).await;
        assert!(out.stderr.contains("gh auth login"), "{out:?}");
        assert!(fx.gh.requests().is_empty(), "a request without a token");
    })
    .await;
}

// TST-9:
// OPS-5: a `gh` that exits 3 gives exit 7 naming `gh auth token`, the status and `gh_path`.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops5_gh_fails() {
    real(CONFIG_STAY, |fx| async move {
        fx.set_gh(Gh::Exit3);
        let out = auth_answer(&fx).await;
        let gh = fx.gh_path().to_string_lossy().into_owned();
        assert!(out.stderr.contains("gh auth token"), "{out:?}");
        assert!(out.stderr.contains("exit status 3"), "{out:?}");
        assert!(out.stderr.contains(&gh), "{out:?}");
        assert!(!out.stderr.contains("gh auth login"), "{out:?}");
    })
    .await;
}

// TST-9:
// OPS-5: a `gh` that never exits is killed after 10 s (its recorded PID is dead), and the message
// names the timeout.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_ops5_gh_hang_killed_after_10_s() {
    real(CONFIG_STAY, |fx| async move {
        fx.set_gh(Gh::Hang);
        let started = fx.mono();
        let first = fx.cli_bg(&["list"], &[]);
        fx.until("the fake gh records its PID", s(5), || {
            fx.gh_pid().is_some()
        })
        .await;
        let gh = fx.gh_pid().expect("the PID");
        // Its first PID only: a later read may start another.
        fx.until("the hanging gh is killed", s(12), || !alive(gh))
            .await;
        let killed = fx.mono().since(started);
        assert!(killed >= s(10) - s(1), "killed after {killed:?}");
        let out = fx.finish_bg(first, s(5)).await;
        let out = if out.code == Some(7) {
            out
        } else {
            assert_eq!(out.code, Some(9), "{out:?}");
            auth_answer(&fx).await
        };
        assert!(out.stderr.contains("timed out"), "{out:?}");
        assert!(out.stderr.contains("gh auth token"), "{out:?}");
        // OPS-7: a stop kills a running `gh` too.
        let out = fx.cli(&["stop"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
    })
    .await;
}
