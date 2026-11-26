//! The CLI's own signals (CLI-5, CLI-6).

use nix::sys::signal::Signal;

use crate::fixture::{real, signal};
use crate::{CONFIG_STAY, s};

// TST-9:
// CLI-5, CLI-6: a held `wait` sent `SIGINT` exits 130, sent `SIGTERM` 143, each within 1 s.
#[tokio::test(flavor = "multi_thread")]
async fn tst9_cli5_exit_codes_130_and_143() {
    real(CONFIG_STAY, |fx| async move {
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        assert!(out.stdout.contains("o/r#1"), "{out:?}");
        for (sig, code) in [(Signal::SIGINT, 130), (Signal::SIGTERM, 143)] {
            let bg = fx.cli_bg(&["wait", "o/r#1", "--until", "merged"], &[]);
            fx.until_waiters(1).await;
            let pid = bg.pid();
            let sent = fx.mono();
            signal(pid, sig);
            let out = fx.finish_bg(bg, s(2)).await;
            let after = fx.mono().since(sent);
            assert_eq!(out.code, Some(code), "{sig}: {out:?}");
            assert!(after < s(1), "{sig}: exited {after:?} after the signal");
            fx.until_waiters(0).await;
        }
    })
    .await;
}
