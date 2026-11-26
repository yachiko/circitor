#![cfg(feature = "test-endpoint")]
//! T30: TST-8's scenarios about checks and settling, run on the harness (T29) against the fake
//! GitHub. Each is a scripted world on the test clock, looked at through the CLI's `--json` output
//! and the fake's request log, and each ends with `h.finish()` (TST-11).
//!
//! Times are offsets from the start of the clock: `go(&h, 40)` moves it to 40 s. GitHub's clock and
//! the test clock start together, so a world changed `after(s(8))` shows at offset 8. With OPS-2's
//! defaults the first poll finds the PRs at 0 and refreshes them at 1 (POL-2's gap), a poll comes
//! every 30 s and a hot PR is refreshed every 10 s (POL-13, POL-14).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use circitor::api::{ChangesBody, Envelope, ListBody, Outcome, RecordBody, WaitBody};
use circitor::cli::Output;
use circitor::clock::{Clock, Mono};
use circitor::model::{CheckKind, ChecksState, Event, EventKind, Phase, Record, Status};
use circitor::testkit::github::{AskedOf, PrId, RunId, World};
use circitor::testkit::harness::{Bg, Harness, Opts};
use circitor::testkit::server::{Aim, Fault, Times};

/// The viewer's login in every world.
const LOGIN: &str = "octocat";
/// MOD-6: the settle time of a list with checks, of an empty one, and the margin of the shortcut (proposal).
const SETTLE_WITH_CHECKS: Duration = Duration::from_secs(20);
const SETTLE_EMPTY: Duration = Duration::from_secs(60);
const SETTLE_MARGIN: Duration = Duration::from_secs(5);
/// POL-8: one page of checks, and how many are read at most (proposal).
const CHECK_PAGE: usize = 100;
const CHECKS_READ_MAX: usize = 300;
/// A settled PR's `updatedAt` is this old at its first read: over `SETTLE_EMPTY + SETTLE_MARGIN`.
const OLD: Duration = Duration::from_secs(120);
/// A PR whose `updatedAt` is too young to settle at once, with or without checks.
const YOUNG: Duration = Duration::from_secs(10);
/// The poll interval of OPS-2.
const POLL: u64 = 30;
/// The PR every world is about.
const PR1: &str = "octo/app#1";

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The offset `secs` from the start of the clock.
fn at(secs: u64) -> Mono {
    Mono::ZERO.plus(s(secs))
}

/// A 40-character SHA that starts with `prefix`.
fn sha(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

/// The first seven characters, as the outcome lines print them.
fn short(sha: &str) -> &str {
    &sha[..7]
}

// ---- the world ----

/// A PR in `octo/app`, authored by the viewer, with `updatedAt` `age` before now and no checks.
fn pr_aged(w: &mut World, age: Duration) -> PrId {
    let repo = w.repo("octo", "app");
    let pr = w.open_pr(repo);
    let now = w.now();
    w.set_created_at(pr, now.minus(s(3600)));
    w.set_updated_at(pr, now.minus(age));
    pr
}

/// A settled PR (MOD-6): `updatedAt` two minutes old, so its first read settles it at once.
fn settled_pr(w: &mut World) -> PrId {
    assert!(OLD > SETTLE_EMPTY + SETTLE_MARGIN);
    pr_aged(w, OLD)
}

/// A check run named `name`, completed with `conclusion`.
fn done_run(w: &mut World, pr: PrId, name: &str, conclusion: &str) -> RunId {
    let run = w.run(pr, name);
    w.finish(run, conclusion);
    run
}

/// A running one.
fn running_run(w: &mut World, pr: PrId, name: &str) -> RunId {
    let run = w.run(pr, name);
    w.start(run);
    run
}

/// A settled PR with `n` passed checks, then each of `tail` as (count, status, conclusion).
fn bulk(w: &mut World, n: usize, tail: &[(usize, &str, Option<&str>)]) -> (PrId, Vec<RunId>) {
    let pr = settled_pr(w);
    let mut runs = w.runs(pr, n, "COMPLETED", Some("SUCCESS"));
    for (count, status, conclusion) in tail {
        runs.extend(w.runs(pr, *count, status, *conclusion));
    }
    (pr, runs)
}

/// Starts the harness on a world `f` scripts, and hands back what `f` returned.
async fn start_with<T: Send + 'static>(
    opts: Opts,
    f: impl FnOnce(&mut World) -> T + Send + 'static,
) -> (Harness, T) {
    let slot = Arc::new(Mutex::new(None));
    let put = slot.clone();
    let h = Harness::start(opts.world(move |w| {
        w.set_login(LOGIN);
        *put.lock().unwrap() = Some(f(w));
    }))
    .await;
    let made = slot.lock().unwrap().take().expect("the world was scripted");
    assert_eq!(h.clock().mono(), Mono::ZERO);
    (h, made)
}

async fn start<T: Send + 'static>(
    f: impl FnOnce(&mut World) -> T + Send + 'static,
) -> (Harness, T) {
    start_with(Opts::default(), f).await
}

fn node(h: &Harness, pr: PrId) -> String {
    h.github().world(|w| w.node_id(pr).to_string())
}

// ---- the clock ----

/// Moves the clock to `secs` after the start.
async fn go(h: &Harness, secs: u64) {
    let now = h.clock().mono();
    assert!(at(secs) >= now, "go({secs}): the clock is at {now:?}");
    h.clock().advance(at(secs).since(now)).await;
}

/// Runs a command that doesn't wait on GitHub: it must be done once the clock has settled.
async fn run(h: &Harness, args: &[&str]) -> Output {
    let bg = h.cli_bg(args);
    h.clock().settle().await;
    assert!(bg.is_done(), "{args:?} needs the clock");
    bg.await
}

/// Runs a command that may need a request (POL-2's gap), moving the clock by 100 ms until it's done.
async fn drive(h: &Harness, args: &[&str]) -> Output {
    let bg = h.cli_bg(args);
    for _ in 0..50 {
        h.clock().settle().await;
        if bg.is_done() {
            return bg.await;
        }
        h.clock().advance(ms(100)).await;
    }
    panic!("{args:?} didn't end within 5 s of the clock");
}

/// A `wait` run twice, as text and as `--json`: the same waiter twice, so one world gives both.
struct Wait {
    text: Bg,
    json: Bg,
}

fn wait(h: &Harness, args: &[&str]) -> Wait {
    let mut json_args = args.to_vec();
    json_args.push("--json");
    Wait {
        text: h.cli_bg(args),
        json: h.cli_bg(&json_args),
    }
}

