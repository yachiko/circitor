use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use super::stub::{Act, Resp, Stub, golden};
use super::*;
use crate::api::{Ask, Until, WaitBody};
use crate::cli::fake::{FakeGit, FakeSpawner};
use crate::cli::render::WaitAsk;
use crate::cli::{Git, InterruptTrigger};
use crate::clock::{Clock, TestClock, Time};
use crate::testkit::ShortDir;

const CLI_VERSION: &str = "0.2.0+bbb";
const ORIGIN: &str = "git@github.com:octo/app.git";
const BRANCH: &str = "retry-sync";

struct Fix {
    dir: ShortDir,
    clock: Arc<TestClock>,
}

fn fix() -> Fix {
    Fix {
        dir: ShortDir::new(),
        clock: TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap()),
    }
}

impl Fix {
    fn sock(&self) -> PathBuf {
        self.dir.path().join("s.sock")
    }

    fn config_file(&self) -> PathBuf {
        self.dir.path().join("cfg/circitor/config.toml")
    }

    fn vars(&self) -> Env {
        let d = self.dir.path();
        let mut env = Env {
            home: Some(d.to_path_buf()),
            ..Env::default()
        };
        for (k, v) in [
            ("XDG_STATE_HOME", d.join("st")),
            ("XDG_CONFIG_HOME", d.join("cfg")),
            ("CIRCITOR_SOCKET", self.sock()),
        ] {
            env.vars.insert(k.into(), v.display().to_string());
        }
        env
    }

    fn env_with(&self, git: Arc<dyn Git>, spawner: Arc<FakeSpawner>) -> CliEnv {
        CliEnv {
            clock: self.clock.clone(),
            tz: chrono::FixedOffset::east_opt(0).unwrap(),
            cwd: self.dir.path().to_path_buf(),
            env: self.vars(),
            platform: Platform::Linux,
            version: CLI_VERSION.into(),
            spawner,
            git,
            interrupt: Interrupt::never(),
            request_limit: Duration::from_secs(90),
        }
    }

    /// In the repository `octo/app`, on the branch `retry-sync`; nothing is ever spawned.
    fn env(&self) -> CliEnv {
        self.env_with(
            FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
            FakeSpawner::never_binds(),
        )
    }

    fn stub(&self) -> Stub {
        Stub::start(&self.sock(), CLI_VERSION, &self.clock)
    }

    fn secs(&self) -> f64 {
        self.clock.mono().as_duration().as_secs_f64()
    }

    /// Lets the sockets and tasks run, without moving the clock, until `cond` holds.
    async fn until(&self, cond: impl Fn() -> bool) {
        for _ in 0..5_000 {
            if cond() {
                return;
            }
            self.clock.settle().await;
        }
        panic!("the condition never held");
    }

    /// Advances in 100 ms steps until `h` is done, or `max` has passed.
    async fn until_done<T>(&self, h: &tokio::task::JoinHandle<T>, max: Duration) {
        let end = self.clock.mono().plus(max);
        while !h.is_finished() && self.clock.mono() < end {
            self.clock.advance(Duration::from_millis(100)).await;
        }
        self.until(|| h.is_finished()).await;
    }
}

