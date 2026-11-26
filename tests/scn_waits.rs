#![cfg(feature = "test-endpoint")]
//! T31: TST-8's scenarios about a PR's life and the waits on it: `merged`, `closed`, `reopened`,
//! `conflict`, `mergeable-unknown`, `base-changed`, `head-moved`, `rerun`, `waiter-limit`,
//! `new-review`, `changed-kind`, `baseline` and `gone-returns`. Each runs the daemon on the test
//! clock against the fake GitHub, looks through the CLI's `--json` output, and ends with
//! `h.finish()` (TST-11).
//!
//! The world: `octo/app#N`, authored by the fake's login. "Settled" means `updatedAt` two minutes
//! old at the first read (MOD-6's shortcut). The values below are the specs' (proposals all).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::api::{ChangesBody, Envelope, ListBody, Outcome, Reason, StatusBody, WaitBody};
use circitor::cli::Output;
use circitor::clock::{Clock, Mono, Time};
use circitor::model::{
    ChecksState, Event, EventKind, ItemKind, Mergeable, PrState, Record, Status,
};
use circitor::testkit::github::{AskedOf, PrId, RepoId, World};
use circitor::testkit::harness::{Bg, Harness, Opts};
use serde::de::DeserializeOwned;

/// MOD-8: how long a known `mergeable` is kept while GitHub answers `UNKNOWN`.
const KEEP_MERGEABLE: Duration = Duration::from_secs(60);
/// POL-11: the follow-up read after one that read `UNKNOWN`.
const FOLLOW_UP: Duration = Duration::from_secs(3);
/// API-11: a `--sha` that never matches ends a wait at the first refresh sent this long after it began.
const HEAD_MOVED_AFTER: Duration = Duration::from_secs(60);
/// API-14.
const WAITERS_MAX: usize = 64;
/// POL-21: a PR whose node reads `NOT_FOUND` on every poll for this long is gone.
const GONE_AFTER: Duration = Duration::from_secs(300);
/// OPS-2.
const HOT_INTERVAL: Duration = Duration::from_secs(10);
/// MOD-6: settling a list with checks.
const SETTLE: Duration = Duration::from_secs(20);

const PR1: &str = "octo/app#1";

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
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

/// A PR whose `updatedAt` is two minutes old: settled at its first read (MOD-6).
fn settled_pr(w: &mut World, repo: RepoId) -> PrId {
    let now = w.now();
    let pr = w.open_pr(repo);
    w.set_created_at(pr, now.minus(s(3600)));
    w.set_updated_at(pr, now.minus(s(120)));
    pr
}

async fn adv(h: &Harness, n: u64) {
    h.clock().advance(s(n)).await;
}

/// Moves the clock in 100 ms steps until every command is done (at most 90 s), then awaits them.
async fn drive(h: &Harness, bgs: Vec<Bg>) -> Vec<Output> {
    for _ in 0..900 {
        if bgs.iter().all(Bg::is_done) {
            break;
        }
        h.clock().advance(ms(100)).await;
    }
    assert!(
        bgs.iter().all(Bg::is_done),
        "a command didn't end within 90 s"
    );
    let mut outs = Vec::new();
    for b in bgs {
        outs.push(b.await);
    }
    outs
}

/// The same for one command.
async fn drive1(h: &Harness, bg: Bg) -> Output {
    drive(h, vec![bg]).await.remove(0)
}

fn env<T: DeserializeOwned>(out: &Output) -> Envelope<T> {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

fn waited(out: &Output) -> WaitBody {
    env::<WaitBody>(out).body
}

fn has_line(out: &Output, line: &str) -> bool {
    out.stdout.lines().any(|l| l == line)
}

fn words(e: &Event) -> Vec<&'static str> {
    e.kinds.iter().map(EventKind::word).collect()
}

fn has_kind(events: &[Event], word: &str) -> bool {
    events.iter().any(|e| words(e).contains(&word))
}

fn kind<'a>(events: &'a [Event], word: &str) -> Option<&'a EventKind> {
    events
        .iter()
        .flat_map(|e| e.kinds.iter())
        .find(|k| k.word() == word)
}

fn node_id(h: &Harness, pr: PrId) -> String {
    h.github().world(|w| w.node_id(pr).to_string())
}

/// A request that named the PR's node ID, as the log has it.
#[derive(Clone, Copy, Debug)]
struct Hit {
    at: Mono,
    wall: Time,
    of: AskedOf,
}

fn hits(h: &Harness, id: &str) -> Vec<Hit> {
    h.github()
        .requests()
        .into_iter()
        .filter_map(|r| {
            r.nodes.iter().find(|(n, _)| n == id).map(|(_, of)| Hit {
                at: r.mono,
                wall: r.wall,
                of: *of,
            })
        })
        .collect()
}

/// A PR's records through `list --json` (which also keeps the tier active).
async fn listed(h: &Harness, all: bool) -> Vec<Record> {
    let out = if all {
        h.cli(&["list", "--all", "--json"]).await
    } else {
        h.cli(&["list", "--json"]).await
    };
    assert_eq!(out.code, 0, "{out:?}");
    env::<ListBody>(&out).body.records
}

async fn record(h: &Harness, name: &str) -> Record {
    listed(h, true)
        .await
        .into_iter()
        .find(|r| r.pr.to_string() == name)
        .unwrap_or_else(|| panic!("{name} isn't listed"))
}

/// `<epoch>-0`: the cursor before any revision (STA-9).
async fn epoch0(h: &Harness) -> String {
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let cursor = env::<StatusBody>(&out).cursor;
    let (epoch, _) = cursor.split_once('-').expect("a cursor is <epoch>-<rev>");
    format!("{epoch}-0")
}

