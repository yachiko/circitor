#![cfg(feature = "test-endpoint")]
//! T34: TST-6's figures on the whole daemon, counted from what the fake GitHub received (U1).
//! Each test runs the harness on the test clock, counts from `FakeGitHub::requests()`, and ends
//! with `h.finish()` (TST-11). Points are summed from the fake's log; a poll is a request that held
//! a finding's first page, and a PR's refreshes are the requests naming its node ID.
//!
//! **The basic world** (TST-6): 10 tracked PRs, settled (`updatedAt` two minutes old), passed
//! checks, `mergeable` known, nothing changing: no follow-up (POL-11), no item request (POL-12).
//! The first poll is two requests; every later one is one request of 1 point (plan decision P10).
//! **The measured hour** begins at the send time of the second poll.
//!
//! Every figure is a constant named after TST-6's bullet. All of them are proposals (OPS-2's
//! defaults, POL-17's 80 % and 100 %, POL-23's table, STA-3's 200).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::cli::Output;
use circitor::clock::{Clock, Mono, Time};
use circitor::testkit::github::{AskedOf, PrId, World};
use circitor::testkit::harness::{Bg, Harness, Opts, START};
use circitor::testkit::server::{Fault, LimitStatus, Request, Times};
use serde_json::Value;

// TST-6, bullet 1: the basic world's hours.
const ACTIVE_HOUR_MAX_POINTS: u32 = 130;
const ACTIVE_HOUR_MIN_POLLS: usize = 110;
const IDLE_HOUR_MAX_POINTS: u32 = 15;
const IDLE_HOUR_MIN_POLLS: usize = 11;
// Bullet 2: one hot PR.
const HOT_ACTIVE_EXTRA_MAX: u32 = 250;
const HOT_IDLE_RATE_PER_HOUR: u32 = 350;
const IDLE_PART_OF_A_WAIT_SECS: u64 = 3000;
// Bullet 3: a waited-on PR.
const WAITED_MIN_REFRESHES: usize = 230;
const HOT_GAP_MIN: Duration = Duration::from_secs(10);
const HOT_GAP_MAX: Duration = Duration::from_secs(12);
const SHOW_HOT_FOR: u64 = 120;
const ENDED_REFRESHES: std::ops::RangeInclusive<usize> = 9..=11;
// Bullet 4: slowed and capped, with `quota.max_per_hour` 60 so the states are reached in minutes.
const SMALL_BUDGET: u32 = 60;
const SLOWED_ABOVE: u32 = 48;
// Bullet 5: 200 tracked PRs.
const TWO_HUNDRED: usize = 200;
const TWO_HUNDRED_MAX_POINTS: u32 = 600;
// Config values the tests name (OPS-2).
const INTERVAL: u64 = 30;
const IDLE_INTERVAL: u64 = 300;
const IDLE_AFTER: u64 = 600;
const RESERVE: u32 = 500;

const PR1: &str = "octo/app#1";
const PR_GONE: &str = "octo/app#99";

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---- the world ----

/// `n` settled PRs of the viewer's in `octo/app`: `updatedAt` two minutes old and a little apart,
/// one passed check each, `MERGEABLE` (the world's default). `octo/app#1..n`.
fn quiet(w: &mut World, n: usize) -> Vec<PrId> {
    let now = w.now();
    let repo = w.repo("octo", "app");
    (0..n)
        .map(|i| {
            let pr = w.open_pr(repo);
            w.set_created_at(pr, now.minus(s(3600)));
            w.set_updated_at(pr, now.minus(s(120 + i as u64)));
            w.runs(pr, 1, "COMPLETED", Some("SUCCESS"));
            pr
        })
        .collect()
}

/// The basic world: 10 quiet PRs.
fn basic(w: &mut World) {
    quiet(w, 10);
}

fn node_id(h: &Harness, pr: PrId) -> String {
    h.github().world(|w| w.node_id(pr).to_string())
}

/// The harness on a world scripted by `f` (before the daemon's first poll); what `f` returns comes back.
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

// ---- the log ----

fn log(h: &Harness) -> Vec<Request> {
    h.github().requests()
}

fn is_poll(r: &Request) -> bool {
    matches!(r.finding, Some(None))
}

fn polls(rs: &[Request]) -> Vec<&Request> {
    rs.iter().filter(|r| is_poll(r)).collect()
}

fn naming<'a>(rs: &'a [Request], id: &str) -> Vec<&'a Request> {
    rs.iter()
        .filter(|r| r.nodes.iter().any(|(n, _)| n == id))
        .collect()
}

fn points(rs: &[Request]) -> u32 {
    rs.iter().map(|r| r.cost).sum()
}

fn at(r: &Request) -> Duration {
    r.mono.as_duration()
}

fn between(rs: &[Request], from: Mono, to: Mono) -> Vec<Request> {
    rs.iter()
        .filter(|r| r.mono >= from && r.mono < to)
        .cloned()
        .collect()
}

fn after(rs: &[Request], from: Mono) -> Vec<Request> {
    rs.iter().filter(|r| r.mono > from).cloned().collect()
}

fn gaps(rs: &[&Request]) -> Vec<Duration> {
    rs.windows(2).map(|p| at(p[1]) - at(p[0])).collect()
}

/// On a failure: the request log's send times and costs for the measured span.
fn dump(rs: &[Request]) -> String {
    let mut out = String::new();
    for r in rs {
        out.push_str(&format!(
            "\n  #{} t={:.1}s cost={} {} nodes={}{}",
            r.index,
            at(r).as_secs_f64(),
            r.cost,
            match &r.finding {
                Some(None) => "finding".to_string(),
                Some(Some(c)) => format!("finding after {c}"),
                None => "-".to_string(),
            },
            r.nodes.len(),
            r.fault
                .as_ref()
                .map(|f| format!(" fault={f:?}"))
                .unwrap_or_default(),
        ));
    }
    out
}

fn now(h: &Harness) -> Mono {
    h.clock().mono()
}

/// `d` before `m` (no earlier than zero).
fn before(m: Mono, d: Duration) -> Mono {
    Mono::ZERO.plus(m.as_duration().saturating_sub(d))
}

// ---- the clock ----

async fn adv(h: &Harness, n: u64) {
    h.clock().advance(s(n)).await;
}

/// Moves the clock to `t` (a no-op when it's there or past).
async fn adv_to(h: &Harness, t: Mono) {
    let here = now(h);
    if t > here {
        h.clock().advance(t.since(here)).await;
    }
}

/// Moves the clock in steps of `step` until `cond` holds; panics after `limit` seconds.
async fn until(h: &Harness, step: u64, limit: u64, what: &str, cond: impl Fn(&Harness) -> bool) {
    let mut moved = 0;
    while !cond(h) {
        assert!(moved < limit, "{what}: not so after {limit} s");
        adv(h, step).await;
        moved += step;
    }
}

// ---- the commands ----

fn json(out: &Output) -> Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
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

