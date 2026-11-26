#![cfg(feature = "test-endpoint")]
//! T29: the harness, and the CLI against a running daemon. Every test runs on the test clock and
//! ends with `h.finish()` (TST-11). The world: `o/r#1`, open, its one check passed, `MERGEABLE`,
//! `updatedAt` two minutes before the start, by the fake's login; `o/x#2`, another author's.

use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::api::{Envelope, StatusBody};
use circitor::cli::notify::{SOCKET_VAR, TOKEN_VAR};
use circitor::clock::Clock;
use circitor::config::Config;
use circitor::daemon::Mode;
use circitor::log::LogSink;
use circitor::notify::{Posted, Poster, RealPoster, SessionToken};
use circitor::testkit::github::{PrId, World};
use circitor::testkit::harness::{
    BAD_CONFIG, Bg, Harness, Opts, spawner_bad_config, spawner_exits_ok, spawner_never_binds,
};
use circitor::testkit::server::{Aim, Fault, Times};
use circitor::testkit::{
    MARKER_CHECK, MARKER_LOGIN, MARKER_SESSION_TOKEN, MARKER_TITLE, MARKER_TOKEN, MARKER_TOKEN_2,
    TEST_VERSION,
};
use circitor::token::TokenRead;

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The world's two PRs.
#[derive(Clone, Copy)]
struct Prs {
    r1: PrId,
}

fn script(w: &mut World) -> Prs {
    let start = w.now();
    let r = w.repo("o", "r");
    let r1 = w.open_pr(r);
    let check = w.run(r1, &format!("{MARKER_CHECK} build"));
    w.finish(check, "SUCCESS");
    w.set_created_at(r1, start.minus(s(3600)));
    w.set_updated_at(r1, start.minus(s(120)));
    let x = w.repo("o", "x");
    for _ in 0..2 {
        let pr = w.open_pr(x);
        w.set_author(pr, Some("someone-else"));
        w.set_created_at(pr, start.minus(s(3600)));
        w.set_updated_at(pr, start.minus(s(600)));
    }
    Prs { r1 }
}

/// The harness on that world, with `opts` beside it.
async fn setup(opts: Opts) -> (Harness, Prs) {
    let slot = Arc::new(Mutex::new(None));
    let put = slot.clone();
    let h = Harness::start(opts.world(move |w| {
        *put.lock().unwrap() = Some(script(w));
    }))
    .await;
    let prs = slot.lock().unwrap().expect("the world was scripted");
    (h, prs)
}

/// Moves the clock in 100 ms steps until the command is done (at most 60 s), then awaits it.
async fn till_done(h: &Harness, bg: Bg) -> circitor::cli::Output {
    for _ in 0..600 {
        if bg.is_done() {
            break;
        }
        h.clock().advance(ms(100)).await;
    }
    assert!(bg.is_done(), "the command didn't end within 60 s");
    bg.await
}

async fn status(h: &Harness) -> Envelope<StatusBody> {
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    serde_json::from_str(&out.stdout).expect("status --json decodes")
}

fn json(out: &circitor::cli::Output) -> serde_json::Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

fn node_id(h: &Harness, pr: PrId) -> String {
    h.github().world(|w| w.node_id(pr).to_string())
}

// ---- the commands ----

// TST-5: every command against the in-process daemon (CLI-13).
#[tokio::test]
async fn m3_each_command() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let list = h.cli(&["list"]).await;
    assert_eq!(list.code, 0, "{list:?}");
    let last = list.stdout.lines().last().unwrap_or_default();
    assert!(last.starts_with("cursor: "), "{list:?}");
    for args in [
        &["show", "o/r#1"][..],
        &["changes"],
        &["track", "o/x#2"],
        &["untrack", "o/x#2"],
        &["refresh"],
        &["status"],
    ] {
        let out = till_done(&h, h.cli_bg(args)).await;
        assert_eq!(out.code, 0, "{args:?}: {out:?}");
    }
    let st = status(&h).await;
    assert_eq!(st.body.mode, Mode::Spawned);
    let stop = till_done(&h, h.cli_bg(&["stop"])).await;
    assert_eq!(stop.code, 0, "{stop:?}");
    assert!(!h.paths().socket.exists());
    assert_eq!(h.exited(), Some(0));
    h.finish().await;
}