/// The PR's events since `cursor`, through `changes <pr> --since <cursor> --json`.
async fn events(h: &Harness, pr: &str, cursor: &str) -> Vec<Event> {
    let out = h.cli(&["changes", pr, "--since", cursor, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    env::<ChangesBody>(&out).body.events
}

/// The state file's entry for `pr`.
fn file_entry(h: &Harness, pr: &str) -> Option<serde_json::Value> {
    let text = h.state_file()?;
    let v: serde_json::Value = serde_json::from_str(&text).expect("state.json is JSON");
    v["prs"].as_array()?.iter().find(|e| e["pr"] == pr).cloned()
}

async fn waiters(h: &Harness) -> u32 {
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    env::<StatusBody>(&out).body.waiters
}

// ---------------------------------------------------------------------------------------------
// merged, closed
// ---------------------------------------------------------------------------------------------

// TST-8: merged
// POL-10: a PR that leaves the open list is refreshed by ID, and that shows `merged`.
// API-11: `merged` satisfies `until=merged` and ends a `checks-done` wait whose checks aren't done.
// MOD-8: a merge adds no `mergeable` kind.
#[tokio::test]
async fn scn_merged() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        let slow = w.run(pr, "slow");
        w.start(slow);
        pr
    })
    .await;
    let id = node_id(&h, p);
    // The first poll, and then outside POL-15's 5 s window.
    adv(&h, 20).await;
    assert_eq!(record(&h, PR1).await.state, PrState::Open);
    let waits = vec![
        h.cli_bg(&["wait", PR1, "--until", "merged"]),
        h.cli_bg(&["wait", PR1, "--until", "merged", "--json"]),
        h.cli_bg(&["wait", PR1, "--until", "checks-done"]),
        h.cli_bg(&["wait", PR1, "--until", "checks-done", "--json"]),
    ];
    adv(&h, 3).await;
    assert!(waits.iter().all(|b| !b.is_done()));
    let merged_at = h.clock().mono();
    h.github().world(|w| w.merge(p));
    let outs = drive(&h, waits).await;

    // `--until merged`: satisfied.
    assert_eq!(outs[0].code, 0, "{:?}", outs[0]);
    assert!(has_line(&outs[0], "merged"), "{:?}", outs[0]);
    let b = waited(&outs[1]);
    assert_eq!(outs[1].code, 0, "{:?}", outs[1]);
    assert_eq!(b.outcome, Outcome::Satisfied);
    assert_eq!(b.reason, None);
    // `--until checks-done`: ended, `merged`.
    assert_eq!(outs[2].code, 4, "{:?}", outs[2]);
    assert!(has_line(&outs[2], "ended: merged"), "{:?}", outs[2]);
    let c = waited(&outs[3]);
    assert_eq!(outs[3].code, 4, "{:?}", outs[3]);
    assert_eq!(c.outcome, Outcome::Ended);
    assert_eq!(c.reason, Some(Reason::Merged));

    // The record.
    for body in [&b, &c] {
        let r = body.record.as_ref().expect("the wait's record");
        assert_eq!(r.state, PrState::Merged);
        assert_eq!(r.status, Status::Merged);
        assert_eq!(r.mergeable, Mergeable::Unknown);
        assert_eq!(r.merge_state, "unknown");
        assert!(r.merged_at.is_some());
        // MOD-8: the merge makes a `merged` kind and no `mergeable` kind.
        assert!(has_kind(&body.events, "merged"), "{:?}", body.events);
        assert!(!has_kind(&body.events, "mergeable"), "{:?}", body.events);
    }

    // After the merge a request naming the node ID showed it (POL-10).
    assert!(
        hits(&h, &id).iter().any(|x| x.at > merged_at),
        "no request named the PR after the merge"
    );
    // `list` leaves it out; `list --all` has it.
    assert!(listed(&h, false).await.is_empty());
    assert!(
        listed(&h, true)
            .await
            .iter()
            .any(|r| r.pr.to_string() == PR1 && r.state == PrState::Merged)
    );
    h.finish().await;
}

// TST-8: closed
// API-11: a close ends `merged` and `checks-done` waits, and satisfies a `changed` wait.
#[tokio::test]
async fn scn_closed() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        let slow = w.run(pr, "slow");
        w.start(slow);
        pr
    })
    .await;
    adv(&h, 20).await;
    let waits = vec![
        h.cli_bg(&["wait", PR1, "--until", "merged"]),
        h.cli_bg(&["wait", PR1, "--until", "merged", "--json"]),
        h.cli_bg(&["wait", PR1, "--until", "checks-done"]),
        h.cli_bg(&["wait", PR1, "--until", "checks-done", "--json"]),
        h.cli_bg(&["wait", PR1, "--until", "changed"]),
        h.cli_bg(&["wait", PR1, "--until", "changed", "--json"]),
    ];
    adv(&h, 3).await;
    assert!(waits.iter().all(|b| !b.is_done()));
    h.github().world(|w| w.close(p));
    let outs = drive(&h, waits).await;

    assert_eq!(outs[0].code, 4, "{:?}", outs[0]);
    assert!(has_line(&outs[0], "ended: closed"), "{:?}", outs[0]);
    let merged = waited(&outs[1]);
    assert_eq!(outs[1].code, 4, "{:?}", outs[1]);
    assert_eq!(
        (merged.outcome, merged.reason),
        (Outcome::Ended, Some(Reason::Closed))
    );

    assert_eq!(outs[2].code, 4, "{:?}", outs[2]);
    assert!(has_line(&outs[2], "ended: closed"), "{:?}", outs[2]);
    let checks = waited(&outs[3]);
    assert_eq!(outs[3].code, 4, "{:?}", outs[3]);
    assert_eq!(
        (checks.outcome, checks.reason),
        (Outcome::Ended, Some(Reason::Closed))
    );

    assert_eq!(outs[4].code, 0, "{:?}", outs[4]);
    assert!(has_line(&outs[4], "changed"), "{:?}", outs[4]);
    let changed = waited(&outs[5]);
    assert_eq!(outs[5].code, 0, "{:?}", outs[5]);
    assert_eq!(
        (changed.outcome, changed.reason),
        (Outcome::Satisfied, None)
    );

    for body in [&merged, &checks, &changed] {
        let r = body.record.as_ref().expect("the wait's record");
        assert_eq!(r.state, PrState::Closed);
        // A closed PR keeps the `mergeable` it had (docs/02 §4).
        assert_eq!(r.mergeable, Mergeable::Yes);
        assert!(has_kind(&body.events, "closed"), "{:?}", body.events);
    }
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// reopened
// ---------------------------------------------------------------------------------------------

// TST-8: reopened
// POL-10: a closed PR the open list shows again is refreshed as an open one at the next poll.
// STA-2: the retention clock is cleared when `state` becomes `open`.
// MOD-6: a reopening is a new settle start: `checks_pending` until a refresh 20 s later.
#[tokio::test]
async fn scn_reopened() {
    let keep_done = s(300);
    let (h, p) = start(Opts::default().keep_done(keep_done), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        let build = w.run(pr, "build");
        w.finish(build, "SUCCESS");
        pr
    })
    .await;
    let id = node_id(&h, p);
    adv(&h, 5).await;
    let cursor = epoch0(&h).await;
    let r = record(&h, PR1).await;
    assert_eq!((r.state, r.status), (PrState::Open, Status::Ready));

    // Closed at K, and seen closed at the next poll.
    adv(&h, 10).await;
    let k = h.clock().mono();
    h.github().world(|w| w.close(p));
    let mut seen = false;
    for _ in 0..40 {
        adv(&h, 1).await;
        if record(&h, PR1).await.state == PrState::Closed {
            seen = true;
            break;
        }
    }
    assert!(seen, "the close wasn't seen within 40 s");
    // The saver has written it ended.
    let mut written = false;
    for _ in 0..30 {
        adv(&h, 1).await;
        if file_entry(&h, PR1).is_some_and(|e| !e["ended"].is_null()) {
            written = true;
            break;
        }
    }
    assert!(written, "the state file never read the PR ended");

    // Reopened at R = K + 60 s.
    let now = h.clock().mono();
    adv(&h, k.plus(s(60)).since(now).as_secs()).await;
    let reopened_at = h.clock().mono();
    h.github().world(|w| w.reopen(p));

    // The first poll sent after R shows it open.
    let mut open = false;
    for _ in 0..40 {
        adv(&h, 1).await;
        if record(&h, PR1).await.state == PrState::Open {
            open = true;
            break;
        }
    }
    assert!(open, "the reopening wasn't seen within 40 s");
    let t0 = hits(&h, &id)
        .into_iter()
        .find(|x| x.at >= reopened_at)
        .expect("a request named the PR after R")
        .at;
    let r = record(&h, PR1).await;
    assert!(!r.checks.settled);
    assert_eq!(r.status, Status::ChecksPending);
    let evs = events(&h, PR1, &cursor).await;
    assert!(has_kind(&evs, "reopened"), "{evs:?}");

    // `ready` only from a refresh sent at least 20 s after the one that showed it open.
    let mut ready = false;
    for _ in 0..90 {
        adv(&h, 1).await;
        if record(&h, PR1).await.status == Status::Ready {
            ready = true;
            assert!(
                hits(&h, &id)
                    .iter()
                    .any(|x| x.at >= t0.plus(SETTLE) && x.at <= h.clock().mono()),
                "ready without a refresh sent {SETTLE:?} after the first"
            );
            break;
        }
    }
    assert!(ready, "the PR never became ready");

    // The retention clock was cleared: the file reads `ended: null`, and past K + 300 s and two
    // ticks the PR is still listed, with no `untracked` event.
    let e = file_entry(&h, PR1).expect("the PR is in the file");
    assert!(e["ended"].is_null(), "{e}");
    let target = k.plus(keep_done + s(40));
    while h.clock().mono() < target {
        adv(&h, 5).await;
        listed(&h, false).await;
    }
    assert!(
        listed(&h, false)
            .await
            .iter()
            .any(|r| r.pr.to_string() == PR1 && r.state == PrState::Open)
    );
    let e = file_entry(&h, PR1).expect("the PR is still in the file");
    assert!(e["ended"].is_null(), "{e}");
    let evs = events(&h, PR1, &cursor).await;
    assert!(!has_kind(&evs, "untracked"), "{evs:?}");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// conflict, base-changed
