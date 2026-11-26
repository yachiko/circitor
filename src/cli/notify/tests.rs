use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::stub::{Resp, Stub, golden};
use super::*;
use crate::cli::args::{self, Command};
use crate::cli::fake::{FakeGit, FakeSpawner};
use crate::cli::{Branch, Git, Interrupt};
use crate::clock::{Clock, TestClock, Time};
use crate::config::{Env, Platform};
use crate::testkit::{MARKER_SESSION_TOKEN, ShortDir, golden as gold};

const CLI_VERSION: &str = "0.2.0+bbb";
const ORIGIN: &str = "git@github.com:octo/app.git";
const BRANCH: &str = "retry-sync";
const TOKEN: &str = MARKER_SESSION_TOKEN;

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

    /// The session's socket: a name nothing listens on (and in a directory that may not exist).
    fn session(&self) -> String {
        self.dir.path().join("session.sock").display().to_string()
    }

    fn vars(&self, session: bool, token: bool) -> Env {
        let d = self.dir.path();
        let mut env = Env {
            home: Some(d.to_path_buf()),
            ..Env::default()
        };
        for (k, v) in [
            ("XDG_STATE_HOME", d.join("st").display().to_string()),
            ("XDG_CONFIG_HOME", d.join("cfg").display().to_string()),
            ("CIRCITOR_SOCKET", self.sock().display().to_string()),
        ] {
            env.vars.insert(k.into(), v);
        }
        if session {
            env.vars.insert(SOCKET_VAR.into(), self.session());
        }
        if token {
            env.vars.insert(TOKEN_VAR.into(), TOKEN.into());
        }
        env
    }

    fn env_with(&self, git: Arc<dyn Git>, spawner: Arc<FakeSpawner>, vars: Env) -> CliEnv {
        CliEnv {
            clock: self.clock.clone(),
            tz: chrono::FixedOffset::east_opt(0).unwrap(),
            cwd: self.dir.path().to_path_buf(),
            env: vars,
            platform: Platform::Linux,
            version: CLI_VERSION.into(),
            spawner,
            git,
            interrupt: Interrupt::never(),
            request_limit: Duration::from_secs(90),
        }
    }

    fn env(&self) -> CliEnv {
        self.env_with(
            FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
            FakeSpawner::never_binds(),
            self.vars(true, true),
        )
    }

    fn stub(&self) -> Stub {
        Stub::start(&self.sock(), CLI_VERSION, &self.clock)
    }

    async fn until(&self, cond: impl Fn() -> bool) {
        for _ in 0..5_000 {
            if cond() {
                return;
            }
            self.clock.settle().await;
        }
        panic!("the condition never held");
    }

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

/// Parses `line` as `main` would and runs `notify` as `commands::run` does.
async fn notify(env: &CliEnv, line: &str) -> Output {
    let parsed = args::parse(&argv(line), &env.env).expect("a valid notify line");
    let Command::Notify { pr, what, json } = parsed.command else {
        panic!("not a notify line: {line}");
    };
    run(pr.as_deref(), &what, json, parsed.no_spawn, env).await
}

fn go(env: &CliEnv, line: &str) -> tokio::task::JoinHandle<Output> {
    let env = env.clone();
    let line = line.to_string();
    tokio::spawn(async move { notify(&env, &line).await })
}

fn check(name: &str, text: &str) {
    gold::check(&format!("tests/golden/notify_cmd/{name}.txt"), text);
}

fn body_of(seen: &super::stub::Seen) -> Value {
    serde_json::from_str(&seen.body).expect("a JSON body")
}

/// `notify-registered.json` with `f` applied.
fn registered(f: impl FnOnce(&mut Value)) -> Resp {
    let mut v = golden("notify-registered.json");
    f(&mut v);
    Resp::json(v)
}

fn already(f: impl FnOnce(&mut Value)) -> Resp {
    let mut v = golden("notify-already.json");
    f(&mut v);
    Resp::json(v)
}