// TST-5: a background wait the world satisfies.
#[tokio::test]
async fn m3_wait_is_satisfied() {
    let (h, p) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let wait = h.cli_bg(&["wait", "o/r#1", "--until", "merged"]);
    h.clock().advance(s(2)).await;
    assert!(!wait.is_done());
    h.github().world(|w| w.merge(p.r1));
    let out = till_done(&h, wait).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.lines().any(|l| l == "merged"), "{out:?}");
    h.finish().await;
}

// POL-14, API-13: a waiter makes its PR hot: refreshed on arrival, then every hot_interval.
#[tokio::test]
async fn m3_waiter_makes_hot() {
    let (h, p) = setup(Opts::default()).await;
    let id = node_id(&h, p.r1);
    // Outside POL-15's 5 s window of the first poll's refresh.
    h.clock().advance(s(20)).await;
    let began = h.clock().mono();
    let wait = h.cli_bg(&["wait", "o/r#1", "--until", "merged"]);
    h.clock().advance(s(60)).await;
    let naming: Vec<_> = h
        .github()
        .requests()
        .into_iter()
        .filter(|r| r.mono >= began && r.node_ids().contains(&id.as_str()))
        .collect();
    let first = naming
        .first()
        .expect("a request names it after the wait began");
    assert!(first.mono.since(began) <= s(1), "{first:?}");
    let gaps: Vec<Duration> = naming
        .windows(2)
        .map(|w| w[1].mono.since(w[0].mono))
        .collect();
    assert!(gaps.len() >= 5, "{gaps:?}");
    for g in &gaps {
        assert!(*g >= s(10) && *g <= s(12), "{gaps:?}");
    }
    wait.cancel().await;
    let ended = h.clock().mono();
    h.clock().advance(s(60)).await;
    for r in h.github().requests() {
        if r.mono > ended && r.node_ids().contains(&id.as_str()) {
            assert!(r.finding.is_some(), "a hot refresh after the cancel: {r:?}");
        }
    }
    h.finish().await;
}

// POL-16: an untracked PR is read through, by name, and tracked as `asked`.
#[tokio::test]
async fn m3_show_reads_through() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let before = h.github().requests().len();
    let out = till_done(&h, h.cli_bg(&["show", "o/x#2"])).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.lines().any(|l| l.contains("o/x#2")), "{out:?}");
    let lookups: Vec<_> = h.github().requests()[before..]
        .iter()
        .filter(|r| r.lookup == Some(("o".into(), "x".into(), 2)))
        .cloned()
        .collect();
    assert_eq!(lookups.len(), 1, "{lookups:?}");
    assert!(lookups[0].finding.is_none(), "{lookups:?}");
    assert_eq!(status(&h).await.body.tracked.asked, 1);
    h.finish().await;
}

// API-14, TST-5: a cancelled wait's connection is closed, and its waiter goes with it.
#[tokio::test]
async fn m3_cancel_frees_the_waiter() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let wait = h.cli_bg(&["wait", "o/r#1", "--until", "merged"]);
    h.clock().settle().await;
    assert_eq!(status(&h).await.body.waiters, 1);
    assert!(!wait.is_done());
    wait.cancel().await;
    assert_eq!(status(&h).await.body.waiters, 0);
    h.finish().await;
}

// CLI-6: a wait that sees nothing ends at its 100 s with exit 3 and its three lines.
#[tokio::test]
async fn m3_wait_times_out() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let wait = h.cli_bg(&["wait", "o/r#1", "--until", "merged"]);
    h.clock().advance(s(99)).await;
    assert!(!wait.is_done());
    h.clock().advance(s(1)).await;
    let out = till_done(&h, wait).await;
    assert!(h.clock().mono().as_duration() <= s(106));
    assert_eq!(out.code, 3, "{out:?}");
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{out:?}");
    assert!(
        lines[0].starts_with("timeout after 100s: o/r#1 "),
        "{out:?}"
    );
    let cursor = lines[2].strip_prefix("cursor: ").expect("the cursor line");
    assert_eq!(
        lines[1],
        format!("again: circitor wait o/r#1 --until merged --since {cursor}")
    );
    assert_eq!(status(&h).await.body.waiters, 0);
    h.finish().await;
}