// ---------------------------------------------------------------------------------------------

// TST-8: conflict
// MOD-11: `CONFLICTING` gives `status: conflict`, above `checks_pending` (docs/02 §4, d1.log).
#[tokio::test]
async fn scn_conflict() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let now = w.now();
        let pr = w.open_pr(repo);
        // Created at start - 1 s, so C + 3 s is start + 2 s.
        w.set_created_at(pr, now.minus(s(1)));
        w.set_updated_at(pr, now.minus(s(1)));
        w.set_mergeable(pr, "UNKNOWN");
        w.after(s(2), move |w| {
            w.set_mergeable(pr, "CONFLICTING");
            w.set_merge_state(pr, Some("DIRTY"));
        });
        pr
    })
    .await;
    adv(&h, 10).await;
    let r = record(&h, PR1).await;
    assert_eq!(r.mergeable, Mergeable::No);
    assert_eq!(r.merge_state, "dirty");
    // Its empty checks list hasn't settled, so `checks_pending` is also true: `conflict` is first.
    assert!(!r.checks.settled);
    assert_eq!(r.checks.state, ChecksState::None);
    assert_eq!(r.status, Status::Conflict);
    h.finish().await;
}

// TST-8: base-changed
// MOD-8: a changed base returns `mergeable` to `unknown` at once, well inside the 60 s.
#[tokio::test]
async fn scn_base_changed() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        w.after(s(10), move |w| {
            w.set_base(pr, "develop");
            w.set_mergeable(pr, "UNKNOWN");
        });
        pr
    })
    .await;
    // The first poll reads `MERGEABLE` on `main`.
    adv(&h, 1).await;
    let cursor = epoch0(&h).await;
    let r = record(&h, PR1).await;
    assert_eq!((r.base.as_str(), r.mergeable), ("main", Mergeable::Yes));
    let read_at = h.clock().wall();
    // The next refresh is the poll 30 s after the first.
    adv(&h, 40).await;
    let r = record(&h, PR1).await;
    assert!(r.fetched_at.since(read_at) < KEEP_MERGEABLE);
    assert_eq!(r.base, "develop");
    assert_eq!(r.mergeable, Mergeable::Unknown);
    let evs: Vec<Event> = events(&h, PR1, &cursor)
        .await
        .into_iter()
        .filter(|e| !words(e).contains(&"tracked"))
        .collect();
    assert_eq!(evs.len(), 1, "{evs:?}");
    match kind(&evs, "edited") {
        Some(EventKind::Edited { fields }) => assert_eq!(fields, &["base"]),
        other => panic!("no `edited` kind: {other:?}"),
    }
    assert!(has_kind(&evs, "mergeable"), "{evs:?}");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// head-moved
// ---------------------------------------------------------------------------------------------

/// SHAs `A` and `B`: 40 hex characters each.
const A_SHA: &str = "dce9dbe0123456789abcdef0123456789abcdef0";
const B_SHA: &str = "cf54ec40123456789abcdef0123456789abcdef0";

/// A settled PR at `A` with a check that keeps running.
fn at_a_running(w: &mut World) -> PrId {
    let repo = w.repo("octo", "app");
    let pr = settled_pr(w, repo);
    w.push_sha(pr, A_SHA);
    w.set_updated_at(pr, w.now().minus(s(120)));
    let slow = w.run(pr, "slow");
    w.start(slow);
    pr
}

// TST-8: head-moved
// API-11: a `checks-done --sha` wait whose head matched and then moved ends with `head_moved`.
#[tokio::test]
async fn scn_head_moved_push() {
    let (h, p) = start(Opts::default(), at_a_running).await;
    adv(&h, 20).await;
    assert_eq!(record(&h, PR1).await.head_sha, A_SHA);
    let waits = vec![
        h.cli_bg(&["wait", PR1, "--until", "checks-done", "--sha", A_SHA]),
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "checks-done",
            "--sha",
            A_SHA,
            "--json",
        ]),
    ];
    adv(&h, 5).await;
    assert!(waits.iter().all(|b| !b.is_done()));
    // Pushed at P; GitHub serves the old head for 3 s.
    h.github().world(|w| {
        w.push_lagged(p, Some(B_SHA), FOLLOW_UP);
    });
    let outs = drive(&h, waits).await;
    assert_eq!(outs[0].code, 4, "{:?}", outs[0]);
    assert!(
        has_line(&outs[0], "ended: head moved to cf54ec4, expected dce9dbe"),
        "{:?}",
        outs[0]
    );
    let b = waited(&outs[1]);
    assert_eq!(outs[1].code, 4, "{:?}", outs[1]);
    assert_eq!(
        (b.outcome, b.reason),
        (Outcome::Ended, Some(Reason::HeadMoved))
    );
    h.finish().await;
}