impl Wait {
    fn is_done(&self) -> bool {
        self.text.is_done() && self.json.is_done()
    }

    /// Both ends: the text's output, and the JSON's body.
    async fn end(self) -> (Output, Envelope<WaitBody>) {
        let text = self.text.await;
        let json = self.json.await;
        assert_eq!(text.code, json.code, "{text:?} {json:?}");
        let body = decode(&json);
        (text, body)
    }
}

/// Moves the clock a second at a time until the wait is done (at most `limit` seconds); the clock then.
async fn until_done(h: &Harness, w: &Wait, limit: u64) -> Mono {
    for _ in 0..limit {
        if w.is_done() {
            break;
        }
        h.clock().advance(s(1)).await;
    }
    assert!(
        w.is_done(),
        "the wait wasn't done at {:?}",
        h.clock().mono()
    );
    h.clock().mono()
}

/// A command that must finish at once, whose `--json` output is `T`.
fn decode<T: serde::de::DeserializeOwned>(out: &Output) -> T {
    serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

// ---- looking ----

/// `show <pr> --json` (and `--fresh`): the record, with its items.
async fn show(h: &Harness, pr: &str, fresh: bool) -> Record {
    let mut args = vec!["show", pr, "--json"];
    if fresh {
        args.push("--fresh");
    }
    let out = drive(h, &args).await;
    assert_eq!(out.code, 0, "{out:?}");
    let body: Envelope<RecordBody> = decode(&out);
    body.body.record.expect("a record")
}

/// `list --json`: every record, without items.
async fn list(h: &Harness) -> Envelope<ListBody> {
    let out = run(h, &["list", "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    decode(&out)
}

async fn list_one(h: &Harness, pr: &str) -> Record {
    list(h)
        .await
        .body
        .records
        .into_iter()
        .find(|r| r.pr.to_string() == pr)
        .unwrap_or_else(|| panic!("{pr} isn't in the list"))
}

/// `changes <pr> --json`, with `--since` when given.
async fn changes(h: &Harness, pr: &str, since: Option<&str>) -> Envelope<ChangesBody> {
    let mut args = vec!["changes", pr, "--json"];
    if let Some(c) = since {
        args.extend(["--since", c]);
    }
    let out = run(h, &args).await;
    assert_eq!(out.code, 0, "{out:?}");
    decode(&out)
}

/// The cursor now, for `changes --since`.
async fn cursor(h: &Harness, pr: &str) -> String {
    changes(h, pr, None).await.cursor
}

fn kinds(e: &Event) -> Vec<&'static str> {
    e.kinds.iter().map(EventKind::word).collect()
}

fn has_kind(e: &Event, word: &str) -> bool {
    kinds(e).contains(&word)
}

/// The send times of a PR's refreshes, oldest first: the requests that read its record.
fn refreshes(h: &Harness, id: &str) -> Vec<Mono> {
    h.github()
        .requests()
        .iter()
        .filter(|r| {
            r.nodes.iter().any(|(n, asked)| {
                n == id && matches!(asked, AskedOf::Record | AskedOf::RecordAndItems)
            })
        })
        .map(|r| r.mono)
        .collect()
}

/// The latest refresh sent by `now`, or none yet.
fn latest(h: &Harness, id: &str, now: Mono) -> Option<Mono> {
    refreshes(h, id).into_iter().rfind(|m| *m <= now)
}

/// The first refresh sent at or after `t`.
fn first_at_or_after(h: &Harness, id: &str, t: Mono) -> Option<Mono> {
    refreshes(h, id).into_iter().find(|m| *m >= t)
}

/// MOD-6: a sample of `list` where the PR reads `ready` exactly when the latest refresh sent by
/// now is at least `settle` after the settle start `from`, and `checks_pending` otherwise. Says
/// whether the clock is past `from + settle` while the latest refresh isn't yet.
async fn sample_law(h: &Harness, id: &str, from: Mono, settle: Duration) -> bool {
    let now = h.clock().mono();
    let rec = list_one(h, PR1).await;
    let last = latest(h, id, now).expect("a refresh was sent");
    let due = last.since(from) >= settle && last >= from;
    let want = if due {
        Status::Ready
    } else {
        Status::ChecksPending
    };
    assert_eq!(
        rec.status, want,
        "at {now:?}: latest refresh {last:?}, settle start {from:?}, {rec:?}"
    );
    now.since(from) > settle && !due
}

fn outcome_line(out: &Output, line: &str) {
    assert!(
        out.stdout.lines().any(|l| l == line),
        "no line `{line}` in {out:?}"
    );
}

fn names(items: &[circitor::model::FailedItem]) -> Vec<&str> {
    items.iter().map(|i| i.name.as_str()).collect()
}

// ---- push-old-head ----

// TST-8: push-old-head
// API-10: after a push GitHub serves the old head with its green result for 3 s; a wait with
// `--sha` doesn't take it, one without does.
#[tokio::test]
async fn scn_push_old_head() {
    const P: u64 = 40;
    let (a, b) = (sha("dce9dbe"), sha("cf54ec4"));
    let (a_world, b_world) = (a.clone(), b.clone());
    let (h, pr) = start(move |w| {
        let pr = settled_pr(w);
        w.push_sha(pr, &a_world);
        done_run(w, pr, "fast", "SUCCESS");
        done_run(w, pr, "slow", "SUCCESS");
        let now = w.now();
        w.set_updated_at(pr, now.minus(OLD));
        w.at(now.plus(s(P)), move |w| {
            w.push_lagged(pr, Some(&b_world), s(3));
            w.after(s(3), move |w| w.set_mergeable(pr, "UNKNOWN"));
            w.after(s(6), move |w| {
                w.set_mergeable(pr, "MERGEABLE");
                let fast = w.run(pr, "fast");
                let slow = w.run(pr, "slow");
                w.after(s(3), move |w| {
                    w.start(fast);
                    w.start(slow);
                });
                w.after(s(9), move |w| w.finish(fast, "SUCCESS"));
                w.after(s(46), move |w| w.finish(slow, "SUCCESS"));
            });
        });
        pr
    })
    .await;
    let id = node(&h, pr);
    go(&h, P).await;
    assert_eq!(list_one(&h, PR1).await.status, Status::Ready);

    // Without `--sha`: it holds at once, on the old head.
    let first = wait(&h, &["wait", PR1, "--until", "checks-done"]);
    for _ in 0..50 {
        h.clock().settle().await;
        if first.is_done() {
            break;
        }
        h.clock().advance(ms(100)).await;
    }
    assert!(first.is_done());
    assert!(h.clock().mono().since(at(P)) <= s(1));
    let (text, body) = first.end().await;
    assert_eq!(text.code, 0, "{text:?}");
    outcome_line(&text, &format!("checks passed (2/2) on {}", short(&a)));
    assert_eq!(body.body.outcome, Outcome::Satisfied);
    assert_eq!(body.body.record.expect("a record").head_sha, a);

    // With it: not done at P+50, done after P+52.
    let second = wait(
        &h,
        &[
            "wait",
            PR1,
            "--until",
            "checks-done",
            "--sha",
            &b,
            "--timeout",
            "100",
        ],
    );
    go(&h, P + 50).await;
    assert!(!second.is_done());
    let done = until_done(&h, &second, 50).await;
    assert!(done > at(P + 52), "{done:?}");
    let last = latest(&h, &id, done).expect("a refresh");
    assert!(last >= at(P + 52), "{last:?}");
    let (text, body) = second.end().await;
    assert_eq!(text.code, 0, "{text:?}");
    assert_eq!(body.body.outcome, Outcome::Satisfied);
    let record = body.body.record.expect("a record");
    assert_eq!(record.head_sha, b);
    assert_eq!(record.checks.state, ChecksState::Passing);
    outcome_line(&text, &format!("checks passed (2/2) on {}", short(&b)));
    let moved =
        body.body.events.iter().flat_map(|e| &e.kinds).any(
            |k| matches!(k, EventKind::Head { from, to } if from == short(&a) && to == short(&b)),
        );
    assert!(moved, "{:?}", body.body.events);
    h.finish().await;
}

// ---- no-ci ----

/// A PR created a second before the start, with no checks ever.
fn no_ci_world(w: &mut World) -> PrId {
    pr_aged(w, s(1))
}

// TST-8: no-ci
// MOD-6: with no checks the PR is `ready` exactly at the first refresh sent 60 s after the settle
// start; between refreshes the clock alone settles nothing.
#[tokio::test]
async fn scn_no_ci_list() {
    let (h, pr) = start(no_ci_world).await;
    let id = node(&h, pr);
    go(&h, 2).await;
    let from = refreshes(&h, &id)[0];
    let mut late_pending = false;
    for t in (5..=120).step_by(5) {
        go(&h, t).await;
        let rec = list_one(&h, PR1).await;
        assert_eq!(rec.checks.state, ChecksState::None);
        late_pending |= sample_law(&h, &id, from, SETTLE_EMPTY).await;
    }
    assert!(
        late_pending,
        "no sample fell past the settle time before the next refresh: {:?}",
        refreshes(&h, &id)
    );
    assert_eq!(list_one(&h, PR1).await.status, Status::Ready);
    h.finish().await;
}

// TST-8: no-ci
// MOD-6, API-10: `checks-done` holds with `none`, at a refresh sent 60 s after the settle start
// and not before.
#[tokio::test]
async fn scn_no_ci_wait() {
    let (h, pr) = start(no_ci_world).await;
    let id = node(&h, pr);
    go(&h, 2).await;
    let from = refreshes(&h, &id)[0];
    let w = wait(&h, &["wait", PR1, "--until", "checks-done"]);
    let done = until_done(&h, &w, 99).await;
    let sent = refreshes(&h, &id);
    let last = *sent.iter().rfind(|m| **m <= done).unwrap();
    let before = *sent.iter().rfind(|m| **m < last).unwrap();
    assert!(last.since(from) >= SETTLE_EMPTY, "{sent:?}");
    assert!(before.since(from) < SETTLE_EMPTY, "{sent:?}");
    let (text, body) = w.end().await;
    assert_eq!(text.code, 0, "{text:?}");
    assert_eq!(body.body.outcome, Outcome::Satisfied);
    let record = body.body.record.expect("a record");
    assert_eq!(record.checks.state, ChecksState::None);
    assert!(record.checks.settled);
    outcome_line(
        &text,
        &format!("no checks reported on {}", short(&record.head_sha)),
    );
    h.finish().await;
}

// ---- first-check-early ----

// TST-8: first-check-early
// MOD-6, MOD-11: one commit status passes before the workflow's checks appear; the PR stays
// `checks_pending` and `checks-done` holds only when the last check has passed.
#[tokio::test]
async fn scn_first_check_early() {
    let (h, _) = start(|w| {
        // C is 3 s before the start: the status at C+3, the checks at C+8, C+15 and C+56.
        let pr = pr_aged(w, s(3));
        let now = w.now();
        w.set_created_at(pr, now.minus(s(3)));
        w.status(pr, "ci/external", "SUCCESS");
        w.after(s(5), move |w| {
            let fast = running_run(w, pr, "fast");
            let slow = running_run(w, pr, "slow");
            w.after(s(7), move |w| w.finish(fast, "SUCCESS"));
            w.after(s(48), move |w| w.finish(slow, "SUCCESS"));
        });
    })
    .await;
    go(&h, 1).await;
    let first = show(&h, PR1, false).await;
    assert_eq!(first.checks.state, ChecksState::Passing);
    assert!(!first.checks.settled);
    assert_eq!(first.checks.total, 1);
    assert_eq!(first.status, Status::ChecksPending);
    let out = run(
        &h,
        &[
            "wait",
            PR1,
            "--until",
            "checks-done",
            "--timeout",
            "0",
            "--json",
        ],
    )
    .await;
    assert_eq!(out.code, 3, "{out:?}");
    let body: Envelope<WaitBody> = decode(&out);
    assert_eq!(body.body.outcome, Outcome::Timeout);

    let w = wait(&h, &["wait", PR1, "--until", "checks-done"]);
    let mut t = 1;
    loop {
        t += 1;
        go(&h, t).await;
        if w.is_done() {
            break;
        }
        assert!(t < 100);
        assert_ne!(list_one(&h, PR1).await.status, Status::Ready, "at {t}");
        if t == 52 {
            assert!(!w.is_done());
        }
    }
    assert!(h.clock().mono() > at(53), "{:?}", h.clock().mono());
    let (text, body) = w.end().await;
    assert_eq!(text.code, 0, "{text:?}");
    assert_eq!(
        body.body.record.expect("a record").checks.state,
        ChecksState::Passing
    );
    outcome_line(
        &text,
        &format!(
            "checks passed (3/3) on {}",
            short(&show(&h, PR1, false).await.head_sha)
        ),
    );
    h.finish().await;
}

// ---- late-ci ----

/// A new PR, or a new head, whose two checks appear `appear` seconds after the creation or the push
/// (at the push, the new head shows from +3 s) and run 10 s. Until they pass no sample of `list`
/// reads `ready`, and a wait doesn't end a second before.
async fn late_ci(appear: u64, new_head: bool) {
    const P: u64 = 40;
    let b = sha("cf54ec4");
    let b_world = b.clone();
    let (h, pr) = start(move |w| {
        let (pr, base) = if new_head {
            let pr = settled_pr(w);
            done_run(w, pr, "old", "SUCCESS");
            let base = w.now().plus(s(P));
            w.at(base, move |w| {
                w.push_lagged(pr, Some(&b_world), s(3));
            });
            (pr, base)
        } else {
            let pr = pr_aged(w, s(0));
            (pr, w.now())
        };
        w.at(base.plus(s(appear)), move |w| {
            let one = running_run(w, pr, "one");
            let two = running_run(w, pr, "two");
            w.after(s(10), move |w| {
                w.finish(one, "SUCCESS");
                w.finish(two, "SUCCESS");
            });
        });
        pr
    })
    .await;
    let id = node(&h, pr);
    let (begin, pass) = if new_head {
        (P, P + appear + 10)
    } else {
        (2, appear + 10)
    };
    go(&h, begin).await;
    let timeout = ["--timeout", "120"];
    let waiting = if new_head {
        wait(
            &h,
            &[
                "wait",
                PR1,
                "--until",
                "checks-done",
                "--sha",
                short(&b),
                timeout[0],
                timeout[1],
            ],
        )
    } else {
        wait(
            &h,
            &[
                "wait",
                PR1,
                "--until",
                "checks-done",
                timeout[0],
                timeout[1],
            ],
        )
    };
    let mut t = begin;
    while t < pass - 1 {
        t += 1;
        go(&h, t).await;
        // The new head's first read is the first refresh that shows it (served from P+3).
        let read = !new_head || first_at_or_after(&h, &id, at(P + 3)).is_some();
        if read {
            assert_eq!(
                list_one(&h, PR1).await.status,
                Status::ChecksPending,
                "at {t}"
            );
        }
        assert!(!waiting.is_done(), "done at {t}, a second before they pass");
    }
    let done = until_done(&h, &waiting, 100).await;
    assert!(done >= at(pass), "{done:?}");
    let (text, body) = waiting.end().await;
    assert_eq!(text.code, 0, "{text:?}");
    let record = body.body.record.expect("a record");
    assert_eq!(record.checks.state, ChecksState::Passing);
    assert_eq!(record.checks.total, 2);
    h.finish().await;
}

// TST-8: late-ci
// MOD-6, MOD-11: a new PR's checks appear at C+6.
#[tokio::test]
async fn scn_late_ci_new_pr_6() {
    late_ci(6, false).await;
}

// TST-8: late-ci
// MOD-6, MOD-11: a new PR's checks appear at C+40, which is why an empty list takes 60 s to settle.
#[tokio::test]
async fn scn_late_ci_new_pr_40() {
    late_ci(40, false).await;
}

// TST-8: late-ci
// MOD-6, MOD-11: a new head's checks appear at P+6.
#[tokio::test]
async fn scn_late_ci_new_head_6() {
    late_ci(6, true).await;
}

// TST-8: late-ci
// MOD-6, MOD-11: a new head's checks appear at P+40.
#[tokio::test]
async fn scn_late_ci_new_head_40() {
    late_ci(40, true).await;
}

// ---- one-fails-others-run ----

// TST-8: one-fails-others-run
// MOD-5, CLI-5: one check fails while another runs for 78 s; the state stays `pending` with
// `failed: 1`, `checks-done` holds when the last finishes, and the CLI exits 8.
#[tokio::test]
async fn scn_one_fails_others_run() {
    let (h, pr) = start(|w| {
        // C is the start; c1.log's checks appear at C+8, two settle at C+11, `slow` at C+89.
        let pr = pr_aged(w, s(0));
        w.after(s(8), move |w| {
            let pass = running_run(w, pr, "pass");
            let fail = running_run(w, pr, "fail");
            let slow = running_run(w, pr, "slow");
            let skipped = w.run(pr, "skipped");
            w.set_run(skipped, "COMPLETED", Some("SKIPPED"));
            w.after(s(3), move |w| {
                w.finish(pass, "SUCCESS");
                w.finish(fail, "FAILURE");
            });
            w.after(s(81), move |w| w.finish(slow, "SUCCESS"));
        });
        pr
    })
    .await;
    let id = node(&h, pr);
    go(&h, 2).await;
    let waiting = wait(
        &h,
        &["wait", PR1, "--until", "checks-done", "--timeout", "120"],
    );
    let mut looked = 0;
    for t in 12..=88 {
        go(&h, t).await;
        assert!(!waiting.is_done(), "done at {t}");
        // The record shows the failure from the first refresh sent at or after C+11, which the
        // cadence decides: look from then on.
        let shown = first_at_or_after(&h, &id, at(11)).is_some_and(|m| m <= h.clock().mono());
        if shown && matches!(t, 12 | 25 | 50 | 75 | 88) {
            looked += 1;
            let rec = show(&h, PR1, false).await;
            assert_eq!(rec.checks.state, ChecksState::Pending, "at {t}");
            assert_eq!(rec.checks.failed, 1, "at {t}");
            assert_eq!(names(&rec.checks.failed_items), ["fail"], "at {t}");
            assert_eq!(rec.status, Status::ChecksPending, "at {t}");
        }
    }
    assert!(
        looked >= 4,
        "looked {looked} times: {:?}",
        refreshes(&h, &id)
    );
    let done = until_done(&h, &waiting, 40).await;
    assert!(done >= at(89), "{done:?}");
    let (text, body) = waiting.end().await;
    assert_eq!(text.code, 8, "{text:?}");
    assert_eq!(body.body.outcome, Outcome::Satisfied);
    let record = body.body.record.expect("a record");
    assert_eq!(record.checks.state, ChecksState::Failing);
    assert_eq!(
        (
            record.checks.total,
            record.checks.completed,
            record.checks.failed
        ),
        (4, 4, 1)
    );
    outcome_line(
        &text,
        &format!("checks failed (1 of 4) on {}", short(&record.head_sha)),
    );
    h.finish().await;
}

// ---- fail-fast ----

/// A settled PR whose `fail` and `slow` run at the start; `fail` ends with `fail_conclusion` at 10 s
/// and `slow` passes 78 s later (c1.log).
fn fail_fast_world(w: &mut World, fail_conclusion: &'static str, slow_after: u64) -> PrId {
    let pr = settled_pr(w);
    let fail = running_run(w, pr, "fail");
    let slow = running_run(w, pr, "slow");
    w.after(s(10), move |w| w.finish(fail, fail_conclusion));
    w.after(s(slow_after), move |w| w.finish(slow, "SUCCESS"));
    pr
}

// TST-8: fail-fast
// API-10, CLI-5, CLI-6, D8: `--fail-fast` answers at the first refresh that shows the failure,
// without waiting for the last check; without it the wait runs to the end.
#[tokio::test]
async fn scn_fail_fast() {
    const F: u64 = 10;
    let (h, pr) = start(|w| fail_fast_world(w, "FAILURE", 88)).await;
    let id = node(&h, pr);
    go(&h, 2).await;
    let fast = wait(
        &h,
        &[
            "wait",
            PR1,
            "--until",
            "checks-done",
            "--fail-fast",
            "--timeout",
            "120",
        ],
    );
    let plain = wait(
        &h,
        &["wait", PR1, "--until", "checks-done", "--timeout", "120"],
    );
    go(&h, F - 1).await;
    assert!(!fast.is_done());
    let done = until_done(&h, &fast, 60).await;
    let sent = refreshes(&h, &id);
    let shows = *sent
        .iter()
        .find(|m| **m >= at(F))
        .expect("a refresh at or after F");
    assert!(done >= shows, "{done:?} before {shows:?}");
    assert!(
        !sent.iter().any(|m| *m > shows && *m <= done),
        "a later refresh came first: {sent:?}"
    );
    assert!(!plain.is_done());
    let (text, body) = fast.end().await;
    assert_eq!(text.code, 8, "{text:?}");
    assert_eq!(body.body.outcome, Outcome::Satisfied);
    let record = body.body.record.expect("a record");
    assert_eq!(record.checks.state, ChecksState::Pending);
    assert_eq!(
        (
            record.checks.failed,
            record.checks.completed,
            record.checks.total
        ),
        (1, 1, 2)
    );
    outcome_line(
        &text,
        &format!(
            "checks failing (1 failed, 1 of 2 done) on {}",
            short(&record.head_sha)
        ),
    );

    go(&h, F + 77).await;
    assert!(!plain.is_done());
    let done = until_done(&h, &plain, 60).await;
    assert!(done > at(F + 78), "{done:?}");
    let (text, body) = plain.end().await;
    assert_eq!(text.code, 8, "{text:?}");
    let record = body.body.record.expect("a record");
    assert_eq!(record.checks.state, ChecksState::Failing);
    outcome_line(
        &text,
        &format!("checks failed (1 of 2) on {}", short(&record.head_sha)),
    );
    h.finish().await;
}

// TST-8: fail-fast
// API-10, CLI-6: when both checks pass `--fail-fast` exits 0, with `checks passed (2/2)`, at the
// same refresh as without it.
#[tokio::test]
async fn scn_fail_fast_both_pass() {
    let (h, _) = start(|w| fail_fast_world(w, "SUCCESS", 30)).await;
    go(&h, 2).await;
    let fast = wait(
        &h,
        &[
            "wait",
            PR1,
            "--until",
            "checks-done",
            "--fail-fast",
            "--timeout",
            "120",
        ],
    );
    let plain = wait(
        &h,
        &["wait", PR1, "--until", "checks-done", "--timeout", "120"],
    );
    let mut done = None;
    for _ in 0..100 {
        h.clock().advance(s(1)).await;
        if fast.is_done() || plain.is_done() {
            done = Some(h.clock().mono());
            break;
        }
    }
    let done = done.expect("a wait ended");
    assert!(
        fast.is_done() && plain.is_done(),
        "they ended at different refreshes"
    );
    assert!(done >= at(30), "{done:?}");
    let (fast_text, fast_body) = fast.end().await;
    let (plain_text, plain_body) = plain.end().await;
    for text in [&fast_text, &plain_text] {
        assert_eq!(text.code, 0, "{text:?}");
        outcome_line(
            text,
            &format!(
                "checks passed (2/2) on {}",
                short(&fast_body.body.record.as_ref().unwrap().head_sha)
            ),
        );
    }
    assert_eq!(
        fast_body.body.record.unwrap().fetched_at,
        plain_body.body.record.unwrap().fetched_at
    );
    h.finish().await;
}

// ---- skipped-and-cancelled ----

// TST-8: skipped-and-cancelled
// MOD-5: a skipped check passes; a cancelled one fails.
#[tokio::test]
async fn scn_skipped_and_cancelled() {
    let (h, _) = start(|w| {
        let skipped = settled_pr(w);
        done_run(w, skipped, "pass", "SUCCESS");
        done_run(w, skipped, "skipped", "SKIPPED");
        let cancelled = settled_pr(w);
        done_run(w, cancelled, "pass", "SUCCESS");
        let slow = running_run(w, cancelled, "slow");
        w.after(s(16), move |w| {
            w.set_run(slow, "COMPLETED", Some("CANCELLED"))
        });
    })
    .await;
    go(&h, POLL + 5).await;

    let ok = show(&h, "octo/app#1", false).await;
    assert_eq!(ok.checks.state, ChecksState::Passing);
    assert_eq!(ok.checks.failed, 0);
    assert_eq!(ok.status, Status::Ready);
    let out = run(
        &h,
        &[
            "wait",
            "octo/app#1",
            "--until",
            "checks-done",
            "--timeout",
            "0",
        ],
    )
    .await;
    assert_eq!(out.code, 0, "{out:?}");

    let bad = show(&h, "octo/app#2", false).await;
    assert_eq!(bad.checks.state, ChecksState::Failing);
    assert_eq!(bad.checks.failed, 1);
    assert_eq!(names(&bad.checks.failed_items), ["slow"]);
    assert_eq!(bad.status, Status::ChecksFailing);
    let out = run(
        &h,
        &[
            "wait",
            "octo/app#2",
            "--until",
            "checks-done",
            "--timeout",
            "0",
        ],
    )
    .await;
    assert_eq!(out.code, 8, "{out:?}");
    h.finish().await;
}

// ---- commit-status ----

// TST-8: commit-status
// MOD-3, MOD-4: a commit status goes pending, then failure, in the same list as the check runs;
// posted again as pending it reads `in_progress` again.
#[tokio::test]
async fn scn_commit_status() {
    const S: u64 = 10;
    let (h, _) = start(|w| {
        let pr = settled_pr(w);
        done_run(w, pr, "pass", "SUCCESS");
        let now = w.now();
        w.at(now.plus(s(S)), move |w| {
            w.status(pr, "circitor/manual", "PENDING")
        });
        w.at(now.plus(s(S + 16)), move |w| {
            w.status(pr, "circitor/manual", "FAILURE")
        });
        w.at(now.plus(s(S + 40)), move |w| {
            w.status(pr, "circitor/manual", "PENDING")
        });
    })
    .await;
    go(&h, 5).await;
    let before = show(&h, PR1, false).await;
    assert_eq!(before.status, Status::Ready);
    let c = cursor(&h, PR1).await;

    let item = |rec: &Record| {
        let items = rec.checks.items.as_ref().expect("items");
        let run_item = items.iter().find(|i| i.name == "pass").expect("the run");
        assert_eq!(run_item.kind, CheckKind::Run);
        assert!(run_item.id.is_some());
        let status = items
            .iter()
            .find(|i| i.name == "circitor/manual")
            .expect("the status");
        assert_eq!(status.kind, CheckKind::Status);
        assert_eq!(status.id, None);
        assert_eq!(items.len(), 2);
        status.clone()
    };

    go(&h, S + 6).await;
    let pending = show(&h, PR1, true).await;
    let status = item(&pending);
    assert_eq!(
        (status.phase, status.result.as_deref()),
        (Phase::InProgress, None)
    );
    assert_eq!(pending.checks.state, ChecksState::Pending);

    go(&h, S + 18).await;
    let failed = show(&h, PR1, true).await;
    let status = item(&failed);
    assert_eq!(
        (status.phase, status.result.as_deref()),
        (Phase::Completed, Some("failure"))
    );
    assert_eq!(failed.checks.state, ChecksState::Failing);
    assert_eq!(names(&failed.checks.failed_items), ["circitor/manual"]);

    go(&h, S + 42).await;
    let again = show(&h, PR1, true).await;
    let status = item(&again);
    assert_eq!(
        (status.phase, status.result.as_deref()),
        (Phase::InProgress, None)
    );
    assert_eq!(again.checks.state, ChecksState::Pending);

    let events = changes(&h, PR1, Some(&c)).await.body.events;
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(events.iter().all(|e| has_kind(e, "checks")), "{events:?}");
    h.finish().await;
}

// ---- check-out-of-order ----

// TST-8: check-out-of-order
// MOD-4, MOD-14: a check read as running and then as queued makes no event, and `checks.state`
// stays `pending`; each read is believed.
#[tokio::test]
async fn scn_check_out_of_order() {
    let (h, (fast, slow)) = start(|w| {
        let pr = settled_pr(w);
        (running_run(w, pr, "fast"), running_run(w, pr, "slow"))
    })
    .await;
    go(&h, 10).await;
    let first = show(&h, PR1, true).await;
    assert_eq!(first.checks.state, ChecksState::Pending);
    let c = cursor(&h, PR1).await;

    go(&h, 13).await;
    h.github().world(|w| {
        w.misread(fast, "QUEUED");
        w.misread(slow, "QUEUED");
    });
    go(&h, 16).await;
    let second = show(&h, PR1, true).await;
    let items = second.checks.items.as_ref().expect("items");
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i.phase == Phase::Queued), "{items:?}");
    assert_eq!(second.checks.state, ChecksState::Pending);
    assert_eq!(second.rev, first.rev);
    let moved = changes(&h, PR1, Some(&c)).await;
    assert!(moved.body.events.is_empty(), "{:?}", moved.body.events);
    assert_eq!(moved.cursor, c);

    // The next read is as it is: running again, still no event.
    go(&h, 27).await;
    let third = show(&h, PR1, false).await;
    let items = third.checks.items.as_ref().expect("items");
    assert!(
        items.iter().all(|i| i.phase == Phase::InProgress),
        "{items:?}"
    );
    assert_eq!(third.rev, first.rev);
    assert!(changes(&h, PR1, Some(&c)).await.body.events.is_empty());
    h.finish().await;
}

