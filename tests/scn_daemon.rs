#![cfg(feature = "test-endpoint")]
//! T33: TST-8's scenarios about the daemon's own life and its failures: `cursor-reset`,
//! `github-down`, `token-rotated`, `sleep`, `restart`, `starting`, `first-run`, `restart-offline`,
//! `crash`, `files-removed` and `clock-back`. Each runs on the test clock, in one process, and ends
//! with `h.finish()` (TST-11). Login `octocat`; the user's PRs are `octo/app#N`, other people's
//! `else/lib#N`; a "settled" PR has an `updatedAt` two minutes old at the first read.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::cli::Output;
use circitor::config::Config;
use circitor::testkit::github::{PrId, World};
use circitor::testkit::harness::{Bg, Harness, Opts};
use circitor::testkit::server::{Aim, Fault, Times};
use circitor::testkit::{MARKER_GITHUB_TEXT, MARKER_TOKEN, MARKER_TOKEN_2};
use circitor::token::{GhFailure, Token, TokenRead};
use serde_json::Value;

/// POL-19: the first retry after a failure, and the most a retry waits.
const BACKOFF_FIRST: u64 = 5;
const BACKOFF_MAX: u64 = 300;
/// POL-19: a request is abandoned after this long.
const REQUEST_TIMEOUT: u64 = 20;
/// API-18: how long a `list` waits for the first poll.
const STARTING_WAIT: u64 = 10;
/// POL-3: the periodic token read.
const TOKEN_EVERY: u64 = 600;
/// STA-10: the event history.
const HISTORY: i64 = 500;
/// OPS-2: the poll interval of the active tier.
const INTERVAL: u64 = 30;
/// The login of the fake GitHub's viewer in these scenarios.
const LOGIN: &str = "octocat";

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

async fn adv(h: &Harness, secs: u64) {
    h.clock().advance(s(secs)).await;
}

fn mono_secs(h: &Harness) -> u64 {
    use circitor::clock::Clock;
    h.clock().mono().as_duration().as_secs()
}

fn json(out: &Output) -> Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

/// The error body of a failed `--json` command (API-3): it's on stdout.
fn error_of(out: &Output) -> Value {
    json(out)["error"].clone()
}

async fn status(h: &Harness) -> Value {
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    json(&out)
}

/// A command that needs no clock time: it's done once the daemon has settled.
async fn quick(h: &Harness, args: &[&str]) -> Output {
    let bg = h.cli_bg(args);
    h.clock().settle().await;
    assert!(
        bg.is_done(),
        "`{}` isn't done though nothing is waiting on the clock",
        args.join(" ")
    );
    bg.await
}

/// Moves the clock in steps of `step` seconds until the command is done (at most `limit` seconds).
async fn till_done(h: &Harness, bg: Bg, step: u64, limit: u64) -> Output {
    let mut moved = 0;
    while !bg.is_done() && moved < limit {
        adv(h, step).await;
        moved += step;
    }
    assert!(bg.is_done(), "the command wasn't done after {limit} s");
    bg.await
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap_or_else(|| panic!("no `{key}` list in {v}"))
        .iter()
        .map(|r| {
            r["pr"].as_str().map_or_else(
                || r.as_str().unwrap_or_default().to_string(),
                str::to_string,
            )
        })
        .collect()
}

fn record<'a>(v: &'a Value, pr: &str) -> &'a Value {
    v["records"]
        .as_array()
        .and_then(|rs| rs.iter().find(|r| r["pr"] == pr))
        .unwrap_or_else(|| panic!("no record of {pr} in {v}"))
}

/// A settled PR of the user's in `octo/app` (number: the next in it).
fn mine(w: &mut World) -> PrId {
    let start = w.now();
    let repo = w.repo("octo", "app");
    let pr = w.open_pr(repo);
    w.set_created_at(pr, start.minus(s(3600)));
    w.set_updated_at(pr, start.minus(s(120)));
    pr
}

/// The viewer is `octocat`, with `n` settled PRs `octo/app#1..n`, and nine other people's `else/lib#1..9`.
fn world(w: &mut World, n: usize) -> Vec<PrId> {
    w.set_login(LOGIN);
    let prs = (0..n).map(|_| mine(w)).collect();
    others(w);
    prs
}

/// Nine PRs of other people's, `else/lib#1..9`: never tracked unless asked for.
fn others(w: &mut World) {
    let start = w.now();
    let repo = w.repo("else", "lib");
    for _ in 0..9 {
        let pr = w.open_pr(repo);
        w.set_author(pr, Some("someone-else"));
        w.set_created_at(pr, start.minus(s(3600)));
        w.set_updated_at(pr, start.minus(s(600)));
    }
}

type Slot = Arc<Mutex<Vec<PrId>>>;

/// The ids of the world's PRs, in order, as the fake knows them.
fn node_ids(h: &Harness, prs: &[PrId]) -> Vec<String> {
    h.github()
        .world(|w| prs.iter().map(|p| w.node_id(*p).to_string()).collect())
}

/// How many requests the fake has seen that hold a finding (a poll's first request).
fn polls(h: &Harness) -> usize {
    h.github()
        .requests()
        .iter()
        .filter(|r| r.finding.is_some())
        .count()
}