// TST-8: head-moved
// API-11: a SHA that never matches ends the wait at the first refresh sent 60 s after it began.
#[tokio::test]
async fn scn_head_moved_wrong_sha() {
    let (h, p) = start(Opts::default(), at_a_running).await;
    let id = node_id(&h, p);
    adv(&h, 20).await;
    let began = h.clock().mono();
    let args = [
        "wait",
        PR1,
        "--until",
        "checks-done",
        "--sha",
        "deadbeef",
        "--timeout",
        "100",
    ];
    let plain = h.cli_bg(&args);
    let mut json_args = args.to_vec();
    json_args.push("--json");
    let json = h.cli_bg(&json_args);
    adv(&h, 59).await;
    assert!(!plain.is_done() && !json.is_done(), "ended before 60 s");
    // It ends at the first refresh sent at least 60 s after it began.
    let mut done_at = None;
    for _ in 0..200 {
        h.clock().advance(ms(100)).await;
        if plain.is_done() && json.is_done() {
            done_at = Some(h.clock().mono());
            break;
        }
    }
    let done_at = done_at.expect("the waits never ended");
    let first = hits(&h, &id)
        .into_iter()
        .find(|x| x.at >= began.plus(HEAD_MOVED_AFTER))
        .expect("a refresh was sent 60 s after the wait began");
    assert!(first.at <= done_at && done_at.since(first.at) <= s(1));
    let outs = drive(&h, vec![plain, json]).await;
    assert_eq!(outs[0].code, 4, "{:?}", outs[0]);
    assert!(
        has_line(&outs[0], "ended: head moved to dce9dbe, expected deadbee"),
        "{:?}",
        outs[0]
    );
    let b = waited(&outs[1]);
    assert_eq!(
        (b.outcome, b.reason),
        (Outcome::Ended, Some(Reason::HeadMoved))
    );
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// waiter-limit
// ---------------------------------------------------------------------------------------------

// TST-8: waiter-limit
// API-14: a 65th waiter gets `limit`, exit 10; a client that closes frees its place.
#[tokio::test]
async fn scn_waiter_limit() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo);
    })
    .await;
    adv(&h, 20).await;
    // One at a time: `Bg::cancel` counts the connections open when its command began.
    let mut held: Vec<Bg> = Vec::new();
    for _ in 0..WAITERS_MAX {
        held.push(h.cli_bg(&["wait", PR1, "--until", "merged", "--timeout", "3600"]));
        h.clock().advance(ms(10)).await;
    }
    assert!(held.iter().all(|b| !b.is_done()));
    assert_eq!(waiters(&h).await as usize, WAITERS_MAX);

    // The 65th.
    let out = h
        .cli(&["wait", PR1, "--until", "merged", "--timeout", "3600"])
        .await;
    assert_eq!(out.code, 10, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    let json = h
        .cli(&[
            "wait",
            PR1,
            "--until",
            "merged",
            "--timeout",
            "3600",
            "--json",
        ])
        .await;
    assert_eq!(json.code, 10, "{json:?}");
    let body: circitor::api::ErrorBody = serde_json::from_str(&json.stdout)
        .or_else(|_| serde_json::from_str(&json.stderr))
        .unwrap_or_else(|e| panic!("{e}: {json:?}"));
    assert_eq!(body.error.code, circitor::api::ErrorCode::Limit);
    assert_eq!(waiters(&h).await as usize, WAITERS_MAX);

    // A client that closes frees its place.
    held.pop().expect("a waiter").cancel().await;
    assert_eq!(waiters(&h).await as usize, WAITERS_MAX - 1);
    held.push(h.cli_bg(&["wait", PR1, "--until", "merged", "--timeout", "3600"]));
    h.clock().advance(ms(10)).await;
    assert!(held.iter().all(|b| !b.is_done()));
    assert_eq!(waiters(&h).await as usize, WAITERS_MAX);

    while let Some(b) = held.pop() {
        b.cancel().await;
    }
    assert_eq!(waiters(&h).await, 0);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// rerun
// ---------------------------------------------------------------------------------------------

/// A settled PR at `A` whose check `build` completed with `FAILURE`.
fn failed_build(w: &mut World) -> (PrId, circitor::testkit::github::RunId) {
    let repo = w.repo("octo", "app");
    let pr = settled_pr(w, repo);
    w.push_sha(pr, A_SHA);
    w.set_updated_at(pr, w.now().minus(s(120)));
    let build = w.run(pr, "build");
    w.finish(build, "FAILURE");
    (pr, build)
}

/// The cursor an answer carries.
async fn cursor_now(h: &Harness) -> String {
    let out = h.cli(&["changes", PR1, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    env::<ChangesBody>(&out).cursor
}

// TST-8: rerun
// API-10: `--rerun-since <cursor>` doesn't hold on the previous run's result; it holds once a
// `checks` event is recorded after the cursor and the checks are done.
#[tokio::test]
async fn scn_rerun() {
    let (h, (_, build)) = start(Opts::default(), failed_build).await;
    adv(&h, 20).await;
    let r = record(&h, PR1).await;
    assert_eq!(
        (r.checks.state, r.checks.settled),
        (ChecksState::Failing, true)
    );
    assert_eq!(r.status, Status::ChecksFailing);

    // Before the re-run, `checks-done` holds with the previous result: exit 8.
    let out = drive1(
        &h,
        h.cli_bg(&["wait", PR1, "--until", "checks-done", "--timeout", "0"]),
    )
    .await;
    assert_eq!(out.code, 8, "{out:?}");
    let c = cursor_now(&h).await;

    // Re-run at R: queued at R + 5, running at R + 8, passed at R + 45.
    let rerun_at = h.clock().mono();
    h.github().world(|w| {
        w.after(s(5), move |w| w.rerun(build));
        w.after(s(8), move |w| w.start(build));
        w.after(s(45), move |w| w.finish(build, "SUCCESS"));
    });
    adv(&h, 1).await;
    let args = [
        "wait",
        PR1,
        "--until",
        "checks-done",
        "--rerun-since",
        c.as_str(),
    ];
    let plain = h.cli_bg(&args);
    let mut json_args = args.to_vec();
    json_args.push("--json");
    let json = h.cli_bg(&json_args);
    // R + 3: the record still reads `failing` and settled, and the wait isn't done.
    adv(&h, 2).await;
    let r = record(&h, PR1).await;
    assert_eq!(
        (r.checks.state, r.checks.settled),
        (ChecksState::Failing, true)
    );
    assert!(!plain.is_done() && !json.is_done());
    // R + 44: not done.
    adv(&h, 41).await;
    assert!(!plain.is_done() && !json.is_done());
    // It ends after R + 45.
    let outs = drive(&h, vec![plain, json]).await;
    assert!(h.clock().mono() >= rerun_at.plus(s(45)));
    assert_eq!(outs[0].code, 0, "{:?}", outs[0]);
    assert!(
        outs[0]
            .stdout
            .lines()
            .any(|l| l.starts_with("checks passed (1/1)")),
        "{:?}",
        outs[0]
    );
    let b = waited(&outs[1]);
    assert_eq!((b.outcome, b.reason), (Outcome::Satisfied, None));
    let checks_kinds = b
        .events
        .iter()
        .flat_map(|e| e.kinds.iter())
        .filter(|k| k.word() == "checks")
        .count();
    assert_eq!(checks_kinds, 2, "{:?}", b.events);
    assert_eq!(
        b.record.expect("a record").checks.state,
        ChecksState::Passing
    );
    h.finish().await;
}

// TST-8: rerun
// API-10: with no re-run at all the wait times out, and its `again:` line repeats `--rerun-since`.
#[tokio::test]
async fn scn_rerun_none() {
    let (h, _) = start(Opts::default(), failed_build).await;
    adv(&h, 20).await;
    let c = cursor_now(&h).await;
    let out = drive1(
        &h,
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "checks-done",
            "--rerun-since",
            &c,
            "--timeout",
            "20",
        ]),
    )
    .await;
    assert_eq!(out.code, 3, "{out:?}");
    let again = out
        .stdout
        .lines()
        .find(|l| l.starts_with("again:"))
        .unwrap_or_else(|| panic!("no `again:` line: {out:?}"));
    assert!(again.contains(&format!("--rerun-since {c}")), "{again}");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// new-review
// ---------------------------------------------------------------------------------------------

// TST-8: new-review
// POL-12: a count rises; one extra request fetches the items; one `activity` event carries them.
// MOD-12: a thread reply arrives as a review with `inline: 1`.
#[tokio::test]
async fn scn_new_review() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    let id = node_id(&h, p);
    // The first poll, its first refresh, and then quiet until the next poll.
    adv(&h, 10).await;
    let cursor = epoch0(&h).await;
    let r = record(&h, PR1).await;
    assert_eq!((r.counts.comments, r.counts.reviews), (0, 0));

    // V: hubot submits a review with one inline comment.
    let before = hits(&h, &id).len();
    let (review, v) = h.github().world(|w| {
        let at = w.now();
        (w.review_by(p, Some("hubot"), "CHANGES_REQUESTED", 1), at)
    });
    let out = drive1(&h, h.cli_bg(&["show", PR1, "--fresh", "--json"])).await;
    assert_eq!(out.code, 0, "{out:?}");
    // One request names the node for the refresh, and one more asks the items.
    let after: Vec<Hit> = hits(&h, &id)[before..].to_vec();
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(after[0].of, AskedOf::Record, "{after:?}");
    assert_eq!(after[1].of, AskedOf::Items, "{after:?}");
    assert_eq!(after[1].at.since(after[0].at), s(1), "{after:?}");

    let evs: Vec<Event> = events(&h, PR1, &cursor)
        .await
        .into_iter()
        .filter(|e| !words(e).contains(&"tracked"))
        .collect();
    assert_eq!(evs.len(), 1, "{evs:?}");
    let Some(EventKind::Activity {
        counts,
        items,
        more,
    }) = kind(&evs, "activity")
    else {
        panic!("no `activity` kind: {evs:?}");
    };
    assert_eq!((counts.reviews, counts.comments), (1, 0));
    assert!(!more);
    assert_eq!(items.len(), 1, "{items:?}");
    let it = &items[0];
    assert_eq!(it.kind, ItemKind::Review);
    assert_eq!(it.id, review);
    assert_eq!(it.author.as_deref(), Some("hubot"));
    assert_eq!(it.at, v);
    assert_eq!(it.review_state.as_deref(), Some("changes_requested"));
    assert_eq!(it.inline, Some(1));
    assert_eq!(it.fetch, format!("repos/octo/app/pulls/1/reviews/{review}"));
    let fetch_inline = it.fetch_inline.as_deref().expect("fetch_inline");
    assert_eq!(fetch_inline, format!("{}/comments", it.fetch));

    // V + 7: someone replies in the thread: a second review, `COMMENTED`, with one inline comment.
    let now = h.clock().mono();
    let v7 = after[0].at.plus(s(7));
    if v7 > now {
        adv(&h, v7.since(now).as_secs()).await;
    }
    // Outside POL-15's 5 s window of the last refresh.
    let last = hits(&h, &id).last().expect("a hit").at;
    let idle = h.clock().mono().since(last);
    if idle < s(6) {
        adv(&h, 6 - idle.as_secs()).await;
    }
    let before = hits(&h, &id).len();
    let (reply, v2) = h.github().world(|w| {
        let at = w.now();
        (w.thread_reply(p), at)
    });
    let out = drive1(&h, h.cli_bg(&["show", PR1, "--fresh", "--json"])).await;
    assert_eq!(out.code, 0, "{out:?}");
    let after: Vec<Hit> = hits(&h, &id)[before..].to_vec();
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(
        (after[0].of, after[1].of),
        (AskedOf::Record, AskedOf::Items)
    );
    let evs: Vec<Event> = events(&h, PR1, &cursor)
        .await
        .into_iter()
        .filter(|e| !words(e).contains(&"tracked"))
        .collect();
    assert_eq!(evs.len(), 2, "{evs:?}");
    let Some(EventKind::Activity {
        counts,
        items,
        more,
    }) = kind(&evs[1..], "activity")
    else {
        panic!("no `activity` kind in the second event: {evs:?}");
    };
    assert_eq!(counts.reviews, 2);
    assert!(!more);
    assert_eq!(items.len(), 1, "{items:?}");
    let it = &items[0];
    assert_eq!((it.kind, it.id), (ItemKind::Review, reply));
    assert_ne!(reply, review);
    assert_eq!(it.at, v2);
    assert_eq!(it.review_state.as_deref(), Some("commented"));
    assert_eq!(it.inline, Some(1));
    assert_eq!(it.fetch, format!("repos/octo/app/pulls/1/reviews/{reply}"));
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// changed-kind
// ---------------------------------------------------------------------------------------------

/// A settled PR with one check running; at `K` (5 s in) it passes, and at `K + 40` `then` happens.
fn check_then(
    then: impl FnOnce(&mut World, PrId) + Send + 'static,
) -> impl FnOnce(&mut World) -> PrId + Send + 'static {
    move |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        let slow = w.run(pr, "slow");
        w.start(slow);
        w.after(s(5), move |w| w.finish(slow, "SUCCESS"));
        w.after(s(45), move |w| then(w, pr));
        pr
    }
}