// ---- settled-at-once ----

/// Four settled-at-once worlds: a PR with `fast` and `slow` passed.
fn passed_pair(w: &mut World, age: Duration) -> PrId {
    let pr = pr_aged(w, age);
    done_run(w, pr, "fast", "SUCCESS");
    done_run(w, pr, "slow", "SUCCESS");
    pr
}

// TST-8: settled-at-once
// MOD-6: `updatedAt` two minutes old: `ready` and settled at the first refresh, also after a restart.
#[tokio::test]
async fn scn_settled_at_once_old() {
    let (h, pr) = start(|w| passed_pair(w, OLD)).await;
    let id = node(&h, pr);
    go(&h, 10).await;
    let rec = list_one(&h, PR1).await;
    assert_eq!(rec.status, Status::Ready);
    assert!(rec.checks.settled);
    assert_eq!(refreshes(&h, &id).len(), 1);
    let old = list(&h).await.cursor;

    h.restart().await;
    h.clock().advance(s(10)).await;
    let rec = list_one(&h, PR1).await;
    assert_eq!(rec.status, Status::Ready);
    assert!(rec.checks.settled);
    let events = changes(&h, PR1, Some(&old)).await.body.events;
    assert!(events.iter().any(|e| has_kind(e, "tracked")), "{events:?}");
    assert!(events.iter().all(|e| !has_kind(e, "status")), "{events:?}");
    h.finish().await;
}

