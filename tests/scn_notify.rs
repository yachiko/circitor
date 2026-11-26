#![cfg(feature = "test-endpoint")]
//! T49: TST-8's scenarios about **notify**: `notify-outcome`, `notify-already`, `notify-ends`,
//! `notify-session-gone`, `notify-stop`, `notify-keeps-active`, `notify-keeps-ended`,
//! `notify-problem`, `notify-no-session` and `notify-register`, and the subscription sentences of
//! `sleep` and `clock-back`. Each runs the whole daemon on the test clock against the fake GitHub,
//! registers through the CLI's command as a function, and looks at what the fake session socket
//! got. Each ends with `h.finish()` (TST-11), whose sweep covers the session's token (NTF-9) and
//! every posted notice (NTF-8).
//!
//! The world: `octo/app#N`, authored by the fake's login. "Settled" means `updatedAt` two minutes
//! old at the first read. The values below are the specs' (proposals all).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::api::{Envelope, NotifyBody, Outcome};
use circitor::cli::Output;
use circitor::cli::notify::SOCKET_VAR;
use circitor::clock::{Clock, Mono};
use circitor::config::Config;
use circitor::testkit::MARKER_SESSION_TOKEN;
use circitor::testkit::github::{PrId, RepoId, RunId, World};
use circitor::testkit::harness::{Bg, Harness, Opts};
use circitor::testkit::server::{Aim, Fault, Times};
use circitor::testkit::session::{FakeSession, Notice};

/// NTF-4, POL-26: the notifier's tick.
const TICK: Duration = Duration::from_secs(10);
/// NTF-4: ten minutes in `auth` before a `problem`.
const PROBLEM_AFTER: Duration = Duration::from_secs(600);
/// OPS-2.
const INTERVAL: Duration = Duration::from_secs(30);
const IDLE_INTERVAL: Duration = Duration::from_secs(300);
const IDLE_AFTER: Duration = Duration::from_secs(600);
/// NTF-2.
const SUBSCRIPTIONS_MAX: usize = 64;
const PER_SOCKET_MAX: usize = 32;
/// NTF-4: an expiry waits at most this for a refresh.
const CATCH_UP: Duration = Duration::from_secs(60);
/// NTF-10.
const NO_SPAWN_WAIT: Duration = Duration::from_secs(8);
/// STA-1: a subscription's end keeps its PR this long.
const KEEP_AFTER_END: Duration = Duration::from_secs(3600);
/// OPS-7, NTF-10.
const STOP_DEADLINE: Duration = Duration::from_secs(5);

const PR1: &str = "octo/app#1";
const PR2: &str = "octo/app#2";
const A: &str = "dce9dbe";

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A 40-character SHA that starts with `prefix`.
fn sha(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

/// The harness on a world scripted by `f` (before the daemon's first poll), with `opts` beside it.
async fn start<T: Send + 'static>(
    opts: Opts,
    f: impl FnOnce(&mut World) -> T + Send + 'static,
) -> (Harness, T) {
    let slot = Arc::new(Mutex::new(None));
    let put = slot.clone();
    let h = Harness::start(opts.world(move |w| {
        *put.lock().unwrap() = Some(f(w));
    }))
    .await;
    let t = slot.lock().unwrap().take().expect("the world was scripted");
    (h, t)
}

/// A PR whose `updatedAt` is two minutes old: settled at its first read.
fn settled_pr(w: &mut World, repo: RepoId) -> PrId {
    let now = w.now();
    let pr = w.open_pr(repo);
    w.set_created_at(pr, now.minus(s(3600)));
    w.set_updated_at(pr, now.minus(s(120)));
    pr
}

/// A settled PR at head `A` with one queued check (a push moves `updatedAt`, so it's set back after).
fn pr_at_a(w: &mut World, repo: RepoId) -> (PrId, RunId) {
    let pr = settled_pr(w, repo);
    w.push_sha(pr, &sha(A));
    let run = w.run(pr, "build");
    let now = w.now();
    w.set_updated_at(pr, now.minus(s(120)));
    (pr, run)
}

/// The same with the check `IN_PROGRESS`.
fn running_pr(w: &mut World, repo: RepoId) -> (PrId, RunId) {
    let (pr, run) = pr_at_a(w, repo);
    w.start(run);
    (pr, run)
}

/// `n` settled open PRs in `octo/app`, numbered from 1.
fn settled_prs(w: &mut World, n: usize) -> Vec<PrId> {
    let repo = w.repo("octo", "app");
    (0..n).map(|_| settled_pr(w, repo)).collect()
}

async fn adv(h: &Harness, n: u64) {
    h.clock().advance(s(n)).await;
}

/// Moves the clock in 100 ms steps until the command is done (at most `max` seconds), then awaits it.
async fn drive_for(h: &Harness, bg: Bg, max: u64) -> Output {
    for _ in 0..(max * 10) {
        if bg.is_done() {
            break;
        }
        h.clock().advance(ms(100)).await;
    }
    assert!(bg.is_done(), "a command didn't end within {max} s");
    bg.await
}

async fn drive1(h: &Harness, bg: Bg) -> Output {
    drive_for(h, bg, 90).await
}

/// Runs `circitor <args>` to its end, moving the clock as it needs.
async fn run(h: &Harness, args: &[&str]) -> Output {
    let bg = h.cli_bg(args);
    drive1(h, bg).await
}

/// `notify <pr> <args>` through the harness's session.
async fn notify(h: &Harness, pr: &str, args: &[&str]) -> Output {
    let mut a = vec!["notify", pr];
    a.extend_from_slice(args);
    run(h, &a).await
}

/// The same from another session's environment.
async fn notify_from(h: &Harness, session: &FakeSession, pr: &str, args: &[&str]) -> Output {
    let mut a = vec!["notify", pr];
    a.extend_from_slice(args);
    let bg = h
        .cmd(&a)
        .var(SOCKET_VAR, session.path().to_str().unwrap())
        .bg();
    drive1(h, bg).await
}

fn has_line(out: &Output, line: &str) -> bool {
    out.stdout.lines().any(|l| l == line)
}

fn has_prefix(out: &Output, prefix: &str) -> bool {
    out.stdout.lines().any(|l| l.starts_with(prefix))
}

/// The `cursor:` line a command printed.
fn cursor_of(out: &Output) -> String {
    out.stdout
        .lines()
        .find_map(|l| l.strip_prefix("cursor: "))
        .unwrap_or_else(|| panic!("no cursor line: {out:?}"))
        .to_string()
}