// TST-8: changed-kind
// API-10: `--kind activity` isn't satisfied by a check; a plain `changed` is.
// API-11: the review satisfies the `--kind activity` wait.
#[tokio::test]
async fn scn_changed_kind_review() {
    let (h, _) = start(
        Opts::default(),
        check_then(|w, pr| {
            w.review_by(pr, Some("hubot"), "COMMENTED", 0);
        }),
    )
    .await;
    // The waits begin before K; the first poll is at 0.
    let cursor = epoch0(&h).await;
    let kind_wait = h.cli_bg(&[
        "wait",
        PR1,
        "--until",
        "changed",
        "--kind",
        "activity",
        "--timeout",
        "120",
    ]);
    let kind_json = h.cli_bg(&[
        "wait",
        PR1,
        "--until",
        "changed",
        "--kind",
        "activity",
        "--timeout",
        "120",
        "--json",
    ]);
    let any_wait = h.cli_bg(&["wait", PR1, "--until", "changed", "--timeout", "120"]);
    // K is 5 s in. 20 s after it, a `checks` and a `status` event were recorded.
    adv(&h, 25).await;
    let evs = events(&h, PR1, &cursor).await;
    assert!(
        has_kind(&evs, "checks") && has_kind(&evs, "status"),
        "{evs:?}"
    );
    assert!(!kind_wait.is_done() && !kind_json.is_done());
    // The plain `changed` ends at the check's refresh.
    assert!(any_wait.is_done());
    let out = any_wait.await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(has_line(&out, "changed"), "{out:?}");

    // The `--kind activity` waits end after K + 40.
    let outs = drive(&h, vec![kind_wait, kind_json]).await;
    assert_eq!(outs[0].code, 0, "{:?}", outs[0]);
    assert!(has_line(&outs[0], "changed"), "{:?}", outs[0]);
    let b = waited(&outs[1]);
    assert_eq!((b.outcome, b.reason), (Outcome::Satisfied, None));
    let Some(EventKind::Activity { items, .. }) = kind(&b.events, "activity") else {
        panic!("no `activity` kind: {:?}", b.events);
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, ItemKind::Review);
    assert_eq!(items[0].author.as_deref(), Some("hubot"));
    h.finish().await;
}

// TST-8: changed-kind
// API-11: a merge ends `--kind activity` with `merged`, exit 4; `--kind activity,merged` is satisfied.
#[tokio::test]
async fn scn_changed_kind_merge() {
    let (h, _) = start(Opts::default(), check_then(|w, pr| w.merge(pr))).await;
    let waits = vec![
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "changed",
            "--kind",
            "activity",
            "--timeout",
            "120",
        ]),
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "changed",
            "--kind",
            "activity",
            "--timeout",
            "120",
            "--json",
        ]),
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "changed",
            "--kind",
            "activity,merged",
            "--timeout",
            "120",
        ]),
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "changed",
            "--kind",
            "activity,merged",
            "--timeout",
            "120",
            "--json",
        ]),
    ];
    adv(&h, 25).await;
    assert!(
        waits.iter().all(|b| !b.is_done()),
        "a check satisfied a kind wait"
    );
    let outs = drive(&h, waits).await;
    assert_eq!(outs[0].code, 4, "{:?}", outs[0]);
    assert!(has_line(&outs[0], "ended: merged"), "{:?}", outs[0]);
    let b = waited(&outs[1]);
    assert_eq!(
        (b.outcome, b.reason),
        (Outcome::Ended, Some(Reason::Merged))
    );
    assert_eq!(outs[2].code, 0, "{:?}", outs[2]);
    assert!(has_line(&outs[2], "changed"), "{:?}", outs[2]);
    let b = waited(&outs[3]);
    assert_eq!((b.outcome, b.reason), (Outcome::Satisfied, None));
    h.finish().await;
}