const REG: &str = "notify octo/lib#5 --until checks-done --sha 1a2b3c4";

// NTF-11: registered, four lines, exit 0, the request's body.
#[tokio::test]
async fn ntf11_registered() {
    let f = fix();
    let stub = f.stub();
    let out = notify(&f.env(), REG).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stderr, "");
    check("registered", &out.stdout);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "registered: octo/lib#5 until checks-done on 1a2b3c4, for 1d"
    );
    assert_eq!(
        lines[2],
        "a confirmation was posted to this session; if none arrives, notices are held or refused here: run circitor notify octo/lib#5 --cancel, then use circitor wait"
    );
    assert!(lines[3].starts_with("cursor: "), "{lines:?}");
    let sent = stub.of("notify");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].line, "POST /v1/prs/octo/lib/5/notify");
    let body = body_of(&sent[0]);
    assert_eq!(body["until"], "checks-done");
    assert_eq!(body["sha"], "1a2b3c4");
    assert_eq!(body["socket"], f.session());
    assert_eq!(body["token"], TOKEN);
    assert_eq!(body["limit"], 50);
    assert!(body.get("for").is_none(), "{body}");
}

// NTF-11: `for` is sent only when given, and the line says it.
#[tokio::test]
async fn ntf11_for_is_sent_only_when_given() {
    let f = fix();
    let stub = f.stub();
    let out = notify(&f.env(), &format!("{REG} --for 3600")).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout.lines().next().unwrap(),
        "registered: octo/lib#5 until checks-done on 1a2b3c4, for 1h"
    );
    assert_eq!(body_of(&stub.of("notify")[0])["for"], 3600);
}

// NTF-11: `replaced an earlier notify`, in NTF-11's order, registered or already known.
#[tokio::test]
async fn ntf11_replaced_line() {
    let f = fix();
    let stub = f.stub();
    stub.script(
        "notify",
        [
            registered(|v| v["replaced"] = true.into()),
            already(|v| v["replaced"] = true.into()),
        ],
    );
    let out = notify(&f.env(), REG).await;
    check("registered_replaced", &out.stdout);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines[3], "replaced an earlier notify", "{lines:?}");
    assert!(lines[4].starts_with("cursor: "), "{lines:?}");
    let out = notify(&f.env(), REG).await;
    assert_eq!(out.code, 0, "{out:?}");
    check("already_replaced", &out.stdout);
    let lines: Vec<&str> = out.stdout.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with("already: "))
        .unwrap();
    assert_eq!(lines[at + 1], "replaced an earlier notify", "{lines:?}");
    assert!(lines[at + 2].starts_with("cursor: "), "{lines:?}");
}

// NTF-11: a reset and its records, events, a merged record, a problem.
#[tokio::test]
async fn ntf11_registered_extra_lines() {
    let f = fix();
    let stub = f.stub();
    let list = golden("list.json");
    let events = golden("notify-already.json")["events"].clone();
    stub.script(
        "notify",
        [
            registered(|v| {
                v["reset"] = true.into();
                v["records"] = list["records"].clone();
            }),
            registered(|v| v["events"] = events.clone()),
            registered(|v| {
                v["daemon"]["problem"] = "rate_limited".into();
                v["daemon"]["message"] = "GitHub asks for a pause".into();
                v["daemon"]["until"] = "2026-11-24T22:30:00Z".into();
            }),
            registered(|v| {
                v["record"]["state"] = "merged".into();
                v["record"]["merged_at"] = "2026-11-24T21:55:00Z".into();
                v["record"]["closed_at"] = "2026-11-24T21:55:00Z".into();
                v["record"]["status"] = "merged".into();
            }),
        ],
    );
    let env = f.env();
    let out = notify(&env, REG).await;
    check("registered_reset", &out.stdout);
    let first = out
        .stdout
        .lines()
        .position(|l| l.starts_with("registered: "))
        .unwrap();
    assert!(
        first > 0,
        "the reset comes before the confirmation: {}",
        out.stdout
    );

    let out = notify(&env, REG).await;
    check("registered_events", &out.stdout);
    let first = out
        .stdout
        .lines()
        .position(|l| l.starts_with("registered: "))
        .unwrap();
    assert!(first > 0, "events come first: {}", out.stdout);

    let out = notify(&env, REG).await;
    check("registered_problem", &out.stdout);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(lines[lines.len() - 2].starts_with("problem: "), "{lines:?}");
    assert!(lines[lines.len() - 1].starts_with("cursor: "), "{lines:?}");

    let out = notify(&env, REG).await;
    check("registered_merged", &out.stdout);
    assert!(
        out.stdout.lines().nth(1).unwrap().contains("merged"),
        "{}",
        out.stdout
    );
}