/// A PR that doesn't settle at its first read reads `checks_pending` until a refresh sent
/// `SETTLE_WITH_CHECKS` after its settle start, and `ready` from that one.
async fn settles_late(h: &Harness, pr: PrId) {
    let id = node(h, pr);
    go(h, 2).await;
    let from = refreshes(h, &id)[0];
    let (mut pending, mut ready) = (false, false);
    for t in (5..=70).step_by(5) {
        go(h, t).await;
        sample_law(h, &id, from, SETTLE_WITH_CHECKS).await;
        match list_one(h, PR1).await.status {
            Status::ChecksPending => pending = true,
            Status::Ready => ready = true,
            other => panic!("{other:?}"),
        }
    }
    assert!(pending && ready, "pending {pending}, ready {ready}");
}

// TST-8: settled-at-once
// MOD-6: `updatedAt` 10 s old reads `checks_pending` until a refresh sent 20 s after the settle start.
#[tokio::test]
async fn scn_settled_at_once_young() {
    assert!(YOUNG < SETTLE_WITH_CHECKS + SETTLE_MARGIN);
    let (h, pr) = start(|w| passed_pair(w, YOUNG)).await;
    settles_late(&h, pr).await;
    h.finish().await;
}

// TST-8: settled-at-once
// MOD-6: with no `Date` header a response never settles at once.
#[tokio::test]
async fn scn_settled_at_once_no_date() {
    let opts = Opts::default().github(|gh| gh.without_date_header());
    let (h, pr) = start_with(opts, |w| passed_pair(w, OLD)).await;
    settles_late(&h, pr).await;
    h.finish().await;
}