fn argv(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

async fn cli(env: &CliEnv, line: &str) -> Output {
    run(&argv(line), env).await
}

fn go(env: &CliEnv, line: &str) -> tokio::task::JoinHandle<Output> {
    let env = env.clone();
    let args = argv(line);
    tokio::spawn(async move { run(&args, &env).await })
}

fn pr(s: &str) -> PrName {
    PrName::parse(s).unwrap()
}

fn rctx(f: &Fix) -> RenderCtx {
    RenderCtx {
        now: f.clock.wall(),
        tz: chrono::FixedOffset::east_opt(0).unwrap(),
        cli_version: CLI_VERSION.into(),
    }
}

fn decode<T: DeserializeOwned>(body: &str) -> Envelope<T> {
    serde_json::from_str(body).unwrap()
}

fn target(req: &Request) -> String {
    format!("{} {}", req.method(), req.path_and_query())
}

fn wait_ask(name: &str, cond: Ask) -> WaitAsk {
    WaitAsk {
        pr: pr(name),
        cond,
        timeout: crate::cli::DEFAULT_WAIT_TIMEOUT,
        timeout_given: false,
        json: false,
    }
}

// ---- Answers built from the golden files ----

fn status_with_login(login: Option<&str>) -> Resp {
    let mut v = golden("status.json");
    v["version"] = CLI_VERSION.into();
    v["github"]["login"] = login.map_or(Value::Null, |l| l.into());
    Resp::ok(v)
}

/// A record of `octo/app` on `BRANCH` by `ana`, from `list.json`'s first.
fn rec(number: u64, state: &str, closed_at: Option<&str>) -> Value {
    let mut r = golden("list.json")["records"][0].clone();
    r["pr"] = format!("octo/app#{number}").into();
    r["url"] = format!("https://github.com/octo/app/pull/{number}").into();
    r["state"] = state.into();
    r["closed_at"] = closed_at.map_or(Value::Null, |c| c.into());
    if state == "merged" {
        r["merged_at"] = r["closed_at"].clone();
    }
    r
}

fn list_of(records: Vec<Value>) -> Resp {
    let mut v = golden("list.json");
    v["records"] = Value::Array(records);
    Resp::ok(v)
}

fn refresh_of(outcome: &str) -> Resp {
    let mut v = golden("refresh.json");
    v["outcome"] = outcome.into();
    if !matches!(outcome, "polled" | "shared") {
        v["cost"] = 1.into();
        v["daemon"]["problem"] = outcome.into();
        v["daemon"]["message"] = format!("the poll stopped: {outcome}").into();
    }
    if outcome == "shared" {
        v["cost"] = 0.into();
    }
    Resp::ok(v)
}

/// `wait-satisfied.json` with `f` applied.
fn wait_with(f: impl FnOnce(&mut Value)) -> Resp {
    let mut v = golden("wait-satisfied.json");
    f(&mut v);
    Resp::ok(v)
}

// ---- CLI-1, U10 ----

// CLI-1, U10: each command with an answer reaches the daemon, sends what `Request` builds, and prints its
// `render` function's text, exit 0.
#[tokio::test]
async fn cli1_each_command() {
    let f = fix();
    let stub = f.stub();
    let env = f.env();
    let ctx = rctx(&f);
    let body = |route: &str| {
        super::stub::Resp::file(match route {
            "list" => "list.json",
            "show" => "show.json",
            "changes" => "changes.json",
            "track" => "track.json",
            "untrack" => "untrack-null.json",
            "refresh" => "refresh.json",
            "wait" => "wait-satisfied.json",
            _ => unreachable!(),
        })
        .body
    };
    let p12 = pr("octo/app#12");
    let merged = Ask::new(Until::Merged);
    let mut status = golden("status.json");
    status["version"] = CLI_VERSION.into();
    let status = status.to_string();

    type Case = (&'static str, &'static str, Request, String);
    let cases: Vec<Case> = vec![
        (
            "list",
            "list",
            Request::List {
                all: false,
                since: None,
                checks: false,
            },
            render::list(&decode(&body("list")), false, &ctx),
        ),
        (
            "show octo/app#12",
            "show",
            Request::Show {
                pr: p12.clone(),
                fresh: false,
            },
            render::show(&decode(&body("show")), &ctx),
        ),
        (
            "changes",
            "changes",
            Request::Changes {
                pr: None,
                since: None,
                limit: Some(50),
            },
            render::changes(&decode(&body("changes")), None, &ctx),
        ),
        (
            "changes octo/app#12 --since k3x9q2ab-20",
            "changes",
            Request::Changes {
                pr: Some(p12.clone()),
                since: Some("k3x9q2ab-20".into()),
                limit: Some(50),
            },
            render::changes(&decode(&body("changes")), Some(&p12), &ctx),
        ),
        (
            "track octo/app#12",
            "track",
            Request::Track { pr: p12.clone() },
            render::tracked(&decode(&body("track")), &p12, &ctx),
        ),
        (
            "untrack octo/app#12",
            "untrack",
            Request::Untrack { pr: p12.clone() },
            render::tracked(&decode(&body("untrack")), &p12, &ctx),
        ),
        (
            "refresh",
            "refresh",
            Request::Refresh,
            render::refresh(&decode(&body("refresh")), &ctx),
        ),
        (
            "status",
            "status",
            Request::Status,
            render::status(&decode(&status), &ctx),
        ),
        (
            "wait octo/app#12 --until merged",
            "wait",
            Request::Wait {
                pr: p12.clone(),
                cond: merged.clone(),
                timeout: 100,
                limit: Some(50),
            },
            render::wait(
                &decode::<WaitBody>(&body("wait")),
                &wait_ask("octo/app#12", merged.clone()),
                &ctx,
            ),
        ),
    ];
    for (line, route, req, expected) in cases {
        let out = cli(&env, line).await;
        assert_eq!(out.code, 0, "{line}: {out:?}");
        assert_eq!(out.stdout, expected, "{line}");
        assert_eq!(out.stderr, "", "{line}");
        let sent = stub.of(route);
        assert_eq!(sent.last(), Some(&target(&req)), "{line}");
    }
    // D8: `--fail-fast` and `--kind` go into the wait's query.
    let out = cli(&env, "wait octo/app#12 --until checks-done --fail-fast").await;
    assert_eq!(out.code, exit::CHECKS_FAILED, "{out:?}");
    let sent = stub.of("wait").pop().unwrap();
    assert!(
        sent.contains("until=checks-done&fail_fast=1&timeout=100&limit=50"),
        "{sent}"
    );
    let out = cli(&env, "wait octo/app#12 --until changed --kind activity").await;
    assert_eq!(out.code, 0, "{out:?}");
    let sent = stub.of("wait").pop().unwrap();
    assert!(
        sent.contains("until=changed&kind=activity&timeout=100"),
        "{sent}"
    );
}

// ---- CLI-2 ----

/// A `Git` that counts its calls.
struct CountingGit {
    inner: Arc<FakeGit>,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Git for CountingGit {
    async fn origin_url(&self, cwd: &Path) -> Option<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.origin_url(cwd).await
    }
    async fn push_branch(&self, cwd: &Path) -> Branch {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.push_branch(cwd).await
    }
}

fn counting(origin: Option<&str>, branch: Branch) -> Arc<CountingGit> {
    Arc::new(CountingGit {
        inner: FakeGit::new(origin, branch),
        calls: AtomicUsize::new(0),
    })
}

// CLI-2: a full name and a URL are used as they are, with no `git`; `12` takes `origin`'s repository;
// nothing is `Ok(None)`; `12` outside a repository is exit 2.
#[tokio::test]
async fn cli2_name_offline() {
    let f = fix();
    let git = counting(Some(ORIGIN), Branch::Named(BRANCH.into()));
    let env = f.env_with(git.clone(), FakeSpawner::never_binds());
    for arg in ["o/r#12", "https://github.com/o/r/pull/12"] {
        let got = name_offline(&env, Some(arg)).await.unwrap();
        assert_eq!(got, Some(pr("o/r#12")), "{arg}");
    }
    assert_eq!(git.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        name_offline(&env, Some("12")).await.unwrap(),
        Some(pr("octo/app#12"))
    );
    assert_eq!(name_offline(&env, None).await.unwrap(), None);

    let outside = f.env_with(counting(None, Branch::NoRepo), FakeSpawner::never_binds());
    let out = name_offline(&outside, Some("12")).await.unwrap_err();
    assert_eq!(out.code, exit::USAGE);
    assert!(out.stderr.contains("owner/repo#12"), "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1);
    // A name that isn't one is exit 2 too.
    let out = name_offline(&env, Some("#12")).await.unwrap_err();
    assert_eq!(out.code, exit::USAGE);
}

// CLI-2: `12` uses `origin`'s repository.
#[tokio::test]
async fn cli2_number_uses_origin() {
    let f = fix();
    let stub = f.stub();
    let out = cli(&f.env(), "show 12").await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        stub.of("show"),
        vec![target(&Request::Show {
            pr: pr("octo/app#12"),
            fresh: false
        })]
    );
}

// CLI-2: outside a git repository: exit 2, no request sent, nothing started.
#[tokio::test]
async fn cli2_outside_a_repository() {
    let f = fix();
    let stub = f.stub();
    let spawner = FakeSpawner::never_binds();
    let env = f.env_with(FakeGit::new(None, Branch::NoRepo), spawner.clone());
    for line in [
        "show",
        "show 12",
        "wait --until merged",
        "wait 12 --until merged",
    ] {
        let out = cli(&env, line).await;
        assert_eq!(out.code, exit::USAGE, "{line}: {out:?}");
        assert_eq!(out.stdout, "", "{line}");
        assert_eq!(out.stderr.lines().count(), 1, "{line}: {out:?}");
    }
    assert!(stub.requests().is_empty(), "{:?}", stub.requests());
    assert_eq!(spawner.spawns(), 0);
    // No daemon at all: still exit 2, still nothing started.
    drop(stub);
    let out = cli(&env, "show").await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
    assert_eq!(spawner.spawns(), 0);
}

// CLI-2: a detached HEAD is exit 2.
#[tokio::test]
async fn cli2_detached_head() {
    let f = fix();
    let stub = f.stub();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Detached),
        FakeSpawner::never_binds(),
    );
    let out = cli(&env, "show").await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
    assert!(out.stderr.contains("detached"), "{out:?}");
    assert!(stub.requests().is_empty());
}