// NTF-11: with a reset, `again:` repeats the notify, `--for` before `--since`.
#[tokio::test]
async fn ntf11_already_with_reset() {
    let f = fix();
    let stub = f.stub();
    let reset = |v: &mut Value| {
        v["reset"] = true.into();
        v["records"] = golden("list.json")["records"].clone();
    };
    stub.script("notify", [already(reset), already(reset)]);
    let cond = "octo/lib#8 --until checks-done";
    let out = notify(&f.env(), &format!("notify {cond}")).await;
    assert_eq!(out.code, 0, "{out:?}");
    check("already_reset", &out.stdout);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines[lines.len() - 2],
        "again: circitor notify octo/lib#8 --until checks-done --since k3x9q2ab-28"
    );
    assert!(lines[lines.len() - 1].starts_with("cursor: "));

    let out = notify(&f.env(), &format!("notify {cond} --for 3600")).await;
    check("already_reset_for", &out.stdout);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines[lines.len() - 2],
        "again: circitor notify octo/lib#8 --until checks-done --for 3600 --since k3x9q2ab-28"
    );
}

// NTF-10: `--no-spawn` with no daemon waits 8 s for one, starts none.
#[tokio::test]
async fn ntf10_no_spawn_waits() {
    let f = fix();
    let spawner = FakeSpawner::never_binds();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        spawner.clone(),
        f.vars(true, true),
    );
    // A daemon that starts answering 5 s on.
    let h = go(&env, &format!("{REG} --no-spawn"));
    f.clock.advance(Duration::from_secs(5)).await;
    assert!(!h.is_finished());
    let stub = f.stub();
    f.until_done(&h, Duration::from_secs(3)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(stub.of("notify").len(), 1);
    assert_eq!(spawner.spawns(), 0);

    // None at all: exit 6, once 8 s have passed and not before.
    let f = fix();
    let spawner = FakeSpawner::never_binds();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        spawner.clone(),
        f.vars(true, true),
    );
    let h = go(&env, &format!("{REG} --no-spawn"));
    f.clock.advance(Duration::from_millis(7_500)).await;
    f.until(|| true).await;
    assert!(!h.is_finished(), "still waiting at 7.5 s");
    f.until_done(&h, Duration::from_secs(2)).await;
    let out = h.await.unwrap();
    assert_eq!(out.code, exit::NO_DAEMON, "{out:?}");
    assert_eq!(out.stdout, "");
    assert!(f.clock.mono().as_duration() >= Duration::from_secs(8));
    assert_eq!(spawner.spawns(), 0);
}

// NTF-11: an answer already known is exit 0, whatever the checks did.
#[tokio::test]
async fn ntf11_already() {
    let f = fix();
    let stub = f.stub();
    stub.script("notify", [Resp::file("notify-already.json")]);
    let out = notify(&f.env(), "notify octo/lib#8 --until checks-done").await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stderr, "");
    check("already", &out.stdout);
    assert!(
        out.stdout.contains("already: checks failed ("),
        "{}",
        out.stdout
    );
}