// TST-8: settled-at-once
// MOD-6: GitHub's clock 5 minutes ahead of the local one, `updatedAt` 10 s old by GitHub's clock:
// the age is taken from `Date`, never from the local clock.
#[tokio::test]
async fn scn_settled_at_once_skew() {
    let ahead = s(300);
    let opts = Opts::default().skew_ahead(ahead);
    let (h, pr) = start_with(opts, move |w| {
        let pr = pr_aged(w, s(0));
        let github_now = w.now().plus(ahead);
        w.set_updated_at(pr, github_now.minus(YOUNG));
        done_run(w, pr, "fast", "SUCCESS");
        done_run(w, pr, "slow", "SUCCESS");
        pr
    })
    .await;
    settles_late(&h, pr).await;
    h.finish().await;
}

// ---- unlisted-values ----

// TST-8: unlisted-values
// MOD-2, MOD-5: a check concluding `STARTUP_FAILURE`, or an unknown `FOO`, reads `failing`, and
// the record decodes.
#[tokio::test]
async fn scn_unlisted_values() {
    let (h, _) = start(|w| {
        let pr = settled_pr(w);
        done_run(w, pr, "pass", "SUCCESS");
        done_run(w, pr, "boot", "STARTUP_FAILURE");
        done_run(w, pr, "odd", "FOO");
    })
    .await;
    go(&h, 5).await;
    let out = run(&h, &["show", PR1, "--json"]).await;
    assert_eq!(out.code, 0, "{out:?}");
    let body: Envelope<RecordBody> = decode(&out);
    let rec = body.body.record.expect("a record");
    let items = rec.checks.items.as_ref().expect("items");
    let result = |name: &str| {
        let item = items.iter().find(|i| i.name == name).expect(name);
        assert_eq!(item.phase, Phase::Completed);
        item.result.clone()
    };
    assert_eq!(result("boot").as_deref(), Some("startup_failure"));
    assert_eq!(result("odd").as_deref(), Some("foo"));
    assert_eq!(rec.checks.state, ChecksState::Failing);
    assert_eq!(rec.checks.failed, 2);
    let mut failed = names(&rec.checks.failed_items);
    failed.sort_unstable();
    assert_eq!(failed, ["boot", "odd"]);
    assert_eq!(rec.status, Status::ChecksFailing);
    let out = run(
        &h,
        &["wait", PR1, "--until", "checks-done", "--timeout", "0"],
    )
    .await;
    assert_eq!(out.code, 8, "{out:?}");
    h.finish().await;
}