// TST-8: changed-kind
// API-11: a close ends `--kind activity` with `closed`.
#[tokio::test]
async fn scn_changed_kind_close() {
    let (h, _) = start(Opts::default(), check_then(|w, pr| w.close(pr))).await;
    let waits = vec![
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "changed",
            "--kind",
            "activity",
            "--timeout",
            "120",
        ]),
        h.cli_bg(&[
            "wait",
            PR1,
            "--until",
            "changed",
            "--kind",
            "activity",
            "--timeout",
            "120",
            "--json",
        ]),
    ];
    adv(&h, 25).await;
    assert!(waits.iter().all(|b| !b.is_done()));
    let outs = drive(&h, waits).await;
    assert_eq!(outs[0].code, 4, "{:?}", outs[0]);
    assert!(has_line(&outs[0], "ended: closed"), "{:?}", outs[0]);
    let b = waited(&outs[1]);
    assert_eq!(
        (b.outcome, b.reason),
        (Outcome::Ended, Some(Reason::Closed))
    );
    h.finish().await;
}

// TST-8: changed-kind
// API-10, API-1: an unknown kind is `bad_request`; `--kind` with `--until merged` is the CLI's own
// usage error and sends nothing.
#[tokio::test]
async fn scn_changed_kind_bad() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    adv(&h, 20).await;
    for bad in ["foo", "tracked"] {
        let out = h
            .cli(&["wait", PR1, "--until", "changed", "--kind", bad, "--json"])
            .await;
        assert_eq!(out.code, 2, "{out:?}");
        let body: circitor::api::ErrorBody = serde_json::from_str(&out.stdout)
            .or_else(|_| serde_json::from_str(&out.stderr))
            .unwrap_or_else(|e| panic!("{e}: {out:?}"));
        assert_eq!(body.error.code, circitor::api::ErrorCode::BadRequest);
        assert!(body.error.message.contains("kind"), "{:?}", body.error);
    }
    // Nothing reaches the daemon, so no arrival refresh is forced.
    let sent = h.github().requests().len();
    let out = h
        .cli(&["wait", PR1, "--until", "merged", "--kind", "activity"])
        .await;
    assert_eq!(out.code, 2, "{out:?}");
    adv(&h, 1).await;
    assert_eq!(h.github().requests().len(), sent);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// baseline
// ---------------------------------------------------------------------------------------------

// TST-8: baseline
// STA-5: a PR first tracked with comments and reviews makes no `activity` event; what arrives
// afterwards does, with one item.
#[tokio::test]
async fn scn_baseline() {
    let (h, (p1, p2)) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let mine = w.open_pr(repo);
        for _ in 0..5 {
            w.comment(mine);
        }
        for _ in 0..2 {
            w.review(mine, "COMMENTED", 0);
        }
        w.set_updated_at(mine, w.now().minus(s(120)));
        // Someone else's, taken in with `track`.
        let other_repo = w.repo("octo", "other");
        let other = w.open_pr(other_repo);
        w.set_author(other, Some("someone-else"));
        for _ in 0..5 {
            w.comment(other);
        }
        w.set_updated_at(other, w.now().minus(s(120)));
        (mine, other)
    })
    .await;
    adv(&h, 10).await;
    let cursor = epoch0(&h).await;
    let out = drive1(&h, h.cli_bg(&["track", "octo/other#1"])).await;
    assert_eq!(out.code, 0, "{out:?}");

    for (name, source, comments, reviews) in [
        (PR1, circitor::model::Source::Authored, 5, 2),
        ("octo/other#1", circitor::model::Source::Manual, 5, 0),
    ] {
        let evs = events(&h, name, &cursor).await;
        assert_eq!(evs.len(), 1, "{name}: {evs:?}");
        assert_eq!(
            evs[0].kinds.iter().map(EventKind::word).collect::<Vec<_>>(),
            ["tracked"],
            "{name}: {evs:?}"
        );
        assert!(
            matches!(kind(&evs, "tracked"), Some(EventKind::Tracked { source: s }) if *s == source),
            "{name}: {evs:?}"
        );
        let r = record(&h, name).await;
        assert_eq!(
            (r.counts.comments, r.counts.reviews),
            (comments, reviews),
            "{name}"
        );
    }

    // A sixth comment on each: one `activity` event with one item.
    adv(&h, 5).await;
    let (c1, c2, at) = h.github().world(|w| {
        let at = w.now();
        (
            w.comment_by(p1, Some("hubot")),
            w.comment_by(p2, Some("hubot")),
            at,
        )
    });
    // A tracked PR of someone else's is refreshed with the polls it falls due in.
    adv(&h, 75).await;
    for (name, c, repo) in [(PR1, c1, "octo/app"), ("octo/other#1", c2, "octo/other")] {
        let evs: Vec<Event> = events(&h, name, &cursor)
            .await
            .into_iter()
            .filter(|e| !words(e).contains(&"tracked"))
            .collect();
        assert_eq!(evs.len(), 1, "{name}: {evs:?}");
        let Some(EventKind::Activity {
            counts,
            items,
            more,
        }) = kind(&evs, "activity")
        else {
            panic!("{name}: no `activity` kind: {evs:?}");
        };
        assert_eq!(counts.comments, 6, "{name}");
        assert!(!more);
        assert_eq!(items.len(), 1, "{name}: {items:?}");
        assert_eq!(
            (items[0].kind, items[0].id),
            (ItemKind::Comment, c),
            "{name}"
        );
        assert_eq!(
            items[0].fetch,
            format!("repos/{repo}/issues/comments/{c}"),
            "{name}"
        );
    }
    // The file remembers the newest comment's time.
    let e = file_entry(&h, PR1).expect("the PR is in the file");
    assert_eq!(e["seen"]["comment"]["at"], at.rfc3339(), "{e}");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// gone-returns
// ---------------------------------------------------------------------------------------------