fn body(out: &Output) -> Envelope<NotifyBody> {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

/// `status --json`, which is no request that moves the tier (POL-13).
async fn status(h: &Harness) -> serde_json::Value {
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    serde_json::from_str(&out.stdout).expect("status is JSON")
}

async fn subs(h: &Harness) -> u64 {
    status(h).await["subscriptions"].as_u64().expect("a count")
}

async fn tier(h: &Harness) -> String {
    status(h).await["daemon"]["tier"]
        .as_str()
        .expect("a tier")
        .to_string()
}

async fn tracked(h: &Harness) -> u64 {
    let v = status(h).await;
    ["authored", "manual", "asked"]
        .iter()
        .map(|k| v["tracked"][k].as_u64().expect("a count"))
        .sum()
}

fn notices(h: &Harness) -> Vec<Notice> {
    h.session().notices()
}

/// How many lines of the daemon's log hold every one of `needles`.
fn log_count(h: &Harness, needles: &[&str]) -> usize {
    h.log()
        .contents()
        .lines()
        .filter(|l| needles.iter().all(|n| l.contains(n)))
        .count()
}

fn node_id(h: &Harness, pr: PrId) -> String {
    h.github().world(|w| w.node_id(pr).to_string())
}

/// The monotonic times of the requests that named the node, and whether each held a finding.
fn hits(h: &Harness, id: &str) -> Vec<(Mono, bool)> {
    h.github()
        .requests()
        .into_iter()
        .filter(|r| r.node_ids().contains(&id))
        .map(|r| (r.mono, r.finding.is_some()))
        .collect()
}

/// The monotonic times of every request that held a finding.
fn poll_times(h: &Harness) -> Vec<Mono> {
    h.github()
        .requests()
        .into_iter()
        .filter(|r| r.finding.is_some())
        .map(|r| r.mono)
        .collect()
}

/// Every gap between two polls from `from` on is at least `least` less 1 s.
fn polls_keep(h: &Harness, from: Mono, least: Duration) {
    let times: Vec<Mono> = poll_times(h).into_iter().filter(|t| *t >= from).collect();
    for w in times.windows(2) {
        let gap = w[1].since(w[0]);
        assert!(
            gap + s(1) >= least,
            "two polls {gap:?} apart, under {least:?} less 1 s: {times:?}"
        );
    }
}

fn config(f: impl FnOnce(&mut Config)) -> Opts {
    Opts::default().config(f)
}

// ---------------------------------------------------------------------------------------------
// notify-outcome
// ---------------------------------------------------------------------------------------------

// TST-8: notify-outcome
// NTF-5: it's evaluated by polls and never makes its PR hot.
// NTF-6: one notice for the confirmation and one for the outcome.
// NTF-7: the confirmation's words.
// NTF-8: the outcome's words.
#[tokio::test]
async fn scn_notify_outcome() {
    let (h, (pr, run)) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        running_pr(w, repo)
    })
    .await;
    let id = node_id(&h, pr);
    // The first poll.
    adv(&h, 20).await;
    let out = notify(&h, PR1, &["--until", "checks-done", "--sha", A]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let first = out.stdout.lines().next().expect("a first line");
    assert!(
        first.starts_with("registered: octo/app#1 until checks-done on dce9dbe, for 1d"),
        "{out:?}"
    );
    let cursor = cursor_of(&out);
    // When the command has returned, the confirmation has been posted.
    let got = notices(&h);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].token.as_deref(), Some(MARKER_SESSION_TOKEN));
    assert_eq!(
        got[0].text,
        "circitor: watching octo/app#1 until checks-done on dce9dbe, for 1d. If you can read this, notices reach this session."
    );
    assert_eq!(subs(&h).await, 1);
    let registered_at = h.clock().mono();

    // The checks pass; the next poll finds it.
    h.github().world(|w| w.finish(run, "SUCCESS"));
    adv(&h, 40).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[1].token.as_deref(), Some(MARKER_SESSION_TOKEN));
    assert_eq!(
        got[1].text,
        format!(
            "circitor: octo/app#1 checks passed (1/1) on dce9dbe. Details: circitor changes octo/app#1 --since {cursor}"
        )
    );
    assert_eq!(subs(&h).await, 0);
    // Ten minutes on, no third notice.
    adv(&h, 600).await;
    assert_eq!(notices(&h).len(), 2);

    // Never hot: between the registration's own refresh and the outcome, the PR is read as often as
    // a settled PR is, and only by a poll.
    let named = hits(&h, &id);
    let ended = got[1].mono;
    let during: Vec<(Mono, bool)> = named
        .iter()
        .copied()
        .filter(|(t, _)| *t > registered_at && *t <= ended)
        .collect();
    assert!(
        during.iter().all(|(_, finding)| *finding),
        "a request without a finding named the PR: {named:?}"
    );
    for w in during.windows(2) {
        assert!(
            w[1].0.since(w[0].0) + s(1) >= INTERVAL,
            "the PR was read {:?} apart: {during:?}",
            w[1].0.since(w[0].0)
        );
    }
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-already
// ---------------------------------------------------------------------------------------------