// STA-9: a cursor from before a restart is from another epoch.
// TST-8: cursor-reset
#[tokio::test]
async fn scn_cursor_reset_epoch() {
    let h = Harness::start(Opts::default().world(|w| {
        world(w, 3);
    }))
    .await;
    adv(&h, 5).await;
    let before = json(&h.cli(&["list", "--json"]).await);
    let c1 = before["cursor"].as_str().expect("a cursor").to_string();
    assert_eq!(
        before["records"].as_array().map(Vec::len),
        Some(3),
        "{before}"
    );

    // Each start has its own seed, so the epoch is new.
    h.restart().await;
    adv(&h, 5).await;
    let now = json(&h.cli(&["list", "--json"]).await);
    let c_now = now["cursor"].as_str().expect("a cursor").to_string();
    assert_ne!(
        c1.split('-').next(),
        c_now.split('-').next(),
        "the epoch is new"
    );

    // STA-9: `changes --since c1`: reset, every record, and the new epoch's events.
    let out = h.cli(&["changes", "--since", &c1, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    assert_eq!(v["reset"], true, "{v}");
    assert_eq!(v["records"].as_array().map(Vec::len), Some(3), "{v}");
    let events = v["events"].as_array().expect("events");
    assert_eq!(events.len(), 3, "the new epoch's events: {v}");
    for e in events {
        assert!(
            e["kinds"]
                .as_array()
                .is_some_and(|ks| ks.iter().any(|k| k["kind"] == "tracked")),
            "a loaded PR's first event holds `tracked`: {e}"
        );
    }
    let text = h.cli(&["changes", "--since", &c1]).await;
    assert_eq!(text.code, 0, "{text:?}");
    assert!(
        text.stdout
            .starts_with("reset: cursor too old, current state follows"),
        "{text:?}"
    );

    // `list --since c1`: reset and every record.
    let v = json(&h.cli(&["list", "--since", &c1, "--json"]).await);
    assert_eq!(v["reset"], true, "{v}");
    assert_eq!(names(&v, "records").len(), 3, "{v}");

    // `wait --until changed --since c1`: satisfied at once, with the reset.
    let wait = h.cli_bg(&[
        "wait",
        "octo/app#1",
        "--until",
        "changed",
        "--since",
        &c1,
        "--json",
    ]);
    h.clock().settle().await;
    assert!(wait.is_done(), "a too old cursor ends a wait at once");
    let out = wait.await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    assert_eq!(v["outcome"], "satisfied", "{v}");
    assert_eq!(v["reset"], true, "{v}");
    h.finish().await;
}

// STA-9, STA-10: a cursor whose events fell out of the 500, and one above the counter.
// TST-8: cursor-reset
#[tokio::test]
async fn scn_cursor_reset_history() {
    let slot = Slot::default();
    let put = slot.clone();
    let h = Harness::start(Opts::default().world(move |w| {
        *put.lock().unwrap() = world(w, 26);
    }))
    .await;
    let prs = slot.lock().unwrap().clone();
    adv(&h, 5).await;
    let v = json(&h.cli(&["list", "--json"]).await);
    assert_eq!(v["records"].as_array().map(Vec::len), Some(26), "{v}");
    let c2 = v["cursor"].as_str().expect("a cursor").to_string();
    let (epoch, rev2) = c2.split_once('-').expect("<epoch>-<rev>");
    let rev2: i64 = rev2.parse().expect("a rev");

    // 26 PRs flip `draft` at every poll, until over 500 events follow the cursor.
    let mut rev = rev2;
    let mut round = 0;
    while rev - rev2 <= HISTORY + 20 {
        round += 1;
        assert!(round < 40, "{round} polls and only {} events", rev - rev2);
        h.github().world(|w| {
            for pr in &prs {
                let now = w.is_draft(*pr);
                w.set_draft(*pr, !now);
            }
        });
        let before = h.github().requests().len();
        for _ in 0..(INTERVAL * 2) {
            adv(&h, 1).await;
            if h.github().requests().len() >= before + 2 {
                break;
            }
        }
        let v = json(&h.cli(&["list", "--json"]).await);
        let c = v["cursor"].as_str().expect("a cursor");
        rev = c.split_once('-').expect("cursor").1.parse().expect("a rev");
    }

    let out = h.cli(&["changes", "--since", &c2, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    assert_eq!(v["reset"], true, "{v}");
    assert_eq!(v["events"], serde_json::json!([]), "no events with a reset");
    assert_eq!(v["records"].as_array().map(Vec::len), Some(26), "{v}");

    // A cursor of the same epoch with a `rev` above the counter.
    let above = format!("{epoch}-{}", rev + 1000);
    let v = json(&h.cli(&["changes", "--since", &above, "--json"]).await);
    assert_eq!(v["reset"], true, "{v}");
    assert_eq!(v["records"].as_array().map(Vec::len), Some(26), "{v}");

    // API-1: a cursor that isn't one.
    let out = h.cli(&["list", "--since", "nonsense", "--json"]).await;
    assert_eq!(out.code, 2, "{out:?}");
    assert_eq!(error_of(&out)["code"], "bad_request", "{out:?}");
    h.finish().await;
}

// ---- github-down ----

/// Moves the clock to `secs` of monotonic time (never back).
async fn adv_to(h: &Harness, secs: u64) {
    let now = mono_secs(h);
    assert!(secs >= now, "adv_to({secs}) from {now}");
    adv(h, secs - now).await;
}

/// How many log lines hold `needle`.
fn log_count(h: &Harness, needle: &str) -> usize {
    h.log()
        .contents()
        .lines()
        .filter(|l| l.contains(needle))
        .count()
}

/// The monotonic seconds of the polls' requests (not a read-through's) from `from` on.
fn sent_at(h: &Harness, from: u64) -> Vec<u64> {
    h.github()
        .requests()
        .iter()
        .filter(|r| r.lookup.is_none())
        .map(|r| r.mono.as_duration().as_secs())
        .filter(|m| *m >= from)
        .collect()
}

fn lookups(h: &Harness) -> usize {
    h.github()
        .requests()
        .iter()
        .filter(|r| r.lookup.is_some())
        .count()
}

/// Every record of a `list --json` is (not) stale.
fn all_stale(v: &Value, stale: bool) {
    let records = v["records"].as_array().expect("records");
    assert!(!records.is_empty(), "{v}");
    for r in records {
        assert_eq!(r["stale"], stale, "{r}");
    }
}

/// Moves the clock a second at a time until a request was answered with a fault; its monotonic second.
async fn until_faulted(h: &Harness) -> u64 {
    for _ in 0..120 {
        adv(h, 1).await;
        if let Some(r) = h.github().requests().iter().find(|r| r.fault.is_some()) {
            return r.mono.as_duration().as_secs();
        }
    }
    panic!("no request met the fault in 120 s");
}

// POL-19, OPS-16, POL-16: 5xx for 10 minutes.
// TST-8: github-down
#[tokio::test]
async fn scn_github_down() {
    let slot = Slot::default();
    let put = slot.clone();
    let h = Harness::start(Opts::default().world(move |w| {
        *put.lock().unwrap() = world(w, 3);
    }))
    .await;
    let prs = slot.lock().unwrap().clone();
    adv(&h, 5).await;
    let wait = h.cli_bg(&[
        "wait",
        "octo/app#1",
        "--until",
        "merged",
        "--timeout",
        "900",
        "--json",
    ]);
    adv(&h, 20).await;
    assert_eq!(quick(&h, &["list"]).await.code, 0);
    assert_eq!(status(&h).await["waiters"], 1);

    // 5xx from `t`.
    h.github().fault(Fault::Status(503), Times::UntilCleared);
    let t = until_faulted(&h).await;

    // Each gap double the last, capped at 300 s: the read-through meanwhile.
    adv_to(&h, t + 76).await;
    let before = lookups(&h);
    let out = quick(&h, &["show", "else/lib#9", "--json"]).await;
    assert_eq!(out.code, 9, "{out:?}");
    let e = error_of(&out);
    assert_eq!(e["code"], "github_unavailable", "{out:?}");
    assert!(
        e["until"].is_string(),
        "the answer says until when: {out:?}"
    );
    assert_eq!(
        lookups(&h),
        before + 1,
        "one read-through request in the step"
    );
    let again = quick(&h, &["show", "else/lib#9", "--json"]).await;
    assert_eq!(again.code, 9, "{again:?}");
    assert_eq!(lookups(&h), before + 1, "none in the same step");

    adv_to(&h, t + 120).await;
    let v = json(&quick(&h, &["list", "--json"]).await);
    all_stale(&v, true);
    assert_eq!(v["daemon"]["problem"], "github_unavailable", "{v}");
    assert!(v["daemon"]["until"].is_string(), "{v}");
    assert_eq!(status(&h).await["waiters"], 1, "the wait is still held");

    adv_to(&h, t + 156).await;
    let before = lookups(&h);
    let out = quick(&h, &["show", "else/lib#9", "--json"]).await;
    assert_eq!(out.code, 9, "{out:?}");
    assert_eq!(
        lookups(&h),
        before + 1,
        "after the next failed try, one more"
    );

    for at in [240, 360, 480, 600] {
        adv_to(&h, t + at).await;
        let v = json(&quick(&h, &["list", "--json"]).await);
        all_stale(&v, true);
        assert_eq!(v["daemon"]["problem"], "github_unavailable", "{v}");
        assert!(!wait.is_done());
    }
    let sent: Vec<u64> = sent_at(&h, t).into_iter().map(|m| m - t).collect();
    assert_eq!(
        sent,
        [0, BACKOFF_FIRST, 15, 35, 75, 155, 315],
        "the polls' requests, none before 615 s"
    );
    // Each gap is double the last, to at most `BACKOFF_MAX`.
    let mut gap = BACKOFF_FIRST;
    for pair in sent.windows(2) {
        assert_eq!(pair[1] - pair[0], gap, "{sent:?}");
        gap = (gap * 2).min(BACKOFF_MAX);
    }
    // One line for the problem's start, none for each retry.
    let log = h.log().contents();
    assert_eq!(
        log_count(&h, "problem begin code=github_unavailable"),
        1,
        "{log}"
    );
    assert_eq!(log_count(&h, "problem end"), 0, "{log}");

    // Clear the fault: the next try succeeds, a poll follows at once.
    h.github().clear_faults();
    adv_to(&h, t + 620).await;
    let after: Vec<u64> = sent_at(&h, t + 600).into_iter().map(|m| m - t).collect();
    assert_eq!(
        after.first(),
        Some(&615),
        "the next try at 615 s: {after:?}"
    );
    // The try that recovers is a poll itself: its finding and its refreshes in one request.
    let first = h
        .github()
        .requests()
        .into_iter()
        .find(|r| r.mono.as_duration().as_secs() >= t + 600)
        .expect("a request");
    assert!(
        first.finding.is_some() && first.fault.is_none(),
        "{first:?}"
    );
    let v = json(&quick(&h, &["list", "--json"]).await);
    all_stale(&v, false);
    assert_eq!(v["daemon"]["problem"], Value::Null, "{v}");
    let log = h.log().contents();
    assert_eq!(log_count(&h, "problem end"), 1, "{log}");
    assert_eq!(
        log_count(&h, "problem begin code=github_unavailable"),
        1,
        "{log}"
    );

    h.github().world(|w| w.merge(prs[0]));
    let out = till_done(&h, wait, 1, 60).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(&out)["outcome"], "satisfied", "{out:?}");
    h.finish().await;
}

/// POL-19: a poll fails in one way for 60 s, then GitHub is well again. `fault` is what the fake
/// does (`None`: it refuses connections); returns the harness, with the daemon recovered and `t`,
/// the monotonic second of the first failure, for the caller to look at the requests.
async fn down_for_a_minute(fault: Option<Fault>) -> (Harness, u64) {
    let h = Harness::start(Opts::default().world(|w| {
        world(w, 3);
    }))
    .await;
    adv(&h, 5).await;
    assert_eq!(quick(&h, &["list"]).await.code, 0);
    let t = match &fault {
        Some(f) => {
            h.github().fault(f.clone(), Times::UntilCleared);
            until_faulted(&h).await
        }
        None => {
            h.github().refuse().await;
            let mut found = None;
            for _ in 0..120 {
                adv(&h, 1).await;
                if status(&h).await["daemon"]["problem"] == "github_unavailable" {
                    found = Some(mono_secs(&h));
                    break;
                }
            }
            found.expect("a refused connection is a problem")
        }
    };
    adv_to(&h, t + 60).await;
    let v = json(&quick(&h, &["list", "--json"]).await);
    all_stale(&v, true);
    assert_eq!(v["daemon"]["problem"], "github_unavailable", "{v}");
    assert!(v["daemon"]["until"].is_string(), "{v}");
    // One line for the start, none for each retry.
    assert_eq!(
        log_count(&h, "problem begin code=github_unavailable"),
        1,
        "{}",
        h.log().contents()
    );
    assert_eq!(log_count(&h, "problem end"), 0);

    h.github().clear_faults();
    h.github().listen().await;
    let mut recovered = false;
    for _ in 0..300 {
        adv(&h, 1).await;
        if status(&h).await["daemon"]["problem"] == Value::Null {
            recovered = true;
            break;
        }
    }
    assert!(recovered, "no recovery within 300 s of the end");
    let v = json(&quick(&h, &["list", "--json"]).await);
    all_stale(&v, false);
    assert_eq!(log_count(&h, "problem end"), 1, "{}", h.log().contents());
    assert_eq!(log_count(&h, "problem begin code=github_unavailable"), 1);
    (h, t)
}

// POL-19: a hang is abandoned 20 s after it was sent, and the next try comes 5 s later.
// TST-8: github-down
#[tokio::test]
async fn scn_github_down_hang() {
    let (h, t) = down_for_a_minute(Some(Fault::Hang)).await;
    let sent: Vec<u64> = sent_at(&h, t).into_iter().map(|m| m - t).collect();
    assert_eq!(sent[..2], [0, REQUEST_TIMEOUT + BACKOFF_FIRST], "{sent:?}");
    h.finish().await;
}

// POL-19: a refused connection.
// TST-8: github-down
#[tokio::test]
async fn scn_github_down_refused() {
    let (h, _) = down_for_a_minute(None).await;
    h.finish().await;
}

// POL-19, POL-24: a 200 whose body isn't JSON.
// TST-8: github-down
#[tokio::test]
async fn scn_github_down_not_json() {
    let (h, t) = down_for_a_minute(Some(Fault::NotJson)).await;
    let sent: Vec<u64> = sent_at(&h, t).into_iter().map(|m| m - t).collect();
    assert_eq!(sent[..4], [0, BACKOFF_FIRST, 15, 35], "{sent:?}");
    h.finish().await;
}

// OPS-16: a redirect isn't followed: its target's hit count stays 0.
// TST-8: github-down
#[tokio::test]
async fn scn_github_down_redirect() {
    let (h, t) = down_for_a_minute(Some(Fault::Redirect)).await;
    let sent: Vec<u64> = sent_at(&h, t).into_iter().map(|m| m - t).collect();
    assert_eq!(sent[..4], [0, BACKOFF_FIRST, 15, 35], "{sent:?}");
    assert_eq!(h.github().redirect_hits(), 0);
    h.finish().await;
}

// ---- token-rotated ----

fn token(marker: &str) -> TokenRead {
    TokenRead::Token(Token::new(marker.to_string()))
}

/// Whether a request's `Authorization` carries `marker`; the value is never printed.
fn carries(r: &circitor::testkit::server::Request, marker: &str) -> bool {
    r.authorization
        .as_deref()
        .is_some_and(|a| a.contains(marker))
}

// POL-20: one 401, the token is read again, the repeat carries the new one and is answered.
// TST-8: token-rotated
#[tokio::test]
async fn scn_token_rotated_once() {
    let h = Harness::start(Opts::default().world(|w| {
        world(w, 3);
    }))
    .await;
    adv(&h, 5).await;
    assert_eq!(quick(&h, &["list"]).await.code, 0);
    let reads = h.token().reads();
    let seen = h.github().requests().len();

    h.github().fault(Fault::Unauthorized, Times::Requests(1));
    h.token().set(token(MARKER_TOKEN_2));
    let t = until_faulted(&h).await;
    adv_to(&h, t + 3).await;

    let after: Vec<_> = h.github().requests().into_iter().skip(seen).collect();
    let refused: Vec<_> = after
        .iter()
        .filter(|r| r.fault == Some(Fault::Unauthorized))
        .collect();
    assert_eq!(refused.len(), 1, "{after:?}");
    assert!(
        carries(refused[0], MARKER_TOKEN),
        "the refused one carried the old token"
    );
    let repeats: Vec<_> = after
        .iter()
        .filter(|r| carries(r, MARKER_TOKEN_2))
        .collect();
    assert_eq!(repeats.len(), 1, "the request was repeated once: {after:?}");
    assert!(
        repeats[0].fault.is_none() && repeats[0].finding.is_some(),
        "and answered: {after:?}"
    );
    assert_eq!(h.token().reads(), reads + 1, "the token was read once more");
    let st = status(&h).await;
    assert_eq!(st["daemon"]["problem"], Value::Null, "{st}");
    all_stale(&json(&quick(&h, &["list", "--json"]).await), false);
    h.finish().await;
}

// POL-20: a token that stays bad: `auth`, answers from memory, a read-through exits 7.
// TST-8: token-rotated
#[tokio::test]
async fn scn_token_rotated_stays_bad() {
    let h = Harness::start(Opts::default().world(|w| {
        world(w, 3);
    }))
    .await;
    adv(&h, 5).await;
    let wait = h.cli_bg(&[
        "wait",
        "octo/app#1",
        "--until",
        "merged",
        "--timeout",
        "900",
        "--json",
    ]);
    adv(&h, 20).await;

    h.github().fault(Fault::Unauthorized, Times::UntilCleared);
    let t = until_faulted(&h).await;
    let mut in_auth = false;
    for _ in 0..60 {
        adv(&h, 1).await;
        if status(&h).await["daemon"]["problem"] == "auth" {
            in_auth = true;
            break;
        }
    }
    assert!(
        in_auth,
        "no `auth` problem within 60 s of the first 401 (at {t} s)"
    );
    // POL-20: the request was repeated once, with a token read in between.
    let refused = h
        .github()
        .requests()
        .iter()
        .filter(|r| r.fault == Some(Fault::Unauthorized))
        .count();
    assert!(refused >= 2, "the 401 and its repeat: {refused}");

    let out = quick(&h, &["list"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(
        lines.last().is_some_and(|l| l.starts_with("cursor: ")),
        "{out:?}"
    );
    assert!(
        lines[lines.len() - 2].starts_with("problem: auth"),
        "{out:?}"
    );
    let v = json(&quick(&h, &["list", "--json"]).await);
    all_stale(&v, true);
    assert_eq!(v["daemon"]["problem"], "auth", "{v}");
    let out = quick(&h, &["show", "octo/app#1"]).await;
    assert_eq!(out.code, 0, "from memory: {out:?}");
    let out = quick(&h, &["show", "else/lib#9", "--json"]).await;
    assert_eq!(out.code, 7, "{out:?}");
    assert!(
        !out.stdout.contains(MARKER_GITHUB_TEXT) && !out.stderr.contains(MARKER_GITHUB_TEXT),
        "the 401's body: {out:?}"
    );
    let e = error_of(&out);
    assert_eq!(e["code"], "auth", "{out:?}");
    let message = e["message"].as_str().unwrap_or_default();
    let source = status(&h).await["token_source"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        message.contains(&source),
        "the message names the token source `{source}`: {message}"
    );
    assert!(!wait.is_done(), "the wait stays held");
    assert_eq!(status(&h).await["waiters"], 1);

    // Clear the fault and give the other token: back within 60 s.
    h.github().clear_faults();
    h.token().set(token(MARKER_TOKEN_2));
    let cleared = mono_secs(&h);
    let seen = h.github().requests().len();
    let mut back = false;
    for _ in 0..60 {
        adv(&h, 1).await;
        if status(&h).await["daemon"]["problem"] == Value::Null {
            back = true;
            break;
        }
    }
    assert!(
        back,
        "still in `auth` {} s after the fault went",
        mono_secs(&h) - cleared
    );
    assert!(
        h.github()
            .requests()
            .iter()
            .skip(seen)
            .any(|r| carries(r, MARKER_TOKEN_2) && r.fault.is_none())
    );
    all_stale(&json(&quick(&h, &["list", "--json"]).await), false);

    // OPS-9, POL-20: the 401's body is nowhere.
    assert!(
        !h.log().contents().contains(MARKER_GITHUB_TEXT),
        "{}",
        h.log().contents()
    );
    wait.cancel().await;
    h.finish().await;
}

// ---- sleep ----

const TWO_HOURS: u64 = 7200;

/// POL-22, TST-4: the wall clock jumps two hours, with (`both`) or without the monotonic clock.
/// Returns the harness just after the wake's poll, and the monotonic second of the wake.
async fn sleep_run(both: bool, down_at_wake: bool) -> (Harness, u64) {
    let slot = Slot::default();
    let put = slot.clone();
    let h = Harness::start(Opts::default().world(move |w| {
        let prs = world(w, 2);
        // The second PR merges one hour on.
        let second = prs[1];
        w.after(s(3600), move |w| w.merge(second));
        *put.lock().unwrap() = prs;
    }))
    .await;
    adv(&h, 5).await;
    assert_eq!(quick(&h, &["list"]).await.code, 0);
    let wait = h.cli_bg(&[
        "wait",
        "octo/app#1",
        "--until",
        "merged",
        "--timeout",
        "600",
        "--json",
    ]);
    adv(&h, 15).await;
    assert!(!wait.is_done());
    if down_at_wake {
        h.github().fault(Fault::Status(503), Times::UntilCleared);
    }
    let before = h.github().requests().len();

    if both {
        h.clock().sleep_both(s(TWO_HOURS)).await;
    } else {
        h.clock().sleep_wall_only(s(TWO_HOURS)).await;
    }
    let wake = mono_secs(&h);
    // A sleep is noticed at the next tick (POL-26), a wait's deadline checked then.
    let out = till_done(&h, wait, 1, 20).await;
    assert_eq!(out.code, 3, "{out:?}");
    let v = json(&out);
    assert_eq!(v["outcome"], "timeout", "{v}");

    // One poll on waking, never one per missed interval; the next an interval later at the soonest.
    adv(&h, 4 * INTERVAL).await;
    let finds: Vec<u64> = h
        .github()
        .requests()
        .iter()
        .skip(before)
        .filter(|r| r.finding.is_some())
        .map(|r| r.mono.as_duration().as_secs())
        .collect();
    assert!(!finds.is_empty(), "no poll after waking");
    let first = finds[0];
    assert!(
        first <= wake + 10 + 1,
        "the poll on waking: {finds:?} (woke at {wake})"
    );
    if !down_at_wake {
        let in_window = finds.iter().filter(|m| **m < first + INTERVAL).count();
        assert_eq!(
            in_window, 1,
            "exactly one poll in the first interval: {finds:?}"
        );
        if let Some(next) = finds.get(1) {
            assert!(*next >= first + INTERVAL, "{finds:?}");
        }
    }
    (h, wake)
}

// POL-22, TST-4: the monotonic clock didn't count the suspend: the sleep is noticed and logged.
// TST-8: sleep
#[tokio::test]
async fn scn_sleep_wall_only() {
    let (h, _) = sleep_run(false, false).await;
    assert_eq!(
        log_count(&h, "sleep seconds=7200"),
        1,
        "{}",
        h.log().contents()
    );
    let out = quick(&h, &["show", "octo/app#2", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(&out)["record"]["state"], "merged", "{out:?}");
    h.finish().await;
}

// POL-22, TST-4: the monotonic clock counted it: no sleep is noticed, the same outcomes follow.
// TST-8: sleep
#[tokio::test]
async fn scn_sleep_both() {
    let (h, _) = sleep_run(true, false).await;
    assert_eq!(log_count(&h, "sleep seconds="), 0, "{}", h.log().contents());
    let out = quick(&h, &["show", "octo/app#2", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(&out)["record"]["state"], "merged", "{out:?}");
    h.finish().await;
}

// POL-22, MOD-16: on waking every record is stale, though its `fetched_at` is seconds old on the
// monotonic clock; the wake poll fails, so they stay stale until the next applied refresh.
// TST-8: sleep
#[tokio::test]
async fn scn_sleep_stale() {
    let (h, _) = sleep_run(false, true).await;
    assert_eq!(
        log_count(&h, "sleep seconds=7200"),
        1,
        "{}",
        h.log().contents()
    );
    let v = json(&quick(&h, &["list", "--json"]).await);
    all_stale(&v, true);
    assert_eq!(v["daemon"]["problem"], "github_unavailable", "{v}");
    h.github().clear_faults();
    let mut fresh = false;
    for _ in 0..300 {
        adv(&h, 1).await;
        let v = json(&quick(&h, &["list", "--json"]).await);
        if v["daemon"]["problem"] == Value::Null {
            all_stale(&v, false);
            fresh = true;
            break;
        }
    }
    assert!(fresh, "the next applied refresh clears it");
    h.finish().await;
}

// ---- restart ----

/// The kinds an event holds, in order.
fn kinds(e: &Value) -> Vec<String> {
    e["kinds"]
        .as_array()
        .expect("kinds")
        .iter()
        .map(|k| k["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The event's kind of this word.
fn kind_of<'a>(e: &'a Value, word: &str) -> &'a Value {
    e["kinds"]
        .as_array()
        .and_then(|ks| ks.iter().find(|k| k["kind"] == word))
        .unwrap_or_else(|| panic!("no `{word}` in {e}"))
}

/// The epoch of a cursor.
fn epoch_of(cursor: &str) -> &str {
    cursor.split_once('-').expect("<epoch>-<rev>").0
}

// STA-5, STA-8, MOD-15, API-18: a PR saved open merges and gets comments while no daemon runs.
// TST-8: restart
#[tokio::test]
async fn scn_restart() {
    let slot = Slot::default();
    let put = slot.clone();
    let h = Harness::start(Opts::default().world(move |w| {
        let prs = world(w, 1);
        // One comment before the first read: the baseline (STA-5).
        w.comment(prs[0]);
        *put.lock().unwrap() = prs;
    }))
    .await;
    let prs = slot.lock().unwrap().clone();
    let id = node_ids(&h, &prs)[0].clone();
    adv(&h, 5).await;
    let out = quick(&h, &["track", "else/lib#2"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(h.stop().await, 0);
    let saved: Value = serde_json::from_str(&h.state_file().expect("a state file")).expect("JSON");
    assert_eq!(saved["prs"].as_array().map(Vec::len), Some(2), "{saved}");

    // No daemon: #1 merges and gets 2 more comments; 10 minutes pass.
    h.github().world(|w| {
        w.comment(prs[0]);
        w.comment(prs[0]);
        w.merge(prs[0]);
    });
    adv(&h, 600).await;

    // Restart with #1's node answering an error for the first request only.
    h.github()
        .pr_error(prs[0], "FORBIDDEN", Aim::node(&id), Times::Requests(1));
    h.restart().await;
    adv(&h, 5).await;
    let v = json(&quick(&h, &["list", "--json"]).await);
    assert_eq!(v["pending"], serde_json::json!(["octo/app#1"]), "{v}");
    assert!(
        v["records"]
            .as_array()
            .is_some_and(|rs| rs.iter().all(|r| r["pr"] != "octo/app#1")),
        "no record of it yet: {v}"
    );
    let cursor0 = format!("{}-0", epoch_of(v["cursor"].as_str().expect("a cursor")));

    // A wait on it is held, and nothing reads it through.
    let wait = h.cli_bg(&["wait", "octo/app#1", "--until", "merged", "--json"]);
    h.clock().settle().await;
    assert!(!wait.is_done(), "the wait is held");
    let by_name = |h: &Harness| {
        h.github()
            .requests()
            .iter()
            .filter(|r| r.lookup == Some(("octo".into(), "app".into(), 1)))
            .count()
    };
    assert_eq!(by_name(&h), 0, "no by-name request for #1");

    // After the next poll the wait is satisfied.
    let out = till_done(&h, wait, 1, 60).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(&out)["outcome"], "satisfied", "{out:?}");
    assert_eq!(by_name(&h), 0, "still no by-name request");

    let out = h
        .cli(&["changes", "octo/app#1", "--since", &cursor0, "--json"])
        .await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    assert_eq!(v["reset"], false, "{v}");
    let events = v["events"].as_array().expect("events");
    assert_eq!(events.len(), 1, "one first event: {v}");
    let e = &events[0];
    let mut held = kinds(e);
    held.sort();
    assert_eq!(
        held,
        ["activity", "merged", "tracked"],
        "tracked, merged and activity, no other: {e}"
    );
    assert_eq!(kind_of(e, "tracked")["source"], "authored", "{e}");
    let activity = kind_of(e, "activity");
    assert_eq!(activity["more"], false, "{e}");
    assert_eq!(
        activity["items"].as_array().map(Vec::len),
        Some(2),
        "exactly the 2 new comments: {e}"
    );
    assert_eq!(activity["counts"]["comments"], 3, "{e}");

    let v = json(&quick(&h, &["list", "--json", "--all"]).await);
    assert_eq!(record(&v, "else/lib#2")["source"], "manual", "{v}");
    assert_eq!(record(&v, "octo/app#1")["state"], "merged", "{v}");
    h.finish().await;
}

// ---- starting ----

// API-18: before the first poll answers, `list` and `changes` wait up to 10 s, then exit 9 `starting`.
// TST-8: starting
#[tokio::test]
async fn scn_starting() {
    let h = Harness::start(
        Opts::default()
            .world(|w| {
                world(w, 2);
            })
            .hold_first(),
    )
    .await;
    let mut hold = h.take_hold().expect("the first answer is held");
    hold.arrived().await;
    let list = h.cli_bg(&["list", "--json"]);
    let changes = h.cli_bg(&["changes", "--json"]);
    // `status` answers at once, throughout.
    let st = status(&h).await;
    assert_eq!(st["daemon"]["problem"], Value::Null, "{st}");
    adv(&h, STARTING_WAIT - 1).await;
    assert!(
        !list.is_done() && !changes.is_done(),
        "still waiting at 9 s"
    );
    assert_eq!(status(&h).await["tracked"]["authored"], 0);
    adv(&h, 1).await;
    for (name, bg) in [("list", list), ("changes", changes)] {
        assert!(bg.is_done(), "`{name}` is done at 10 s");
        let out = bg.await;
        assert_eq!(out.code, 9, "{name}: {out:?}");
        assert_eq!(error_of(&out)["code"], "starting", "{name}: {out:?}");
    }
    assert_eq!(status(&h).await["daemon"]["problem"], Value::Null);
    // Let it go: the first poll finishes and the commands answer.
    hold.release();
    adv(&h, 5).await;
    let out = quick(&h, &["list", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        json(&out)["records"].as_array().map(Vec::len),
        Some(2),
        "{out:?}"
    );
    h.finish().await;
}

// API-18, OPS-5: a token read 8 s slow: a `list` begun at 0 s is answered once the first poll is done.
// TST-8: starting
#[tokio::test]
async fn scn_starting_slow_token() {
    let h = Harness::start(
        Opts::default()
            .world(|w| {
                world(w, 2);
            })
            .token_delay(s(8)),
    )
    .await;
    let list = h.cli_bg(&["list", "--json"]);
    h.clock().settle().await;
    assert!(!list.is_done());
    let mut moved = 0;
    while !list.is_done() && moved < STARTING_WAIT {
        adv(&h, 1).await;
        moved += 1;
    }
    assert!(list.is_done());
    assert!(moved < STARTING_WAIT, "answered before 10 s, at {moved} s");
    let out = list.await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        json(&out)["records"].as_array().map(Vec::len),
        Some(2),
        "{out:?}"
    );
    h.finish().await;
}

// ---- first-run ----

// API-18, POL-20, OPS-5: no login from the start: `list` exits 7 at once, naming `gh auth login`;
// after a login the next `list` is answered within 5 s.
// TST-8: first-run
#[tokio::test]
async fn scn_first_run_no_login() {
    let h = Harness::start(
        Opts::default()
            .world(|w| {
                world(w, 2);
            })
            .token(TokenRead::NoLogin),
    )
    .await;
    // `status` answers at once.
    let st = status(&h).await;
    assert_eq!(st["daemon"]["problem"], "auth", "{st}");
    // `list`, with no advance: exit 7, not `starting`.
    let out = quick(&h, &["list", "--json"]).await;
    assert_eq!(out.code, 7, "{out:?}");
    let e = error_of(&out);
    assert_eq!(e["code"], "auth", "{out:?}");
    assert!(
        e["message"]
            .as_str()
            .unwrap_or_default()
            .contains("gh auth login"),
        "{out:?}"
    );
    assert_eq!(
        h.github().requests().len(),
        0,
        "no request is sent without a token"
    );

    // A login: the next `list` is answered within 5 s.
    // POL-20: a demand reads the token again when the last read ended 5 s ago.
    adv(&h, 5).await;
    h.token().set(token(MARKER_TOKEN));
    let list = h.cli_bg(&["list", "--json"]);
    let out = till_done(&h, list, 1, 5).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        json(&out)["records"].as_array().map(Vec::len),
        Some(2),
        "{out:?}"
    );
    h.finish().await;
}

// API-18, POL-3, OPS-5: a token read 8 s slow: what needs GitHub waits for it, `status` doesn't.
// TST-8: first-run
#[tokio::test]
async fn scn_first_run_slow_token() {
    let h = Harness::start(
        Opts::default()
            .world(|w| {
                world(w, 2);
            })
            .token_delay(s(8)),
    )
    .await;
    let list = h.cli_bg(&["list", "--json"]);
    let show = h.cli_bg(&["show", "else/lib#9", "--json"]);
    adv(&h, 1).await;
    let st = status(&h).await;
    assert_eq!(st["daemon"]["problem"], Value::Null, "{st}");
    adv(&h, 6).await;
    assert!(!list.is_done() && !show.is_done(), "not done at 7 s");
    let mut moved = 7;
    while !(list.is_done() && show.is_done()) && moved < STARTING_WAIT + 5 {
        adv(&h, 1).await;
        moved += 1;
    }
    assert!(
        list.is_done() && show.is_done(),
        "both are done after the read, at {moved} s"
    );
    let show = show.await;
    assert_eq!(show.code, 0, "{show:?}");
    // POL-2: the `show`'s request goes between the poll's two, so the first poll ends at 10 s: a
    // `list` begun at 0 s may meet API-18's wait, and then says `starting` (reported, T33's open items).
    let list = list.await;
    assert!(
        list.code == 0 || error_of(&list)["code"] == "starting",
        "{list:?}"
    );
    h.finish().await;
}

// API-18, POL-19: GitHub refusing connections from the start: `list` exits 9 within 1 s, not after 10.
// TST-8: first-run
#[tokio::test]
async fn scn_first_run_github_refusing() {
    let h = Harness::start(
        Opts::default()
            .world(|w| {
                world(w, 2);
            })
            .no_daemon(),
    )
    .await;
    h.github().refuse().await;
    h.restart().await;
    let list = h.cli_bg(&["list", "--json"]);
    adv(&h, 1).await;
    assert!(
        list.is_done(),
        "answered within 1 s, not after {STARTING_WAIT} s"
    );
    let out = list.await;
    assert_eq!(out.code, 9, "{out:?}");
    let e = error_of(&out);
    assert_eq!(e["code"], "github_unavailable", "{out:?}");
    assert!(e["until"].is_string(), "{out:?}");
    h.github().listen().await;
    h.finish().await;
}

// POL-3, OPS-5: the periodic read: a failure keeps the token, no login drops it.
// TST-8: first-run
#[tokio::test]
async fn scn_first_run_periodic_reads() {
    let h = Harness::start(Opts::default().world(|w| {
        world(w, 2);
    }))
    .await;
    adv(&h, 5).await;
    assert_eq!(quick(&h, &["list"]).await.code, 0);
    let reads = h.token().reads();

    // A failing read: the token in hand is kept.
    h.token().set(TokenRead::Failed(GhFailure::Exit(1)));
    adv(&h, TOKEN_EVERY).await;
    assert!(h.token().reads() > reads, "the token was read again");
    let st = status(&h).await;
    assert_eq!(st["daemon"]["problem"], Value::Null, "{st}");
    assert_eq!(st["last_poll"]["outcome"], "polled", "{st}");
    all_stale(&json(&quick(&h, &["list", "--json"]).await), false);
    assert_eq!(
        log_count(&h, "token read failed"),
        1,
        "{}",
        h.log().contents()
    );
    assert!(h.github().requests().iter().all(|r| r.fault.is_none()));

    // No login: the token is dropped.
    h.token().set(TokenRead::NoLogin);
    adv(&h, TOKEN_EVERY).await;
    let st = status(&h).await;
    assert_eq!(st["daemon"]["problem"], "auth", "{st}");
    let out = quick(&h, &["show", "else/lib#9", "--json"]).await;
    assert_eq!(out.code, 7, "{out:?}");
    assert_eq!(error_of(&out)["code"], "auth", "{out:?}");
    h.finish().await;
}

// ---- restart-offline ----

/// The state file's entries, parsed.
fn saved(h: &Harness) -> Vec<Value> {
    let text = h.state_file().expect("a state file");
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
    v["prs"].as_array().expect("prs").clone()
}

fn entry<'a>(entries: &'a [Value], pr: &str) -> &'a Value {
    entries
        .iter()
        .find(|e| e["pr"] == pr)
        .unwrap_or_else(|| panic!("no entry for {pr} in {entries:?}"))
}

// STA-5, STA-8, API-18: a daemon restarts with GitHub down, idle-exits writing the file; a restart
// with GitHub up finds every saved entry unchanged.
// TST-8: restart-offline
#[tokio::test]
async fn scn_restart_offline() {
    let slot = Slot::default();
    let put = slot.clone();
    let h = Harness::start(Opts::default().world(move |w| {
        let prs = world(w, 3);
        // `octo/app#2` is another person's: three entries in all.
        w.set_author(prs[1], Some("someone-else"));
        // One comment before the first read: the baseline (STA-5).
        w.comment(prs[0]);
        *put.lock().unwrap() = prs;
    }))
    .await;
    let prs = slot.lock().unwrap().clone();
    adv(&h, 5).await;
    assert_eq!(quick(&h, &["track", "else/lib#2"]).await.code, 0);
    // #3 merges and is seen at `T`.
    h.github().world(|w| w.merge(prs[2]));
    adv(&h, 2 * INTERVAL).await;
    let v = json(&quick(&h, &["list", "--json", "--all"]).await);
    assert_eq!(record(&v, "octo/app#3")["state"], "merged", "{v}");
    assert_eq!(h.stop().await, 0);
    let before = saved(&h);
    assert_eq!(before.len(), 3, "{before:?}");
    let seen_at = entry(&before, "octo/app#3")["ended"]["seen_at"].clone();
    assert!(seen_at.is_string(), "{before:?}");
    assert_eq!(entry(&before, "else/lib#2")["source"], "manual");

    // GitHub answers 5xx; the daemon restarts with `exit_after` 300.
    h.github().fault(Fault::Status(503), Times::UntilCleared);
    let config = Config {
        exit_after: Some(s(300)),
        ..Config::default()
    };
    h.restart_with(config.clone()).await;
    let out = quick(&h, &["list", "--json"]).await;
    assert_eq!(out.code, 9, "{out:?}");
    assert_eq!(error_of(&out)["code"], "github_unavailable", "{out:?}");
    let st = status(&h).await;
    let tracked = &st["tracked"];
    assert_eq!(tracked["authored"], 2, "{st}");
    assert_eq!(tracked["manual"], 1, "{st}");
    assert_eq!(tracked["asked"], 0, "{st}");

    // No further request: it exits by itself 300 s after the `list`, writing the file.
    adv(&h, 299 - 1).await;
    assert_eq!(h.exited(), None);
    let mut moved = 0;
    while h.exited().is_none() && moved < 10 {
        adv(&h, 1).await;
        moved += 1;
    }
    assert_eq!(h.exited(), Some(0), "idle exit");
    assert_eq!(
        log_count(&h, "stop cause=idle"),
        1,
        "{}",
        h.log().contents()
    );
    assert_eq!(saved(&h), before, "the file, entry for entry");

    // GitHub is back; 2 comments land on #1; restart.
    h.github().clear_faults();
    h.github().world(|w| {
        w.comment(prs[0]);
        w.comment(prs[0]);
    });
    h.restart_with(config).await;
    adv(&h, 5).await;
    let v = json(&quick(&h, &["list", "--json", "--all"]).await);
    assert_eq!(record(&v, "octo/app#1")["counts"]["comments"], 3, "{v}");
    assert_eq!(record(&v, "else/lib#2")["source"], "manual", "{v}");
    assert_eq!(record(&v, "octo/app#3")["state"], "merged", "{v}");
    let cursor0 = format!("{}-0", epoch_of(v["cursor"].as_str().expect("a cursor")));
    let out = h
        .cli(&["changes", "octo/app#1", "--since", &cursor0, "--json"])
        .await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    let events = v["events"].as_array().expect("events");
    assert_eq!(events.len(), 1, "{v}");
    let mut held = kinds(&events[0]);
    held.sort();
    assert_eq!(held, ["activity", "tracked"], "{v}");
    let activity = kind_of(&events[0], "activity");
    assert_eq!(
        activity["items"].as_array().map(Vec::len),
        Some(2),
        "exactly the 2 new comments: {v}"
    );
    assert_eq!(activity["more"], false, "{v}");

    // #3's saved time is still `T`.
    assert_eq!(h.stop().await, 0);
    let after = saved(&h);
    assert_eq!(
        entry(&after, "octo/app#3")["ended"]["seen_at"],
        seen_at,
        "{after:?}"
    );
    assert_eq!(
        entry(&after, "octo/app#3")["ended"]["state"],
        "merged",
        "{after:?}"
    );
    h.finish().await;
}

// ---- crash ----

// STA-7, OPS-4, POL-4: the daemon crashes with a found PR's first refresh in flight; the next start
// loads the file as last written, replaces the stale socket, and finds the PR again.
// TST-8: crash
#[tokio::test]
async fn scn_crash() {
    let h = Harness::start(Opts::default().world(|w| {
        world(w, 1);
    }))
    .await;
    adv(&h, 5).await;
    // A client keeps the tier active, so the polls come every 30 s.
    assert_eq!(quick(&h, &["list"]).await.code, 0);
    adv(&h, 10).await;
    let before = h.state_file().expect("#1 is saved");
    assert_eq!(saved(&h).len(), 1, "{before}");

    // `octo/app#2` is opened; the next poll finds it.
    h.github().world(|w| {
        let repo = w.repo("octo", "app");
        let pr = w.open_pr(repo);
        let now = w.now();
        w.set_created_at(pr, now);
    });
    let seen = polls(&h);
    for _ in 0..(2 * INTERVAL) {
        adv(&h, 1).await;
        if polls(&h) > seen {
            break;
        }
    }
    assert!(polls(&h) > seen, "the next poll was sent");
    // Its finding is answered; the first refresh of #2 is sent a second later, and held.
    let mut hold = h.github().hold_next();
    adv(&h, 1).await;
    hold.arrived().await;
    assert!(hold.is_holding(), "#2's first refresh is in flight");
    let v = json(&quick(&h, &["list", "--json"]).await);
    assert_eq!(names(&v, "records"), ["octo/app#1"], "{v}");
    assert_eq!(
        v["pending"],
        serde_json::json!(["octo/app#2"]),
        "#2 is found, not tracked: {v}"
    );

    h.crash().await;
    hold.release();
    assert_eq!(
        h.state_file().expect("the file"),
        before,
        "the file is byte for byte what it was"
    );
    assert!(h.paths().socket.exists(), "the stale socket is left");

    h.restart().await;
    let st = status(&h).await;
    assert_eq!(
        st["paths"]["socket"],
        h.paths().socket.display().to_string(),
        "{st}"
    );
    adv(&h, 5).await;
    let v = json(&quick(&h, &["list", "--json"]).await);
    let mut found = names(&v, "records");
    found.sort();
    assert_eq!(found, ["octo/app#1", "octo/app#2"], "{v}");
    let cursor0 = format!("{}-0", epoch_of(v["cursor"].as_str().expect("a cursor")));
    let out = h
        .cli(&["changes", "octo/app#2", "--since", &cursor0, "--json"])
        .await;
    assert_eq!(out.code, 0, "{out:?}");
    let v = json(&out);
    let events = v["events"].as_array().expect("events");
    assert!(
        events
            .iter()
            .any(|e| kinds(e).contains(&"tracked".to_string())),
        "a `tracked` event in the new epoch: {v}"
    );
    h.finish().await;
}

// ---- files-removed ----

/// A harness with a daemon that has polled once.
async fn running() -> Harness {
    let h = Harness::start(Opts::default().world(|w| {
        world(w, 2);
    }))
    .await;
    adv(&h, 5).await;
    assert_eq!(quick(&h, &["list"]).await.code, 0);
    h
}

/// The daemon has stopped for files not its own, within a minute, with the stop's cause logged.
async fn stopped_for_files(h: &Harness) {
    assert_eq!(h.exited(), None);
    adv(h, 60).await;
    assert_eq!(
        h.exited(),
        Some(0),
        "stopped within a minute: {}",
        h.log().contents()
    );
    assert_eq!(
        log_count(h, "stop cause=files"),
        1,
        "{}",
        h.log().contents()
    );
}

// STA-11: the socket removed under a running daemon: it stops, and the next command starts a new one.
// TST-8: files-removed
#[tokio::test]
async fn scn_files_removed_socket() {
    let h = running().await;
    std::fs::remove_file(&h.paths().socket).expect("the socket file is there");
    stopped_for_files(&h).await;
    // The next command starts a new daemon, through the spawner.
    let spawns = h.spawns();
    let list = h.cli_bg(&["list", "--json"]);
    let out = till_done(&h, list, 1, 30).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(h.spawns(), spawns + 1, "one spawn");
    assert_eq!(
        json(&out)["records"].as_array().map(Vec::len),
        Some(2),
        "{out:?}"
    );
    assert_eq!(h.exited(), None, "the new daemon runs");
    h.finish().await;
}

// STA-11: the lock file replaced by a new one (another inode): stopped within a minute.
// TST-8: files-removed
#[tokio::test]
async fn scn_files_removed_lock() {
    let h = running().await;
    let lock = h.paths().lock_file.clone();
    std::fs::remove_file(&lock).expect("the lock file is there");
    std::fs::write(&lock, b"").expect("a new lock file");
    stopped_for_files(&h).await;
    h.finish().await;
}

// STA-11: another socket at the daemon's path: the daemon stops and leaves it.
// TST-8: files-removed
#[tokio::test]
async fn scn_files_removed_foreign_socket() {
    use std::os::unix::fs::MetadataExt;
    let h = running().await;
    let path = h.paths().socket.clone();
    std::fs::remove_file(&path).expect("the socket file is there");
    let theirs = std::os::unix::net::UnixListener::bind(&path).expect("a socket of the test's own");
    let inode = std::fs::metadata(&path).expect("it exists").ino();
    stopped_for_files(&h).await;
    let after = std::fs::metadata(&path).expect("the test's socket is still there");
    assert_eq!(after.ino(), inode, "the same inode");
    drop(theirs);
    h.finish().await;
}

// ---- clock-back ----

/// Whether `pr` has a record in `list --all`.
async fn has_record(h: &Harness, pr: &str) -> bool {
    let v = json(&quick(h, &["list", "--json", "--all"]).await);
    v["records"]
        .as_array()
        .is_some_and(|rs| rs.iter().any(|r| r["pr"] == pr))
}

// POL-26, STA-2, POL-22: the wall clock steps back an hour: no sleep is noticed, nothing goes stale,
// the quota count isn't reset, a wait's deadline is unchanged, and retention goes by the wall clock.
// TST-8: clock-back
#[tokio::test]
async fn scn_clock_back() {
    const KEEP_DONE: u64 = 7200;
    let slot = Slot::default();
    let put = slot.clone();
    let h = Harness::start(Opts::default().keep_done(s(KEEP_DONE)).world(move |w| {
        *put.lock().unwrap() = world(w, 2);
    }))
    .await;
    let prs = slot.lock().unwrap().clone();
    adv(&h, 5).await;
    assert_eq!(quick(&h, &["list"]).await.code, 0);

    // #1 is merged and seen at `s`.
    h.github().world(|w| w.merge(prs[0]));
    adv(&h, 2 * INTERVAL).await;
    assert!(has_record(&h, "octo/app#1").await);
    adv(&h, 10).await;
    let entries = saved(&h);
    let seen = entry(&entries, "octo/app#1")["ended"]["seen_at"]
        .as_str()
        .expect("a saved time")
        .to_string();
    let seen = circitor::clock::Time::parse(&seen)
        .expect("RFC 3339")
        .unix_millis();

    // A wait on #2, begun at `w`.
    let began = mono_secs(&h);
    let wait = h.cli_bg(&[
        "wait",
        "octo/app#2",
        "--until",
        "merged",
        "--timeout",
        "100",
        "--json",
    ]);
    let n = status(&h).await["cost_last_hour"].as_u64().expect("a cost");
    assert!(n > 0);
    adv_to(&h, began + 10).await;
    h.clock().set_wall_back(s(3600)).await;

    // No sleep is noticed (a clock set back isn't one), and nothing is newly stale.
    adv(&h, 20).await;
    assert_eq!(log_count(&h, "sleep seconds="), 0, "{}", h.log().contents());
    all_stale(&json(&quick(&h, &["list", "--json"]).await), false);
    assert!(status(&h).await["cost_last_hour"].as_u64().expect("a cost") >= n);

    // A wait's deadline is unchanged: not done at `w` + 99 s, exits 3 at `w` + 100 s.
    adv_to(&h, began + 99).await;
    assert!(!wait.is_done(), "not done at 99 s");
    adv_to(&h, began + 100).await;
    assert!(wait.is_done(), "done at 100 s");
    let out = wait.await;
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(json(&out)["outcome"], "timeout", "{out:?}");

    // 30 minutes on, the quota count hasn't fallen below what it was.
    adv(&h, 1800).await;
    assert!(status(&h).await["cost_last_hour"].as_u64().expect("a cost") >= n);
    assert_eq!(log_count(&h, "sleep seconds="), 0);

    // Retention goes by the wall clock: tracked while it reads under `s` + `keep_done`.
    let wall_ms = |h: &Harness| {
        use circitor::clock::Clock;
        h.clock().wall().unix_millis()
    };
    let due = seen + i64::try_from(KEEP_DONE).expect("fits") * 1000;
    while wall_ms(&h) < due - 30_000 {
        let left = u64::try_from((due - 30_000 - wall_ms(&h)) / 1000).expect("positive");
        adv(&h, left.clamp(1, 500)).await;
        assert!(
            has_record(&h, "octo/app#1").await,
            "still tracked at wall {} (due {due})",
            wall_ms(&h)
        );
    }
    let mut crossed = None;
    for _ in 0..60 {
        let reads_under = wall_ms(&h) < due;
        let present = has_record(&h, "octo/app#1").await;
        if reads_under {
            assert!(
                present,
                "tracked while the wall clock reads under `s` + 7200 s"
            );
        }
        if !present {
            crossed = Some(wall_ms(&h));
            break;
        }
        adv(&h, 1).await;
    }
    let dropped = crossed.expect("dropped within a minute of the wall clock's reaching it");
    assert!(dropped >= due, "dropped at {dropped}, due at {due}");
    assert!(
        dropped - due <= 10_000,
        "at the first tick after: {} ms late",
        dropped - due
    );
    h.finish().await;
}