// ---- starting and replacing ----

// CLI-7: a command starts the daemon through the spawner, once.
#[tokio::test]
async fn m3_command_starts_the_daemon() {
    let (h, _) = setup(Opts::default().no_daemon()).await;
    let out = till_done(&h, h.cli_bg(&["list"])).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(h.spawns(), 1);
    let again = till_done(&h, h.cli_bg(&["list"])).await;
    assert_eq!(again.code, 0, "{again:?}");
    assert_eq!(h.spawns(), 1);
    h.finish().await;
}

// CLI-7: a daemon that exits with an error, one that never binds, one that exits 0 with no holder,
// and `--no-spawn`.
#[tokio::test]
async fn m3_spawner_fails() {
    let (h, _) = setup(Opts::default().no_daemon()).await;

    let began = h.clock().mono();
    let bad = h.cmd(&["list"]).spawner(spawner_bad_config()).bg();
    let out = till_done(&h, bad).await;
    assert_eq!(out.code, 6, "{out:?}");
    assert!(h.clock().mono().since(began) < s(1));
    assert!(out.stderr.contains(BAD_CONFIG), "{out:?}");

    for spawner in [spawner_never_binds(), spawner_exits_ok()] {
        let began = h.clock().mono();
        let out = till_done(&h, h.cmd(&["list"]).spawner(spawner).bg()).await;
        assert_eq!(out.code, 6, "{out:?}");
        let took = h.clock().mono().since(began);
        assert!(took >= s(8) && took <= ms(8_200), "{took:?}");
    }

    let out = h.cli(&["list", "--no-spawn"]).await;
    assert_eq!(out.code, 6, "{out:?}");
    assert_eq!(h.spawns(), 0);
    h.finish().await;
}

// CLI-9: an older daemon is replaced by one the spawner starts.
#[tokio::test]
async fn m3_older_daemon_is_replaced() {
    let (h, _) = setup(Opts::default().version("1.2.2+a")).await;
    h.clock().advance(s(5)).await;
    let out = till_done(&h, h.cli_bg(&["list"])).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(h.spawns(), 1);
    assert_eq!(status(&h).await.version, TEST_VERSION);
    h.finish().await;
}

// CLI-9: a newer, a foreground and an unparsable one are kept, with a warning.
#[tokio::test]
async fn m3_other_daemons_are_kept() {
    for (version, mode) in [
        ("1.2.4", Mode::Spawned),
        ("1.2.2+a", Mode::Foreground),
        ("banana", Mode::Spawned),
    ] {
        let (h, _) = setup(Opts::default().version(version).mode(mode)).await;
        h.clock().advance(s(5)).await;
        let out = h.cli(&["list"]).await;
        assert_eq!(out.code, 0, "{version}: {out:?}");
        assert!(out.stderr.contains("warning"), "{version}: {out:?}");
        assert_eq!(h.spawns(), 0, "{version}");
        h.finish().await;
    }
}

// CLI-2, TST-5: a bare PR comes from the repository's branch.
#[tokio::test]
async fn m3_bare_pr() {
    let (h, p) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let branch = "branch-1";
    assert_eq!(
        h.github().world(|w| w.number(p.r1)),
        1,
        "the PR's head branch is branch-<number>"
    );
    let repo = h.git_repo("https://github.com/o/r.git", branch).await;
    let out = till_done(&h, h.cmd(&["show"]).cwd(&repo).bg()).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout.lines().any(|l| l.starts_with("o/r#1")),
        "{out:?}"
    );
    let outside = till_done(&h, h.cli_bg(&["show"])).await;
    assert_eq!(outside.code, 2, "{outside:?}");
    h.finish().await;
}