/// Runs `show` with no PR against `stub`, scripted by `script`.
async fn bare(f: &Fix, script: impl FnOnce(&Stub)) -> (Output, Stub) {
    let stub = f.stub();
    stub.default("status", status_with_login(Some("ana")));
    script(&stub);
    let out = cli(&f.env(), "show").await;
    (out, stub)
}

// CLI-2, TST-8 `bare-pr`: the branch's PR, found against the daemon.
#[tokio::test]
async fn cli2_bare_flow() {
    let f = fix();
    let show_of = |n: &str| {
        target(&Request::Show {
            pr: pr(n),
            fresh: false,
        })
    };

    // Found at once: no refresh is sent.
    let (out, stub) = bare(&f, |_| {}).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(stub.of("refresh").len(), 0);
    assert_eq!(stub.of("show"), vec![show_of("octo/app#12")]);
    let lists = stub.of("list");
    assert_eq!(lists, vec!["GET /v1/prs?state=all".to_string()]);
    drop(stub);

    // Found after one refresh.
    let (out, stub) = bare(&f, |s| {
        s.script("list", [list_of(vec![])]);
        s.default("list", list_of(vec![rec(14, "open", None)]));
    })
    .await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(stub.of("refresh").len(), 1);
    assert_eq!(stub.of("list").len(), 2);
    assert_eq!(stub.of("show"), vec![show_of("octo/app#14")]);
    drop(stub);

    // After `shared`, the second list is still read.
    let (out, stub) = bare(&f, |s| {
        s.script("list", [list_of(vec![])]);
        s.default("list", list_of(vec![rec(15, "open", None)]));
        s.default("refresh", refresh_of("shared"));
    })
    .await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(stub.of("list").len(), 2);
    assert_eq!(stub.of("show"), vec![show_of("octo/app#15")]);
    drop(stub);

    // A merged and a closed match: the later `closed_at`, with the note.
    let (out, stub) = bare(&f, |s| {
        s.default(
            "list",
            list_of(vec![
                rec(16, "merged", Some("2026-11-24T21:00:00Z")),
                rec(17, "closed", Some("2026-11-24T21:30:00Z")),
            ]),
        );
    })
    .await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(stub.of("show"), vec![show_of("octo/app#17")]);
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");
    assert!(out.stderr.contains("octo/app#17"), "{out:?}");
    assert!(out.stderr.contains("closed"), "{out:?}");
    drop(stub);

    // Two open matches: exit 2 with both names.
    let (out, stub) = bare(&f, |s| {
        s.default(
            "list",
            list_of(vec![rec(18, "open", None), rec(19, "open", None)]),
        );
    })
    .await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
    assert!(out.stderr.contains("octo/app#18") && out.stderr.contains("octo/app#19"));
    assert_eq!(stub.of("show").len(), 0);
    drop(stub);

    // No match: after `polled` exit 2; after `shared` exit 2 with the hint; after `rate_limited` 9;
    // after `auth` 7.
    for (outcome, code) in [
        ("polled", exit::USAGE),
        ("shared", exit::USAGE),
        ("rate_limited", exit::UNAVAILABLE),
        ("auth", exit::AUTH),
    ] {
        let (out, stub) = bare(&f, |s| {
            s.default("list", list_of(vec![]));
            s.default("refresh", refresh_of(outcome));
        })
        .await;
        assert_eq!(out.code, code, "{outcome}: {out:?}");
        assert_eq!(out.stdout, "", "{outcome}");
        assert_eq!(out.stderr.lines().count(), 1, "{outcome}: {out:?}");
        assert_eq!(stub.of("list").len(), 2, "{outcome}");
        assert_eq!(stub.of("show").len(), 0, "{outcome}");
        let hint = out.stderr.contains("by number");
        assert_eq!(hint, outcome == "shared", "{outcome}: {out:?}");
        if outcome == "polled" {
            assert!(out.stderr.contains("owner/repo#12"), "{out:?}");
        }
        if code != exit::USAGE {
            assert!(out.stderr.starts_with(outcome), "{out:?}");
        }
        drop(stub);
    }

    // Another author's PR on the branch never matches (CLI-2).
    let (out, stub) = bare(&f, |s| {
        let mut theirs = rec(20, "open", None);
        theirs["author"] = "ben".into();
        s.default("list", list_of(vec![theirs]));
    })
    .await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
    drop(stub);
}