// TST-8: gone-returns
// MOD-11, STA-2: a gone PR returned open gets `reopened` and its retention clock cleared; returned
// merged, `merged` and the clock started again, at the refresh that showed it.
#[tokio::test]
async fn scn_gone_returns() {
    let (h, (p1, p2, _p3)) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        (
            settled_pr(w, repo),
            settled_pr(w, repo),
            settled_pr(w, repo),
        )
    })
    .await;
    let (n1, n2) = ("octo/app#1", "octo/app#2");
    let id2 = node_id(&h, p2);
    adv(&h, 5).await;
    let cursor = epoch0(&h).await;
    assert_eq!(listed(&h, false).await.len(), 3);

    // #1 and #2 are hidden: off the open list, `NOT_FOUND` by node, until both read `gone`.
    h.github().world(|w| {
        w.hide(p1);
        w.hide(p2);
    });
    let hidden_at = h.clock().mono();
    let mut gone = false;
    for _ in 0..60 {
        adv(&h, 10).await;
        let rs = listed(&h, true).await;
        let state_of = |n: &str| rs.iter().find(|r| r.pr.to_string() == n).map(|r| r.state);
        if state_of(n1) == Some(PrState::Gone) && state_of(n2) == Some(PrState::Gone) {
            gone = true;
            break;
        }
    }
    assert!(gone, "both PRs never read gone");
    // POL-21: not before 5 minutes of refreshes that each read `NOT_FOUND`.
    assert!(h.clock().mono().since(hidden_at) >= GONE_AFTER);
    for n in [n1, n2] {
        let e = file_entry(&h, n).unwrap_or_else(|| panic!("{n} isn't in the file"));
        assert_eq!(e["ended"]["state"], "gone", "{e}");
    }

    // #1 comes back open, #2 merged.
    let before = hits(&h, &id2).len();
    h.github().world(|w| {
        w.merge(p2);
        w.unhide(p1);
        w.unhide(p2);
    });
    let mut open = false;
    for _ in 0..40 {
        adv(&h, 1).await;
        if record(&h, n1).await.state == PrState::Open {
            open = true;
            break;
        }
    }
    assert!(open, "#1 wasn't open again within 40 s");
    let r = record(&h, n1).await;
    assert!(!r.checks.settled);
    assert_eq!(r.status, Status::ChecksPending);
    let evs = events(&h, n1, &cursor).await;
    assert!(has_kind(&evs, "reopened"), "{evs:?}");
    // The saver writes it again, `ended: null`.
    adv(&h, 15).await;
    let e = file_entry(&h, n1).expect("#1 is in the file");
    assert!(e["ended"].is_null(), "{e}");

    // #2: an ended PR's period is `idle_interval`.
    let mut merged = false;
    for _ in 0..70 {
        adv(&h, 5).await;
        listed(&h, false).await;
        if record(&h, n2).await.state == PrState::Merged {
            merged = true;
            break;
        }
    }
    assert!(merged, "#2 never read merged");
    let evs = events(&h, n2, &cursor).await;
    assert!(has_kind(&evs, "merged"), "{evs:?}");
    // Its retention clock starts at the refresh that showed it merged.
    let shown = *hits(&h, &id2)
        .get(before)
        .expect("a request named #2 after its return");
    adv(&h, 15).await;
    let e = file_entry(&h, n2).expect("#2 is in the file");
    assert_eq!(e["ended"]["state"], "merged", "{e}");
    assert_eq!(e["ended"]["seen_at"], shown.wall.rfc3339(), "{e}");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// mergeable-unknown
// ---------------------------------------------------------------------------------------------

/// The events of a PR since `cursor` that aren't `tracked`.
async fn changes_since(h: &Harness, pr: &str, cursor: &str) -> Vec<Event> {
    events(h, pr, cursor)
        .await
        .into_iter()
        .filter(|e| !words(e).contains(&"tracked"))
        .collect()
}

async fn list_env(h: &Harness) -> Envelope<ListBody> {
    let out = h.cli(&["list", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    env(&out)
}

/// No request names `id` between 2 s and 4 s after `at`: no POL-11 follow-up.
fn no_follow_up(h: &Harness, id: &str, at: Mono) {
    let late: Vec<Hit> = hits(h, id)
        .into_iter()
        .filter(|x| x.at >= at.plus(s(2)) && x.at <= at.plus(s(4)))
        .collect();
    assert!(late.is_empty(), "a follow-up after {at:?}: {late:?}");
}

// TST-8: mergeable-unknown (kept)
// MOD-8: `UNKNOWN` on an unchanged head within 60 s of the last `yes` makes no event and the
// record keeps `yes`; the first one after that reads `unknown`, with one `mergeable` event.
// POL-11: nothing follows either read by 3 s (a `list` isn't a request naming the PR).
#[tokio::test]
async fn scn_mergeable_unknown_kept() {
    // A 25 s interval keeps the polls' reads clear of the 60 s boundary (at 1, 26, 51, 76 s).
    let (h, p) = start(Opts::default().config(|c| c.interval = s(25)), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    let id = node_id(&h, p);
    adv(&h, 2).await;
    let r0 = hits(&h, &id).first().expect("the first refresh").at;
    let cursor = epoch0(&h).await;
    let first = list_env(&h).await;
    assert_eq!(first.body.records[0].mergeable, Mergeable::Yes);
    // From now on GitHub answers `UNKNOWN` on the same head.
    h.github().world(|w| w.unknown_for(p, 100_000));

    let (mut seen, mut under, mut over) = (hits(&h, &id).len(), 0, false);
    for _ in 0..120 {
        adv(&h, 1).await;
        let now = hits(&h, &id);
        if now.len() == seen {
            continue;
        }
        let sent = now.last().expect("a hit").at.since(r0);
        seen = now.len();
        let e = list_env(&h).await;
        let r = &e.body.records[0];
        if sent < KEEP_MERGEABLE - s(5) {
            // A refresh sent under 60 s after the last `yes`: kept, no event, the cursor unchanged.
            assert_eq!(r.mergeable, Mergeable::Yes, "{sent:?} after r0");
            assert_eq!(e.cursor, first.cursor, "{sent:?} after r0");
            under += 1;
        } else if sent > KEEP_MERGEABLE + s(5) {
            // The first one over 60 s after it.
            assert_eq!(r.mergeable, Mergeable::Unknown, "{sent:?} after r0");
            over = true;
            break;
        }
    }
    assert!(under >= 1 && over, "{under} reads under 60 s, over: {over}");
    let evs = changes_since(&h, PR1, &cursor).await;
    assert_eq!(evs.len(), 1, "{evs:?}");
    assert!(
        matches!(kind(&evs, "mergeable"), Some(EventKind::Mergeable { mergeable, .. }) if *mergeable == Mergeable::Unknown),
        "{evs:?}"
    );
    adv(&h, 10).await;
    for x in hits(&h, &id) {
        no_follow_up(&h, &id, x.at);
    }
    h.finish().await;
}

// TST-8: mergeable-unknown (after a sleep)
// MOD-8: the first read on waking from a sleep over 60 s reads `unknown`. MOD-8 has no sleep
// signal: it's the 60 s of wall time since the last `yes` (a shorter sleep keeps `yes`).
#[tokio::test]
async fn scn_mergeable_unknown_sleep() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    adv(&h, 2).await;
    let cursor = epoch0(&h).await;
    assert_eq!(record(&h, PR1).await.mergeable, Mergeable::Yes);
    h.github().world(|w| w.unknown_for(p, 100_000));
    let id = node_id(&h, p);
    let before = hits(&h, &id).len();
    h.clock().sleep_wall_only(s(120)).await;
    // The first read on waking: the PR falls due by the monotonic clock, which didn't move
    // (POL-13, POL-26), so a tick notices the sleep and the next poll refreshes it.
    let mut read = false;
    for _ in 0..60 {
        adv(&h, 1).await;
        if hits(&h, &id).len() > before {
            read = true;
            break;
        }
    }
    assert!(read, "no read after waking");
    let r = record(&h, PR1).await;
    assert_eq!(r.mergeable, Mergeable::Unknown);
    let evs = changes_since(&h, PR1, &cursor).await;
    assert_eq!(evs.len(), 1, "{evs:?}");
    assert!(has_kind(&evs, "mergeable"), "{evs:?}");
    h.finish().await;
}

// TST-8: mergeable-unknown (after a push)
// MOD-8: after a push the record reads `unknown`, then the new value, each a visible change.
// POL-11: the refresh that read `UNKNOWN` is followed by one request 3 s after its answer.
#[tokio::test]
async fn scn_mergeable_unknown_push() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    let id = node_id(&h, p);
    adv(&h, 20).await;
    let cursor = epoch0(&h).await;
    // A waiter holds the PR hot.
    let wait = h.cli_bg(&["wait", PR1, "--until", "merged", "--timeout", "3600"]);
    h.clock().advance(ms(100)).await;
    let n = hits(&h, &id).len();
    let mut next = None;
    for _ in 0..300 {
        h.clock().advance(ms(100)).await;
        if hits(&h, &id).len() > n {
            next = Some(hits(&h, &id)[n].at);
            break;
        }
    }
    let seen = next.expect("a hot refresh");
    // Pushed at P, so that the next hot refresh (10 s after this one) falls at P + 4 s: GitHub
    // serves the old head until P + 3 s, then the new head with `UNKNOWN`, and `MERGEABLE` from P + 6 s.
    let pushed = seen.plus(HOT_INTERVAL - s(4));
    let wait_for = pushed.since(h.clock().mono());
    h.github().world(|w| {
        w.after(wait_for, move |w| {
            w.push_lagged(p, None, FOLLOW_UP);
        });
        w.after(wait_for + FOLLOW_UP, move |w| w.set_mergeable(p, "UNKNOWN"));
        w.after(wait_for + FOLLOW_UP * 2, move |w| {
            w.set_mergeable(p, "MERGEABLE")
        });
    });
    adv(&h, 30).await;
    let evs: Vec<Event> = changes_since(&h, PR1, &cursor)
        .await
        .into_iter()
        .filter(|e| has_kind(std::slice::from_ref(e), "mergeable"))
        .collect();
    assert_eq!(evs.len(), 2, "{evs:?}");
    assert!(words(&evs[0]).contains(&"head"), "{evs:?}");
    assert!(
        matches!(kind(&evs[..1], "mergeable"), Some(EventKind::Mergeable { mergeable, .. }) if *mergeable == Mergeable::Unknown),
        "{evs:?}"
    );
    assert!(
        matches!(kind(&evs[1..], "mergeable"), Some(EventKind::Mergeable { mergeable, .. }) if *mergeable == Mergeable::Yes),
        "{evs:?}"
    );
    // The refresh that read `UNKNOWN` (sent in [P + 3 s, P + 6 s)) is followed by one request 3 s later.
    let all = hits(&h, &id);
    let i = all
        .iter()
        .position(|x| x.at >= pushed.plus(FOLLOW_UP) && x.at < pushed.plus(FOLLOW_UP * 2))
        .expect("a refresh read `UNKNOWN`");
    let gap = all[i + 1].at.since(all[i].at);
    assert_eq!(gap, FOLLOW_UP, "{:?}", &all[i..]);
    wait.cancel().await;
    h.finish().await;
}

// TST-8: mergeable-unknown (twenty reads)
// POL-11: each refresh that reads `UNKNOWN` gets at most one follow-up, and a follow-up never another.
#[tokio::test]
async fn scn_mergeable_unknown_twenty() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    let id = node_id(&h, p);
    adv(&h, 20).await;
    h.github().world(|w| w.unknown_for(p, 20));
    let wait = h.cli_bg(&["wait", PR1, "--until", "merged", "--timeout", "3600"]);
    adv(&h, 300).await;
    let all = hits(&h, &id);
    let gaps: Vec<Duration> = all.windows(2).map(|w| w[1].at.since(w[0].at)).collect();
    assert!(gaps.len() >= 20, "{gaps:?}");
    let mut previous_was_follow_up = false;
    for g in &gaps {
        let follow_up = *g == FOLLOW_UP;
        assert!(follow_up || *g >= HOT_INTERVAL, "a gap of {g:?}: {gaps:?}");
        assert!(
            !(follow_up && previous_was_follow_up),
            "two 3 s gaps in a row: {gaps:?}"
        );
        previous_was_follow_up = follow_up;
    }
    assert!(gaps.contains(&FOLLOW_UP), "{gaps:?}");
    wait.cancel().await;
    h.finish().await;
}