// OPS-5, TST-5: no login, and a slow token source.
#[tokio::test]
async fn m3_token_outcomes() {
    let (h, _) = setup(Opts::default().token(TokenRead::NoLogin)).await;
    let out = till_done(&h, h.cli_bg(&["list"])).await;
    assert_eq!(out.code, 7, "{out:?}");
    assert!(out.stderr.contains("gh auth login"), "{out:?}");
    h.finish().await;

    let (h, _) = setup(Opts::default().token_delay(s(9))).await;
    h.clock().advance(s(2)).await;
    assert_eq!(h.token().reads(), 1);
    let st = status(&h).await;
    assert!(st.body.last_poll.is_none(), "the read is still held");
    assert!(h.github().requests().is_empty());
    h.finish().await;
}

// TST-5, STA-9: a crash leaves the socket; a restart serves again, with another epoch.
#[tokio::test]
async fn m3_crash_and_restart() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let first = h.cli(&["list", "--json"]).await;
    assert_eq!(first.code, 0, "{first:?}");
    let cursor = json(&first)["cursor"].as_str().unwrap().to_string();
    h.crash().await;
    assert!(h.paths().socket.exists());
    h.restart().await;
    let out = till_done(&h, h.cli_bg(&["list", "--json"])).await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    let named = |key: &str| {
        v[key].as_array().is_some_and(|a| {
            a.iter()
                .any(|x| x == "o/r#1" || x["pr"] == "o/r#1" || x["name"] == "o/r#1")
        })
    };
    assert!(named("records") || named("pending"), "{v}");
    let old = till_done(&h, h.cli_bg(&["list", "--since", &cursor])).await;
    assert_eq!(old.code, 0, "{old:?}");
    assert_eq!(
        old.stdout.lines().next(),
        Some("reset: cursor too old, current state follows"),
        "{old:?}"
    );
    h.finish().await;
}

// OPS-8, TST-5: the daemon's own exit is visible, and a restart follows it.
#[tokio::test]
async fn m3_own_exit_and_restart() {
    let (h, _) = setup(Opts::default()).await;
    h.restart_with(Config {
        exit_after: Some(s(1)),
        ..Config::default()
    })
    .await;
    assert_eq!(h.exited(), None);
    h.clock().advance(s(5)).await;
    assert_eq!(h.exited(), Some(0));
    h.restart().await;
    let out = h.cli(&["status"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    h.finish().await;
}

// TST-5: a fault and a world set in `opts` hold from the daemon's first request.
#[tokio::test]
async fn m3_opts_hold_from_the_first_request() {
    let (h, _) =
        setup(Opts::default().fault(Fault::Status(502), Aim::All, Times::Requests(1))).await;
    h.clock().advance(s(30)).await;
    let requests = h.github().requests();
    assert_eq!(requests[0].fault, Some(Fault::Status(502)), "{requests:?}");
    assert!(requests.len() > 1, "{requests:?}");
    let out = till_done(&h, h.cli_bg(&["list", "--json"])).await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    let names: Vec<String> = v["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["pr"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(names, ["o/r#1"], "{v}");
    h.finish().await;
}

// TST-7: what only a raw request can send.
#[tokio::test]
async fn m3_api_raw() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let (status, body) = h.api_raw("GET", "/v1/prs?nope=1", None).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("bad_request"), "{body}");
    let (status, body) = h
        .api_raw("GET", "/v1/prs/o/r/1/wait?until=merged&timeout=3601", None)
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("bad_request"), "{body}");
    h.finish().await;
}

// ---- the sweep (TST-11) ----

// TST-11: a token in a command's output fails the test.
#[tokio::test]
#[should_panic(expected = "TST-11")]
async fn m3_sweep_finds_a_token() {
    let (h, _) = setup(Opts::default()).await;
    let out = h.cli(&["show", MARKER_TOKEN]).await;
    assert_eq!(out.code, 2, "{out:?}");
    assert!(out.stderr.contains(MARKER_TOKEN));
    h.finish().await;
}

#[tokio::test]
#[should_panic(expected = "TST-11")]
async fn m3_sweep_finds_the_second_token() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(5)).await;
    let (status, body) = h
        .api_raw("GET", &format!("/v1/prs?{MARKER_TOKEN_2}=1"), None)
        .await;
    assert_eq!(status, 400);
    assert!(body.contains(MARKER_TOKEN_2), "{body}");
    h.finish().await;
}

