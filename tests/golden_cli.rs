#![cfg(feature = "test-endpoint")]
//! T35: golden output end to end (TST-7). Every command's text, every outcome line, every error
//! line, every `--json` body and every notice, produced by the real CLI against the real daemon on
//! the harness, equals a file under `tests/golden/cli/`; every exit code of CLI-5 but 130 and 143
//! has a test. The files are written only with `CIRCITOR_UPDATE_GOLDEN=1` and read against CLI-3
//! and CLI-6 afterwards.
//!
//! The world: `octo/app#N`, authored by the fake's login, "settled" (`updatedAt` two minutes old at
//! the first read, MOD-6's shortcut) unless a case says otherwise. The clock starts at
//! 2026-11-24T22:00:00Z and every advance is an exact amount.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::api::{Envelope, ListBody, StatusBody};
use circitor::cli::{Output, exit};
use circitor::clock::Clock;
use circitor::testkit::github::{PrId, RepoId, World};
use circitor::testkit::golden;
use circitor::testkit::harness::{Bg, Harness, Opts};
use circitor::testkit::{MARKER_SESSION_TOKEN, TEST_VERSION};

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

// ---------------------------------------------------------------------------------------------
// The helpers
// ---------------------------------------------------------------------------------------------

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

/// Moves the clock in 100 ms steps until every command is done (at most 400 s), then awaits them.
async fn drive(h: &Harness, bgs: Vec<Bg>) -> Vec<Output> {
    for _ in 0..4000 {
        if bgs.iter().all(Bg::is_done) {
            break;
        }
        h.clock().advance(ms(100)).await;
    }
    assert!(
        bgs.iter().all(Bg::is_done),
        "a command didn't end within 400 s"
    );
    let mut outs = Vec::new();
    for b in bgs {
        outs.push(b.await);
    }
    outs
}

async fn drive1(h: &Harness, bg: Bg) -> Output {
    drive(h, vec![bg]).await.remove(0)
}

/// A command that may wait on the clock, to its end.
async fn run(h: &Harness, args: &[&str]) -> Output {
    drive1(h, h.cli_bg(args)).await
}

/// TST-7's two replacements, and no other: the temporary root by `<tmp>`, an OS error's text by
/// `<os error>` (`No such file or directory (os error 2)`).
fn scrub(h: &Harness, text: &str) -> String {
    let text = text.replace(&h.dir().display().to_string(), "<tmp>");
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        out.push_str(&scrub_os_error(line));
    }
    out
}

fn scrub_os_error(line: &str) -> String {
    let Some(end) = line.find(" (os error ").map(|i| {
        let close = line[i..].find(')').map_or(line.len(), |c| i + c + 1);
        (i, close)
    }) else {
        return line.to_string();
    };
    // The text of the error is the words before ` (os error N)`, from the last `: ` on.
    let head = &line[..end.0];
    let start = head.rfind(": ").map_or(0, |i| i + 2);
    format!("{}<os error>{}", &head[..start], &line[end.1..])
}

fn golden_path(case: &str, ext: &str) -> String {
    format!("tests/golden/cli/{case}.{ext}")
}

/// Compares the command's stdout with `<case>.out` and, when it isn't empty, its stderr with
/// `<case>.err` (TST-7). The exit code is asserted by the test.
fn gold(h: &Harness, case: &str, out: &Output) {
    golden::check(&golden_path(case, "out"), &scrub(h, &out.stdout));
    if out.stderr.is_empty() {
        let err = Path::new(env!("CARGO_MANIFEST_DIR")).join(golden_path(case, "err"));
        assert!(
            !err.exists(),
            "{case}: stderr is empty but {} exists",
            err.display()
        );
    } else {
        golden::check(&golden_path(case, "err"), &scrub(h, &out.stderr));
    }
}

/// The same for a `--json` command: stdout is one line, equal to `<case>.json`.
fn gold_json(h: &Harness, case: &str, out: &Output) {
    assert!(out.stdout.ends_with('\n'), "{case}: {out:?}");
    assert_eq!(out.stdout.matches('\n').count(), 1, "{case}: {out:?}");
    golden::check(&golden_path(case, "json"), &scrub(h, &out.stdout));
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

fn lines(out: &Output) -> Vec<&str> {
    out.stdout.lines().collect()
}

/// A repository's cursor, through `status --json`.
async fn cursor(h: &Harness) -> String {
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    serde_json::from_str::<Envelope<StatusBody>>(&out.stdout)
        .expect("status decodes")
        .cursor
}

async fn listed(h: &Harness) -> Envelope<ListBody> {
    let out = h.cli(&["list", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    serde_json::from_str(&out.stdout).expect("list decodes")
}

// ---------------------------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------------------------

// TST-7: CLI-3's `list`: several PRs in API-6's order with different `status` words; `list --all`.
#[tokio::test]
async fn golden_list() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        // `ready`: its one check passed, approved.
        let a = settled_pr(w, repo);
        w.set_updated_at(a, w.now().minus(s(60)));
        let c = w.run(a, "build");
        w.finish(c, "SUCCESS");
        w.set_review_decision(a, Some("APPROVED"));
        // `checks_failing`.
        let b = settled_pr(w, repo);
        w.set_updated_at(b, w.now().minus(s(300)));
        let c = w.run(b, "build");
        w.finish(c, "FAILURE");
        // `checks_pending`.
        let d = settled_pr(w, repo);
        w.set_updated_at(d, w.now().minus(s(400)));
        let c = w.run(d, "build");
        w.start(c);
        // `draft`.
        let e = settled_pr(w, repo);
        w.set_draft(e, true);
        w.set_updated_at(e, w.now().minus(s(500)));
        // `conflict`.
        let f = settled_pr(w, repo);
        w.set_updated_at(f, w.now().minus(s(600)));
        w.set_mergeable(f, "CONFLICTING");
        w.set_merge_state(f, Some("DIRTY"));
        // `changes_requested`.
        let g = settled_pr(w, repo);
        w.set_updated_at(g, w.now().minus(s(700)));
        w.set_review_decision(g, Some("CHANGES_REQUESTED"));
        // `review_required`.
        let i = settled_pr(w, repo);
        w.set_updated_at(i, w.now().minus(s(800)));
        w.set_review_decision(i, Some("REVIEW_REQUIRED"));
        // Merged after the first poll: only `--all` shows it.
        settled_pr(w, repo)
    })
    .await;
    adv(&h, 30).await;
    // A request makes the tier active (POL-13); the merge is then seen within a poll.
    listed(&h).await;
    h.github().world(|w| w.merge(p));
    adv(&h, 40).await;
    let open = h.cli(&["list"]).await;
    assert_eq!(open.code, exit::OK, "{open:?}");
    gold(&h, "list", &open);
    let all = h.cli(&["list", "--all"]).await;
    assert_eq!(all.code, exit::OK, "{all:?}");
    gold(&h, "list_all", &all);
    h.finish().await;
}