// ---- head-back ----

// TST-8: head-back
// MOD-6: a force-push back to the previous SHA, whose checks passed, reads `checks_pending` until
// settled again, 20 s after the refresh that showed it.
#[tokio::test]
async fn scn_head_back() {
    const P: u64 = 40;
    let (a, b) = (sha("dce9dbe"), sha("cf54ec4"));
    let (a_world, b_world) = (a.clone(), b.clone());
    let (h, pr) = start(move |w| {
        let pr = settled_pr(w);
        w.push_sha(pr, &a_world);
        done_run(w, pr, "fast", "SUCCESS");
        done_run(w, pr, "slow", "SUCCESS");
        let now = w.now();
        w.set_updated_at(pr, now.minus(OLD));
        let (a_back, b_push) = (a_world.clone(), b_world.clone());
        w.at(now.plus(s(P)), move |w| {
            w.push_lagged(pr, Some(&b_push), s(3));
        });
        w.at(now.plus(s(P + 12)), move |w| {
            w.push_lagged(pr, Some(&a_back), s(3));
        });
        pr
    })
    .await;
    let id = node(&h, pr);
    go(&h, 35).await;
    assert_eq!(list_one(&h, PR1).await.status, Status::Ready);
    let c = cursor(&h, PR1).await;

    // Both heads are read: B at P+5 (its hot refresh then falls at P+15, which shows A again).
    go(&h, P + 5).await;
    let on_b = show(&h, PR1, true).await;
    assert_eq!(on_b.head_sha, b);
    go(&h, P + 16).await;
    let back = show(&h, PR1, true).await;
    assert_eq!(back.head_sha, a);
    assert_eq!(back.checks.state, ChecksState::Passing);
    assert!(!back.checks.settled);
    assert_eq!(back.status, Status::ChecksPending);

    let shown = first_at_or_after(&h, &id, at(P + 15)).expect("a refresh that showed A again");
    let mut ready = false;
    for t in ((P + 20)..=(P + 100)).step_by(5) {
        go(&h, t).await;
        let rec = list_one(&h, PR1).await;
        let last = latest(&h, &id, h.clock().mono()).expect("a refresh");
        let want = if last.since(shown) >= SETTLE_WITH_CHECKS {
            Status::Ready
        } else {
            Status::ChecksPending
        };
        assert_eq!(
            rec.status, want,
            "at {t}: latest refresh {last:?}, A shown at {shown:?}"
        );
        ready |= rec.status == Status::Ready;
    }
    assert!(ready);
    let heads: Vec<(String, String)> = changes(&h, PR1, Some(&c))
        .await
        .body
        .events
        .iter()
        .flat_map(|e| e.kinds.clone())
        .filter_map(|k| match k {
            EventKind::Head { from, to } => Some((from, to)),
            _ => None,
        })
        .collect();
    assert_eq!(
        heads,
        [
            (short(&a).to_string(), short(&b).to_string()),
            (short(&b).to_string(), short(&a).to_string())
        ]
    );
    h.finish().await;
}