#[tokio::test]
#[should_panic(expected = "TST-11")]
async fn m3_sweep_finds_a_title_in_the_log() {
    let (h, _) = setup(Opts::default()).await;
    h.log()
        .write_line(&format!("info planted {MARKER_TITLE}"))
        .unwrap();
    h.finish().await;
}

#[tokio::test]
#[should_panic(expected = "without finish")]
async fn m3_drop_without_finish_panics() {
    let (h, _) = setup(Opts::default()).await;
    drop(h);
}

// ---- the session socket (TST-16) ----

// TST-16: every command's environment names the fake session socket and a marker token.
#[tokio::test]
async fn tst16_a_command_has_the_session() {
    let (h, _) = setup(Opts::default().no_daemon()).await;
    assert!(h.session().path().exists());
    h.finish().await;

    let (h, _) = setup(Opts::default()).await;
    let session = h.session();
    let env = h.env();
    assert_eq!(env.get(SOCKET_VAR), Some(session.path().to_str().unwrap()));
    assert_eq!(env.get(TOKEN_VAR), Some(MARKER_SESSION_TOKEN));
    let ino = std::fs::metadata(session.path()).unwrap().ino();
    h.restart().await;
    assert_eq!(std::fs::metadata(session.path()).unwrap().ino(), ino);
    h.crash().await;
    assert_eq!(std::fs::metadata(session.path()).unwrap().ino(), ino);
    let b = h.new_session("b");
    assert_ne!(b.path(), session.path());
    assert_eq!(b.path().parent(), session.path().parent());
    h.restart().await;
    h.finish().await;
}

/// Posts `text` to `h.session()` as the daemon's poster would, with the marker token.
async fn post(h: &Harness, text: &str) {
    let token = SessionToken::new(MARKER_SESSION_TOKEN.to_string());
    let posted = RealPoster::new()
        .post(h.session().path(), Some(&token), text)
        .await;
    assert_eq!(posted, Posted::Done);
    h.clock().settle().await;
}

// TST-16: what the daemon's poster writes is what the socket records.
#[tokio::test]
async fn tst16_the_daemon_can_post_to_it() {
    let (h, _) = setup(Opts::default()).await;
    h.clock().advance(s(3)).await;
    post(&h, "circitor: hello").await;
    let notices = h.session().notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].token.as_deref(), Some(MARKER_SESSION_TOKEN));
    assert_eq!(notices[0].text, "circitor: hello");
    assert_eq!(notices[0].at, h.clock().wall());
    h.finish().await;
}

#[tokio::test]
#[should_panic(expected = "TST-11")]
async fn m3_sweep_finds_a_session_token() {
    let (h, _) = setup(Opts::default()).await;
    h.log()
        .write_line(&format!("info planted {MARKER_SESSION_TOKEN}"))
        .unwrap();
    h.finish().await;
}

#[tokio::test]
#[should_panic(expected = "TST-11")]
async fn m3_sweep_finds_a_title_in_a_notice() {
    let (h, _) = setup(Opts::default()).await;
    post(&h, &format!("circitor: {MARKER_TITLE} merged")).await;
    h.finish().await;
}

#[tokio::test]
#[should_panic(expected = "TST-11")]
async fn m3_sweep_finds_a_login_in_a_notice() {
    let (h, _) = setup(Opts::default()).await;
    post(&h, &format!("circitor: reviewed by {MARKER_LOGIN}")).await;
    h.finish().await;
}

#[tokio::test]
#[should_panic(expected = "connection 0")]
async fn m3_sweep_finds_a_malformed_notice() {
    use tokio::io::AsyncWriteExt;
    let (h, _) = setup(Opts::default()).await;
    let mut c = tokio::net::UnixStream::connect(h.session().path())
        .await
        .unwrap();
    c.write_all(b"plain text, not a notice\n").await.unwrap();
    drop(c);
    h.clock().settle().await;
    h.finish().await;
}