// TST-8: mergeable-unknown (at start)
// POL-11: every PR reading `UNKNOWN` at start shares one follow-up request.
#[tokio::test]
async fn scn_mergeable_unknown_start() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        for _ in 0..5 {
            let pr = settled_pr(w, repo);
            w.unknown_for(pr, 1);
        }
    })
    .await;
    // The first poll, before the next one is due.
    adv(&h, 20).await;
    let reqs = h.github().requests();
    assert_eq!(reqs.len(), 3, "{reqs:?}");
    assert!(reqs[0].finding.is_some(), "{:?}", reqs[0]);
    assert_eq!(reqs[1].nodes.len(), 5, "{:?}", reqs[1]);
    assert_eq!(reqs[2].nodes.len(), 5, "{:?}", reqs[2]);
    assert!(reqs[2].finding.is_none());
    let mut a = reqs[1].node_ids();
    let mut b = reqs[2].node_ids();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b);
    assert_eq!(reqs[2].mono.since(reqs[1].mono), FOLLOW_UP);
    h.finish().await;
}

/// Whether `status --json` raises the warning `code`.
async fn warned(h: &Harness, code: &str) -> bool {
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    env::<StatusBody>(&out)
        .body
        .warnings
        .iter()
        .any(|w| serde_json::to_value(w.code).expect("a code") == code)
}

// TST-8: mergeable-unknown (reserve)
// POL-23: in reserve, POL-11's follow-up isn't sent.
#[tokio::test]
async fn scn_mergeable_unknown_reserve() {
    let reset = Time::parse("2026-11-24T23:00:00Z").expect("a time");
    let (h, p) = start(
        Opts::default().github(move |gh| gh.set_rate(400, reset)),
        |w| {
            let repo = w.repo("octo", "app");
            settled_pr(w, repo)
        },
    )
    .await;
    let id = node_id(&h, p);
    // Past the last `yes` by 60 s and POL-15's 30 s window.
    adv(&h, 70).await;
    assert!(warned(&h, "quota_reserve").await);
    h.github().world(|w| w.unknown_for(p, 1000));
    let before = hits(&h, &id).len();
    let wait = h.cli_bg(&["wait", PR1, "--until", "merged", "--timeout", "3600"]);
    adv(&h, 100).await;
    // The arrival refresh read `UNKNOWN`, and nothing followed it.
    let after = hits(&h, &id);
    assert_eq!(after.len(), before + 1, "{:?}", &after[before..]);
    assert_eq!(record(&h, PR1).await.mergeable, Mergeable::Unknown);
    wait.cancel().await;
    h.finish().await;
}

// TST-8: mergeable-unknown (capped)
// POL-23: capped, a forced refresh is served and POL-11's follow-up isn't sent.
#[tokio::test]
async fn scn_mergeable_unknown_capped() {
    let (h, p) = start(Opts::default().quota(60, 500), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo)
    })
    .await;
    let id = node_id(&h, p);
    // Poll until circitor's own count reaches the budget of 60 points.
    let mut capped = false;
    for _ in 0..400 {
        adv(&h, 30).await;
        listed(&h, false).await;
        if warned(&h, "quota_capped").await {
            capped = true;
            break;
        }
    }
    assert!(capped, "never capped");
    adv(&h, 60).await;
    h.github().world(|w| w.unknown_for(p, 1000));
    let before = hits(&h, &id).len();
    let wait = h.cli_bg(&["wait", PR1, "--until", "merged", "--timeout", "3600"]);
    adv(&h, 100).await;
    let after = hits(&h, &id);
    assert_eq!(after.len(), before + 1, "{:?}", &after[before..]);
    assert_eq!(record(&h, PR1).await.mergeable, Mergeable::Unknown);
    wait.cancel().await;
    h.finish().await;
}