// ---- CLI-4 ----

// CLI-4: `--json` prints the body as it came, on one line; an error still sets its code.
#[tokio::test]
async fn cli4_json_is_the_body() {
    let f = fix();
    let stub = f.stub();
    let env = f.env();
    let out = cli(&env, "list --json").await;
    assert_eq!(out.code, 0, "{out:?}");
    let body = Resp::file("list.json").body;
    assert_eq!(out.stdout, format!("{body}\n"));
    assert_eq!(out.stdout.lines().count(), 1);

    let bad = Resp::file("error-bad_request.json");
    stub.script("list", [bad.clone()]);
    let out = cli(&env, "list --json").await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
    assert_eq!(out.stdout, format!("{}\n", bad.body));
    assert_eq!(out.stderr.lines().count(), 1, "{out:?}");

    // A wait's body likewise.
    let out = cli(&env, "wait octo/app#12 --until merged --json").await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!("{}\n", Resp::file("wait-satisfied.json").body)
    );
}

// ---- CLI-5 ----

// CLI-5: each API-3 error has one code, one stderr line and an empty stdout.
#[tokio::test]
async fn cli5_error_codes() {
    let f = fix();
    let stub = f.stub();
    let env = f.env();
    for (code, exit_code) in [
        ("bad_request", 2),
        ("not_found", 5),
        ("unknown_route", 1),
        ("auth", 7),
        ("starting", 9),
        ("github_unavailable", 9),
        ("rate_limited", 9),
        ("limit", 10),
    ] {
        stub.script("list", [Resp::file(&format!("error-{code}.json"))]);
        let out = cli(&env, "list").await;
        assert_eq!(out.code, exit_code, "{code}: {out:?}");
        assert_eq!(out.stdout, "", "{code}");
        assert_eq!(out.stderr.lines().count(), 1, "{code}: {out:?}");
        assert!(out.stderr.starts_with(code), "{code}: {out:?}");
    }
    // A 200 that doesn't decode.
    stub.script("list", [Resp::ok(json!({ "version": "9.9.9" }))]);
    let out = cli(&env, "list").await;
    assert_eq!(out.code, exit::ERROR, "{out:?}");
    assert_eq!(out.stderr.lines().count(), 1);
    // An error code this CLI doesn't know (a newer daemon's) is 1.
    let mut newer = golden("error-limit.json");
    newer["error"]["code"] = "something_new".into();
    stub.script("list", [Resp::error(newer)]);
    let out = cli(&env, "list").await;
    assert_eq!(out.code, exit::ERROR, "{out:?}");
    // API-16: `rate_limited` in a refresh's 200 exits as the API error of that name.
    stub.script("refresh", [refresh_of("rate_limited")]);
    let out = cli(&env, "refresh").await;
    assert_eq!(out.code, exit::UNAVAILABLE, "{out:?}");
    assert!(out.stdout.starts_with("refresh: rate_limited"), "{out:?}");
    // A usage error is 2, before anything else.
    let out = cli(&env, "list --nope").await;
    assert_eq!(out.code, exit::USAGE, "{out:?}");
}

