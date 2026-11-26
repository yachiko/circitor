use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;
use crate::clock::{TestClock, Time};
use crate::testkit::{MARKER_TOKEN, MARKER_TOKEN_2, ShortDir};

fn clock() -> Arc<TestClock> {
    TestClock::new(Time::parse("2026-11-24T22:00:00Z").expect("a time"))
}

/// Writes an executable script; the file is closed before this returns.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn env(path: &str, extra: &[(&str, &str)]) -> Env {
    let mut env = Env::default();
    env.vars.insert("PATH".into(), path.into());
    for (k, v) in extra {
        env.vars.insert((*k).into(), (*v).into());
    }
    env
}

struct Rig {
    dir: ShortDir,
    clock: Arc<TestClock>,
}

impl Rig {
    fn new() -> Rig {
        Rig {
            dir: ShortDir::new(),
            clock: clock(),
        }
    }
    fn p(&self) -> &Path {
        self.dir.path()
    }
    fn source(&self, gh_path: &str, env: Env) -> Arc<GhTokenSource> {
        GhTokenSource::with_fixed_dirs(
            gh_path,
            env,
            self.clock.clone(),
            self.clock.activity(),
            vec![],
        )
    }
    fn plain_env(&self) -> Env {
        env("/usr/bin:/bin", &[])
    }
    /// A source whose `gh_path` is an absolute path to a script with `body`.
    fn gh(&self, body: &str) -> Arc<GhTokenSource> {
        let path = script(self.p(), "gh", body);
        self.source(path.to_str().expect("utf8"), self.plain_env())
    }
}

/// "Text file busy": another test thread forking while a script was open can make the first spawn
/// fail. Test helper only: production reports `CantRun`.
async fn read(src: &GhTokenSource) -> TokenRead {
    for _ in 0..5000 {
        let r = src.read().await;
        tokio::task::yield_now().await;
        if !matches!(r, TokenRead::Failed(GhFailure::CantRun)) {
            return r;
        }
    }
    src.read().await
}

fn is_token(r: &TokenRead, want: &str) -> bool {
    matches!(r, TokenRead::Token(t) if t.expose() == want)
}

fn lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