/// `status --json`: not a client request (API-5).
async fn status(h: &Harness) -> Value {
    let out = quick(h, &["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    json(&out)
}

fn problem(v: &Value) -> Option<String> {
    v["daemon"]["problem"].as_str().map(str::to_owned)
}

fn tier(v: &Value) -> String {
    v["daemon"]["tier"].as_str().unwrap_or_default().to_owned()
}

fn warnings(v: &Value) -> Vec<String> {
    v["warnings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|w| w["code"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// A client request: from memory, so it's done at once (API-6).
async fn list(h: &Harness) {
    let out = quick(h, &["list", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
}

/// The first `list`, a client request that waits for the first poll (API-18): the clock moves in
/// 1 s steps until it's done. After it the tier is active.
async fn boot(h: &Harness) {
    let first = h.cli_bg(&["list", "--json"]);
    for _ in 0..60 {
        if first.is_done() {
            break;
        }
        adv(h, 1).await;
    }
    assert!(first.is_done(), "the first list wasn't answered in 60 s");
    let out = first.await;
    assert_eq!(out.code, 0, "{out:?}");
}

/// The first poll of a daemon left idle: two requests, the finding and the refresh. No client
/// request is made, so the tier stays idle (POL-13).
async fn first_poll_idle(h: &Harness) {
    until(h, 1, 30, "the first poll's two requests", |h| {
        log(h).len() >= 2
    })
    .await;
    h.clock().settle().await;
}

/// 1 s steps until `n` polls are in the log.
async fn until_polls(h: &Harness, n: usize) {
    until(h, 1, 3000, "the poll", |h| polls_in_log(h) >= n).await;
}

fn polls_in_log(h: &Harness) -> usize {
    log(h).iter().filter(|r| is_poll(r)).count()
}

/// A client request every 300 s (well inside `idle_after`) until `end`: the tier stays active.
async fn keep_active_until(h: &Harness, end: Mono) {
    while now(h) < end {
        let next = std::cmp::min(now(h).plus(s(IDLE_INTERVAL)), end);
        adv_to(h, next).await;
        if now(h) < end {
            list(h).await;
        }
    }
}

/// The measured span: from the second poll's send time for an hour.
struct Hour {
    start: Mono,
    end: Mono,
    reqs: Vec<Request>,
}

impl Hour {
    fn points(&self) -> u32 {
        points(&self.reqs)
    }

    fn polls(&self) -> Vec<&Request> {
        polls(&self.reqs)
    }

    fn dump(&self) -> String {
        format!(
            "the measured hour {:?} to {:?}:{}",
            self.start.as_duration(),
            self.end.as_duration(),
            dump(&self.reqs)
        )
    }
}

/// The daemon's own count, `status`'s `cost_last_hour`, against the fake's sum over the same
/// sliding hour. Looked at 10 s after `end`, away from any poll.
async fn assert_cost_matches(h: &Harness) {
    adv(h, 10).await;
    let t = now(h);
    let from = t.as_duration().saturating_sub(s(3600));
    let sum: u32 = log(h)
        .iter()
        .filter(|r| at(r) > from && r.mono <= t)
        .map(|r| r.cost)
        .sum();
    let v = status(h).await;
    assert_eq!(
        v["cost_last_hour"].as_u64(),
        Some(u64::from(sum)),
        "the daemon's cost_last_hour against the fake's sum over the last hour:{}",
        dump(&log(h))
    );
}

// ---- bullet 1: an hour in the active tier, and an hour idle ----

// TST-6: an hour in the active tier costs at most 130 points, and at least 110 polls are sent.
// POL-13: a poll every `interval` while a client asks, a `list` every 5 minutes.
// U1: the cost of watching, in points.
#[tokio::test]
async fn tst6_active_hour_costs_at_most_130() {
    let h = Harness::start(Opts::default().world(basic)).await;
    boot(&h).await;
    assert_eq!(tier(&status(&h).await), "active");
    until_polls(&h, 2).await;
    let t2 = log(&h)[polls_index(&h, 1)].mono;
    let end = t2.plus(s(3600));
    keep_active_until(&h, end).await;
    adv_to(&h, end).await;
    let hour = Hour {
        start: t2,
        end,
        reqs: between(&log(&h), t2, end),
    };
    // The world is quiet: every request of the hour is a poll of one point (P10).
    assert!(
        hour.reqs.iter().all(|r| is_poll(r) && r.cost == 1),
        "a poll of the quiet world took more than one request of 1 point (P10):{}",
        hour.dump()
    );
    eprintln!(
        "active hour: {} points, {} polls",
        hour.points(),
        hour.polls().len()
    );
    assert!(
        hour.points() <= ACTIVE_HOUR_MAX_POINTS,
        "{} points in an active hour, at most {ACTIVE_HOUR_MAX_POINTS}:{}",
        hour.points(),
        hour.dump()
    );
    assert!(
        hour.polls().len() >= ACTIVE_HOUR_MIN_POLLS,
        "{} polls in an active hour, at least {ACTIVE_HOUR_MIN_POLLS}:{}",
        hour.polls().len(),
        hour.dump()
    );
    assert_eq!(tier(&status(&h).await), "active");
    assert_cost_matches(&h).await;
    h.finish().await;
}

/// The index in the log of the `n`-th poll (from 0).
fn polls_index(h: &Harness, n: usize) -> usize {
    log(h)
        .iter()
        .enumerate()
        .filter(|(_, r)| is_poll(r))
        .nth(n)
        .map(|(i, _)| i)
        .expect("the poll is in the log")
}

// TST-6: an hour idle costs at most 15 points, and at least 11 polls are sent.
// POL-13: with no client request at all, the tier at start is idle and polls are `idle_interval` apart.
#[tokio::test]
async fn tst6_idle_hour_costs_at_most_15() {
    let h = Harness::start(Opts::default().world(basic)).await;
    assert_eq!(tier(&status(&h).await), "idle", "the tier at start");
    until_polls(&h, 2).await;
    let t2 = log(&h)[polls_index(&h, 1)].mono;
    let end = t2.plus(s(3600));
    adv_to(&h, end).await;
    let hour = Hour {
        start: t2,
        end,
        reqs: between(&log(&h), t2, end),
    };
    assert!(
        hour.reqs.iter().all(|r| is_poll(r) && r.cost == 1),
        "an idle poll of the quiet world took more than one request of 1 point (P10):{}",
        hour.dump()
    );
    eprintln!(
        "idle hour: {} points, {} polls",
        hour.points(),
        hour.polls().len()
    );
    assert!(
        hour.points() <= IDLE_HOUR_MAX_POINTS,
        "{} points in an idle hour, at most {IDLE_HOUR_MAX_POINTS}:{}",
        hour.points(),
        hour.dump()
    );
    assert!(
        hour.polls().len() >= IDLE_HOUR_MIN_POLLS,
        "{} polls in an idle hour, at least {IDLE_HOUR_MIN_POLLS}:{}",
        hour.polls().len(),
        hour.dump()
    );
    assert_eq!(tier(&status(&h).await), "idle");
    assert_cost_matches(&h).await;
    h.finish().await;
}

// ---- bullet 2 and 3: one hot PR, a waited-on PR ----

/// `wait` on a PR until `merged`, which never happens, for `timeout` seconds.
fn wait_on(h: &Harness, pr: &str, timeout: u64) -> Bg {
    h.cli_bg(&[
        "wait",
        pr,
        "--until",
        "merged",
        "--timeout",
        &timeout.to_string(),
    ])
}

/// Moves the clock in 1 s steps until `bg` is done (at most 30 s), and gives its output.
async fn finish_bg(h: &Harness, bg: Bg) -> Output {
    for _ in 0..30 {
        if bg.is_done() {
            break;
        }
        adv(h, 1).await;
    }
    assert!(
        bg.is_done(),
        "the command wasn't done 30 s after its deadline"
    );
    bg.await
}

/// The active hour with or without one `wait` on `octo/app#1` that spans it: the measured hour, and
/// the PR's node ID.
async fn active_hour(wait: bool) -> (Hour, String) {
    let (h, prs) = start(Opts::default(), |w| quiet(w, 10)).await;
    let id = node_id(&h, prs[0]);
    boot(&h).await;
    until_polls(&h, 2).await;
    let t2 = log(&h)[polls_index(&h, 1)].mono;
    let end = t2.plus(s(3600));
    // The wait's arrival is at the second poll's send time: its deadline is the hour's end.
    let waiter = wait.then(|| wait_on(&h, PR1, 3600));
    keep_active_until(&h, end).await;
    adv_to(&h, end).await;
    let hour = Hour {
        start: t2,
        end,
        reqs: between(&log(&h), t2, end),
    };
    if let Some(w) = waiter {
        let out = finish_bg(&h, w).await;
        assert_eq!(out.code, 3, "the wait ends by its timeout: {out:?}");
    }
    h.finish().await;
    (hour, id)
}

// TST-6: one hot PR adds at most 250 points an hour in the active tier.
// POL-14: a hot PR is refreshed every `hot_interval`, its polls counted.
#[tokio::test]
async fn tst6_one_hot_pr_adds_at_most_250_in_the_active_hour() {
    let (hot, _) = active_hour(true).await;
    let (base, _) = active_hour(false).await;
    let extra = hot.points().saturating_sub(base.points());
    eprintln!(
        "hot hour {} points, base {}, extra {extra}",
        hot.points(),
        base.points()
    );
    assert!(
        extra <= HOT_ACTIVE_EXTRA_MAX,
        "a waited-on PR added {extra} points ({} against {}), at most {HOT_ACTIVE_EXTRA_MAX}:\n{}",
        hot.points(),
        base.points(),
        hot.dump()
    );
}

// TST-6: one waited-on PR is refreshed at least 230 times in the active hour, each about 10 s
// (10 s to 12 s) after its last refresh of any kind.
// POL-14: a poll in flight can hold a hot refresh for POL-2's second.
#[tokio::test]
async fn tst6_waited_on_pr_is_refreshed_every_10_to_12_s() {
    let (hot, id) = active_hour(true).await;
    let refreshes = naming(&hot.reqs, &id);
    eprintln!("waited-on PR: {} refreshes", refreshes.len());
    assert!(
        refreshes.len() >= WAITED_MIN_REFRESHES,
        "{} refreshes of the waited-on PR in the hour, at least {WAITED_MIN_REFRESHES}:\n{}",
        refreshes.len(),
        hot.dump()
    );
    for (i, g) in gaps(&refreshes).into_iter().enumerate() {
        assert!(
            (HOT_GAP_MIN..=HOT_GAP_MAX).contains(&g),
            "the gap before refresh #{} is {g:?}, not 10 s to 12 s:\n{}",
            i + 1,
            hot.dump()
        );
    }
}

// TST-6: in the idle tier one hot PR adds at most an hour's rate of 350, measured over the idle
// part of one 3 600 s wait: a wait's arrival keeps the tier active for `idle_after`, so the
// measured span is the 3 000 s after that (Open question: a whole idle hour can't be staged).
// POL-14, POL-13.
#[tokio::test]
async fn tst6_one_hot_pr_adds_at_most_291_over_3000_idle_seconds() {
    let extra_max = HOT_IDLE_RATE_PER_HOUR * IDLE_PART_OF_A_WAIT_SECS as u32 / 3600;
    assert_eq!(extra_max, 291);
    // Idle alone: the same 3 000 s with no client at all.
    let base = {
        let h = Harness::start(Opts::default().world(basic)).await;
        first_poll_idle(&h).await;
        let from = now(&h).plus(s(IDLE_AFTER));
        let to = from.plus(s(IDLE_PART_OF_A_WAIT_SECS));
        adv_to(&h, to).await;
        let rs = between(&log(&h), from, to);
        h.finish().await;
        points(&rs)
    };
    let h = Harness::start(Opts::default().world(basic)).await;
    first_poll_idle(&h).await;
    let arrival = now(&h);
    let waiter = wait_on(&h, PR1, 3600);
    // The tier is idle once `idle_after` has passed since the wait's arrival.
    let from = arrival.plus(s(IDLE_AFTER));
    let to = from.plus(s(IDLE_PART_OF_A_WAIT_SECS));
    adv_to(&h, from).await;
    assert_eq!(
        tier(&status(&h).await),
        "idle",
        "the tier after idle_after with a waiter held"
    );
    adv_to(&h, to).await;
    let rs = between(&log(&h), from, to);
    let out = finish_bg(&h, waiter).await;
    assert_eq!(out.code, 3, "{out:?}");
    h.finish().await;
    let extra = points(&rs).saturating_sub(base);
    eprintln!("idle hot: {} points over 3000 s, base {base}", points(&rs));
    assert!(
        extra <= extra_max,
        "a waited-on PR added {extra} points over {IDLE_PART_OF_A_WAIT_SECS} s idle ({} against {base}), at most {extra_max}:{}",
        points(&rs),
        dump(&rs)
    );
}

// ---- bullet 3: `show` and hot ----

/// Quiet PRs, the first reading `checks_pending` (a check still running).
fn pending_first(w: &mut World) -> Vec<PrId> {
    let prs = quiet(w, 10);
    w.runs(prs[0], 1, "IN_PROGRESS", None);
    prs
}

fn record_status(v: &Value) -> String {
    v["record"]["status"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

// TST-6: a `show` of a `checks_pending` PR makes it hot for 2 minutes, and then it's back to the
// tier's cadence. POL-14: hot for 2 minutes after a request named it while `checks_pending`.
#[tokio::test]
async fn tst6_show_of_a_checks_pending_pr_is_hot_for_120_s() {
    let (h, prs) = start(Opts::default(), pending_first).await;
    let id = node_id(&h, prs[0]);
    boot(&h).await;
    adv(&h, 3).await;
    let ts = now(&h);
    let out = quick(&h, &["show", PR1, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(record_status(&json(&out)), "checks_pending");
    let hot_end = ts.plus(s(SHOW_HOT_FOR));
    adv_to(&h, ts.plus(s(420))).await;
    let rs = log(&h);
    let hot = between(&rs, ts, hot_end);
    let hot_refreshes = naming(&hot, &id);
    assert!(
        hot_refreshes.len() >= 8,
        "{} refreshes in the 120 s after the show, about 10 s apart:{}",
        hot_refreshes.len(),
        dump(&hot)
    );
    for g in gaps(&hot_refreshes) {
        assert!(
            (HOT_GAP_MIN..=HOT_GAP_MAX).contains(&g),
            "a gap of {g:?} in the hot 120 s, not 10 s to 12 s:{}",
            dump(&hot)
        );
    }
    // Back to the tier's: only polls refresh it, `interval` apart.
    let late = between(&rs, hot_end.plus(HOT_GAP_MAX), ts.plus(s(420)));
    let late_refreshes = naming(&late, &id);
    assert!(
        !late_refreshes.is_empty(),
        "no refresh at all after the 120 s"
    );
    assert!(
        late_refreshes.iter().all(|r| is_poll(r)),
        "a hot refresh after the 120 s were over:{}",
        dump(&late)
    );
    for g in gaps(&late_refreshes) {
        assert!(
            g >= s(INTERVAL - 1),
            "a gap of {g:?} after the 120 s, the tier's is {INTERVAL} s:{}",
            dump(&late)
        );
    }
    h.finish().await;
}

// TST-6: a `show` of a `ready` PR doesn't make it hot: it's refreshed by polls only.
// POL-14.
#[tokio::test]
async fn tst6_show_of_a_ready_pr_is_not_hot() {
    let (h, prs) = start(Opts::default(), |w| quiet(w, 10)).await;
    let id = node_id(&h, prs[0]);
    boot(&h).await;
    adv(&h, 3).await;
    let ts = now(&h);
    let out = quick(&h, &["show", PR1, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(record_status(&json(&out)), "ready");
    adv_to(&h, ts.plus(s(300))).await;
    let rs = after(&log(&h), ts);
    let refreshes = naming(&rs, &id);
    assert!(!refreshes.is_empty(), "no refresh at all in 300 s");
    assert!(
        refreshes.iter().all(|r| is_poll(r)),
        "a refresh outside a poll after a `show` of a ready PR:{}",
        dump(&rs)
    );
    h.finish().await;
}

// ---- bullet 3: a merged PR and an `asked` one ----

// TST-6: a merged or `asked` PR is asked for once per `idle_interval` in the active tier: 9 to
// 11 times over 50 minutes. POL-10, POL-14.
#[tokio::test]
async fn tst6_merged_and_asked_prs_are_refreshed_once_per_idle_interval() {
    let (h, (merged, asked)) = start(Opts::default(), |w| {
        let prs = quiet(w, 10);
        let merging = prs[1];
        w.after(s(5), move |w| w.merge(merging));
        let other = w.repo("octo", "other");
        let now = w.now();
        let theirs = w.open_pr(other);
        w.set_author(theirs, Some("someone-else"));
        w.set_created_at(theirs, now.minus(s(3600)));
        w.set_updated_at(theirs, now.minus(s(300)));
        w.runs(theirs, 1, "COMPLETED", Some("SUCCESS"));
        (prs[1], theirs)
    })
    .await;
    let merged_id = node_id(&h, merged);
    let asked_id = node_id(&h, asked);
    boot(&h).await;
    // The merge is seen at the poll after it; then the record reads `merged`.
    let mut tries = 0;
    loop {
        let out = quick(&h, &["list", "--all", "--json"]).await;
        let v = json(&out);
        let state = v["records"]
            .as_array()
            .and_then(|rs| rs.iter().find(|r| r["pr"] == "octo/app#2"))
            .and_then(|r| r["state"].as_str().map(str::to_owned));
        if state.as_deref() == Some("merged") {
            break;
        }
        tries += 1;
        assert!(tries < 40, "the merged PR never read `merged`");
        adv(&h, 10).await;
    }
    let out = quick(&h, &["show", "octo/other#1", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(json(&out)["record"]["source"], "asked");
    let from = now(&h);
    let to = from.plus(s(3000));
    keep_active_until(&h, to).await;
    adv_to(&h, to).await;
    let rs = between(&log(&h), from, to.plus(s(1)));
    for (what, id) in [("merged", &merged_id), ("asked", &asked_id)] {
        let n = naming(&rs, id).len();
        assert!(
            ENDED_REFRESHES.contains(&n),
            "the {what} PR was refreshed {n} times in 3 000 s, 9 to 11 expected:{}",
            dump(&rs)
        );
    }
    h.finish().await;
}

// ---- bullet 4: slowed and capped, with `quota.max_per_hour` 60 ----

/// Moves the clock in 5 s steps, a `list` every 300 s so the tier stays active, until the fake has
/// answered `target` points.
async fn spend_until(h: &Harness, target: u32) {
    let mut last_list = now(h);
    for _ in 0..4000 {
        if h.github().points() >= target {
            return;
        }
        adv(h, 5).await;
        if now(h).since(last_list) >= s(IDLE_INTERVAL) {
            list(h).await;
            last_list = now(h);
        }
    }
    panic!(
        "{target} points weren't spent: {} so far:{}",
        h.github().points(),
        dump(&log(h))
    );
}

/// The request that took the running sum above `limit` points.
fn crossing(rs: &[Request], limit: u32) -> &Request {
    let mut sum = 0;
    rs.iter()
        .find(|r| {
            sum += r.cost;
            sum > limit
        })
        .expect("the sum went above the limit")
}

// TST-6: above 80 % of `quota.max_per_hour`, `interval` doubles: polls are 60 s apart.
// POL-17, POL-23: slowed; `daemon.problem` reads `null` (API-4).
#[tokio::test]
async fn tst6_slowed_polls_are_60_s_apart() {
    let h = Harness::start(Opts::default().quota(SMALL_BUDGET, RESERVE).world(basic)).await;
    boot(&h).await;
    spend_until(&h, SLOWED_ABOVE + 1).await;
    let crossed = crossing(&log(&h), SLOWED_ABOVE).mono;
    spend_until(&h, SLOWED_ABOVE + 8).await;
    let rs = log(&h);
    let tail = after(&rs, crossed);
    let slowed_refs = polls(&tail);
    assert!(
        slowed_refs.len() >= 5,
        "only {} slowed polls:{}",
        slowed_refs.len(),
        dump(&rs)
    );
    for g in gaps(&slowed_refs) {
        assert!(
            (s(2 * INTERVAL)..=s(2 * INTERVAL + 1)).contains(&g),
            "a gap of {g:?} between slowed polls, not 60 s:{}",
            dump(&rs)
        );
    }
    let v = status(&h).await;
    assert_eq!(problem(&v), None, "slowed reports no problem: {v}");
    assert!(
        !warnings(&v).contains(&"quota_capped".to_string()),
        "capped too early: {v}"
    );
    h.finish().await;
}

// TST-6: above 80 %, `hot_interval` doubles too: a waited-on PR's hot refreshes are 20 s apart.
// POL-17, POL-23, POL-14.
#[tokio::test]
async fn tst6_slowed_hot_refreshes_are_20_s_apart() {
    let (h, prs) = start(Opts::default().quota(SMALL_BUDGET, RESERVE), |w| {
        quiet(w, 10)
    })
    .await;
    let id = node_id(&h, prs[0]);
    boot(&h).await;
    let waiter = wait_on(&h, PR1, 3600);
    spend_until(&h, SLOWED_ABOVE + 1).await;
    let crossed = crossing(&log(&h), SLOWED_ABOVE).mono;
    spend_until(&h, SLOWED_ABOVE + 8).await;
    let rs = log(&h);
    let slowed = after(&rs, crossed);
    let refreshes = naming(&slowed, &id);
    assert!(
        refreshes.len() >= 5,
        "only {} slowed refreshes of the waited-on PR:{}",
        refreshes.len(),
        dump(&rs)
    );
    for g in gaps(&refreshes) {
        assert!(
            (s(2 * 10)..=s(2 * 10 + 2)).contains(&g),
            "a gap of {g:?} between slowed hot refreshes, not 20 s:{}",
            dump(&rs)
        );
    }
    assert!(
        !warnings(&status(&h).await).contains(&"quota_capped".to_string()),
        "capped too early"
    );
    waiter.cancel().await;
    h.finish().await;
}

// TST-6: at 100 %, only forced refreshes, reading through, `track` and their POL-12 requests are
// sent, in 30 s windows. POL-23, POL-15.
#[tokio::test]
async fn tst6_capped_sends_only_demand_in_30_s_windows() {
    let (h, prs) = start(Opts::default().quota(SMALL_BUDGET, RESERVE), |w| {
        let prs = quiet(w, 10);
        let repo = w.repo("octo", "other");
        let now = w.now();
        let others: Vec<PrId> = (0..2)
            .map(|_| {
                let pr = w.open_pr(repo);
                w.set_author(pr, Some("someone-else"));
                w.set_created_at(pr, now.minus(s(3600)));
                w.set_updated_at(pr, now.minus(s(300)));
                w.runs(pr, 1, "COMPLETED", Some("SUCCESS"));
                pr
            })
            .collect();
        let _ = others;
        prs
    })
    .await;
    let id = node_id(&h, prs[0]);
    boot(&h).await;
    let waiter = wait_on(&h, PR1, 3600);
    spend_until(&h, SMALL_BUDGET).await;
    // The warnings are raised at the next tick (POL-26): at most 10 s.
    adv(&h, 10).await;
    let capped_at = now(&h);
    let v = status(&h).await;
    assert!(
        warnings(&v).contains(&"quota_capped".to_string()),
        "not capped at {SMALL_BUDGET} points: {v}"
    );
    assert_eq!(problem(&v).as_deref(), Some("quota_low"));

    // No poll and no hot refresh: five minutes with clients asking and a PR waited on.
    let sent = log(&h).len();
    for _ in 0..3 {
        adv(&h, 100).await;
        list(&h).await;
    }
    assert_eq!(
        log(&h).len(),
        sent,
        "requests while capped, with no demand:{}",
        dump(&after(&log(&h), capped_at))
    );

    // POL-15: two `show --fresh` 10 s apart send one refresh; one 35 s after sends another.
    let before = naming(&log(&h), &id).len();
    let out = quick(&h, &["show", PR1, "--fresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    adv(&h, 10).await;
    let out = quick(&h, &["show", PR1, "--fresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        naming(&log(&h), &id).len(),
        before + 1,
        "two `show --fresh` 10 s apart while capped:{}",
        dump(&after(&log(&h), capped_at))
    );
    adv(&h, 35).await;
    let out = quick(&h, &["show", PR1, "--fresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        naming(&log(&h), &id).len(),
        before + 2,
        "a `show --fresh` 45 s after the last refresh while capped:{}",
        dump(&after(&log(&h), capped_at))
    );

    // POL-15: two `refresh` 15 s apart send one poll; a third 35 s later another.
    adv(&h, 2).await;
    let polls_before = polls_in_log(&h);
    let out = quick(&h, &["refresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    adv(&h, 15).await;
    let out = quick(&h, &["refresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        polls_in_log(&h),
        polls_before + 1,
        "two `refresh` 15 s apart while capped:{}",
        dump(&after(&log(&h), capped_at))
    );
    adv(&h, 35).await;
    let out = quick(&h, &["refresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        polls_in_log(&h),
        polls_before + 2,
        "a `refresh` 50 s after the last one while capped:{}",
        dump(&after(&log(&h), capped_at))
    );

    // Reading through and `track` are sent (POL-16, API-15).
    adv(&h, 5).await;
    let lookups = |h: &Harness| log(h).iter().filter(|r| r.lookup.is_some()).count();
    let n = lookups(&h);
    let out = quick(&h, &["show", "octo/other#1", "--json"]).await;
    assert_eq!(out.code, 0, "show of an untracked PR while capped: {out:?}");
    assert_eq!(lookups(&h), n + 1, "its lookup was sent");
    adv(&h, 5).await;
    let out = quick(&h, &["track", "octo/other#2", "--json"]).await;
    assert_eq!(out.code, 0, "track while capped: {out:?}");
    assert_eq!(lookups(&h), n + 2, "track's lookup was sent");

    // POL-12: a refresh that sees a new comment sends the request for the items too.
    adv(&h, 35).await;
    h.github().world(|w| {
        w.comment(prs[0]);
    });
    let seen = log(&h).len();
    // The refresh, then POL-2's second, then the request for the items.
    let bg = h.cli_bg(&["show", PR1, "--fresh", "--json"]);
    h.clock().settle().await;
    let out = finish_bg(&h, bg).await;
    assert_eq!(out.code, 0, "{out:?}");
    let new = log(&h).split_off(seen);
    assert!(
        new.iter().any(|r| r
            .nodes
            .iter()
            .any(|(n, of)| n == &id && matches!(of, AskedOf::Items | AskedOf::RecordAndItems))),
        "the POL-12 request after a new comment, while capped:{}",
        dump(&new)
    );
    waiter.cancel().await;
    h.finish().await;
}

// ---- bullet 5: 200 tracked PRs ----

// TST-6: with 200 tracked PRs the active tier runs `slowed` or `capped` as STA-3 says, and an
// hour costs at most 600 points. STA-3, POL-17, POL-23.
#[tokio::test]
#[cfg_attr(
    target_os = "macos",
    ignore = "T62: on macOS the clock settles before a large answer reaches the CLI"
)]
async fn tst6_200_prs_cost_at_most_600_and_end_slowed_or_capped() {
    let h = Harness::start(Opts::default().world(|w| {
        quiet(w, TWO_HUNDRED);
    }))
    .await;
    boot(&h).await;
    until_polls(&h, 2).await;
    let t2 = log(&h)[polls_index(&h, 1)].mono;
    let end = t2.plus(s(3600));
    keep_active_until(&h, end).await;
    adv_to(&h, end).await;
    let hour = Hour {
        start: t2,
        end,
        reqs: between(&log(&h), t2, end),
    };
    eprintln!(
        "200 PRs: {} points, {} polls, {} requests",
        hour.points(),
        hour.polls().len(),
        hour.reqs.len()
    );
    assert!(
        hour.points() <= TWO_HUNDRED_MAX_POINTS,
        "{} points in an hour with {TWO_HUNDRED} PRs, at most {TWO_HUNDRED_MAX_POINTS}:{}",
        hour.points(),
        hour.dump()
    );
    // By the end it ran slowed (polls 60 s apart) or capped (`quota_capped`).
    let v = status(&h).await;
    let polled = hour.polls();
    let late: Vec<&Request> = polled
        .iter()
        .copied()
        .filter(|r| at(r) + s(300) >= end.as_duration())
        .collect();
    let slowed = !late.is_empty() && gaps(&late).iter().all(|g| *g >= s(2 * INTERVAL));
    let capped = warnings(&v).contains(&"quota_capped".to_string());
    assert!(
        slowed || capped,
        "an hour with {TWO_HUNDRED} PRs ended neither slowed nor capped:{}",
        hour.dump()
    );
    h.finish().await;
}

// ---- bullet 6: problems and warnings ----

// TST-6, API-4: `daemon.problem` reads `null` when normal and slowed, `quota_low` when capped,
// and `quota_capped` is a warning (API-5). POL-23.
#[tokio::test]
async fn tst6_problem_null_when_normal_and_slowed_quota_low_when_capped() {
    let h = Harness::start(Opts::default().quota(12, RESERVE).world(basic)).await;
    boot(&h).await;
    let v = status(&h).await;
    assert_eq!(problem(&v), None, "normal: {v}");
    assert!(warnings(&v).is_empty(), "normal: {v}");
    // Above 80 % of 12 is 10 points; 100 % is 12.
    let mut slowed_seen = false;
    for _ in 0..2000 {
        let points = h.github().points();
        if points >= 12 {
            break;
        }
        if points >= 10 {
            let v = status(&h).await;
            assert_eq!(problem(&v), None, "slowed at {points} points: {v}");
            slowed_seen = true;
        }
        adv(&h, 5).await;
    }
    assert!(slowed_seen, "never saw 10 or 11 points");
    // The warnings are raised at the next tick (POL-26): at most 10 s.
    adv(&h, 10).await;
    let v = status(&h).await;
    assert_eq!(h.github().points(), 12, "{}", dump(&log(&h)));
    assert_eq!(problem(&v).as_deref(), Some("quota_low"), "capped: {v}");
    assert!(
        warnings(&v).contains(&"quota_capped".to_string()),
        "capped: {v}"
    );
    h.finish().await;
}

fn reset_in(secs: u64) -> Time {
    Time::parse(START).unwrap().plus(s(secs))
}

// TST-6, API-4, API-5: with `remaining` under the reserve, `daemon.problem` is `quota_low` and the
// `quota_reserve` warning is raised. POL-18, POL-23.
#[tokio::test]
async fn tst6_problem_quota_low_in_reserve() {
    let h = Harness::start(Opts::default().world(basic)).await;
    boot(&h).await;
    assert_eq!(problem(&status(&h).await), None);
    h.github().set_rate(400, reset_in(3600));
    // The poll that sees it, then the tick that raises the warning (POL-26).
    adv(&h, INTERVAL + 10).await;
    let v = status(&h).await;
    assert_eq!(problem(&v).as_deref(), Some("quota_low"), "{v}");
    assert!(warnings(&v).contains(&"quota_reserve".to_string()), "{v}");
    assert!(!warnings(&v).contains(&"quota_capped".to_string()), "{v}");
    h.finish().await;
}

// TST-6, API-4: `github_unavailable` in a backoff. POL-19.
#[tokio::test]
async fn tst6_problem_github_unavailable_in_a_backoff() {
    let h = Harness::start(Opts::default().world(basic)).await;
    boot(&h).await;
    h.github().fault(Fault::Status(500), Times::UntilCleared);
    adv(&h, INTERVAL).await;
    let v = status(&h).await;
    assert_eq!(problem(&v).as_deref(), Some("github_unavailable"), "{v}");
    h.github().clear_faults();
    h.finish().await;
}

// TST-6, API-4: `auth` in the auth state. POL-20.
#[tokio::test]
async fn tst6_problem_auth_in_the_auth_state() {
    let h = Harness::start(Opts::default().world(basic)).await;
    boot(&h).await;
    h.github().fault(Fault::Unauthorized, Times::UntilCleared);
    adv(&h, INTERVAL).await;
    let v = status(&h).await;
    assert_eq!(problem(&v).as_deref(), Some("auth"), "{v}");
    h.github().clear_faults();
    h.finish().await;
}

// TST-6, API-4: with `remaining` under the reserve and a 401 that stays, `auth` wins over
// `quota_low`. POL-20, POL-23.
#[tokio::test]
async fn tst6_auth_wins_over_quota_low() {
    let h = Harness::start(
        Opts::default()
            .world(basic)
            .github(|gh| gh.set_rate(400, reset_in(3600))),
    )
    .await;
    boot(&h).await;
    let v = status(&h).await;
    assert_eq!(problem(&v).as_deref(), Some("quota_low"), "{v}");
    h.github().fault(Fault::Unauthorized, Times::UntilCleared);
    // In the reserve the next poll is `idle_interval` after the last.
    adv(&h, IDLE_INTERVAL + 10).await;
    list(&h).await;
    let v = status(&h).await;
    assert_eq!(problem(&v).as_deref(), Some("auth"), "{v}");
    h.github().clear_faults();
    h.finish().await;
}

/// Quota 12 points a hour, a wait on `octo/app#1` to spend them fast: up to the cap, then settled.
/// The waiter comes back.
async fn capped_with_a_waiter() -> (Harness, Bg, String) {
    let (h, prs) = start(Opts::default().quota(12, RESERVE), |w| quiet(w, 10)).await;
    let id = node_id(&h, prs[0]);
    boot(&h).await;
    let waiter = wait_on(&h, PR1, 3600);
    spend_until(&h, 12).await;
    // The warnings are raised at the next tick (POL-26): at most 10 s.
    adv(&h, 10).await;
    let v = status(&h).await;
    assert!(
        warnings(&v).contains(&"quota_capped".to_string()),
        "not capped: {v}"
    );
    (h, waiter, id)
}

// TST-6: with the count at 100 % and `remaining` under the reserve, both `quota_capped` and
// `quota_reserve` are in `warnings`; with `remaining` back, only `quota_capped`; once the hour has
// slid past, none. API-5, POL-23: each warning follows its own condition.
#[tokio::test]
async fn tst6_warnings_raised_together_and_cleared_apart() {
    let (h, waiter, _) = capped_with_a_waiter().await;
    // `remaining` falls under the reserve; the daemon learns it from the next answer, which is a
    // forced refresh (30 s windows while capped).
    adv(&h, 40).await;
    h.github().set_rate(400, reset_in(7200));
    let out = quick(&h, &["show", PR1, "--fresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    adv(&h, 10).await;
    let mut both = warnings(&status(&h).await);
    both.sort();
    assert_eq!(both, ["quota_capped", "quota_reserve"], "both raised");
    // `remaining` is back.
    adv(&h, 40).await;
    h.github().set_rate(5000, reset_in(7200));
    let out = quick(&h, &["show", PR1, "--fresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    adv(&h, 10).await;
    assert_eq!(
        warnings(&status(&h).await),
        ["quota_capped"],
        "`remaining` back: only the cap"
    );
    // The hour slides past: nothing spent is in it any more.
    waiter.cancel().await;
    let last_spent = log(&h).last().expect("requests were sent").mono;
    adv_to(&h, last_spent.plus(s(3600 + 120))).await;
    let v = status(&h).await;
    assert!(warnings(&v).is_empty(), "the hour slid past: {v}");
    assert_eq!(problem(&v), None, "the hour slid past: {v}");
    h.finish().await;
}

// ---- bullet 7: the reserve ----

// TST-6: with `remaining` under the reserve, polls and a waited-on PR's refreshes are
// `idle_interval` apart. POL-18, POL-23.
#[tokio::test]
async fn tst6_reserve_polls_and_hot_refreshes_are_idle_interval_apart() {
    let (h, prs) = start(Opts::default(), |w| quiet(w, 10)).await;
    let id = node_id(&h, prs[0]);
    boot(&h).await;
    let waiter = wait_on(&h, PR1, 3600);
    adv(&h, 20).await;
    h.github().set_rate(400, reset_in(7200));
    let seen = log(&h).len();
    // The next answer reports 399: the reserve.
    until(&h, 1, 60, "a request after `set_rate`", |h| {
        log(h).len() > seen
    })
    .await;
    let first = log(&h)[seen].mono;
    let to = first.plus(s(1500));
    keep_active_until(&h, to).await;
    adv_to(&h, to).await;
    let rs = after(&log(&h), first);
    let all: Vec<&Request> = rs.iter().collect();
    assert!(
        all.len() >= 4,
        "{} requests in 1 500 s in the reserve:{}",
        all.len(),
        dump(&rs)
    );
    for g in gaps(&all) {
        assert!(
            (s(IDLE_INTERVAL)..=s(IDLE_INTERVAL + 11)).contains(&g),
            "a gap of {g:?} in the reserve, not {IDLE_INTERVAL} s:{}",
            dump(&rs)
        );
    }
    assert!(
        !naming(&rs, &id).is_empty() && rs.iter().any(is_poll),
        "polls and the waited-on PR's refreshes:{}",
        dump(&rs)
    );
    waiter.cancel().await;
    h.finish().await;
}

// TST-6: with both the reserve and the cap, no poll and no hot refresh is sent.
// POL-23: capped wins from the bottom up.
#[tokio::test]
async fn tst6_reserve_with_the_cap_sends_none() {
    let (h, waiter, _) = capped_with_a_waiter().await;
    adv(&h, 40).await;
    h.github().set_rate(400, reset_in(7200));
    let out = quick(&h, &["show", PR1, "--fresh", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    adv(&h, 10).await;
    let mut both = warnings(&status(&h).await);
    both.sort();
    assert_eq!(both, ["quota_capped", "quota_reserve"]);
    let sent = log(&h).len();
    let from = now(&h);
    for _ in 0..12 {
        adv(&h, 100).await;
        list(&h).await;
    }
    assert_eq!(
        log(&h).len(),
        sent,
        "requests in 20 minutes with the reserve and the cap:{}",
        dump(&after(&log(&h), from))
    );
    waiter.cancel().await;
    h.finish().await;
}

// ---- bullet 8: exhausted ----

/// Quiet world, the viewer's limit resets at `reset`.
async fn exhausted_world(reset: Time) -> Harness {
    Harness::start(
        Opts::default()
            .world(basic)
            .github(move |gh| gh.set_rate(5000, reset)),
    )
    .await
}

/// The first request that was answered with a fault.
fn faulted(h: &Harness) -> Option<Request> {
    log(h).into_iter().find(|r| r.fault.is_some())
}

/// `show` of a tracked PR (exit 0, from memory) and of an untracked one (exit 9, `rate_limited`,
/// `error.until` the reset time), and neither sends a request.
async fn reads_while_limited(h: &Harness, until: Option<Time>) {
    let sent = log(h).len();
    let out = quick(h, &["show", PR1, "--json"]).await;
    assert_eq!(out.code, 0, "a tracked PR from memory: {out:?}");
    let out = quick(h, &["show", PR_GONE, "--json"]).await;
    assert_eq!(out.code, 9, "an untracked PR: {out:?}");
    let err = json(&out)["error"].clone();
    assert_eq!(err["code"], "rate_limited", "{out:?}");
    if let Some(until) = until {
        assert_eq!(err["until"], until.rfc3339(), "{out:?}");
    }
    assert_eq!(
        log(h).len(),
        sent,
        "a read sent a request:{}",
        dump(&log(h))
    );
}

// TST-6: exhausted: nothing is sent until `x-ratelimit-reset`; a tracked PR is answered from
// memory, a read-through gets `rate_limited` with `until`, exit 9; requests start again within an
// `idle_interval` of the reset. POL-25, POL-23, API-4. Both ways an answer can come.
#[tokio::test]
async fn tst6_exhausted_is_silent_until_the_reset_then_resumes() {
    for limit in [LimitStatus::Forbidden, LimitStatus::Ok] {
        let reset = reset_in(1200);
        let h = exhausted_world(reset).await;
        boot(&h).await;
        h.github().fault(
            Fault::Exhausted {
                status: limit,
                reset: true,
            },
            Times::Requests(1),
        );
        until(&h, 1, 60, "the exhausted answer", |h| faulted(h).is_some()).await;
        h.clock().settle().await;
        let hit = faulted(&h).unwrap();
        let v = status(&h).await;
        assert_eq!(
            problem(&v).as_deref(),
            Some("rate_limited"),
            "{limit:?}: {v}"
        );
        assert_eq!(
            v["daemon"]["until"],
            reset.rfc3339(),
            "{limit:?}: `until` is the reset time: {v}"
        );
        let sent = log(&h).len();
        // Silent until the reset, with clients asking all the while.
        let reset_mono = Mono::ZERO.plus(s(1200));
        while now(&h).plus(s(100)) < reset_mono {
            adv(&h, 100).await;
            list(&h).await;
            reads_while_limited(&h, Some(reset)).await;
        }
        adv_to(&h, before(reset_mono, s(1))).await;
        assert_eq!(
            log(&h).len(),
            sent,
            "{limit:?}: requests before the reset at {:?}:{}",
            reset_mono.as_duration(),
            dump(&after(&log(&h), hit.mono))
        );
        // And sending again within `idle_interval` of it.
        adv_to(&h, reset_mono.plus(s(IDLE_INTERVAL))).await;
        let resumed: Vec<Request> = after(&log(&h), hit.mono);
        eprintln!(
            "exhausted {limit:?}: answered at {:?}, next request at {:?}, reset at {:?}",
            hit.mono.as_duration(),
            resumed.first().map(at),
            reset_mono.as_duration()
        );
        assert!(
            resumed.iter().any(|r| r.mono >= reset_mono),
            "{limit:?}: nothing sent within {IDLE_INTERVAL} s of the reset:{}",
            dump(&log(&h))
        );
        assert!(
            resumed.iter().all(|r| r.mono >= reset_mono),
            "{limit:?}: a request before the reset:{}",
            dump(&resumed)
        );
        h.finish().await;
    }
}

// TST-6: an exhausted answer without `x-ratelimit-reset`: nothing for an hour. POL-25.
#[tokio::test]
async fn tst6_exhausted_without_a_reset_header_waits_an_hour() {
    let h = exhausted_world(reset_in(3600)).await;
    boot(&h).await;
    h.github().fault(
        Fault::Exhausted {
            status: LimitStatus::Forbidden,
            reset: false,
        },
        Times::Requests(1),
    );
    until(&h, 1, 60, "the exhausted answer", |h| faulted(h).is_some()).await;
    h.clock().settle().await;
    let hit = faulted(&h).unwrap();
    let v = status(&h).await;
    assert_eq!(problem(&v).as_deref(), Some("rate_limited"), "{v}");
    let sent = log(&h).len();
    let resume = hit.mono.plus(s(3600));
    while now(&h).plus(s(300)) < resume {
        adv(&h, 300).await;
        list(&h).await;
    }
    adv_to(&h, before(resume, s(1))).await;
    assert_eq!(
        log(&h).len(),
        sent,
        "requests inside the hour after an answer with no reset:{}",
        dump(&after(&log(&h), hit.mono))
    );
    adv_to(&h, resume.plus(s(IDLE_INTERVAL))).await;
    let resumed = after(&log(&h), hit.mono);
    assert!(
        !resumed.is_empty() && resumed.iter().all(|r| r.mono >= resume),
        "nothing sent within {IDLE_INTERVAL} s after the hour, or something before it:{}",
        dump(&resumed)
    );
    h.finish().await;
}

// TST-6: a wall-clock jump past the reset resumes sending: a request is sent within an
// `idle_interval` where, without the jump, none would be. POL-25, POL-26.
#[tokio::test]
async fn tst6_a_wall_jump_past_the_reset_resumes_sending() {
    let reset = reset_in(1200);
    let h = exhausted_world(reset).await;
    boot(&h).await;
    h.github().fault(
        Fault::Exhausted {
            status: LimitStatus::Forbidden,
            reset: true,
        },
        Times::Requests(1),
    );
    until(&h, 1, 60, "the exhausted answer", |h| faulted(h).is_some()).await;
    h.clock().settle().await;
    let hit = faulted(&h).unwrap();
    // Without the jump: nothing, as the test above shows.
    adv(&h, 60).await;
    list(&h).await;
    assert_eq!(after(&log(&h), hit.mono).len(), 0, "{}", dump(&log(&h)));
    let slept = now(&h);
    h.clock().sleep_wall_only(s(1500)).await;
    adv(&h, IDLE_INTERVAL).await;
    let resumed = after(&log(&h), hit.mono);
    assert!(
        !resumed.is_empty(),
        "nothing sent within {IDLE_INTERVAL} s of a wall jump past the reset:{}",
        dump(&log(&h))
    );
    assert!(
        resumed.iter().all(|r| r.mono >= slept),
        "a request before the jump:{}",
        dump(&resumed)
    );
    eprintln!(
        "wall jump: answered at {:?}, next request at {:?}",
        hit.mono.as_duration(),
        at(&resumed[0])
    );
    assert!(
        resumed[0].mono < Mono::ZERO.plus(s(1200)),
        "the first request came after the reset by the monotonic clock: it wasn't the jump:{}",
        dump(&resumed)
    );
    h.finish().await;
}

// ---- bullet 8: a secondary limit ----

/// A tracked PR from memory, an untracked one `rate_limited`, nothing sent.
async fn reads_while_secondary(h: &Harness) {
    reads_while_limited(h, None).await;
}

// TST-6: a secondary limit, as a 403 and as a 200 with an error, with `Retry-After: 90`: nothing
// for 90 s; a read-through meanwhile exits 9, `rate_limited`, and sends nothing. POL-25.
#[tokio::test]
async fn tst6_secondary_with_retry_after_waits_90_s() {
    for limit in [LimitStatus::Forbidden, LimitStatus::Ok] {
        let h = Harness::start(Opts::default().world(basic)).await;
        boot(&h).await;
        h.github().fault(
            Fault::Secondary {
                status: limit,
                retry_after: Some(90),
            },
            Times::Requests(1),
        );
        until(&h, 1, 60, "the secondary-limit answer", |h| {
            faulted(h).is_some()
        })
        .await;
        h.clock().settle().await;
        let hit = faulted(&h).unwrap();
        let v = status(&h).await;
        assert_eq!(
            problem(&v).as_deref(),
            Some("rate_limited"),
            "{limit:?}: {v}"
        );
        let sent = log(&h).len();
        for _ in 0..2 {
            adv(&h, 30).await;
            list(&h).await;
            reads_while_secondary(&h).await;
        }
        adv_to(&h, hit.mono.plus(s(89))).await;
        assert_eq!(
            log(&h).len(),
            sent,
            "{limit:?}: requests inside the 90 s:{}",
            dump(&after(&log(&h), hit.mono))
        );
        adv_to(&h, hit.mono.plus(s(90 + INTERVAL))).await;
        let next = after(&log(&h), hit.mono);
        assert!(
            !next.is_empty() && at(&next[0]) >= at(&hit) + s(90),
            "{limit:?}: the first request after the answer wasn't 90 s or more later:{}",
            dump(&log(&h))
        );
        assert!(
            at(&next[0]) <= at(&hit) + s(90 + INTERVAL),
            "{limit:?}: nothing sent within {INTERVAL} s of the 90:{}",
            dump(&log(&h))
        );
        h.finish().await;
    }
}

// TST-6: a secondary limit with no `Retry-After`: nothing for 60 s, then 120 s after the next in a
// row, up to `idle_interval`. POL-25.
#[tokio::test]
async fn tst6_secondary_without_retry_after_waits_60_s_and_doubles() {
    for limit in [LimitStatus::Forbidden, LimitStatus::Ok] {
        let h = Harness::start(Opts::default().world(basic)).await;
        boot(&h).await;
        // Five in a row: 60, 120, 240, then 300 (`idle_interval`) twice.
        h.github().fault(
            Fault::Secondary {
                status: limit,
                retry_after: None,
            },
            Times::Requests(5),
        );
        until(&h, 1, 60, "the first secondary-limit answer", |h| {
            faulted(h).is_some()
        })
        .await;
        h.clock().settle().await;
        let mut waits = Vec::new();
        for want in [60, 120, 240, 300, 300] {
            let last = log(&h).last().expect("a request").mono;
            let mid = last.plus(s(want / 2));
            adv_to(&h, mid).await;
            list(&h).await;
            reads_while_secondary(&h).await;
            adv_to(&h, last.plus(s(want + INTERVAL + 1))).await;
            let next = after(&log(&h), last);
            assert!(
                !next.is_empty(),
                "{limit:?}: nothing sent {} s after the answer:{}",
                want + INTERVAL + 1,
                dump(&log(&h))
            );
            waits.push((want, at(&next[0]) - last.as_duration()));
        }
        eprintln!("secondary {limit:?}: waits {waits:?}");
        for (want, waited) in &waits {
            assert!(
                *waited >= s(*want) && *waited <= s(want + INTERVAL),
                "{limit:?}: waited {waited:?} after a secondary limit, expected {want} s: {waits:?}\n{}",
                dump(&log(&h))
            );
        }
        h.finish().await;
    }
}

// ---- U1: many clients cost as one ----

/// Over 10 minutes: `waits` waits on `octo/app#1`, and with `others` a `list` and a `show` every 10 s
/// from other sessions. The points spent from the waits' arrival.
async fn ten_minutes(waits: usize, others: bool) -> (u32, Vec<Request>) {
    let h = Harness::start(Opts::default().world(basic)).await;
    boot(&h).await;
    until_polls(&h, 2).await;
    let from = now(&h);
    let to = from.plus(s(600));
    let waiters: Vec<Bg> = (0..waits).map(|_| wait_on(&h, PR1, 600)).collect();
    while now(&h) < to {
        adv(&h, 10).await;
        if others && now(&h) < to {
            list(&h).await;
            let out = quick(&h, &["show", PR1, "--json"]).await;
            assert_eq!(out.code, 0, "{out:?}");
        }
    }
    let rs = between(&log(&h), from, to);
    for w in waiters {
        let out = finish_bg(&h, w).await;
        assert_eq!(out.code, 3, "{out:?}");
    }
    h.finish().await;
    (points(&rs), rs)
}

// U1: five background waits on one PR, and a `list` and a `show` every 10 s from other sessions,
// cost over 10 minutes no more points than one wait alone, give or take one poll: the agents'
// reads come from memory, and the waits share one refresh. POL-14, POL-15.
#[tokio::test]
async fn u1_many_clients_cost_as_one() {
    let (one, one_log) = ten_minutes(1, false).await;
    let (many, many_log) = ten_minutes(5, true).await;
    eprintln!("U1: one wait {one} points, five waits and readers {many}");
    assert!(
        many <= one + 1,
        "five waits and readers cost {many} points in 10 minutes, one wait {one} (plus one poll allowed):\none:{}\nmany:{}",
        dump(&one_log),
        dump(&many_log)
    );
}