// ---- CLI-9 ----

// CLI-9 (a-bad-request-or-unknown-route-from): an older daemon that isn't replaced (a foreground one)
// answering `bad_request` or `unknown_route` gives the hint and exit 1, not 2.
#[tokio::test]
async fn cli9_older_daemon_bad_request() {
    let f = fix();
    let stub = f.stub();
    let mut old = golden("status.json");
    old["version"] = "0.1.0".into();
    old["mode"] = "foreground".into();
    stub.default("status", Resp::ok(old));
    let env = f.env();
    for code in ["bad_request", "unknown_route"] {
        stub.script("list", [Resp::file(&format!("error-{code}.json"))]);
        let out = cli(&env, "list").await;
        assert_eq!(out.code, exit::ERROR, "{code}: {out:?}");
        assert!(
            out.stderr.contains("older than this CLI"),
            "{code}: {out:?}"
        );
        // A wait passes the flag on as well.
        stub.script("wait", [Resp::file(&format!("error-{code}.json"))]);
        let out = cli(&env, "wait octo/app#12 --until merged").await;
        assert_eq!(out.code, exit::ERROR, "{code}: {out:?}");
        assert!(
            out.stderr.contains("older than this CLI"),
            "{code}: {out:?}"
        );
    }
}

// ---- CLI-8 ----