// TST-7: CLI-3's `list --since`: a PR dropped since the cursor and not tracked now is an
// `untracked` line; a PR found and not yet read is a `pending` line (API-18).
#[tokio::test]
async fn golden_list_since() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let a = settled_pr(w, repo);
        w.set_updated_at(a, w.now().minus(s(60)));
        let x = w.repo("octo", "lib");
        let b = settled_pr(w, x);
        w.set_author(b, Some("someone-else"));
        b
    })
    .await;
    adv(&h, 30).await;
    let tracked = h.cli(&["track", "octo/lib#1"]).await;
    assert_eq!(tracked.code, exit::OK, "{tracked:?}");
    let c = cursor(&h).await;
    let out = h.cli(&["untrack", "octo/lib#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    // A PR the next poll finds, whose first refresh is in flight: `pending`.
    h.github().world(|w| {
        let repo = w.repo("octo", "app");
        let n = settled_pr(w, repo);
        w.set_updated_at(n, w.now().minus(s(30)));
    });
    let created = h.clock().mono();
    let found_by = |h: &Harness, since: circitor::clock::Mono| {
        h.github()
            .requests()
            .last()
            .is_some_and(|r| r.finding.is_some() && r.mono >= since)
    };
    for _ in 0..40 {
        if found_by(&h, created) {
            break;
        }
        adv(&h, 1).await;
    }
    let hold = h.github().hold_next();
    adv(&h, 1).await;
    assert!(hold.is_holding());
    let out = h.cli(&["list", "--since", &c]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "list_since", &out);
    hold.release();
    adv(&h, 2).await;
    let _ = p;
    h.finish().await;
}

// TST-7: CLI-3's `list` without `--since` prints at most 50 PRs and says how many more there are.
#[tokio::test]
async fn golden_list_more() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        w.open_prs(repo, 51, s(60));
    })
    .await;
    // The first poll reads 51 PRs; a `list` before it ends waits on the clock.
    let out = run(&h, &["list"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert_eq!(
        lines(&out).len(),
        50 + 2,
        "50 PRs, the count and the cursor"
    );
    gold(&h, "list_more", &out);
    h.finish().await;
}

// TST-7: CLI-3's stale mark with its error, and `problem:` before the cursor, on `list` and `show`.
#[tokio::test]
async fn golden_stale_record_and_problem() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let a = settled_pr(w, repo);
        let c = w.run(a, "build");
        w.finish(c, "SUCCESS");
        let b = settled_pr(w, repo);
        w.set_updated_at(b, w.now().minus(s(300)));
        (a, b)
    })
    .await;
    adv(&h, 20).await;
    // `octo/app#2`'s own refresh fails from now on, and the finding still works.
    h.github().pr_error(
        p.1,
        "FORBIDDEN",
        circitor::testkit::server::Aim::All,
        circitor::testkit::server::Times::UntilCleared,
    );
    listed(&h).await;
    adv(&h, 250).await;
    let out = h.cli(&["list"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "list_stale", &out);
    let out = h.cli(&["show", "octo/app#2"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "show_stale", &out);
    h.github().clear_faults();
    h.finish().await;
}

// TST-7: CLI-3's `problem:` line, relative time first and the local time after.
#[tokio::test]
async fn golden_list_problem() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo);
    })
    .await;
    adv(&h, 20).await;
    listed(&h).await;
    let reset = h.clock().wall().plus(s(17 * 60 - 20));
    h.github().set_rate(0, reset);
    h.github().fault(
        circitor::testkit::server::Fault::Exhausted {
            status: circitor::testkit::server::LimitStatus::Forbidden,
            reset: true,
        },
        circitor::testkit::server::Times::UntilCleared,
    );
    adv(&h, 40).await;
    // Exactly 22:05:00.
    let to_go = Duration::from_secs(5 * 60).saturating_sub(h.clock().mono().as_duration());
    h.clock().advance(to_go).await;
    let out = h.cli(&["list"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "list_problem", &out);
    h.github().clear_faults();
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------------------------

/// A PR with a passed check, a failed one and a running one.
fn checks_world(w: &mut World) -> PrId {
    let repo = w.repo("octo", "app");
    let pr = settled_pr(w, repo);
    let a = w.run(pr, "build");
    w.finish(a, "SUCCESS");
    let b = w.run(pr, "lint");
    w.finish(b, "SUCCESS");
    let c = w.run(pr, "test");
    w.finish(c, "FAILURE");
    let d = w.run(pr, "deploy");
    w.start(d);
    pr
}

// TST-7: CLI-3's `show`: a failed and a running check with their URLs, passed ones as a number.
#[tokio::test]
async fn golden_show() {
    let (h, _) = start(Opts::default(), checks_world).await;
    adv(&h, 20).await;
    let out = h.cli(&["show", "octo/app#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "show", &out);
    h.finish().await;
}

// TST-7: CLI-3's `show` on `mergeable unknown`, with its hint.
#[tokio::test]
async fn golden_show_mergeable_unknown() {
    let (h, _) = start(Opts::default(), |w| {
        let pr = checks_world(w);
        w.unknown_for(pr, 1000);
    })
    .await;
    adv(&h, 20).await;
    let out = h.cli(&["show", "octo/app#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert!(
        out.stdout.contains(
            "mergeable unknown: GitHub is computing it; circitor wait octo/app#1 --until changed"
        ),
        "{out:?}"
    );
    gold(&h, "show_unknown", &out);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// status, refresh, stop, track, untrack
// ---------------------------------------------------------------------------------------------

// TST-7: CLI-3's `status`: healthy; with the CLI's version differing.
#[tokio::test]
async fn golden_status() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo);
    })
    .await;
    adv(&h, 20).await;
    let out = h.cli(&["status"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "status", &out);
    let out = h.cmd(&["status"]).version("1.3.0+other").run().await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "status_cli_differs", &out);
    h.finish().await;
}

// TST-7: CLI-3's `status` with a problem and its `until`, and two warnings.
#[tokio::test]
async fn golden_status_problem() {
    let (h, p) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let a = settled_pr(w, repo);
        let b = settled_pr(w, repo);
        w.set_updated_at(b, w.now().minus(s(300)));
        (a, b)
    })
    .await;
    adv(&h, 20).await;
    listed(&h).await;
    // POL-21: a node that errors on every refresh raises `pr_failing` after 5 minutes.
    h.github().pr_error(
        p.1,
        "FORBIDDEN",
        circitor::testkit::server::Aim::All,
        circitor::testkit::server::Times::UntilCleared,
    );
    adv(&h, 330).await;
    // POL-18: under the reserve; and the primary limit used up.
    let reset = h.clock().wall().plus(s(600));
    h.github().set_rate(0, reset);
    h.github().fault(
        circitor::testkit::server::Fault::Exhausted {
            status: circitor::testkit::server::LimitStatus::Forbidden,
            reset: true,
        },
        circitor::testkit::server::Times::UntilCleared,
    );
    adv(&h, 60).await;
    let out = h.cli(&["status"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert!(out.stdout.contains("pr_failing since"), "{out:?}");
    assert!(out.stdout.contains("quota_reserve since"), "{out:?}");
    assert!(out.stdout.contains("\nproblem: rate_limited"), "{out:?}");
    gold(&h, "status_problem", &out);
    h.github().clear_faults();
    h.finish().await;
}

// TST-7: CLI-3's `refresh`: `polled` and `shared`.
#[tokio::test]
async fn golden_refresh() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo);
    })
    .await;
    adv(&h, 20).await;
    let out = run(&h, &["refresh"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "refresh_polled", &out);
    // Within POL-15's window of the one just done: shared.
    let out = run(&h, &["refresh"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "refresh_shared", &out);
    h.finish().await;
}

// TST-7: CLI-3's `stop`: `stopped`, then `not running`.
#[tokio::test]
async fn golden_stop() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo);
    })
    .await;
    adv(&h, 20).await;
    let out = run(&h, &["stop"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "stop_stopped", &out);
    let out = run(&h, &["stop"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "stop_not_running", &out);
    h.finish().await;
}

// TST-7: CLI-3's `track` and `untrack`, and `untrack` of a PR that isn't tracked (API-15).
#[tokio::test]
async fn golden_track_untrack() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        settled_pr(w, repo);
        let lib = w.repo("octo", "lib");
        let x = settled_pr(w, lib);
        w.set_author(x, Some("someone-else"));
    })
    .await;
    adv(&h, 20).await;
    let out = run(&h, &["track", "octo/lib#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "track", &out);
    let out = run(&h, &["untrack", "octo/lib#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "untrack", &out);
    let out = run(&h, &["untrack", "octo/lib#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "untrack_not_tracked", &out);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// changes
// ---------------------------------------------------------------------------------------------

/// The PRs of the "every kind" world.
struct Kinds {
    busy: PrId,
    merged: PrId,
    closed: PrId,
    reopened: PrId,
    gone: PrId,
}

fn kinds_world(w: &mut World) -> Kinds {
    let repo = w.repo("octo", "app");
    let busy = settled_pr(w, repo);
    w.set_updated_at(busy, w.now().minus(s(60)));
    let c = w.run(busy, &format!("{} build", circitor::testkit::MARKER_CHECK));
    w.start(c);
    let merged = settled_pr(w, repo);
    let closed = settled_pr(w, repo);
    let reopened = settled_pr(w, repo);
    let gone = settled_pr(w, repo);
    for pr in [merged, closed, reopened, gone] {
        w.set_updated_at(pr, w.now().minus(s(300)));
    }
    let lib = w.repo("octo", "lib");
    let x = settled_pr(w, lib);
    w.set_author(x, Some("someone-else"));
    Kinds {
        busy,
        merged,
        closed,
        reopened,
        gone,
    }
}

// TST-7: CLI-3's `changes`: one line for each kind of event the fake can cause, and for a new
// comment, a new review with inline comments, `edited`, `tracked` and `untracked`.
#[tokio::test]
async fn golden_changes_every_kind() {
    let (h, k) = start(Opts::default(), kinds_world).await;
    adv(&h, 20).await;
    listed(&h).await;
    let c0 = cursor(&h).await;

    // A manual track and its untrack, in the same stretch.
    let out = run(&h, &["track", "octo/lib#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    // A push, a draft, a conflict, a title and a base, a comment, a review with inline comments.
    h.github().world(|w| {
        w.push(k.busy);
        w.set_draft(k.busy, true);
        w.set_mergeable(k.busy, "CONFLICTING");
        w.set_merge_state(k.busy, Some("DIRTY"));
        w.set_title(k.busy, "Renamed");
        w.set_base(k.busy, "develop");
        w.comment(k.busy);
        w.review(k.busy, "CHANGES_REQUESTED", 3);
        w.set_review_decision(k.busy, Some("CHANGES_REQUESTED"));
        w.merge(k.merged);
        w.close(k.closed);
        w.close(k.reopened);
        w.hide(k.gone);
    });
    adv(&h, 31).await;
    // The check fails: a `checks` event, and the status follows.
    h.github().world(|w| {
        let run = w.runs(k.busy, 1, "COMPLETED", Some("FAILURE"));
        let _ = run;
    });
    adv(&h, 31).await;
    h.github().world(|w| w.reopen(k.reopened));
    adv(&h, 31).await;
    let out = run(&h, &["untrack", "octo/lib#1"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    // POL-21: not found for 5 minutes is `gone`.
    adv(&h, 320).await;
    let out = h.cli(&["changes", "--since", &c0]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "changes", &out);
    h.finish().await;
}

// TST-7: CLI-3's reset: a cursor of another epoch is never silent (STA-9).
#[tokio::test]
async fn golden_changes_reset() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let a = settled_pr(w, repo);
        let c = w.run(a, "build");
        w.finish(c, "SUCCESS");
    })
    .await;
    adv(&h, 20).await;
    let old = cursor(&h).await;
    h.restart().await;
    adv(&h, 20).await;
    let out = h.cli(&["changes", "--since", &old]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert!(
        out.stdout
            .starts_with("reset: cursor too old, current state follows\n"),
        "{out:?}"
    );
    gold(&h, "changes_reset", &out);
    h.finish().await;
}

// TST-7: CLI-3's 50 events: `more events:` names the command that goes on.
#[tokio::test]
async fn golden_changes_more_events() {
    let (h, ids) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        w.open_prs(repo, 51, s(60))
    })
    .await;
    let out = run(&h, &["list"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    let c = cursor(&h).await;
    h.github().world(|w| {
        for pr in &ids {
            w.set_draft(*pr, true);
        }
    });
    adv(&h, 120).await;
    let out = h.cli(&["changes", "--since", &c]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    let more: Vec<&str> = out
        .stdout
        .lines()
        .filter(|l| l.starts_with("more events: "))
        .collect();
    assert_eq!(more.len(), 1, "{out:?}");
    assert!(
        more[0].starts_with("more events: circitor changes --since "),
        "{out:?}"
    );
    gold(&h, "changes_more_events", &out);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// wait
// ---------------------------------------------------------------------------------------------

const PR1: &str = "octo/app#1";

/// `octo/app#1`, settled, with `build` passed and `test` running.
fn running_world(w: &mut World) -> (PrId, circitor::testkit::github::RunId) {
    let repo = w.repo("octo", "app");
    let pr = settled_pr(w, repo);
    let a = w.run(pr, "build");
    w.finish(a, "SUCCESS");
    let b = w.run(pr, "test");
    w.start(b);
    (pr, b)
}

/// Starts the waits `args` after the first poll, lets them settle in, runs `change` on the world,
/// and drives them to their ends.
async fn waits(h: &Harness, args: &[&[&str]], change: impl FnOnce(&mut World)) -> Vec<Output> {
    let bgs: Vec<Bg> = args.iter().map(|a| h.cli_bg(a)).collect();
    adv(h, 3).await;
    assert!(bgs.iter().all(|b| !b.is_done()));
    h.github().world(change);
    drive(h, bgs).await
}

// TST-7: CLI-6's `checks passed (n/n) on <sha>`.
#[tokio::test]
async fn golden_wait_checks_passed() {
    let (h, (_, b)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(&h, &[&["wait", PR1, "--until", "checks-done"]], |w| {
        w.finish(b, "SUCCESS")
    })
    .await;
    assert_eq!(outs[0].code, exit::OK, "{:?}", outs[0]);
    gold(&h, "wait_checks_passed", &outs[0]);
    // `--json`: the answer is known, and the body is one line.
    let out = h
        .cli(&["wait", PR1, "--until", "checks-done", "--json"])
        .await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold_json(&h, "wait_satisfied", &out);
    h.finish().await;
}

// TST-7: CLI-6's `checks failed (1 of n) on <sha>`, exit 8 (CLI-5).
#[tokio::test]
async fn cli5_exit_8_checks_failed() {
    let (h, (_, b)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(&h, &[&["wait", PR1, "--until", "checks-done"]], |w| {
        w.finish(b, "FAILURE")
    })
    .await;
    assert_eq!(outs[0].code, exit::CHECKS_FAILED, "{:?}", outs[0]);
    gold(&h, "wait_checks_failed", &outs[0]);
    h.finish().await;
}

// TST-7: D8's `--fail-fast`: `checks failing (1 failed, 1 of 2 done) on <sha>`, exit 8; and its
// timeout's `again:` repeats `--fail-fast`.
#[tokio::test]
async fn cli5_exit_8_fail_fast() {
    let (h, a) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        let a = w.run(pr, "build");
        w.start(a);
        let b = w.run(pr, "test");
        w.start(b);
        a
    })
    .await;
    adv(&h, 20).await;
    // Nothing fails: the timeout's `again:` repeats `--fail-fast` (CLI-6).
    let outs = waits(
        &h,
        &[&["wait", PR1, "--until", "checks-done", "--fail-fast"]],
        |_| {},
    )
    .await;
    assert_eq!(outs[0].code, exit::TIMEOUT, "{:?}", outs[0]);
    assert!(
        outs[0]
            .stdout
            .contains(" --until checks-done --fail-fast --since "),
        "{:?}",
        outs[0]
    );
    gold(&h, "wait_fail_fast_timeout", &outs[0]);
    // One fails and the other still runs.
    let outs = waits(
        &h,
        &[&["wait", PR1, "--until", "checks-done", "--fail-fast"]],
        |w| w.finish(a, "FAILURE"),
    )
    .await;
    assert_eq!(outs[0].code, exit::CHECKS_FAILED, "{:?}", outs[0]);
    gold(&h, "wait_fail_fast", &outs[0]);
    h.finish().await;
}

// TST-7: CLI-6's `no checks reported on <sha>`, exit 0.
#[tokio::test]
async fn golden_wait_no_checks() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        w.open_pr(repo);
    })
    .await;
    let outs = waits(&h, &[&["wait", PR1, "--until", "checks-done"]], |_| {}).await;
    assert_eq!(outs[0].code, exit::OK, "{:?}", outs[0]);
    gold(&h, "wait_no_checks", &outs[0]);
    h.finish().await;
}

// TST-7: CLI-6's `merged`, `changed` and `ended: merged`, which D8's `--kind` ends too.
#[tokio::test]
async fn golden_wait_merged() {
    let (h, (pr, _)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(
        &h,
        &[
            &["wait", PR1, "--until", "merged"],
            &["wait", PR1, "--until", "changed"],
            &["wait", PR1, "--until", "checks-done"],
            &["wait", PR1, "--until", "changed", "--kind", "activity"],
        ],
        |w| w.merge(pr),
    )
    .await;
    assert_eq!(outs[0].code, exit::OK, "{:?}", outs[0]);
    gold(&h, "wait_merged", &outs[0]);
    assert_eq!(outs[1].code, exit::OK, "{:?}", outs[1]);
    gold(&h, "wait_changed", &outs[1]);
    assert_eq!(outs[2].code, exit::ENDED, "{:?}", outs[2]);
    gold(&h, "wait_ended_merged", &outs[2]);
    assert_eq!(outs[3].code, exit::ENDED, "{:?}", outs[3]);
    gold(&h, "wait_kind_activity_ended_merged", &outs[3]);
    h.finish().await;
}

// TST-7: CLI-6's `ended: closed`, exit 4.
#[tokio::test]
async fn cli5_exit_4_closed() {
    let (h, (pr, _)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(&h, &[&["wait", PR1, "--until", "merged"]], |w| w.close(pr)).await;
    assert_eq!(outs[0].code, exit::ENDED, "{:?}", outs[0]);
    gold(&h, "wait_ended_closed", &outs[0]);
    h.finish().await;
}

// TST-7: CLI-6's `ended: gone`, exit 4: not found for 5 minutes (POL-21).
#[tokio::test]
async fn cli5_exit_4_gone() {
    let (h, (pr, _)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(
        &h,
        &[&["wait", PR1, "--until", "merged", "--timeout", "600"]],
        |w| w.hide(pr),
    )
    .await;
    assert_eq!(outs[0].code, exit::ENDED, "{:?}", outs[0]);
    gold(&h, "wait_ended_gone", &outs[0]);
    h.finish().await;
}

// TST-7: CLI-6's `ended: head moved to …, expected …`, exit 4: `--sha` that never matches (API-11).
#[tokio::test]
async fn cli5_exit_4_head_moved() {
    let (h, _) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(
        &h,
        &[&["wait", PR1, "--until", "checks-done", "--sha", "1a2b3c4"]],
        |_| {},
    )
    .await;
    assert_eq!(outs[0].code, exit::ENDED, "{:?}", outs[0]);
    gold(&h, "wait_ended_head_moved", &outs[0]);
    h.finish().await;
}

// TST-7: CLI-6's timeout with `again:` and `cursor:`; with `--sha` while the head isn't it; and
// the narrower conditions' `again:` lines (D8).
#[tokio::test]
async fn cli5_exit_3_timeout() {
    let (h, _) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(
        &h,
        &[
            &["wait", PR1, "--until", "merged"],
            &[
                "wait",
                PR1,
                "--until",
                "checks-done",
                "--sha",
                "1a2b3c4",
                "--timeout",
                "50",
            ],
            &["wait", PR1, "--until", "changed", "--kind", "activity"],
        ],
        |_| {},
    )
    .await;
    for o in &outs {
        assert_eq!(o.code, exit::TIMEOUT, "{o:?}");
    }
    gold(&h, "wait_timeout", &outs[0]);
    gold(&h, "wait_timeout_sha", &outs[1]);
    assert!(outs[1].stdout.contains(", not 1a2b3c4"), "{:?}", outs[1]);
    gold(&h, "wait_kind_activity_timeout", &outs[2]);
    assert!(
        outs[2].stdout.contains(" --kind activity --since "),
        "{:?}",
        outs[2]
    );
    h.finish().await;
}

// TST-7: CLI-6's `<pr> no record yet`: a PR loaded from the state file has none until its first
// refresh, and `wait` doesn't read it through (STA-8).
#[tokio::test]
async fn golden_wait_no_record() {
    let (h, (pr, _)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    h.github().pr_error(
        pr,
        "FORBIDDEN",
        circitor::testkit::server::Aim::All,
        circitor::testkit::server::Times::UntilCleared,
    );
    h.restart().await;
    let outs = waits(
        &h,
        &[&["wait", PR1, "--until", "merged", "--timeout", "30"]],
        |_| {},
    )
    .await;
    assert_eq!(outs[0].code, exit::TIMEOUT, "{:?}", outs[0]);
    assert!(
        outs[0].stdout.contains("octo/app#1 no record yet"),
        "{:?}",
        outs[0]
    );
    gold(&h, "wait_no_record", &outs[0]);
    h.github().clear_faults();
    adv(&h, 5).await;
    h.finish().await;
}

// TST-7: CLI-6's truncated `failing`: the failed check is among the 100 unread (MOD-7).
#[tokio::test]
async fn golden_wait_truncated() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let pr = settled_pr(w, repo);
        w.runs(pr, 300, "COMPLETED", Some("SUCCESS"));
        w.runs(pr, 99, "COMPLETED", Some("SUCCESS"));
        w.runs(pr, 1, "COMPLETED", Some("FAILURE"));
    })
    .await;
    adv(&h, 20).await;
    let outs = [run(&h, &["wait", PR1, "--until", "checks-done"]).await];
    assert_eq!(outs[0].code, exit::CHECKS_FAILED, "{:?}", outs[0]);
    assert!(
        outs[0]
            .stdout
            .contains("(truncated: unread checks may still run)"),
        "{:?}",
        outs[0]
    );
    gold(&h, "wait_truncated", &outs[0]);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// notify, and the notices
// ---------------------------------------------------------------------------------------------

/// A notice's words with its values replaced, so it can be held against T46's file for the same
/// case, which has other values: the PR's name, a short SHA, a cursor, every number.
fn shape(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let trimmed = word.trim_end_matches(['.', ',', ':', ';']);
            let tail = &word[trimmed.len()..];
            let core = if ["checks-done", "merged", "changed"].contains(&trimmed) {
                "<until>".to_string()
            } else if trimmed.starts_with(|c: char| c.is_ascii_digit())
                && trimmed
                    .chars()
                    .all(|c| c.is_ascii_digit() || "dhms".contains(c))
            {
                "<dur>".to_string()
            } else if trimmed.contains('/') && trimmed.contains('#') {
                "<pr>".to_string()
            } else if trimmed.len() == 7 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
                "<sha>".to_string()
            } else if trimmed
                .split_once('-')
                .is_some_and(|(e, n)| e.len() == 8 && n.chars().all(|c| c.is_ascii_digit()))
            {
                "<cursor>".to_string()
            } else {
                trimmed
                    .chars()
                    .map(|c| if c.is_ascii_digit() { '#' } else { c })
                    .collect()
            };
            format!("{core}{tail}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A notice as a session got it (NTF-6): its auth line carried the session's token, its text is
/// `<case>.notice`, and its words are the file T46 wrote for `t46`, but for the values (NTF-7, NTF-8).
fn notice(n: &circitor::testkit::session::Notice, case: &str, t46: &str) {
    assert_eq!(
        n.token.as_deref(),
        Some(MARKER_SESSION_TOKEN),
        "{case}: {n:?}"
    );
    golden::check(&golden_path(case, "notice"), &format!("{}\n", n.text));
    let theirs = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/notify/{t46}.txt")),
    )
    .unwrap_or_else(|e| panic!("{t46}: {e}"));
    assert_eq!(
        shape(&n.text),
        shape(theirs.trim_end()),
        "{case}: the words differ from T46's {t46}"
    );
}

fn notices(h: &Harness) -> Vec<circitor::testkit::session::Notice> {
    h.session().notices()
}

/// A world with `octo/app#1` running a check, and a second PR `octo/app#2`.
fn notify_world(w: &mut World) -> (PrId, circitor::testkit::github::RunId) {
    let repo = w.repo("octo", "app");
    let pr = settled_pr(w, repo);
    let run = w.run(pr, &format!("{} test", circitor::testkit::MARKER_CHECK));
    w.start(run);
    let other = settled_pr(w, repo);
    w.set_updated_at(other, w.now().minus(s(300)));
    (pr, run)
}

// TST-7: NTF-11's registered block, with `--for`, replaced, and `--kind … --since … --for`;
// NTF-7's confirmation for each; CLI-3's `status` with `subscriptions: 2`.
#[tokio::test]
async fn golden_notify_registered() {
    let (h, _) = start(Opts::default(), notify_world).await;
    adv(&h, 20).await;
    let out = run(&h, &["notify", PR1, "--until", "checks-done"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "notify_registered", &out);
    let out = run(&h, &["notify", PR1, "--until", "merged", "--for", "3600"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "notify_registered_for", &out);
    let out = h.cli(&["status"]).await;
    assert!(out.stdout.contains("subscriptions: 2\n"), "{out:?}");
    gold(&h, "status_subscriptions", &out);
    // Again with the same condition: replaced.
    let out = run(&h, &["notify", PR1, "--until", "merged", "--for", "3600"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert!(
        out.stdout.contains("replaced an earlier notify\n"),
        "{out:?}"
    );
    gold(&h, "notify_registered_replaced", &out);
    let c = cursor(&h).await;
    let out = run(
        &h,
        &[
            "notify", PR1, "--until", "changed", "--kind", "review", "--since", &c, "--for", "7200",
        ],
    )
    .await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "notify_registered_kind_since", &out);

    let n = notices(&h);
    assert_eq!(n.len(), 4, "{n:?}");
    notice(
        &n[0],
        "notice_confirmation_checks_done",
        "confirmation_plain",
    );
    notice(&n[1], "notice_confirmation_for", "confirmation_plain");
    notice(&n[2], "notice_confirmation_replaced", "confirmation_plain");
    notice(
        &n[3],
        "notice_confirmation_kind_since",
        "confirmation_kind_since",
    );
    h.finish().await;
}

// TST-7: NTF-11's `already:` with a failed check; and after a registration it replaces.
#[tokio::test]
async fn golden_notify_already() {
    let (h, (pr, chk)) = start(Opts::default(), notify_world).await;
    adv(&h, 20).await;
    let c0 = cursor(&h).await;
    h.github().world(|w| {
        w.finish(chk, "FAILURE");
        w.review(pr, "APPROVED", 0);
    });
    adv(&h, 35).await;
    // A failed check: known at once.
    let out = run(&h, &["notify", PR1, "--until", "checks-done"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "notify_already", &out);
    // A held subscription for a review is replaced by a request that finds one in the history
    // since `c0`: the answer is known, nothing is kept, and the earlier one is dropped (NTF-2).
    let out = run(
        &h,
        &["notify", PR1, "--until", "changed", "--kind", "activity"],
    )
    .await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert!(out.stdout.starts_with("registered: "), "{out:?}");
    let out = run(
        &h,
        &[
            "notify", PR1, "--until", "changed", "--kind", "activity", "--since", &c0,
        ],
    )
    .await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "notify_already_replaced", &out);
    assert_eq!(
        notices(&h).len(),
        1,
        "the confirmation of the held one, and no more"
    );
    h.finish().await;
}

// TST-7: NTF-11's `--cancel`: `cancelled: 1`, then `cancelled: 0`.
#[tokio::test]
async fn golden_notify_cancel() {
    let (h, _) = start(Opts::default(), notify_world).await;
    adv(&h, 20).await;
    let out = run(&h, &["notify", PR1, "--until", "merged"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    let out = run(&h, &["notify", PR1, "--cancel"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "notify_cancelled", &out);
    let out = run(&h, &["notify", PR1, "--cancel"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold(&h, "notify_cancelled_none", &out);
    h.finish().await;
}

/// The notice about `pr` that isn't its confirmation.
fn outcome_of(h: &Harness, pr: &str) -> circitor::testkit::session::Notice {
    let found: Vec<_> = notices(h)
        .into_iter()
        .filter(|n| n.text.contains(&format!("{pr} ")) && !n.text.contains("watching"))
        .collect();
    assert_eq!(found.len(), 1, "{pr}: {found:?}");
    found.into_iter().next().unwrap()
}

// TST-7: NTF-8's outcome notices: passed, failed, `checks failing` under `--fail-fast`, `merged`,
// `ended: closed`.
#[tokio::test]
async fn golden_notices_outcomes() {
    let (h, w5) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let mut out = Vec::new();
        for i in 0..5u64 {
            let pr = settled_pr(w, repo);
            w.set_updated_at(pr, w.now().minus(s(60 + i)));
            let r = w.unnamed_run(pr);
            w.start(r);
            out.push((pr, r));
        }
        // #3 has a second check, still running when the first fails.
        let extra = w.unnamed_run(out[2].0);
        w.start(extra);
        out
    })
    .await;
    adv(&h, 20).await;
    for args in [
        &["notify", "octo/app#1", "--until", "checks-done"][..],
        &["notify", "octo/app#2", "--until", "checks-done"],
        &[
            "notify",
            "octo/app#3",
            "--until",
            "checks-done",
            "--fail-fast",
        ],
        &["notify", "octo/app#4", "--until", "merged"],
        &["notify", "octo/app#5", "--until", "merged"],
    ] {
        let out = run(&h, args).await;
        assert_eq!(out.code, exit::OK, "{args:?}: {out:?}");
    }
    h.github().world(|w| {
        w.finish(w5[0].1, "SUCCESS");
        w.finish(w5[1].1, "FAILURE");
        w.finish(w5[2].1, "FAILURE");
        w.merge(w5[3].0);
        w.close(w5[4].0);
    });
    adv(&h, 90).await;
    notice(
        &outcome_of(&h, "octo/app#1"),
        "notice_outcome_checks_passed",
        "outcome_checks_passed",
    );
    notice(
        &outcome_of(&h, "octo/app#2"),
        "notice_outcome_checks_failed",
        "outcome_checks_failed",
    );
    notice(
        &outcome_of(&h, "octo/app#3"),
        "notice_outcome_checks_failing",
        "outcome_checks_failing",
    );
    notice(
        &outcome_of(&h, "octo/app#4"),
        "notice_outcome_merged",
        "outcome_merged",
    );
    notice(
        &outcome_of(&h, "octo/app#5"),
        "notice_outcome_ended_closed",
        "outcome_ended_closed",
    );
    h.finish().await;
}

// TST-7: NTF-8's `expired`: `--for 60` and a minute.
#[tokio::test]
async fn golden_notice_expired() {
    let (h, _) = start(Opts::default(), notify_world).await;
    adv(&h, 20).await;
    let out = run(
        &h,
        &[
            "notify",
            "octo/app#2",
            "--until",
            "changed",
            "--kind",
            "review,checks",
            "--for",
            "60",
        ],
    )
    .await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    adv(&h, 61).await;
    let n = notices(&h);
    assert_eq!(n.len(), 2, "{n:?}");
    assert!(n[1].text.contains("expired after 1m"), "{n:?}");
    notice(&n[1], "notice_expired", "expired_for_given");
    h.finish().await;
}

// TST-7: NTF-8's `stopped`, ending `--no-spawn` (NTF-10).
#[tokio::test]
async fn golden_notice_stopped() {
    let (h, (pr, _)) = start(Opts::default(), notify_world).await;
    let sha = h.github().world(|w| w.head(pr)[..7].to_string());
    adv(&h, 20).await;
    let out = run(
        &h,
        &["notify", PR1, "--until", "checks-done", "--sha", &sha],
    )
    .await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    let out = run(&h, &["stop"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    let n = notices(&h);
    assert_eq!(n.len(), 2, "{n:?}");
    assert!(n[1].text.ends_with("--no-spawn"), "{n:?}");
    notice(&n[1], "notice_stopped", "stopped_no_spawn");
    h.finish().await;
}

// TST-7: NTF-8's `History too old` form, after over 500 events (STA-10).
#[tokio::test]
async fn golden_notice_history_too_old() {
    let (h, ids) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        let target = settled_pr(w, repo);
        w.set_updated_at(target, w.now().minus(s(60)));
        let r = w.unnamed_run(target);
        w.start(r);
        let others = w.open_prs(repo, 30, s(10));
        (target, others)
    })
    .await;
    adv(&h, 20).await;
    let out = run(&h, &["notify", PR1, "--until", "merged"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    // 30 PRs flip `draft` at every poll: 30 events each time.
    for i in 0..18 {
        let flip = i % 2 == 0;
        let others = ids.1.clone();
        h.github().world(move |w| {
            for pr in &others {
                w.set_draft(*pr, flip);
            }
        });
        adv(&h, 31).await;
    }
    let target = ids.0;
    h.github().world(|w| w.merge(target));
    adv(&h, 35).await;
    let n = outcome_of(&h, PR1);
    assert!(n.text.contains("History too old"), "{n:?}");
    notice(&n, "notice_history_too_old", "outcome_history_too_old");
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// Error lines and exit codes (CLI-5, API-3)
// ---------------------------------------------------------------------------------------------

use circitor::cli::notify::SOCKET_VAR;
use circitor::testkit::harness::{BAD_CONFIG, spawner_bad_config, spawner_never_binds};
use circitor::testkit::server::{Fault, LimitStatus, Times};
use circitor::token::TokenRead;

/// A stderr-only case: stdout empty, the one line of stderr is the file.
fn gold_err(h: &Harness, case: &str, out: &Output, code: i32) {
    assert_eq!(out.code, code, "{case}: {out:?}");
    assert!(out.stdout.is_empty(), "{case}: {out:?}");
    assert_eq!(out.stderr.matches('\n').count(), 1, "{case}: {out:?}");
    gold(h, case, out);
}

fn one_pr(w: &mut World) -> PrId {
    let repo = w.repo("octo", "app");
    settled_pr(w, repo)
}

// TST-7: CLI-5: code 0.
#[tokio::test]
async fn cli5_exit_0_done() {
    let (h, _) = start(Opts::default(), |w| {
        one_pr(w);
    })
    .await;
    adv(&h, 20).await;
    let out = h.cli(&["list"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    h.finish().await;
}

// TST-7: CLI-5, API-3: code 5, `not_found`.
#[tokio::test]
async fn cli5_exit_5_not_found() {
    let (h, _) = start(Opts::default(), |w| {
        one_pr(w);
    })
    .await;
    adv(&h, 20).await;
    let out = run(&h, &["show", "octo/app#99"]).await;
    gold_err(&h, "error_not_found", &out, exit::NOT_FOUND);
    h.finish().await;
}

// TST-7: CLI-5, API-3: code 7, `auth`.
#[tokio::test]
async fn cli5_exit_7_auth() {
    let (h, _) = start(Opts::default().token(TokenRead::NoLogin), |w| {
        one_pr(w);
    })
    .await;
    let out = run(&h, &["list"]).await;
    gold_err(&h, "error_auth", &out, exit::AUTH);
    h.finish().await;
}

// TST-7: CLI-5, API-3: code 9 for `starting`: the first poll hasn't finished after 10 s (API-18).
#[tokio::test]
async fn cli5_exit_9_starting() {
    let (h, _) = start(Opts::default().hold_first(), |w| {
        one_pr(w);
    })
    .await;
    let out = run(&h, &["list"]).await;
    gold_err(&h, "error_starting", &out, exit::UNAVAILABLE);
    h.take_hold().expect("the held first request").release();
    adv(&h, 5).await;
    h.finish().await;
}

// TST-7: CLI-5, API-3: code 9 for `github_unavailable`, with its `until`.
#[tokio::test]
async fn cli5_exit_9_github_unavailable() {
    let (h, _) = start(Opts::default(), |w| {
        one_pr(w);
        let lib = w.repo("octo", "lib");
        let x = settled_pr(w, lib);
        w.set_author(x, Some("someone-else"));
    })
    .await;
    adv(&h, 20).await;
    h.github().fault(Fault::Status(502), Times::UntilCleared);
    // A PR with no record is read through, and GitHub can't be reached.
    let out = run(&h, &["show", "octo/lib#1"]).await;
    gold_err(&h, "error_github_unavailable", &out, exit::UNAVAILABLE);
    h.github().clear_faults();
    h.finish().await;
}

// TST-7: CLI-5, API-3: code 9 for `rate_limited`, with its `until`.
#[tokio::test]
async fn cli5_exit_9_rate_limited() {
    let (h, _) = start(Opts::default(), |w| {
        one_pr(w);
        let lib = w.repo("octo", "lib");
        let x = settled_pr(w, lib);
        w.set_author(x, Some("someone-else"));
    })
    .await;
    adv(&h, 20).await;
    let reset = h.clock().wall().plus(s(17 * 60));
    h.github().set_rate(0, reset);
    h.github().fault(
        Fault::Exhausted {
            status: LimitStatus::Forbidden,
            reset: true,
        },
        Times::UntilCleared,
    );
    let out = run(&h, &["show", "octo/lib#1"]).await;
    gold_err(&h, "error_rate_limited", &out, exit::UNAVAILABLE);
    h.github().clear_faults();
    h.finish().await;
}

// TST-7: CLI-5, API-3: code 10, `limit`: the 65th subscription (NTF-2: 64 held, 32 for one socket).
#[tokio::test]
async fn cli5_exit_10_limit() {
    let (h, _) = start(Opts::default(), |w| {
        let repo = w.repo("octo", "app");
        w.open_prs(repo, 33, s(60));
    })
    .await;
    adv(&h, 60).await;
    let (a, b, c) = (h.session(), h.new_session("second"), h.new_session("third"));
    let mut n = 0;
    for session in [&a, &b] {
        for pr in 1..=32 {
            let name = format!("octo/app#{pr}");
            let out = drive1(
                &h,
                h.cmd(&["notify", &name, "--until", "merged"])
                    .var(SOCKET_VAR, &session.path().display().to_string())
                    .bg(),
            )
            .await;
            assert_eq!(out.code, exit::OK, "{name}: {out:?}");
            n += 1;
        }
    }
    assert_eq!(n, 64);
    let out = drive1(
        &h,
        h.cmd(&["notify", "octo/app#33", "--until", "merged"])
            .var(SOCKET_VAR, &c.path().display().to_string())
            .bg(),
    )
    .await;
    gold_err(&h, "error_limit", &out, exit::LIMIT);
    h.finish().await;
}

// TST-7: CLI-5, NTF-11: code 11 for a socket in the environment that isn't one: `no_session`.
#[tokio::test]
async fn cli5_exit_11_no_session() {
    let (h, _) = start(Opts::default(), |w| {
        one_pr(w);
    })
    .await;
    adv(&h, 20).await;
    let missing = format!("{}/missing.sock", h.dir().display());
    let out = drive1(
        &h,
        h.cmd(&["notify", PR1, "--until", "checks-done"])
            .var(SOCKET_VAR, &missing)
            .bg(),
    )
    .await;
    gold_err(&h, "error_no_session", &out, exit::NO_SESSION);
    let json = drive1(
        &h,
        h.cmd(&["notify", PR1, "--until", "checks-done", "--json"])
            .var(SOCKET_VAR, &missing)
            .bg(),
    )
    .await;
    assert_eq!(json.code, exit::NO_SESSION, "{json:?}");
    h.finish().await;
}

// TST-7: CLI-5, NTF-11: code 11 for no socket in the environment: before anything else, no daemon needed.
#[tokio::test]
async fn cli5_exit_11_no_socket() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let out = h
        .cmd(&["notify", "12", "--until", "checks-done", "--sha", "1a2b3c4"])
        .without(SOCKET_VAR)
        .run()
        .await;
    gold_err(&h, "error_notify_no_socket", &out, exit::NO_SESSION);
    let out = h
        .cmd(&["notify", "--cancel"])
        .without(SOCKET_VAR)
        .run()
        .await;
    gold_err(&h, "error_notify_no_socket_cancel", &out, exit::NO_SESSION);
    assert_eq!(h.spawns(), 0, "nothing was started");
    h.finish().await;
}

// TST-7: CLI-5: code 2, a usage error; a PR that can't be named (CLI-2); `wait --timeout 3601` sends nothing.
#[tokio::test]
async fn cli5_exit_2_usage() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let out = h.cli(&["list", "--bogus"]).await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    gold(&h, "error_usage", &out);
    let out = h
        .cli(&["wait", PR1, "--until", "merged", "--timeout", "3601"])
        .await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
    gold(&h, "error_usage_timeout", &out);
    assert_eq!(h.spawns(), 0, "no request, so no daemon");
    h.finish().await;
}

// TST-7: CLI-5, CLI-2: code 2, a PR that can't be named: a number outside a git repository.
#[tokio::test]
async fn cli5_exit_2_pr_not_named() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let out = h.cli(&["show", "12"]).await;
    gold_err(&h, "error_pr_not_named", &out, exit::USAGE);
    h.finish().await;
}

// TST-7: CLI-5, CLI-7: code 6 with `--no-spawn`, and `status` with no daemon (CLI-8).
#[tokio::test]
async fn cli5_exit_6_no_spawn() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let out = h.cli(&["list", "--no-spawn"]).await;
    gold_err(&h, "error_no_spawn", &out, exit::NO_DAEMON);
    let out = h.cli(&["status"]).await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    gold(&h, "error_status_no_daemon", &out);
    assert_eq!(h.spawns(), 0);
    h.finish().await;
}

// TST-7: CLI-5, CLI-7: code 6 with the log's lines: the spawned daemon exits 1 with OPS-1's message.
#[tokio::test]
async fn cli5_exit_6_daemon_exits() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let bad = h.cmd(&["list"]).spawner(spawner_bad_config()).bg();
    let out = drive1(&h, bad).await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert!(out.stderr.contains(BAD_CONFIG), "{out:?}");
    gold(&h, "error_daemon_exits", &out);
    h.finish().await;
}

// TST-7: CLI-5, CLI-7: code 6 after 8 s: the spawned daemon never binds its socket.
#[tokio::test]
async fn cli5_exit_6_never_binds() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let out = drive1(&h, h.cmd(&["list"]).spawner(spawner_never_binds()).bg()).await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    gold(&h, "error_never_binds", &out);
    h.finish().await;
}

// TST-7: API-1, API-12: what the CLI can't send. Each answers 400 `bad_request`, naming the parameter.
#[tokio::test]
async fn golden_bad_request() {
    let (h, _) = start(Opts::default(), |w| {
        one_pr(w);
    })
    .await;
    adv(&h, 20).await;
    let (status, body) = h.api_raw("GET", "/v1/prs?bogus=1", None).await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body.contains("bad_request") && body.contains("bogus"),
        "{body}"
    );
    golden::check(&golden_path("bad_request_unknown_parameter", "json"), &body);
    let (status, body) = h
        .api_raw(
            "GET",
            "/v1/prs/octo/app/1/wait?until=merged&timeout=3601",
            None,
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body.contains("bad_request") && body.contains("timeout"),
        "{body}"
    );
    golden::check(&golden_path("bad_request_timeout", "json"), &body);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// A stub daemon: code 1, and the endings of a wait without an answer
// ---------------------------------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};

use circitor::clock::SettleProbe;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

/// What the stub does with a request other than `GET /v1/status`.
#[derive(Clone, Copy)]
enum Then {
    /// Answers `testdata/api/error-unknown_route.json`, status 404.
    UnknownRoute,
    /// Reads it and never answers.
    Stall,
}

struct StubState {
    busy: AtomicUsize,
}

impl SettleProbe for StubState {
    fn settled(&self) -> bool {
        self.busy.load(Ordering::SeqCst) == 0
    }
}

/// A few lines of server on the harness's socket path, which a harness with `no_daemon` leaves
/// free. It answers `status` with `version` and `mode`, and everything else as `then` says.
fn stub(
    h: &Harness,
    version: &'static str,
    mode: &'static str,
    then: Then,
) -> tokio::task::JoinHandle<()> {
    let path = h.paths().socket.clone();
    std::fs::create_dir_all(path.parent().expect("the socket has a directory")).unwrap();
    let std = std::os::unix::net::UnixListener::bind(&path).expect("the stub binds");
    std.set_nonblocking(true).unwrap();
    let listener = UnixListener::from_std(std).unwrap();
    let state = Arc::new(StubState {
        busy: AtomicUsize::new(0),
    });
    h.clock().add_probe(state.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut conn, _)) = listener.accept().await else {
                return;
            };
            state.busy.fetch_add(1, Ordering::SeqCst);
            let state = state.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match conn.read(&mut chunk).await {
                        Ok(n) if n > 0 => buf.extend_from_slice(&chunk[..n]),
                        _ => {
                            state.busy.fetch_sub(1, Ordering::SeqCst);
                            return;
                        }
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let line = head.lines().next().unwrap_or_default().to_string();
                let (status, body) = if line.starts_with("GET /v1/status") {
                    let text = std::fs::read_to_string(
                        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/api/status.json"),
                    )
                    .unwrap();
                    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
                    v["version"] = version.into();
                    v["mode"] = mode.into();
                    (200, v.to_string())
                } else if matches!(then, Then::Stall) {
                    state.busy.fetch_sub(1, Ordering::SeqCst);
                    std::future::pending::<()>().await;
                    return;
                } else {
                    let text = std::fs::read_to_string(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("testdata/api/error-unknown_route.json"),
                    )
                    .unwrap();
                    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
                    v["version"] = version.into();
                    (404, v.to_string())
                };
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = conn.write_all(response.as_bytes()).await;
                let _ = conn.shutdown().await;
                let mut b = [0u8; 256];
                while matches!(conn.read(&mut b).await, Ok(n) if n > 0) {}
                state.busy.fetch_sub(1, Ordering::SeqCst);
            });
        }
    })
}

// TST-7: CLI-5, CLI-9: code 1 for a route the daemon doesn't know, from a daemon of the CLI's version.
#[tokio::test]
async fn cli5_exit_1_unknown_route() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let task = stub(&h, TEST_VERSION, "spawned", Then::UnknownRoute);
    let out = run(&h, &["list"]).await;
    gold_err(&h, "error_unknown_route", &out, exit::ERROR);
    task.abort();
    h.finish().await;
}

// TST-7: CLI-5, CLI-9: code 1, not 2, from an older daemon that isn't replaced (`foreground`), with the hint.
#[tokio::test]
async fn cli5_exit_1_older_foreground_daemon() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let task = stub(&h, "1.0.0", "foreground", Then::UnknownRoute);
    let out = run(&h, &["list"]).await;
    assert!(out.stderr.contains("older than this CLI"), "{out:?}");
    // CLI-9's warning, then the line.
    assert_eq!(out.code, exit::ERROR, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 2, "{out:?}");
    gold(&h, "error_older_foreground_daemon", &out);
    task.abort();
    h.finish().await;
}

// TST-7: CLI-6, CLI-7: a daemon that accepts a wait and never answers: past its limit (deadline plus 10 s)
// the wait ends `daemon not answering on <path>`, exit 6.
#[tokio::test]
async fn golden_wait_daemon_not_answering() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let task = stub(&h, TEST_VERSION, "spawned", Then::Stall);
    let out = run(
        &h,
        &[
            "wait",
            PR1,
            "--until",
            "merged",
            "--timeout",
            "20",
            "--since",
            "k3x9q2mz-418",
        ],
    )
    .await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    gold(&h, "wait_daemon_not_answering", &out);
    task.abort();
    h.finish().await;
}

// TST-7: CLI-6, CLI-10: the daemon stops in order under a wait: nobody answers within 8 s, `daemon stopped`.
#[tokio::test]
async fn golden_wait_daemon_stopped() {
    let (h, _) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let wait = h.cli_bg(&["wait", PR1, "--until", "merged"]);
    adv(&h, 3).await;
    assert!(!wait.is_done());
    h.stop().await;
    let out = drive1(&h, wait).await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    gold(&h, "wait_daemon_stopped", &out);
    h.finish().await;
}

// TST-7: CLI-6: a deadline used up before the daemon answers: `timeout after <n>s: <pr> (no answer from
// the daemon)`, exit 3.
#[tokio::test]
async fn golden_wait_no_time_left() {
    let (h, _) = start(Opts::default().no_daemon(), |w| {
        one_pr(w);
    })
    .await;
    let out = drive1(
        &h,
        h.cmd(&[
            "wait",
            PR1,
            "--until",
            "merged",
            "--timeout",
            "5",
            "--since",
            "k3x9q2mz-418",
        ])
        .spawner(spawner_never_binds())
        .bg(),
    )
    .await;
    assert_eq!(out.code, exit::TIMEOUT, "{out:?}");
    gold(&h, "wait_no_time_left", &out);
    h.finish().await;
}

// ---------------------------------------------------------------------------------------------
// --json (CLI-4, API-19)
// ---------------------------------------------------------------------------------------------

use circitor::api::Request;
use circitor::model::PrName;

fn name(s: &str) -> PrName {
    PrName::parse(s).expect("a PR name")
}

/// `--json`'s stdout is the body the daemon sends, byte for byte, and one line (CLI-4).
async fn same_as_api(h: &Harness, out: &Output, req: &Request) {
    let (status, body) = h.api_raw("GET", &req.path_and_query(), None).await;
    assert_eq!(status, 200, "{body}");
    let (a, b) = (
        out.stdout.trim_end_matches('\n'),
        body.trim_end_matches('\n'),
    );
    if a != b {
        let at = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
        panic!(
            "differ at {at}: cli …{} / api …{}",
            &a[at.saturating_sub(40)..(at + 80).min(a.len())],
            &b[at.saturating_sub(40)..(at + 80).min(b.len())]
        );
    }
    assert!(
        out.stdout.ends_with('\n') && out.stdout.matches('\n').count() == 1,
        "{out:?}"
    );
}

/// Every key API-19 gives the route's row is in the body.
fn has_keys(out: &Output, keys: &[&str]) {
    let v = json(out);
    for k in ["version", "cursor", "daemon"].iter().chain(keys.iter()) {
        assert!(v.get(*k).is_some(), "{k} is missing: {out:?}");
    }
}

// TST-7: `--json` of the GET routes, with something to report: equal to the daemon's body.
#[tokio::test]
async fn golden_json() {
    let (h, _) = start(Opts::default(), |w| {
        checks_world(w);
        let lib = w.repo("octo", "lib");
        let x = settled_pr(w, lib);
        w.set_author(x, Some("someone-else"));
    })
    .await;
    adv(&h, 20).await;
    let c = cursor(&h).await;
    let out = h.cli(&["list", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    has_keys(&out, &["records", "untracked", "pending", "reset"]);
    same_as_api(
        &h,
        &out,
        &Request::List {
            all: false,
            since: None,
            checks: false,
        },
    )
    .await;
    gold_json(&h, "json_list", &out);
    // A `show` of a PR with a record is answered from memory and wakes a refresh of it: ask once
    // so the two answers compared below come from the same state.
    let warm = Request::Show {
        pr: name(PR1),
        fresh: false,
    };
    h.api_raw("GET", &warm.path_and_query(), None).await;
    adv(&h, 1).await;
    let out = h.cli(&["show", PR1, "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    has_keys(&out, &["record"]);
    same_as_api(
        &h,
        &out,
        &Request::Show {
            pr: name(PR1),
            fresh: false,
        },
    )
    .await;
    gold_json(&h, "json_show", &out);
    let out = h.cli(&["status", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    same_as_api(&h, &out, &Request::Status).await;
    gold_json(&h, "json_status", &out);
    let out = run(&h, &["track", "octo/lib#1", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    has_keys(&out, &["record"]);
    gold_json(&h, "json_track", &out);
    let out = h.cli(&["changes", "--since", &c, "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    has_keys(
        &out,
        &["events", "records", "pending", "more_events", "reset"],
    );
    same_as_api(
        &h,
        &out,
        &Request::Changes {
            pr: None,
            since: Some(c.clone()),
            limit: Some(50),
        },
    )
    .await;
    gold_json(&h, "json_changes", &out);
    let out = run(&h, &["untrack", "octo/lib#1", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    has_keys(&out, &["record"]);
    gold_json(&h, "json_untrack", &out);
    adv(&h, 10).await;
    let out = run(&h, &["refresh", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    gold_json(&h, "json_refresh", &out);
    h.finish().await;
}

// TST-7: `--json` of every route with nothing to report: every key of API-19's row is there,
// lists are `[]`, flags `false`, `record` `null` where it may be.
#[tokio::test]
async fn golden_json_empty() {
    let (h, _) = start(Opts::default(), |w| {
        // The login's own PRs: none.
        let lib = w.repo("octo", "lib");
        let x = settled_pr(w, lib);
        w.set_author(x, Some("someone-else"));
    })
    .await;
    adv(&h, 20).await;
    let out = h.cli(&["list", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    has_keys(&out, &["records", "untracked", "pending", "reset"]);
    let v = json(&out);
    assert_eq!(v["records"], serde_json::json!([]));
    assert_eq!(v["untracked"], serde_json::json!([]));
    assert_eq!(v["pending"], serde_json::json!([]));
    assert_eq!(v["reset"], false);
    same_as_api(
        &h,
        &out,
        &Request::List {
            all: false,
            since: None,
            checks: false,
        },
    )
    .await;
    gold_json(&h, "json_list_empty", &out);
    let c = cursor(&h).await;
    let out = h.cli(&["changes", "--since", &c, "--json"]).await;
    has_keys(
        &out,
        &["events", "records", "pending", "more_events", "reset"],
    );
    let v = json(&out);
    assert_eq!(v["events"], serde_json::json!([]));
    assert_eq!(v["more_events"], false);
    assert_eq!(v["reset"], false);
    gold_json(&h, "json_changes_empty", &out);
    let out = run(&h, &["untrack", "octo/lib#1", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert_eq!(json(&out)["record"], serde_json::Value::Null);
    gold_json(&h, "json_untrack_null", &out);
    let out = h.cli(&["notify", PR1, "--cancel", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert_eq!(json(&out)["cancelled"], 0);
    gold_json(&h, "json_notify_cancelled_none", &out);
    let out = h.cli(&["status", "--json"]).await;
    gold_json(&h, "json_status_empty", &out);
    h.finish().await;
}

// TST-7: `--json` of `wait` (timed out, no record yet) and of `notify` (registered, already known,
// cancelled): one line each.
#[tokio::test]
async fn golden_json_wait_and_notify() {
    let (h, (pr, chk)) = start(Opts::default(), notify_world).await;
    adv(&h, 20).await;
    let out = run(
        &h,
        &[
            "wait",
            PR1,
            "--until",
            "merged",
            "--timeout",
            "10",
            "--json",
        ],
    )
    .await;
    assert_eq!(out.code, exit::TIMEOUT, "{out:?}");
    has_keys(&out, &["outcome", "record", "events", "reset"]);
    gold_json(&h, "json_wait_timeout", &out);
    let out = run(&h, &["notify", PR1, "--until", "checks-done", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert_eq!(json(&out)["registered"], true);
    gold_json(&h, "json_notify_registered", &out);
    let out = run(&h, &["notify", PR1, "--cancel", "--json"]).await;
    assert_eq!(json(&out)["cancelled"], 1);
    gold_json(&h, "json_notify_cancelled", &out);
    let _ = pr;
    h.github().world(|w| w.finish(chk, "FAILURE"));
    adv(&h, 40).await;
    let out = run(&h, &["notify", PR1, "--until", "checks-done", "--json"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert_eq!(json(&out)["registered"], false);
    gold_json(&h, "json_notify_already", &out);
    h.finish().await;
}

// TST-7: CLI-5, NTF-11: `notify` exits 0 whatever the checks did.
#[tokio::test]
async fn cli5_notify_exits_0_whatever_the_checks_did() {
    let (h, (_, chk)) = start(Opts::default(), notify_world).await;
    adv(&h, 20).await;
    h.github().world(|w| w.finish(chk, "FAILURE"));
    adv(&h, 40).await;
    let out = run(&h, &["notify", PR1, "--until", "checks-done"]).await;
    assert_eq!(out.code, exit::OK, "{out:?}");
    assert!(out.stdout.contains("already: checks failed"), "{out:?}");
    h.finish().await;
}

// TST-7: CLI-5: code 4 for `merged` under `checks-done`.
#[tokio::test]
async fn cli5_exit_4_merged_for_checks_done() {
    let (h, (pr, _)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(&h, &[&["wait", PR1, "--until", "checks-done"]], |w| {
        w.merge(pr)
    })
    .await;
    assert_eq!(outs[0].code, exit::ENDED, "{:?}", outs[0]);
    assert!(
        outs[0].stdout.contains("\nended: merged\n"),
        "{:?}",
        outs[0]
    );
    h.finish().await;
}

// TST-7: CLI-5: code 4 for `merged` under `changed --kind` (D8).
#[tokio::test]
async fn cli5_exit_4_merged_under_kind() {
    let (h, (pr, _)) = start(Opts::default(), running_world).await;
    adv(&h, 20).await;
    let outs = waits(
        &h,
        &[&["wait", PR1, "--until", "changed", "--kind", "review"]],
        |w| w.merge(pr),
    )
    .await;
    assert_eq!(outs[0].code, exit::ENDED, "{:?}", outs[0]);
    assert!(
        outs[0].stdout.contains("\nended: merged\n"),
        "{:?}",
        outs[0]
    );
    h.finish().await;
}