// ---- many-checks ----

/// The requests that name the PR in its first cycle, before the second poll: its refresh and each
/// further page of checks.
fn first_cycle(h: &Harness, id: &str) -> Vec<(Mono, AskedOf)> {
    h.github()
        .requests()
        .iter()
        .filter(|r| r.mono < at(POLL - 5))
        .flat_map(|r| {
            r.nodes
                .iter()
                .filter(|(n, _)| n == id)
                .map(|(_, a)| (r.mono, *a))
        })
        .collect()
}

/// POL-8, MOD-7: `n` passed checks on a settled PR are read in pages of `CHECK_PAGE`, up to
/// `CHECKS_READ_MAX`; more than that are `truncated`, with GitHub's `total`.
async fn pages(n: usize) {
    let (h, (pr, _)) = start(move |w| bulk(w, n, &[])).await;
    let id = node(&h, pr);
    go(&h, 5).await;
    let requests = first_cycle(&h, &id);
    let want = n.min(CHECKS_READ_MAX).div_ceil(CHECK_PAGE);
    assert_eq!(requests.len(), want, "{requests:?}");
    let rec = show(&h, PR1, false).await;
    let read = n.min(CHECKS_READ_MAX);
    assert_eq!(rec.checks.total as usize, n);
    assert_eq!(rec.checks.truncated, n > CHECKS_READ_MAX);
    assert_eq!(rec.checks.items.as_ref().expect("items").len(), read);
    assert_eq!(rec.checks.completed as usize, read);
    assert_eq!(rec.checks.state, ChecksState::Passing);
    h.finish().await;
}