// CLI-8: with no daemon, `status` exits 6 and `stop` 0, and neither starts one; `version`, `help` and
// `guide` resolve no path and open no socket.
#[tokio::test]
async fn cli8_no_daemon() {
    let f = fix();
    let spawner = FakeSpawner::never_binds();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        spawner.clone(),
    );
    let out = cli(&env, "status").await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert_eq!(out.stderr, "not running\n");
    assert_eq!(out.stdout, "");
    let out = cli(&env, "stop").await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, "not running\n");
    assert_eq!(spawner.spawns(), 0);

    // No home, and a socket in a directory that doesn't exist: nothing here needs either.
    let mut nowhere = env.clone();
    nowhere.env = Env::default();
    nowhere.env.vars.insert(
        "CIRCITOR_SOCKET".into(),
        "/nonexistent/circitor/s.sock".into(),
    );
    let out = cli(&nowhere, "version").await;
    assert_eq!(out, stdout(format!("circitor {CLI_VERSION}\n")));
    let out = cli(&nowhere, "help").await;
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("wait"), "{out:?}");
    let out = cli(&nowhere, "--help").await;
    assert_eq!(out.code, 0, "{out:?}");
    let out = cli(&nowhere, "guide").await;
    assert_eq!(out, stdout(GUIDE.to_string()));
    // Any other command resolves the paths: without a home that's its line and exit 6.
    let out = cli(&nowhere, "list").await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert_eq!(spawner.spawns(), 0);
}

// ---- CLI-6, CLI-7 ----

// CLI-6: without `--timeout` the CLI sends API-12's default.
#[tokio::test]
async fn cli6_default_timeout() {
    let f = fix();
    let stub = f.stub();
    let out = cli(&f.env(), "wait octo/app#12 --until merged").await;
    assert_eq!(out.code, 0, "{out:?}");
    let sent = stub.of("wait");
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("&timeout=100&"), "{sent:?}");
}

// CLI-6: one deadline for the whole command: 3 s spent naming the PR leave 27 of 30.
#[tokio::test]
async fn cli6_one_deadline() {
    let f = fix();
    let stub = f.stub();
    stub.default("status", status_with_login(Some("ana")));
    stub.script(
        "list",
        [Resp::file("list.json").after(Duration::from_secs(3))],
    );
    let h = go(&f.env(), "wait --until merged --timeout 30");
    f.until(|| stub.of("list").len() == 1).await;
    f.clock.advance(Duration::from_secs(3)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, 0, "{out:?}");
    let sent = stub.of("wait");
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].starts_with("GET /v1/prs/octo/app/12/wait?"),
        "{sent:?}"
    );
    assert!(sent[0].contains("&timeout=27&"), "{sent:?}");
}

// CLI-6: with under 1 s left nothing is sent: a timeout without an answer, exit 3.
#[tokio::test]
async fn cli6_under_1s_is_not_sent() {
    let f = fix();
    let stub = f.stub();
    stub.default("status", status_with_login(Some("ana")));
    stub.script(
        "list",
        [Resp::file("list.json").after(Duration::from_millis(29_500))],
    );
    let h = go(&f.env(), "wait --until merged --timeout 30");
    f.until(|| stub.of("list").len() == 1).await;
    f.clock.advance(Duration::from_millis(29_500)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, exit::TIMEOUT, "{out:?}");
    assert_eq!(
        out.stdout,
        "timeout after 30s: octo/app#12 (no answer from the daemon)\n\
         again: circitor wait octo/app#12 --until merged --timeout 30\n"
    );
    assert_eq!(stub.of("wait").len(), 0);
}

// CLI-6, CLI-7: `--timeout 0` sends `timeout=0`; unanswered, it ends at 60 s, exit 6.
#[tokio::test]
async fn cli6_timeout_0() {
    let f = fix();
    let stub = f.stub();
    stub.default("wait", Resp::file("wait-satisfied.json").act(Act::Stall));
    let h = go(&f.env(), "wait octo/app#12 --until merged --timeout 0");
    f.until(|| stub.of("wait").len() == 1).await;
    assert!(
        stub.of("wait")[0].contains("&timeout=0&"),
        "{:?}",
        stub.of("wait")
    );
    f.clock.advance(Duration::from_millis(59_900)).await;
    f.clock.settle().await;
    assert!(!h.is_finished());
    f.clock.advance(Duration::from_millis(100)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "daemon not answering on {}\nagain: circitor wait octo/app#12 --until merged --timeout 0\n",
            stub.path().display()
        )
    );
    assert_eq!(f.secs(), 60.0);
}