async fn wait_for_file(path: &Path) {
    for _ in 0..2_000_000 {
        if path.exists() && !std::fs::read_to_string(path).unwrap_or_default().is_empty() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("{} never appeared", path.display());
}

// OPS-5: a token, with a warning on stderr ignored.
#[tokio::test]
async fn ops5_a_token() {
    let rig = Rig::new();
    let src = rig.gh(&format!(
        "echo 'warning: something' >&2\necho {MARKER_TOKEN}"
    ));
    assert!(is_token(&read(&src).await, MARKER_TOKEN));
}

// U13: github.com only.
#[tokio::test]
async fn u13_asks_for_github_com_only() {
    let rig = Rig::new();
    let args = rig.p().join("args");
    let src = rig.gh(&format!(
        "echo \"$@\" > {}\necho {MARKER_TOKEN}",
        args.display()
    ));
    assert!(is_token(&read(&src).await, MARKER_TOKEN));
    assert_eq!(
        std::fs::read_to_string(args).unwrap().trim_end(),
        "auth token --hostname github.com"
    );
}

// OPS-5: no login.
#[tokio::test]
async fn ops5_no_login() {
    let rig = Rig::new();
    let src = rig.gh("echo 'no oauth token found for github.com' >&2\nexit 1");
    assert!(matches!(read(&src).await, TokenRead::NoLogin));
    // Exit 1 for something else is a failure, not no login.
    let rig = Rig::new();
    let src = rig.gh("echo 'boom' >&2\nexit 1");
    assert!(matches!(
        read(&src).await,
        TokenRead::Failed(GhFailure::Exit(1))
    ));
}

// OPS-5: failures.
#[tokio::test]
async fn ops5_a_failure() {
    let cases: [(&str, GhFailure); 5] = [
        ("exit 3", GhFailure::Exit(3)),
        ("kill -9 $$", GhFailure::Killed),
        ("exit 0", GhFailure::NoOutput),
        ("echo", GhFailure::NoOutput),
        ("echo a\necho b", GhFailure::NoOutput),
    ];
    for (body, want) in cases {
        let rig = Rig::new();
        let src = rig.gh(body);
        match read(&src).await {
            TokenRead::Failed(got) => assert_eq!(got, want, "{body}"),
            other => panic!("{body}: {other:?}"),
        }
    }
    for body in ["printf '   \\n'", "printf '\\377\\n'"] {
        let rig = Rig::new();
        let src = rig.gh(body);
        assert!(
            matches!(read(&src).await, TokenRead::Failed(GhFailure::NoOutput)),
            "{body}"
        );
    }
}

// OPS-5: ten seconds.
#[tokio::test]
async fn ops5_ten_seconds() {
    let rig = Rig::new();
    let pid = rig.p().join("pid");
    let src = rig.gh(&format!("echo $$ > {}\nexec sleep 1000", pid.display()));
    let _excused = rig.clock.activity().excuse();
    let s = src.clone();
    let task = tokio::spawn(async move { s.read().await });
    wait_for_file(&pid).await;
    rig.clock.advance(Duration::from_millis(9_999)).await;
    assert!(!task.is_finished());
    rig.clock.advance(Duration::from_millis(1)).await;
    let read = task.await.expect("joined");
    assert!(
        matches!(read, TokenRead::Failed(GhFailure::Timeout)),
        "{read:?}"
    );
    let n: i32 = std::fs::read_to_string(&pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let alive = nix::sys::signal::kill(nix::unistd::Pid::from_raw(n), None);
    assert!(alive.is_err(), "the process still exists");
}

// OPS-5: a path with a slash is the only one tried.
#[tokio::test]
async fn ops5_a_path_with_a_slash_is_the_only_one_tried() {
    let rig = Rig::new();
    let bin = rig.p().join("bin");
    std::fs::create_dir(&bin).unwrap();
    script(&bin, "gh", &format!("echo {MARKER_TOKEN}"));
    let e = env(&format!("{}:/usr/bin:/bin", bin.display()), &[]);
    let missing = rig.p().join("nope");
    let src = rig.source(missing.to_str().unwrap(), e.clone());
    assert!(matches!(
        src.read().await,
        TokenRead::Failed(GhFailure::CantRun)
    ));
    assert_eq!(src.gh_path(), missing.to_str().unwrap());
    // A file without an execute bit.
    let plain = rig.p().join("plain");
    std::fs::write(&plain, "#!/bin/sh\necho x\n").unwrap();
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
    let src = rig.source(plain.to_str().unwrap(), e);
    assert!(matches!(
        src.read().await,
        TokenRead::Failed(GhFailure::CantRun)
    ));
    assert_eq!(src.gh_path(), plain.to_str().unwrap());
}

// OPS-5: a name goes through PATH, then the fixed places.
#[tokio::test]
async fn ops5_a_name_is_looked_up_in_path_then_the_fixed_places() {
    let rig = Rig::new();
    let (a, b, fixed) = (rig.p().join("a"), rig.p().join("b"), rig.p().join("fixed"));
    for d in [&a, &b, &fixed] {
        std::fs::create_dir(d).unwrap();
    }
    script(&b, "gh", &format!("echo {MARKER_TOKEN}"));
    // An empty entry is skipped; a directory called gh is not a file.
    std::fs::create_dir(a.join("gh")).unwrap();
    let path = format!(":{}:{}", a.display(), b.display());
    let mk = |path: &str, dirs: Vec<PathBuf>| {
        GhTokenSource::with_fixed_dirs(
            "gh",
            env(path, &[]),
            rig.clock.clone(),
            rig.clock.activity(),
            dirs,
        )
    };
    let src = mk(&path, vec![fixed.clone()]);
    assert!(is_token(&read(&src).await, MARKER_TOKEN));
    assert_eq!(src.gh_path(), b.join("gh").to_str().unwrap());

    script(&fixed, "gh", &format!("echo {MARKER_TOKEN_2}"));
    let src = mk("/nonexistent", vec![fixed.clone()]);
    assert!(is_token(&read(&src).await, MARKER_TOKEN_2));
    assert_eq!(src.gh_path(), fixed.join("gh").to_str().unwrap());

    let src = mk("/nonexistent", vec![rig.p().join("none")]);
    assert!(matches!(
        src.read().await,
        TokenRead::Failed(GhFailure::CantRun)
    ));
    assert_eq!(src.gh_path(), "gh");
}

// OPS-5: looked up at each read.
#[tokio::test]
async fn ops5_looked_up_again_at_each_read() {
    let rig = Rig::new();
    let (a, b) = (rig.p().join("a"), rig.p().join("b"));
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    script(&a, "gh", &format!("echo {MARKER_TOKEN}"));
    let e = env(&format!("{}:{}", a.display(), b.display()), &[]);
    let src = rig.source("gh", e);
    assert!(is_token(&read(&src).await, MARKER_TOKEN));
    assert_eq!(src.gh_path(), a.join("gh").to_str().unwrap());
    std::fs::remove_file(a.join("gh")).unwrap();
    script(&b, "gh", &format!("echo {MARKER_TOKEN_2}"));
    assert!(is_token(&read(&src).await, MARKER_TOKEN_2));
    assert_eq!(src.gh_path(), b.join("gh").to_str().unwrap());
}

// OPS-5: one run for concurrent reads.
#[tokio::test]
async fn ops5_one_run_for_concurrent_reads() {
    let rig = Rig::new();
    let counter = rig.p().join("count");
    let src = rig.gh(&format!(
        "echo x >> {}\nsleep 0.3\necho {MARKER_TOKEN}",
        counter.display()
    ));
    let (a, b, c) = futures::join!(read(&src), read(&src), read(&src));
    for r in [a, b, c] {
        assert!(is_token(&r, MARKER_TOKEN));
    }
    assert_eq!(lines(&counter), 1);
}

// OPS-5, OPS-7: the run belongs to its callers.
#[tokio::test]
async fn ops5_a_dropped_read_is_not_kept() {
    let rig = Rig::new();
    let counter = rig.p().join("count");
    let hang = rig.p().join("hang");
    std::fs::write(&hang, "").unwrap();
    let src = rig.gh(&format!(
        "echo x >> {c}\nif [ -e {h} ]; then exec sleep 1000; fi\necho {MARKER_TOKEN}",
        c = counter.display(),
        h = hang.display()
    ));
    let excused = rig.clock.activity().excuse();
    let s = src.clone();
    let task = tokio::spawn(async move { s.read().await });
    wait_for_file(&counter).await;
    task.abort();
    let _ = task.await;
    drop(excused);
    std::fs::remove_file(&hang).unwrap();
    assert!(is_token(&read(&src).await, MARKER_TOKEN));
    assert_eq!(lines(&counter), 2);
}

// OPS-5: kind.
#[tokio::test]
async fn ops5_kind() {
    let rig = Rig::new();
    let k = |extra: &[(&str, &str)]| rig.source("gh", env("/usr/bin", extra)).kind();
    assert_eq!(
        k(&[("GH_TOKEN", "x"), ("GITHUB_TOKEN", "y")]),
        TokenSourceKind::GhTokenVar
    );
    assert_eq!(
        k(&[("GH_TOKEN", ""), ("GITHUB_TOKEN", "y")]),
        TokenSourceKind::GithubTokenVar
    );
    assert_eq!(
        k(&[("GH_TOKEN", ""), ("GITHUB_TOKEN", "")]),
        TokenSourceKind::Gh
    );
    assert_eq!(k(&[]), TokenSourceKind::Gh);
}

// OPS-5: the child gets the given environment and nothing else.
#[tokio::test]
async fn ops5_the_child_gets_the_given_environment() {
    let rig = Rig::new();
    let body = format!(
        "if [ -n \"$GH_TOKEN\" ]; then echo \"$GH_TOKEN\"; elif [ -n \"$GITHUB_TOKEN\" ]; then echo \"$GITHUB_TOKEN\"; else echo {MARKER_TOKEN}; fi"
    );
    let path = script(rig.p(), "gh", &body);
    let p = path.to_str().unwrap();
    let src = rig.source(p, env("/usr/bin:/bin", &[("GH_TOKEN", MARKER_TOKEN_2)]));
    assert!(is_token(&read(&src).await, MARKER_TOKEN_2));
    let src = rig.source(
        p,
        env(
            "/usr/bin:/bin",
            &[("GH_TOKEN", ""), ("GITHUB_TOKEN", MARKER_TOKEN_2)],
        ),
    );
    assert!(is_token(&read(&src).await, MARKER_TOKEN_2));
    let src = rig.source(p, rig.plain_env());
    assert!(is_token(&read(&src).await, MARKER_TOKEN));

    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    let dump = rig.p().join("dump");
    let path = script(
        rig.p(),
        "gh2",
        &format!("env > {}\necho {MARKER_TOKEN}", dump.display()),
    );
    let src = rig.source(
        path.to_str().unwrap(),
        env("/usr/bin:/bin", &[("MINE", "1")]),
    );
    assert!(is_token(&read(&src).await, MARKER_TOKEN));
    let text = std::fs::read_to_string(dump).unwrap();
    assert!(!text.contains("CARGO_MANIFEST_DIR"));
    assert!(text.contains("MINE=1"));
}

// OPS-5, TST-11: nothing of gh gets out.
#[tokio::test]
async fn ops5_nothing_of_gh_gets_out() {
    let leak = format!("echo {MARKER_TOKEN}\necho {MARKER_TOKEN} >&2\n");
    let bodies = [
        format!("{leak}exit 3"),
        format!("{leak}kill -9 $$"),
        format!("echo {MARKER_TOKEN} >&2\nexit 0"),
        format!("{leak}echo {MARKER_TOKEN}"),
        format!("echo {MARKER_TOKEN} >&2\nprintf '\\377\\n'"),
        format!("echo 'no oauth token found for github.com' {MARKER_TOKEN} >&2\nexit 1"),
        format!("{leak}exit 0"),
    ];
    for body in bodies {
        let rig = Rig::new();
        let src = rig.gh(&body);
        let r = read(&src).await;
        let shown = format!("{r:?}");
        if matches!(r, TokenRead::Token(_)) {
            assert_eq!(shown, "Token(Token(…))");
        } else {
            assert!(!shown.contains(MARKER_TOKEN), "{shown}");
        }
    }
    assert_eq!(format!("{:?}", Token::new(MARKER_TOKEN.into())), "Token(…)");
}