// TST-8: many-checks
// POL-8: 100 checks are one request.
#[tokio::test]
async fn scn_many_checks_100() {
    pages(100).await;
}

// TST-8: many-checks
// POL-8: 101 checks are two requests.
#[tokio::test]
async fn scn_many_checks_101() {
    pages(101).await;
}

// TST-8: many-checks
// POL-8, MOD-7: 187 checks are two requests, with `total: 187`, not truncated.
#[tokio::test]
async fn scn_many_checks_187() {
    pages(187).await;
}

// TST-8: many-checks
// POL-8, MOD-7: 300 checks are three requests, not truncated.
#[tokio::test]
async fn scn_many_checks_300() {
    pages(300).await;
}

// TST-8: many-checks
// POL-8, MOD-7: 301 checks are truncated, with `total: 301` and 300 items.
#[tokio::test]
async fn scn_many_checks_301() {
    pages(301).await;
}

// TST-8: many-checks
// POL-8, MOD-7: 400 checks are truncated, with `total: 400`, and `completed` counts the items read.
#[tokio::test]
async fn scn_many_checks_400() {
    pages(400).await;
}

/// MOD-7: 400 checks whose first 300 passed and whose unread 100 are `tail`: the state, then
/// `checks-done`'s answer and its exit code.
async fn truncated(
    tail: &'static [(usize, &'static str, Option<&'static str>)],
    want: ChecksState,
) {
    let (h, _) = start(move |w| bulk(w, CHECKS_READ_MAX, tail)).await;
    go(&h, 5).await;
    let rec = show(&h, PR1, false).await;
    assert!(rec.checks.truncated);
    assert_eq!(rec.checks.total, 400);
    assert_eq!(rec.checks.completed as usize, CHECKS_READ_MAX);
    assert_eq!(rec.checks.failed, 0);
    assert_eq!(rec.checks.state, want);
    if want == ChecksState::Failing {
        let out = run(&h, &["wait", PR1, "--until", "checks-done"]).await;
        assert_eq!(out.code, 8, "{out:?}");
        assert!(
            out.stdout
                .contains("(truncated: unread checks may still run)"),
            "{out:?}"
        );
    }
    h.finish().await;
}

// TST-8: many-checks
// MOD-7: 400 checks, the first 300 passed, the unread ones running: `pending`.
#[tokio::test]
async fn scn_many_checks_truncated_pending() {
    truncated(&[(100, "IN_PROGRESS", None)], ChecksState::Pending).await;
}

// TST-8: many-checks
// MOD-7: the unread ones all passed: `passing`.
#[tokio::test]
async fn scn_many_checks_truncated_passing() {
    truncated(&[(100, "COMPLETED", Some("SUCCESS"))], ChecksState::Passing).await;
}

// TST-8: many-checks
// MOD-7, CLI-6: one unread check failed: `failing`, and `checks-done` exits 8 with the truncated mark.
#[tokio::test]
async fn scn_many_checks_truncated_failing() {
    truncated(
        &[
            (99, "COMPLETED", Some("SUCCESS")),
            (1, "COMPLETED", Some("FAILURE")),
        ],
        ChecksState::Failing,
    )
    .await;
}

// TST-8: many-checks
// POL-8: a failed second page leaves the record as it was, and `stale`; with the fault gone the
// next refresh applies and makes one event.
#[tokio::test]
async fn scn_many_checks_failed_page() {
    let (h, (_, runs)) = start(|w| bulk(w, 187, &[])).await;
    go(&h, 5).await;
    let before = show(&h, PR1, false).await;
    assert_eq!(before.checks.total, 187);
    assert!(!before.stale);
    let c = cursor(&h, PR1).await;

    // One check fails on the second page, and the next request without a finding, the poll's
    // second request, answers 502.
    h.github().world(|w| w.finish(runs[150], "FAILURE"));
    h.github()
        .fault_aimed(Fault::Status(502), Aim::WithoutFinding, Times::Requests(1));
    go(&h, POLL + 3).await;
    let held = show(&h, PR1, false).await;
    assert!(held.stale);
    let mut same = held.clone();
    same.stale = false;
    assert_eq!(same, before);
    assert_eq!((held.rev, held.fetched_at), (before.rev, before.fetched_at));
    assert_eq!(held.checks, before.checks);
    assert!(changes(&h, PR1, Some(&c)).await.body.events.is_empty());

    go(&h, POLL + 25).await;
    let applied = show(&h, PR1, false).await;
    assert!(!applied.stale);
    assert_eq!(applied.rev, before.rev + 1);
    assert_eq!(applied.checks.state, ChecksState::Failing);
    assert_eq!(applied.checks.failed, 1);
    let events = changes(&h, PR1, Some(&c)).await.body.events;
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(has_kind(&events[0], "checks"));
    h.finish().await;
}