// CLI-7: a default wait never answered ends at its deadline plus 10 s, not before.
#[tokio::test]
async fn cli7_wait_limit() {
    let f = fix();
    let stub = f.stub();
    stub.default(
        "wait",
        Resp::file("wait-satisfied.json")
            .cursor("k3x9q2ab-30")
            .act(Act::Stall),
    );
    let h = go(&f.env(), "wait octo/app#12 --until merged");
    f.until(|| stub.of("wait").len() == 1).await;
    f.clock.advance(Duration::from_millis(109_900)).await;
    f.clock.settle().await;
    assert!(!h.is_finished());
    f.clock.advance(Duration::from_millis(100)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert!(
        out.stdout.starts_with(&format!(
            "daemon not answering on {}\n",
            stub.path().display()
        )),
        "{out:?}"
    );
    assert_eq!(f.secs(), 110.0);
}

// CLI-5, CLI-6: a wait's answer exits 0, 3, 4 or 8.
#[tokio::test]
async fn cli6_wait_codes() {
    let f = fix();
    let stub = f.stub();
    let env = f.env();
    let ctx = rctx(&f);
    let cases: Vec<(&str, Resp, i32)> = vec![
        ("--until merged", Resp::file("wait-satisfied.json"), 0),
        ("--until checks-done", Resp::file("wait-satisfied.json"), 8),
        (
            "--until checks-done",
            Resp::file("wait-timeout-no-record.json"),
            3,
        ),
        (
            "--until merged",
            wait_with(|v| {
                v["outcome"] = "ended".into();
                v["reason"] = "closed".into();
                v["record"]["state"] = "closed".into();
                v["record"]["closed_at"] = "2026-11-24T21:59:00Z".into();
            }),
            4,
        ),
        (
            "--until checks-done",
            wait_with(|v| {
                let c = &mut v["record"]["checks"];
                c["state"] = "passing".into();
                c["failed"] = 0.into();
                c["failed_items"] = json!([]);
            }),
            0,
        ),
        // `--fail-fast`: satisfied while the checks are still `pending`, one failed.
        (
            "--until checks-done --fail-fast",
            wait_with(|v| {
                let c = &mut v["record"]["checks"];
                c["state"] = "pending".into();
                c["settled"] = false.into();
                c["completed"] = 2.into();
            }),
            8,
        ),
    ];
    for (flags, resp, code) in cases {
        stub.script("wait", [resp.clone()]);
        let line = format!("wait octo/app#12 {flags}");
        let out = cli(&env, &line).await;
        assert_eq!(out.code, code, "{line}: {out:?}");
        let mut cond = Ask::new(if flags.contains("merged") {
            Until::Merged
        } else {
            Until::ChecksDone
        });
        cond.fail_fast = flags.contains("--fail-fast");
        let expected = render::wait(
            &decode::<WaitBody>(&resp.body),
            &wait_ask("octo/app#12", cond),
            &ctx,
        );
        assert_eq!(out.stdout, expected, "{line}");
    }
    // A wait refused before its head is an API error.
    stub.script("wait", [Resp::file("error-limit.json")]);
    let out = cli(&env, "wait octo/app#12 --until merged").await;
    assert_eq!(out.code, exit::LIMIT, "{out:?}");
    assert_eq!(out.stdout, "");
}

// ---- CLI-10 ----

// CLI-10: dropped after the cursor: the same request again with `since` the first cursor and the seconds
// left; its answer is printed.
#[tokio::test]
async fn cli10_drop_after_the_cursor() {
    let f = fix();
    let stub = f.stub();
    let spawner = FakeSpawner::never_binds();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        spawner.clone(),
    );
    stub.script(
        "wait",
        [
            Resp::file("wait-satisfied.json")
                .cursor("first-1")
                .act(Act::HeadThenClose)
                .after(Duration::from_secs(5)),
            Resp::file("wait-satisfied.json").cursor("second-2"),
        ],
    );
    let h = go(&env, "wait octo/app#12 --until merged");
    f.until(|| stub.of("wait").len() == 1).await;
    f.clock.advance(Duration::from_secs(5)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, 0, "{out:?}");
    let sent = stub.of("wait");
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[0].contains("until=merged&timeout=100&"), "{sent:?}");
    assert!(
        sent[1].contains("until=merged&since=first-1&timeout=95&limit=50"),
        "{sent:?}"
    );
    let expected = render::wait(
        &decode::<WaitBody>(&Resp::file("wait-satisfied.json").body),
        &wait_ask("octo/app#12", Ask::new(Until::Merged)),
        &rctx(&f),
    );
    assert_eq!(out.stdout, expected);
    assert_eq!(spawner.spawns(), 0);
}