// NTF-11, D9: no socket in the environment is exit 11 before anything else.
#[tokio::test]
async fn ntf11_no_socket_is_11() {
    let f = fix();
    let stub = f.stub();
    let spawner = FakeSpawner::never_binds();
    // The session's directory doesn't exist, and neither does the state directory.
    let env = |session: Option<&str>| {
        let mut vars = f.vars(false, true);
        if let Some(s) = session {
            vars.vars.insert(SOCKET_VAR.into(), s.into());
        }
        f.env_with(
            FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
            spawner.clone(),
            vars,
        )
    };
    let mut lines = String::new();
    for (line, want) in [
        (
            "notify 12 --until checks-done --sha 1a2b3c4",
            "no session here: use circitor wait 12 --until checks-done --sha 1a2b3c4",
        ),
        (
            "notify 12 --until changed --kind review --since c1 --for 3600",
            "no session here: use circitor wait 12 --until changed --kind review --since c1",
        ),
        (
            "notify --until merged",
            "no session here: use circitor wait --until merged",
        ),
    ] {
        for e in [env(None), env(Some(""))] {
            let out = notify(&e, line).await;
            assert_eq!(out.code, exit::NO_SESSION, "{line}: {out:?}");
            assert_eq!(out.stdout, "", "{line}");
            assert_eq!(out.stderr, format!("{want}\n"), "{line}");
            assert!(!out.stderr.contains("--for") && !out.stderr.contains("--timeout"));
        }
        lines.push_str(&format!("{line}\n  {want}\n"));
    }
    check("no_socket", &lines);
    let out = notify(&env(None), "notify 12 --cancel").await;
    assert_eq!(out.code, exit::NO_SESSION, "{out:?}");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr.lines().count(), 1);
    assert!(!out.stderr.contains("wait"), "{out:?}");
    check("no_socket_cancel", &out.stderr);
    assert!(stub.seen().is_empty(), "{:?}", stub.seen());
    assert_eq!(spawner.spawns(), 0);
}

// NTF-11: `no_session` from the daemon is exit 11, with the PR in full.
#[tokio::test]
async fn ntf11_no_session_is_11() {
    let f = fix();
    let stub = f.stub();
    stub.script("notify", [Resp::file("error-no_session.json")]);
    let out = notify(&f.env(), "notify 12 --until checks-done --sha 1a2b3c4").await;
    assert_eq!(out.code, exit::NO_SESSION, "{out:?}");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "no session here: use circitor wait octo/app#12 --until checks-done --sha 1a2b3c4\n"
    );
}

// NTF-9, NTF-11: the token is optional.
#[tokio::test]
async fn ntf11_token_is_optional() {
    let f = fix();
    let stub = f.stub();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        FakeSpawner::never_binds(),
        f.vars(true, false),
    );
    let out = notify(&env, REG).await;
    assert_eq!(out.code, 0, "{out:?}");
    let body = body_of(&stub.of("notify")[0]);
    assert!(body["token"].is_null(), "{body}");
    assert!(body.as_object().unwrap().contains_key("token"));
}