// TST-8: notify-already
// NTF-2: an answer already known registers nothing.
// NTF-11: it's exit 0, with the cursor.
#[tokio::test]
async fn scn_notify_already() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let (_, run) = pr_at_a(w, repo);
        w.finish(run, "FAILURE");
    })
    .await;
    adv(&h, 20).await;
    let out = notify(&h, PR1, &["--until", "checks-done"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        has_line(&out, "already: checks failed (1 of 1) on dce9dbe"),
        "{out:?}"
    );
    assert!(has_prefix(&out, "cursor: "), "{out:?}");
    assert!(notices(&h).is_empty());
    assert_eq!(subs(&h).await, 0);

    let out = notify(&h, PR1, &["--until", "checks-done", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let b = body(&out).body;
    assert!(!b.registered);
    assert_eq!(b.outcome, Some(Outcome::Satisfied));
    assert!(notices(&h).is_empty());
    assert_eq!(subs(&h).await, 0);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-ends
// ---------------------------------------------------------------------------------------------

/// A world with one settled open PR (its check running), the first poll done.
async fn one_open(opts: Opts) -> (Harness, PrId) {
    let (h, (pr, _)) = start(opts, |w| {
        let repo = w.repo("octo", "app");
        running_pr(w, repo)
    })
    .await;
    adv(&h, 20).await;
    (h, pr)
}

/// A world with one settled open PR with no checks, the first poll done: its status is `open`.
async fn plain_open(opts: Opts) -> (Harness, PrId) {
    let (h, pr) = start(opts, |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    adv(&h, 20).await;
    (h, pr)
}

// TST-8: notify-ends
// NTF-3, NTF-4: closed ends it, with a notice after the confirmation.
#[tokio::test]
async fn scn_notify_ends_closed() {
    let (h, pr) = one_open(Opts::default()).await;
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    h.github().world(|w| w.close(pr));
    adv(&h, 40).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got[0]
            .text
            .starts_with("circitor: watching octo/app#1 until merged")
    );
    assert!(got[1].text.contains("ended: closed"), "{got:?}");
    assert_eq!(subs(&h).await, 0);
    h.finish().await;
}

// TST-8: notify-ends
// NTF-4, NTF-9: `for` ends it at the first tick at or after it.
#[tokio::test]
async fn scn_notify_ends_expired() {
    let (h, _) = plain_open(Opts::default()).await;
    let out = notify(&h, PR1, &["--until", "merged", "--for", "60"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let cursor = cursor_of(&out);
    let base = notices(&h)[0].mono;
    // No second notice at 59 s.
    let to = base.plus(s(59)).since(h.clock().mono());
    h.clock().advance(to).await;
    assert_eq!(notices(&h).len(), 1);
    // By the first tick at or after 60 s: one, holding the expiry.
    h.clock().advance(TICK + ms(500)).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    let at = got[1].mono.since(base);
    assert!(at >= s(60) && at <= s(60) + TICK, "expired after {at:?}");
    assert!(
        got[1].text.contains("expired after 1m; it's ready"),
        "{got:?}"
    );
    assert!(!got[1].text.contains("(stale"), "{got:?}");
    assert!(
        got[1].text.ends_with(&format!(
            "Ask again: circitor notify octo/app#1 --until merged --for 60 --since {cursor}"
        )),
        "{got:?}"
    );
    assert_eq!(subs(&h).await, 0);
    h.finish().await;
}

// TST-8: notify-ends
// NTF-3: a cancel drops it and posts nothing.
#[tokio::test]
async fn scn_notify_ends_cancelled() {
    let (h, pr) = one_open(Opts::default()).await;
    notify(&h, PR1, &["--until", "merged"]).await;
    let out = notify(&h, PR1, &["--cancel"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_line(&out, "cancelled: 1"), "{out:?}");
    assert_eq!(subs(&h).await, 0);
    let out = notify(&h, PR1, &["--cancel"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_line(&out, "cancelled: 0"), "{out:?}");
    h.github().world(|w| w.merge(pr));
    adv(&h, 60).await;
    let got = notices(&h);
    assert_eq!(got.len(), 1, "{got:?}");
    h.finish().await;
}

// TST-8: notify-ends
// NTF-2: the same request replaces; one that differs is held beside it; a cancel drops both.
#[tokio::test]
async fn scn_notify_ends_registered_twice() {
    let (h, _) = one_open(Opts::default()).await;
    let first = notify(&h, PR1, &["--until", "checks-done"]).await;
    assert_eq!(first.code, 0, "{first:?}");
    assert!(!has_line(&first, "replaced an earlier notify"), "{first:?}");
    let second = notify(&h, PR1, &["--until", "checks-done"]).await;
    assert_eq!(second.code, 0, "{second:?}");
    assert!(
        has_line(&second, "replaced an earlier notify"),
        "{second:?}"
    );
    assert_eq!(subs(&h).await, 1);
    // Two confirmations, and none for the first's end.
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got.iter().all(|n| n.text.starts_with("circitor: watching")));

    let third = notify(&h, PR1, &["--until", "checks-done", "--fail-fast"]).await;
    assert_eq!(third.code, 0, "{third:?}");
    assert!(!has_line(&third, "replaced an earlier notify"), "{third:?}");
    assert_eq!(subs(&h).await, 2);
    assert_eq!(notices(&h).len(), 3);
    let out = notify(&h, PR1, &["--cancel"]).await;
    assert!(has_line(&out, "cancelled: 2"), "{out:?}");
    assert_eq!(subs(&h).await, 0);
    assert_eq!(notices(&h).len(), 3);
    h.finish().await;
}

// TST-8: notify-ends
// NTF-9: `subscriptions` counts every one held, and `waiters` none of them.
#[tokio::test]
async fn scn_notify_ends_counted() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        running_pr(w, repo);
        running_pr(w, repo);
    })
    .await;
    adv(&h, 20).await;
    for (pr, until) in [(PR1, "merged"), (PR1, "checks-done"), (PR2, "merged")] {
        let out = notify(&h, pr, &["--until", until]).await;
        assert_eq!(out.code, 0, "{out:?}");
    }
    let v = status(&h).await;
    assert_eq!(v["subscriptions"], 3);
    assert_eq!(v["waiters"], 0);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-session-gone
// ---------------------------------------------------------------------------------------------

/// A settled open PR with `notify --until merged` registered on it, the tier kept by it alone.
async fn registered_merged() -> (Harness, PrId, Mono) {
    let (h, pr) = plain_open(Opts::default()).await;
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(notices(&h).len(), 1);
    let from = h.clock().mono();
    (h, pr, from)
}

// TST-8: notify-session-gone
// NTF-4: a socket that is gone at a tick ends it, and nothing is posted.
#[tokio::test]
async fn scn_notify_session_gone_vanished() {
    let (h, pr, from) = registered_merged().await;
    h.session().vanish();
    // Within one tick.
    h.clock().advance(TICK + ms(500)).await;
    assert_eq!(subs(&h).await, 0);
    assert_eq!(
        log_count(&h, &["subscription ended", "end=session_gone"]),
        1,
        "{}",
        h.log().contents()
    );
    assert_eq!(h.session().notices().len(), 1);
    // The PR then merges: nothing is posted.
    h.github().world(|w| w.merge(pr));
    adv(&h, 600).await;
    assert_eq!(h.session().notices().len(), 1);
    assert_eq!(log_count(&h, &["notice not posted"]), 0);
    polls_keep(&h, from, INTERVAL);
    h.finish().await;
}

// TST-8: notify-session-gone
// NTF-6: a refusing socket looks fine to a tick; only the post finds out.
#[tokio::test]
async fn scn_notify_session_gone_refusing() {
    let (h, pr, from) = registered_merged().await;
    h.session().refuse();
    // The tick drops nothing: the file is the same socket.
    adv(&h, 25).await;
    assert_eq!(subs(&h).await, 1);
    h.github().world(|w| w.merge(pr));
    adv(&h, 40).await;
    assert_eq!(notices(&h).len(), 1, "only the confirmation");
    assert_eq!(
        log_count(&h, &["notice not posted", "reason=session_gone"]),
        1
    );
    assert_eq!(subs(&h).await, 0);
    adv(&h, 600).await;
    assert_eq!(log_count(&h, &["notice not posted"]), 1);
    assert_eq!(notices(&h).len(), 1);
    polls_keep(&h, from, INTERVAL);
    h.finish().await;
}

// TST-8: notify-session-gone
// NTF-4: a new socket at the same path is another socket.
#[tokio::test]
async fn scn_notify_session_gone_taken_over() {
    let (h, _pr, from) = registered_merged().await;
    h.session().vanish();
    h.session().restore();
    let before = h.session().lines().len();
    let confirmations = h.session().notices().len();
    h.clock().advance(TICK + ms(500)).await;
    assert_eq!(subs(&h).await, 0);
    assert_eq!(
        log_count(&h, &["subscription ended", "end=session_gone"]),
        1,
        "{}",
        h.log().contents()
    );
    // Nothing was posted to the new one.
    assert_eq!(h.session().lines().len(), before);
    assert_eq!(h.session().notices().len(), confirmations);
    adv(&h, 120).await;
    assert_eq!(h.session().notices().len(), confirmations);
    polls_keep(&h, from, INTERVAL);
    h.finish().await;
}

// TST-8: notify-session-gone
// NTF-6: a connect that fails at once is one failed post, and never tried again.
#[tokio::test]
#[ignore = "T45: queue_full rebinds the path (another inode), so NTF-6's look sees the session gone and the post never fails with reason=failed"]
async fn scn_notify_session_gone_full_queue() {
    let (h, pr, from) = registered_merged().await;
    h.session().queue_full();
    h.github().world(|w| w.merge(pr));
    adv(&h, 40).await;
    assert_eq!(
        log_count(&h, &["notice not posted", "reason=failed"]),
        1,
        "{}",
        h.log().contents()
    );
    assert_eq!(subs(&h).await, 0);
    adv(&h, 600).await;
    assert_eq!(log_count(&h, &["notice not posted"]), 1);
    assert!(h.session().notices().is_empty());
    polls_keep(&h, from, INTERVAL);
    h.finish().await;
}

// TST-8: notify-session-gone
// NTF-6: a peer that accepts and never reads got it: it counts as posted.
#[tokio::test]
async fn scn_notify_session_gone_never_reads() {
    let (h, pr, from) = registered_merged().await;
    h.session().accept_and_never_read();
    h.github().world(|w| w.merge(pr));
    adv(&h, 40).await;
    assert_eq!(log_count(&h, &["notice not posted"]), 0);
    assert_eq!(subs(&h).await, 0);
    adv(&h, 600).await;
    assert_eq!(log_count(&h, &["notice not posted"]), 0);
    polls_keep(&h, from, INTERVAL);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-stop
// ---------------------------------------------------------------------------------------------

/// Three settled open PRs, each with `notify --until merged`, and the first poll done.
async fn three_registered() -> (Harness, Vec<PrId>) {
    let (h, prs) = start(Opts::default(), |w| settled_prs(w, 3)).await;
    adv(&h, 20).await;
    for pr in [PR1, PR2, "octo/app#3"] {
        let out = notify(&h, pr, &["--until", "merged"]).await;
        assert_eq!(out.code, 0, "{out:?}");
    }
    assert_eq!(notices(&h).len(), 3);
    assert_eq!(subs(&h).await, 3);
    (h, prs)
}

/// The command a notice's `Ask again:` holds, as arguments.
fn ask_again(text: &str) -> Vec<String> {
    let (_, cmd) = text.split_once("Ask again: ").expect("an Ask again");
    let mut words = cmd.split_whitespace();
    assert_eq!(words.next(), Some("circitor"));
    words.map(str::to_string).collect()
}

// TST-8: notify-stop
// NTF-9, NTF-10, OPS-7, CLI-7: a stop tells every subscription, and a wait's connection drops after.
#[tokio::test]
async fn scn_notify_stop() {
    let (h, _) = three_registered().await;
    let wait = h.cli_bg(&["wait", PR1, "--until", "merged"]);
    adv(&h, 3).await;
    assert!(!wait.is_done());
    let began = h.clock().mono();
    let stopped_at = h.clock().wall();
    assert_eq!(h.stop().await, 0);
    assert!(h.clock().mono().since(began) < STOP_DEADLINE);
    let got = notices(&h);
    assert_eq!(got.len(), 6, "{got:?}");
    for n in &got[3..] {
        assert!(n.text.contains("the daemon is stopping"), "{n:?}");
        assert!(n.text.ends_with("--no-spawn"), "{n:?}");
        assert_eq!(n.token.as_deref(), Some(MARKER_SESSION_TOKEN));
        assert!(n.at >= stopped_at);
    }
    // The wait's connection dropped, after the notices.
    let out = drive1(&h, wait).await;
    assert_eq!(out.code, 6, "{out:?}");
    assert!(
        out.stdout.contains("daemon stopped") || out.stderr.contains("daemon stopped"),
        "{out:?}"
    );
    assert_eq!(h.exited(), Some(0));
    // The sweep: the file holds no socket's path and no token, and loads.
    let file = h.state_file().expect("a state file");
    assert!(!file.contains(MARKER_SESSION_TOKEN));
    assert!(!file.contains(h.session().path().to_str().unwrap()));
    serde_json::from_str::<serde_json::Value>(&file).expect("state.json loads");
    h.finish().await;
}

// TST-8: notify-stop
// NTF-9: a crash posts nothing, and nothing is held after it.
#[tokio::test]
async fn scn_notify_stop_crash() {
    let (h, prs) = three_registered().await;
    h.crash().await;
    adv(&h, 30).await;
    assert_eq!(notices(&h).len(), 3);
    h.restart().await;
    adv(&h, 20).await;
    assert_eq!(subs(&h).await, 0);
    h.github().world(|w| {
        for pr in &prs {
            w.merge(*pr);
        }
    });
    adv(&h, 120).await;
    assert_eq!(notices(&h).len(), 3);
    h.finish().await;
}

// TST-8: notify-stop
// NTF-10, CLI-7: `Ask again` after a stop starts no daemon, and gives up after 8 s.
#[tokio::test]
async fn scn_notify_stop_ask_again_no_daemon() {
    let (h, _) = three_registered().await;
    assert_eq!(h.stop().await, 0);
    let text = notices(&h)[3].text.clone();
    let args = ask_again(&text);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let spawns = h.spawns();
    let bg = h.cli_bg(&refs);
    h.clock().advance(NO_SPAWN_WAIT - s(1)).await;
    assert!(!bg.is_done(), "ended before the 8 s");
    let out = drive_for(&h, bg, 5).await;
    assert_eq!(out.code, 6, "{out:?}");
    assert_eq!(h.spawns(), spawns);
    h.finish().await;
}

// TST-8: notify-stop
// CLI-9, NTF-10: `Ask again` finds the replacement daemon and registers with it.
#[tokio::test]
async fn scn_notify_stop_ask_again_replacement() {
    let (h, _) = three_registered().await;
    let old_epoch = {
        let c = status(&h).await["cursor"].as_str().unwrap().to_string();
        c.split('-').next().unwrap().to_string()
    };
    assert_eq!(h.stop().await, 0);
    let text = notices(&h)[3].text.clone();
    let args = ask_again(&text);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let bg = h.cli_bg(&refs);
    adv(&h, 3).await;
    assert!(!bg.is_done());
    h.restart().await;
    let out = drive1(&h, bg).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        has_prefix(&out, "registered: octo/app#1 until merged"),
        "{out:?}"
    );
    let got = notices(&h);
    assert_eq!(got.len(), 7, "{got:?}");
    assert!(
        got[6]
            .text
            .starts_with("circitor: watching octo/app#1 until merged")
    );
    assert_ne!(cursor_of(&out).split('-').next().unwrap(), old_epoch);
    h.finish().await;
}

// TST-8: notify-stop
// NTF-10, STA-11: a stop the daemon chose is not asked for, so its notice has no `--no-spawn`.
#[tokio::test]
async fn scn_notify_stop_chosen() {
    let (h, _) = plain_open(Opts::default()).await;
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let cursor = cursor_of(&out);
    std::fs::remove_file(&h.paths().socket).expect("the daemon's socket file");
    for _ in 0..14 {
        if h.exited().is_some() {
            break;
        }
        adv(&h, 10).await;
    }
    assert_eq!(h.exited(), Some(0));
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[1].text.contains("the daemon is stopping"), "{got:?}");
    assert!(!got[1].text.contains("--no-spawn"), "{got:?}");
    assert!(
        got[1].text.ends_with(&format!("--since {cursor}")),
        "{got:?}"
    );
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-keeps-active
// ---------------------------------------------------------------------------------------------

/// Every gap between two polls from `from` on is at most `most`.
fn polls_within(h: &Harness, from: Mono, most: Duration) {
    let times: Vec<Mono> = poll_times(h).into_iter().filter(|t| *t >= from).collect();
    for w in times.windows(2) {
        let gap = w[1].since(w[0]);
        assert!(gap <= most, "two polls {gap:?} apart, over {most:?}");
    }
}

/// `exit_after` 300 s, `idle_after` its default.
fn exits_after_300() -> Opts {
    assert_eq!(IDLE_AFTER, s(600));
    config(|c| c.exit_after = Some(s(300)))
}

/// A settled open PR with a running check that merges 20 minutes on; the first poll done and
/// `notify --until merged` registered.
async fn merging_registered(opts: Opts) -> (Harness, Mono) {
    let (h, _) = start(opts, |w| {
        let repo = w.repo("octo", "app");
        let (pr, _) = running_pr(w, repo);
        w.after(s(1200), move |w| w.merge(pr));
    })
    .await;
    adv(&h, 20).await;
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let from = h.clock().mono();
    (h, from)
}

/// Moves the clock in 10 s steps until the subscription has ended (at most 2 minutes).
async fn until_ended(h: &Harness) {
    for _ in 0..40 {
        if notices(h).len() > 1 {
            return;
        }
        adv(h, 10).await;
    }
    panic!("the subscription never ended: {}", h.log().contents());
}

// TST-8: notify-keeps-active
// NTF-5, OPS-8, POL-10: a subscription on an open PR holds the active tier and the daemon.
#[tokio::test]
async fn scn_notify_keeps_active_held() {
    let (h, from) = merging_registered(exits_after_300()).await;
    for _ in 0..22 {
        adv(&h, 50).await;
        assert_eq!(tier(&h).await, "active");
        assert_eq!(h.exited(), None);
    }
    polls_keep(&h, from, INTERVAL);
    polls_within(&h, from, INTERVAL + s(2));
    assert_eq!(subs(&h).await, 1);
    assert_eq!(notices(&h).len(), 1);
    h.finish().await;
}

// TST-8: notify-keeps-active
// POL-13: once it ends, the tier follows the last client request, which is long ago.
#[tokio::test]
async fn scn_notify_keeps_active_ended_long_ago() {
    let (h, from) = merging_registered(exits_after_300()).await;
    adv(&h, 1150).await;
    assert_eq!(tier(&h).await, "active");
    until_ended(&h).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[1].text.contains("merged"), "{got:?}");
    let end = got[1].mono;
    assert_eq!(subs(&h).await, 0);
    assert_eq!(tier(&h).await, "idle");
    // The daemon exits 300 s after the end, and not at 299 s.
    let to = end.plus(s(299)).since(h.clock().mono());
    h.clock().advance(to).await;
    assert_eq!(h.exited(), None);
    h.clock().advance(s(2)).await;
    assert_eq!(h.exited(), Some(0));
    assert!(
        h.log().contents().contains("stop cause=idle"),
        "{}",
        h.log().contents()
    );
    polls_keep(&h, from, INTERVAL);
    h.finish().await;
}

// TST-8: notify-keeps-active
// POL-13: a request just before the end moves the tier and the exit.
#[tokio::test]
async fn scn_notify_keeps_active_ended_request_before() {
    let (h, _) = merging_registered(exits_after_300()).await;
    // A `list` 15 minutes after the start.
    adv(&h, 880).await;
    let out = h.cli(&["list", "--all"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let list_at = h.clock().mono();
    until_ended(&h).await;
    let got = notices(&h);
    let end = got[1].mono;
    assert!(end > list_at.plus(s(250)));
    // Not idle at the end: the `list` is under 600 s old.
    assert_eq!(subs(&h).await, 0);
    assert_eq!(tier(&h).await, "active");
    let to = end.plus(s(250)).since(h.clock().mono());
    h.clock().advance(to).await;
    assert!(h.clock().mono().since(list_at) < IDLE_AFTER);
    assert_eq!(tier(&h).await, "active");
    // The daemon exits 300 s after the later of the `list` and the end: the end.
    let to = end.plus(s(299)).since(h.clock().mono());
    h.clock().advance(to).await;
    assert_eq!(h.exited(), None);
    h.clock().advance(s(2)).await;
    assert_eq!(h.exited(), Some(0));
    h.finish().await;
}

// TST-8: notify-keeps-active
// NTF-5, POL-10: an `asked` PR is refreshed every `interval` while it's open, never hot.
#[tokio::test]
async fn scn_notify_keeps_active_asked() {
    let (h, pr) = start(exits_after_300(), |w| {
        let repo = w.repo("else", "lib");
        settled_pr(w, repo);
        let pr = settled_pr(w, repo);
        w.set_author(pr, Some("somebody-else"));
        pr
    })
    .await;
    let id = node_id(&h, pr);
    adv(&h, 20).await;
    let out = notify(&h, "else/lib#2", &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let from = h.clock().mono();
    for _ in 0..60 {
        adv(&h, 60).await;
    }
    let named: Vec<Mono> = hits(&h, &id)
        .into_iter()
        .map(|(t, _)| t)
        .filter(|t| *t > from)
        .collect();
    assert!(named.len() >= 100, "{} reads in an hour", named.len());
    for w in named.windows(2) {
        let gap = w[1].since(w[0]);
        assert!(gap + s(1) >= INTERVAL, "read {gap:?} apart (hot)");
        assert!(gap <= INTERVAL + s(2), "read {gap:?} apart (idle)");
    }
    let v = status(&h).await;
    assert_eq!(v["tracked"]["asked"], 1, "{v}");
    assert_eq!(v["subscriptions"], 1);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-keeps-ended
// ---------------------------------------------------------------------------------------------

// TST-8: notify-keeps-ended
// NTF-4, NTF-5, STA-1, STA-2: a subscription on an ended PR keeps it, not the tier, and its end
// counts as a request's end.
#[tokio::test]
async fn scn_notify_keeps_ended() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        w.merge(pr);
    })
    .await;
    // The PR merged 24 hours before the first request that names it.
    h.clock().advance(s(24 * 3600)).await;
    let out = notify(&h, PR1, &["--until", "changed", "--for", "7200"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        has_prefix(&out, "registered: octo/app#1 until changed"),
        "{out:?}"
    );
    let cursor = cursor_of(&out);
    let reg = h.clock().mono();
    assert_eq!(subs(&h).await, 1);

    // The tier is active for `idle_after` and idle after it, though the subscription is open.
    adv(&h, 590).await;
    assert_eq!(tier(&h).await, "active");
    adv(&h, 20).await;
    assert_eq!(tier(&h).await, "idle");
    let idle_from = h.clock().mono();
    // Past the registration's own 60 minutes, and past `keep_done`, it's still tracked.
    adv(&h, 3600).await;
    assert_eq!(tracked(&h).await, 1);
    assert_eq!(tier(&h).await, "idle");
    polls_keep(&h, idle_from, IDLE_INTERVAL);
    // Nothing yet: a merged PR with nothing new neither satisfies nor ends it.
    assert_eq!(notices(&h).len(), 1);
    assert_eq!(subs(&h).await, 1);

    // At 2 hours it expires, up to a tick (and a catch-up) late.
    let to = reg.plus(s(7190)).since(h.clock().mono());
    h.clock().advance(to).await;
    for _ in 0..30 {
        if notices(&h).len() > 1 {
            break;
        }
        adv(&h, 10).await;
    }
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[1].text.contains("expired after 2h"), "{got:?}");
    assert!(got[1].mono.since(reg) >= s(7200));
    assert!(got[1].mono.since(reg) <= s(7200) + TICK + CATCH_UP);
    assert_eq!(subs(&h).await, 0);
    // Its end keeps the PR 60 minutes more: still tracked 59 minutes on, dropped by 61.
    let end = got[1].mono;
    let to = end.plus(KEEP_AFTER_END - s(60)).since(h.clock().mono());
    h.clock().advance(to).await;
    assert_eq!(tracked(&h).await, 1);
    h.clock().advance(s(130)).await;
    assert_eq!(tracked(&h).await, 0);
    let out = h.cli(&["list", "--since", &cursor, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["untracked"], serde_json::json!(["octo/app#1"]), "{v}");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-problem
// ---------------------------------------------------------------------------------------------

/// The `daemon.problem` code `status --json` reads, if any.
async fn problem(h: &Harness) -> Option<String> {
    status(h).await["daemon"]["problem"]
        .as_str()
        .map(str::to_string)
}

/// Moves the clock in `step`-second steps until `f` is true, at most `max` seconds.
async fn until<F, Fut>(h: &Harness, step: u64, max: u64, what: &str, f: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..=(max / step) {
        if f().await {
            return;
        }
        adv(h, step).await;
    }
    panic!(
        "{what} didn't happen within {max} s: {}",
        h.log().contents()
    );
}

/// The monotonic time of the repeat's 401, where `auth` starts (POL-20): the second 401 sent.
fn auth_since(h: &Harness) -> Mono {
    let hits: Vec<Mono> = h
        .github()
        .requests()
        .into_iter()
        .filter(|r| r.fault == Some(Fault::Unauthorized))
        .map(|r| r.mono)
        .collect();
    assert!(hits.len() >= 2, "{} 401s", hits.len());
    hits[1]
}

// TST-8: notify-problem
// NTF-2, POL-20: registering in `auth` answers `auth`, unless the answer is already known.
#[tokio::test]
async fn scn_notify_problem_registering_in_auth() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        running_pr(w, repo);
        let pr = settled_pr(w, repo);
        w.push_sha(pr, &sha("cf54ec4"));
        let run = w.run(pr, "build");
        let now = w.now();
        w.set_updated_at(pr, now.minus(s(120)));
        w.finish(run, "FAILURE");
    })
    .await;
    adv(&h, 20).await;
    h.github().fault(Fault::Unauthorized, Times::UntilCleared);
    until(&h, 10, 400, "auth", || async {
        problem(&h).await.as_deref() == Some("auth")
    })
    .await;
    let out = notify(&h, PR1, &["--until", "checks-done"]).await;
    assert_eq!(out.code, 7, "{out:?}");
    assert_eq!(subs(&h).await, 0);
    assert!(notices(&h).is_empty());
    // A check that failed and settled before: already known.
    let out = notify(&h, PR2, &["--until", "checks-done"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_prefix(&out, "already: checks failed"), "{out:?}");
    assert_eq!(subs(&h).await, 0);
    assert!(notices(&h).is_empty());
    h.finish().await;
}

// TST-8: notify-problem
// NTF-4, POL-20: ten minutes in `auth` end it, the new token's failed try not restarting them.
#[tokio::test]
async fn scn_notify_problem_auth_while_held() {
    let (h, _) = plain_open(Opts::default()).await;
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    h.github().fault(Fault::Unauthorized, Times::UntilCleared);
    until(&h, 10, 120, "auth", || async {
        problem(&h).await.as_deref() == Some("auth")
    })
    .await;
    let t = auth_since(&h);
    // At t + 5 minutes the token source gives a new token, whose one try also gets a 401.
    let to = t.plus(s(300)).since(h.clock().mono());
    h.clock().advance(to).await;
    let before = h.github().requests().len();
    h.token().set(circitor::token::TokenRead::Token(
        circitor::token::Token::new(circitor::testkit::MARKER_TOKEN_2.to_string()),
    ));
    adv(&h, 60).await;
    assert!(
        h.github().requests().len() > before,
        "the new token wasn't tried"
    );
    assert_eq!(problem(&h).await.as_deref(), Some("auth"));
    // No notice at the tick before t + 10 minutes.
    let to = t.plus(PROBLEM_AFTER - s(1)).since(h.clock().mono());
    h.clock().advance(to).await;
    assert_eq!(notices(&h).len(), 1);
    assert_eq!(subs(&h).await, 1);
    // At the first tick at or after it, one.
    h.clock().advance(TICK + s(1)).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(
        got[1].text,
        "circitor: can't read octo/app#1 from GitHub (auth); notify is cancelled. Details: circitor status"
    );
    assert!(got[1].mono.since(t) >= PROBLEM_AFTER);
    assert_eq!(subs(&h).await, 0);
    assert_eq!(
        log_count(&h, &["subscription ended", "end=problem", "why=auth"]),
        1,
        "{}",
        h.log().contents()
    );
    h.finish().await;
}

// TST-8: notify-problem
// NTF-4, POL-21: a PR that fails to be read for 5 minutes ends its subscription, and no other.
#[tokio::test]
async fn scn_notify_problem_pr_failing() {
    let (h, (p1, _)) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let a = settled_pr(w, repo);
        let b = settled_pr(w, repo);
        (a, b)
    })
    .await;
    adv(&h, 20).await;
    for pr in [PR1, PR2] {
        let out = notify(&h, pr, &["--until", "merged"]).await;
        assert_eq!(out.code, 0, "{out:?}");
    }
    assert_eq!(notices(&h).len(), 2);
    let id = node_id(&h, p1);
    h.github()
        .pr_error(p1, "FORBIDDEN", Aim::node(&id), Times::UntilCleared);
    let raised = || async {
        status(&h).await["warnings"]
            .as_array()
            .is_some_and(|w| w.iter().any(|x| x["code"] == "pr_failing"))
    };
    until(&h, 10, 900, "the pr_failing warning", raised).await;
    // The next tick posts it.
    h.clock().advance(TICK + s(1)).await;
    let got = notices(&h);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(
        got[2].text,
        "circitor: can't read octo/app#1 from GitHub (pr_failing); notify is cancelled. Details: circitor status"
    );
    assert_eq!(subs(&h).await, 1, "the other PR's subscription stays");
    assert_eq!(
        log_count(&h, &["subscription ended", "end=problem", "why=pr_failing"]),
        1
    );
    // A second registration on the failing PR posts its confirmation, then `problem` at the first tick.
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let got = notices(&h);
    assert_eq!(got.len(), 4, "{got:?}");
    assert!(got[3].text.starts_with("circitor: watching octo/app#1"));
    h.clock().advance(TICK + s(1)).await;
    let got = notices(&h);
    assert_eq!(got.len(), 5, "{got:?}");
    assert!(got[4].text.contains("(pr_failing)"), "{got:?}");
    assert_eq!(subs(&h).await, 1);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-register
// ---------------------------------------------------------------------------------------------

// TST-8: notify-register
// NTF-2, NTF-11: a `since` from another epoch resets, and the notice's cursor is the command's.
#[tokio::test]
async fn scn_notify_register_since_before_restart() {
    let (h, (_, run)) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        running_pr(w, repo)
    })
    .await;
    adv(&h, 20).await;
    let out = h.cli(&["changes", PR1, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let old: Envelope<serde_json::Value> = serde_json::from_str(&out.stdout).unwrap();
    h.restart().await;
    adv(&h, 20).await;
    let out = notify(&h, PR1, &["--until", "checks-done", "--since", &old.cursor]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let lines: Vec<&str> = out.stdout.lines().collect();
    let reset = lines
        .iter()
        .position(|l| l.starts_with("reset:"))
        .unwrap_or_else(|| panic!("no reset line: {out:?}"));
    let registered = lines
        .iter()
        .position(|l| l.starts_with("registered:"))
        .unwrap_or_else(|| panic!("no registered line: {out:?}"));
    assert!(reset < registered, "{out:?}");
    assert!(
        lines[reset + 1..registered]
            .iter()
            .any(|l| l.starts_with("octo/app#1")),
        "the records follow the reset line: {out:?}"
    );
    let cursor = cursor_of(&out);
    assert_ne!(cursor, old.cursor);
    h.github().world(|w| w.finish(run, "SUCCESS"));
    adv(&h, 40).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[1].text.contains("checks passed"), "{got:?}");
    assert!(
        got[1].text.ends_with(&format!("--since {cursor}")),
        "{got:?} against {cursor}"
    );
    h.finish().await;
}

// TST-8: notify-register
// NTF-2: a short and a full SHA are two requests; the same SHA in another case is one.
#[tokio::test]
async fn scn_notify_register_side_by_side() {
    let (h, _) = one_open(Opts::default()).await;
    let full = sha(A);
    let out = notify(&h, PR1, &["--until", "checks-done", "--sha", A]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let out = notify(&h, PR1, &["--until", "checks-done", "--sha", &full]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(!has_line(&out, "replaced an earlier notify"), "{out:?}");
    assert_eq!(subs(&h).await, 2);
    assert_eq!(notices(&h).len(), 2);
    let upper = A.to_uppercase();
    let out = notify(&h, PR1, &["--until", "checks-done", "--sha", &upper]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_line(&out, "replaced an earlier notify"), "{out:?}");
    assert_eq!(subs(&h).await, 2);
    assert_eq!(notices(&h).len(), 3);
    h.finish().await;
}

// TST-8: notify-register
// NTF-2: a replacement takes no new place, so it works at the limit for one socket.
#[tokio::test]
async fn scn_notify_register_replacement_at_32() {
    let (h, _) = start(Opts::default(), |w| {
        settled_prs(w, PER_SOCKET_MAX);
    })
    .await;
    adv(&h, 20).await;
    for n in 1..=PER_SOCKET_MAX {
        let out = notify(&h, &format!("octo/app#{n}"), &["--until", "merged"]).await;
        assert_eq!(out.code, 0, "{out:?}");
    }
    assert_eq!(subs(&h).await, 32);
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_line(&out, "replaced an earlier notify"), "{out:?}");
    assert_eq!(subs(&h).await, 32);
    h.finish().await;
}

// TST-8: notify-register
// NTF-3, MOD-1: a renamed PR's subscription is found by its old name, and noticed by its new.
#[tokio::test]
async fn scn_notify_register_renamed() {
    let (h, (pr, repo)) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        (settled_pr(w, repo), repo)
    })
    .await;
    adv(&h, 20).await;
    let b = h.new_session("b");
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let out = notify_from(&h, &b, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(subs(&h).await, 2);
    h.github().world(|w| w.rename_repo(repo, "app2"));
    adv(&h, 40).await;
    let out = notify(&h, PR1, &["--cancel"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_line(&out, "cancelled: 1"), "{out:?}");
    h.github().world(|w| w.merge(pr));
    adv(&h, 40).await;
    let mine = notices(&h);
    assert_eq!(mine.len(), 1, "{mine:?}");
    let theirs = b.notices();
    assert_eq!(theirs.len(), 2, "{theirs:?}");
    assert!(theirs[1].text.contains("octo/app2#1 merged"), "{theirs:?}");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify-no-session
// ---------------------------------------------------------------------------------------------

/// `notify <pr> <args>` with the session variable set to `socket`, or taken out with `None`.
async fn notify_with(h: &Harness, socket: Option<&str>, pr: &str, args: &[&str]) -> Output {
    let mut a = vec!["notify", pr];
    a.extend_from_slice(args);
    let cmd = h.cmd(&a);
    let cmd = match socket {
        Some(path) => cmd.var(SOCKET_VAR, path),
        None => cmd.without(SOCKET_VAR),
    };
    drive1(h, cmd.bg()).await
}

// TST-8: notify-no-session
// NTF-2, NTF-11, CLI-5: a command with no session reaches no daemon, and says what to use instead.
#[tokio::test]
async fn scn_notify_no_session_missing() {
    let (h, _) = plain_open(Opts::default()).await;
    let log = h.log().contents();
    let requests = h.github().requests().len();
    let out = notify_with(&h, None, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 11, "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(
        out.stderr
            .contains("circitor wait octo/app#1 --until merged"),
        "{out:?}"
    );
    assert_eq!(subs(&h).await, 0);
    assert_eq!(h.log().contents(), log);
    assert_eq!(h.github().requests().len(), requests);

    // A path that is a regular file: `no_session`.
    let plain = h.dir().join("plain-file");
    std::fs::write(&plain, "not a socket").unwrap();
    let path = plain.to_str().unwrap();
    let out = notify_with(&h, Some(path), PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 11, "{out:?}");
    let out = notify_with(&h, Some(path), PR1, &["--until", "merged", "--json"]).await;
    assert_eq!(out.code, 11, "{out:?}");
    assert!(out.stdout.contains("no_session"), "{out:?}");
    // A relative path: `bad_request`.
    let out = notify_with(&h, Some("relative.sock"), PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 2, "{out:?}");
    let out = notify_with(
        &h,
        Some("relative.sock"),
        PR1,
        &["--until", "merged", "--json"],
    )
    .await;
    assert_eq!(out.code, 2, "{out:?}");
    assert!(out.stdout.contains("bad_request"), "{out:?}");
    assert_eq!(subs(&h).await, 0);
    assert!(notices(&h).is_empty());
    h.finish().await;
}

// TST-8: notify-no-session
// NTF-2: a confirmation that isn't written keeps nothing, and takes the replaced one with it.
#[tokio::test]
async fn scn_notify_no_session_refused_confirmation() {
    let (h, _) = plain_open(Opts::default()).await;
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(subs(&h).await, 1);
    h.session().refuse();
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 11, "{out:?}");
    assert_eq!(subs(&h).await, 0);
    assert_eq!(notices(&h).len(), 1);
    h.finish().await;
}

// TST-8: notify-no-session
// NTF-2: 32 for one socket; a replacement is no new place.
#[tokio::test]
async fn scn_notify_no_session_the_33rd() {
    let (h, _) = start(Opts::default(), |w| {
        settled_prs(w, PER_SOCKET_MAX + 1);
    })
    .await;
    adv(&h, 20).await;
    for n in 1..=PER_SOCKET_MAX {
        let out = notify(&h, &format!("octo/app#{n}"), &["--until", "merged"]).await;
        assert_eq!(out.code, 0, "{out:?}");
    }
    assert_eq!(subs(&h).await, 32);
    let last = format!("octo/app#{}", PER_SOCKET_MAX + 1);
    let out = notify(&h, &last, &["--until", "merged"]).await;
    assert_eq!(out.code, 10, "{out:?}");
    let out = notify(&h, &last, &["--until", "merged", "--json"]).await;
    assert_eq!(out.code, 10, "{out:?}");
    assert!(out.stdout.contains("limit"), "{out:?}");
    assert_eq!(subs(&h).await, 32);
    // Another session has its own 32.
    let b = h.new_session("b");
    let out = notify_from(&h, &b, &last, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(subs(&h).await, 33);
    h.finish().await;
}

// TST-8: notify-no-session
// NTF-2: 64 in all; an answer already known is never refused, and a replacement takes no place.
#[tokio::test]
async fn scn_notify_no_session_the_65th() {
    let total = SUBSCRIPTIONS_MAX + 2;
    let (h, _) = start(Opts::default(), move |w| {
        let prs = settled_prs(w, total);
        w.merge(prs[total - 1]);
    })
    .await;
    adv(&h, 20).await;
    let b = h.new_session("b");
    let c = h.new_session("c");
    for n in 1..=SUBSCRIPTIONS_MAX {
        let pr = format!("octo/app#{n}");
        let out = if n <= PER_SOCKET_MAX {
            notify(&h, &pr, &["--until", "merged"]).await
        } else {
            notify_from(&h, &b, &pr, &["--until", "merged"]).await
        };
        assert_eq!(out.code, 0, "{pr}: {out:?}");
    }
    assert_eq!(subs(&h).await, SUBSCRIPTIONS_MAX as u64);
    let sixty_fifth = format!("octo/app#{}", SUBSCRIPTIONS_MAX + 1);
    let out = notify_from(&h, &c, &sixty_fifth, &["--until", "merged"]).await;
    assert_eq!(out.code, 10, "{out:?}");
    assert_eq!(subs(&h).await, SUBSCRIPTIONS_MAX as u64);
    // The merged one's answer is already known.
    let merged = format!("octo/app#{total}");
    let out = notify_from(&h, &c, &merged, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_prefix(&out, "already: "), "{out:?}");
    // One of the 64 again is a replacement.
    let out = notify(&h, PR1, &["--until", "merged"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_line(&out, "replaced an earlier notify"), "{out:?}");
    assert_eq!(subs(&h).await, SUBSCRIPTIONS_MAX as u64);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// sleep, clock-back
// ---------------------------------------------------------------------------------------------

/// A settled open PR that merges 30 minutes on, `notify --until merged --for 3600` registered.
async fn merging_in_30_minutes() -> Harness {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        w.after(s(1800), move |w| w.merge(pr));
    })
    .await;
    adv(&h, 20).await;
    let out = notify(&h, PR1, &["--until", "merged", "--for", "3600"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    // At 10 minutes.
    adv(&h, 580).await;
    assert_eq!(notices(&h).len(), 1);
    h
}

// TST-8: sleep
// NTF-4, POL-22: a sleep counts against `for`, but an expiry never beats the catch-up poll.
#[tokio::test]
async fn scn_sleep_notify_wake_poll_answered() {
    let h = merging_in_30_minutes().await;
    h.clock().sleep_wall_only(s(2 * 3600)).await;
    adv(&h, 60).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[1].text.contains("merged"), "{got:?}");
    assert!(!got[1].text.contains("expired"), "{got:?}");
    assert_eq!(subs(&h).await, 0);
    adv(&h, 600).await;
    assert_eq!(notices(&h).len(), 2);
    h.finish().await;
}

// TST-8: sleep
// NTF-4, POL-22: with no answer from GitHub, it expires `CATCH_UP` after the wake, marked stale.
#[tokio::test]
async fn scn_sleep_notify_wake_poll_failing() {
    let h = merging_in_30_minutes().await;
    h.github().fault(Fault::Status(500), Times::UntilCleared);
    h.clock().sleep_wall_only(s(2 * 3600)).await;
    let woke = h.clock().mono();
    assert_eq!(notices(&h).len(), 1);
    h.clock().advance(CATCH_UP - s(5)).await;
    assert_eq!(notices(&h).len(), 1);
    h.clock().advance(s(5) + TICK).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got[1]
            .text
            .contains("expired after 1h; it's ready (stale, "),
        "{got:?}"
    );
    assert!(got[1].mono.since(woke) >= CATCH_UP);
    assert_eq!(subs(&h).await, 0);
    h.finish().await;
}

// TST-8: clock-back
// NTF-4, POL-26: the wall clock stepping back moves no expiry.
#[tokio::test]
async fn scn_clock_back_notify() {
    let (h, _) = plain_open(Opts::default()).await;
    let out = notify(&h, PR1, &["--until", "merged", "--for", "600"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let base = notices(&h)[0].mono;
    adv(&h, 60).await;
    h.clock().set_wall_back(s(3600)).await;
    let to = base.plus(s(599)).since(h.clock().mono());
    h.clock().advance(to).await;
    assert_eq!(notices(&h).len(), 1);
    h.clock().advance(s(1) + TICK).await;
    let got = notices(&h);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[1].text.contains("expired after 10m"), "{got:?}");
    let at = got[1].mono.since(base);
    assert!(at >= s(600) && at <= s(600) + TICK, "expired after {at:?}");
    h.finish().await;
}