// CLI-10, CLI-6: dropped after the cursor with no daemon back: `daemon stopped` after 8 s, with the first
// cursor, exit 6, nothing started.
#[tokio::test]
async fn cli10_no_daemon_back() {
    let f = fix();
    let stub = f.stub();
    let spawner = FakeSpawner::never_binds();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        spawner.clone(),
    );
    stub.script(
        "wait",
        [Resp::file("wait-satisfied.json")
            .cursor("first-1")
            .act(Act::HeadThenVanish)],
    );
    let h = go(&env, "wait octo/app#12 --until merged");
    f.until(|| stub.of("wait").len() == 1).await;
    f.until(|| !stub.path().exists()).await;
    f.until_done(&h, Duration::from_secs(20)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert_eq!(
        out.stdout,
        "daemon stopped\n\
         again: circitor wait octo/app#12 --until merged --since first-1\n\
         cursor: first-1\n"
    );
    let at = f.secs();
    assert!((8.0..=8.2).contains(&at), "ended at {at}");
    assert_eq!(spawner.spawns(), 0);
    assert_eq!(stub.of("wait").len(), 1);
}

// CLI-10: dropped before the cursor: `ensure` runs again; a second drop is exit 6.
#[tokio::test]
async fn cli10_drop_before_the_cursor() {
    let f = fix();
    let stub = f.stub();
    let env = f.env();
    let closed = Resp::file("wait-satisfied.json").act(Act::Close);
    stub.script("wait", [closed.clone(), Resp::file("wait-satisfied.json")]);
    let out = cli(&env, "wait octo/app#12 --until merged").await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(stub.of("wait").len(), 2);
    // `ensure` read the status again before the repeat.
    assert_eq!(stub.of("status").len(), 2);

    stub.script("wait", [closed.clone(), closed]);
    let out = cli(&env, "wait octo/app#12 --until merged").await;
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert_eq!(
        out.stdout,
        "daemon stopped\nagain: circitor wait octo/app#12 --until merged\n"
    );
    assert_eq!(stub.of("wait").len(), 4);
    assert_eq!(stub.of("status").len(), 4);
}

// CLI-6: `SIGINT` and `SIGTERM` end a held wait at once, with nothing printed.
#[tokio::test]
async fn cli6_interrupt() {
    let f = fix();
    let stub = f.stub();
    stub.default("wait", Resp::file("wait-satisfied.json").act(Act::Stall));
    for code in [exit::SIGINT, exit::SIGTERM] {
        let (trigger, interrupt): (InterruptTrigger, Interrupt) = Interrupt::pair();
        let mut env = f.env();
        env.interrupt = interrupt;
        let before = stub.of("wait").len();
        let h = go(&env, "wait octo/app#12 --until merged");
        f.until(|| stub.of("wait").len() == before + 1).await;
        trigger.fire(code);
        let out = h.await.unwrap();
        assert_eq!(
            out,
            Output {
                code,
                ..Output::default()
            }
        );
    }
    assert_eq!(f.secs(), 0.0);
}

// ---- OPS-1 ----

// OPS-1: `daemon --check-config`: no file is 0; a bad file is 1 with its one-line message.
#[tokio::test]
async fn ops1_check_config() {
    let f = fix();
    let env = f.env();
    let out = cli(&env, "daemon --check-config").await;
    assert_eq!(out, Output::default());
    let file = f.config_file();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let testdata = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/config");
    std::fs::copy(testdata.join("full.toml"), &file).unwrap();
    let out = cli(&env, "daemon --check-config").await;
    assert_eq!(out, Output::default());
    for bad in [
        "unknown_key.toml",
        "not_toml.toml",
        "wrong_type.toml",
        "out_of_range.toml",
    ] {
        std::fs::copy(testdata.join(bad), &file).unwrap();
        let out = cli(&env, "daemon --check-config").await;
        let message = config::load(&file).unwrap_err().to_string();
        assert_eq!(out.code, exit::ERROR, "{bad}");
        assert_eq!(out.stderr, format!("{message}\n"), "{bad}");
        assert_eq!(out.stdout, "");
    }
}

// `daemon` without `--check-config` never runs here: `main` hands it over.
#[tokio::test]
async fn daemon_is_not_run_here() {
    let f = fix();
    let out = cli(&f.env(), "daemon").await;
    assert_eq!(out.code, exit::ERROR);
}

// TST-1: the request limit is 90 s, and only a test build lets `CIRCITOR_TEST_CLI_LIMIT` replace it.
#[test]
fn tst1_cli_limit() {
    let mut env = Env::default();
    assert_eq!(request_limit(&env), Duration::from_secs(90));
    env.vars
        .insert("CIRCITOR_TEST_CLI_LIMIT".into(), "2".into());
    let expected = if cfg!(feature = "test-endpoint") {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(90)
    };
    assert_eq!(request_limit(&env), expected);
}