// NTF-11, CLI-8: `--cancel` asks `DELETE`, and with no daemon starts none.
#[tokio::test]
async fn ntf11_cancel() {
    let f = fix();
    let stub = f.stub();
    let out = notify(&f.env(), "notify octo/lib#5 --cancel").await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, "cancelled: 1\n");
    let sent = stub.of("cancel");
    assert_eq!(sent.len(), 1);
    let socket: String = url_encode(&f.session());
    assert_eq!(
        sent[0].line,
        format!("DELETE /v1/prs/octo/lib/5/notify?socket={socket}")
    );
    assert!(!sent[0].line.contains(&f.session()), "percent-encoded");
    check("cancelled", &out.stdout);
    drop(stub);

    // No daemon: `cancelled: 0` at once, nothing started, also with `--no-spawn`.
    for line in [
        "notify octo/lib#5 --cancel",
        "notify octo/lib#5 --cancel --no-spawn",
        "notify --cancel",
    ] {
        let spawner = FakeSpawner::never_binds();
        let env = f.env_with(
            FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
            spawner.clone(),
            f.vars(true, true),
        );
        let before = f.clock.mono();
        let out = notify(&env, line).await;
        assert_eq!(out.code, 0, "{line}: {out:?}");
        assert_eq!(out.stdout, "cancelled: 0\n", "{line}");
        assert_eq!(f.clock.mono(), before, "{line}");
        assert_eq!(spawner.spawns(), 0, "{line}");
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// NTF-11, CLI-5: any other API error keeps its own code and line.
#[tokio::test]
async fn ntf11_api_errors_keep_their_codes() {
    let f = fix();
    let stub = f.stub();
    let env = f.env();
    for (file, code) in [
        ("error-limit.json", 10),
        ("error-bad_request.json", 2),
        ("error-not_found.json", 5),
    ] {
        stub.script("notify", [Resp::file(file)]);
        let out = notify(&env, REG).await;
        assert_eq!(out.code, code, "{file}: {out:?}");
        assert_eq!(out.stdout, "", "{file}");
        let e: crate::api::ErrorBody = serde_json::from_value(golden(file)).expect("an error body");
        let ctx = ctx(&env);
        assert_eq!(
            out.stderr,
            format!("{}\n", render::api_error(&e.error, false, &ctx).trim_end()),
            "{file}"
        );
        assert!(!out.stderr.contains(TOKEN));
    }
}

// NTF-11, CLI-4: `--json` prints the body as it came, and the codes stay.
#[tokio::test]
async fn ntf11_json() {
    let f = fix();
    let stub = f.stub();
    let ok = Resp::file("notify-registered.json");
    let err = Resp::file("error-no_session.json");
    stub.script("notify", [ok.clone(), err.clone()]);
    let env = f.env();
    let out = notify(&env, &format!("{REG} --json")).await;
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{}\n", ok.body));
    assert_eq!(out.stdout.lines().count(), 1);
    assert_eq!(out.stderr, "");
    let out = notify(&env, &format!("{REG} --json")).await;
    assert_eq!(out.code, exit::NO_SESSION, "{out:?}");
    assert_eq!(out.stdout, format!("{}\n", err.body));
    assert!(
        out.stderr
            .starts_with("no session here: use circitor wait octo/lib#5 ")
    );
    assert!(!out.stdout.contains(TOKEN) && !out.stderr.contains(TOKEN));
    // No socket: nothing on stdout.
    let bare = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        FakeSpawner::never_binds(),
        f.vars(false, false),
    );
    let out = notify(&bare, &format!("{REG} --json")).await;
    assert_eq!(out.code, exit::NO_SESSION);
    assert_eq!(out.stdout, "");
}

// CLI-2: no argument takes the branch's PR through `find_bare`.
#[tokio::test]
async fn ntf11_bare_uses_cli2() {
    let f = fix();
    let stub = f.stub();
    let mut status = golden("status.json");
    status["version"] = CLI_VERSION.into();
    status["github"]["login"] = "ana".into();
    stub.script("status", [Resp::json(status.clone()), Resp::json(status)]);
    let out = notify(&f.env(), "notify --until checks-done").await;
    assert_eq!(out.code, 0, "{out:?}");
    let sent = stub.of("notify");
    assert_eq!(sent.len(), 1, "{:?}", stub.seen());
    assert_eq!(sent[0].line, "POST /v1/prs/octo/app/12/notify");
    assert!(
        out.stdout
            .starts_with("registered: octo/app#12 until checks-done, for 1d")
    );
}

// CLI-7: registering with no daemon starts one.
#[tokio::test]
async fn ntf11_starts_the_daemon() {
    let f = fix();
    let spawner = FakeSpawner::never_binds();
    let env = f.env_with(
        FakeGit::new(Some(ORIGIN), Branch::Named(BRANCH.into())),
        spawner.clone(),
        f.vars(true, true),
    );
    let h = go(&env, REG);
    f.until_done(&h, Duration::from_secs(120)).await;
    let _ = h.await.unwrap();
    assert_eq!(spawner.spawns(), 1);
}
