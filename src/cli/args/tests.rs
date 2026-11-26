use super::*;

fn argv(line: &str) -> Vec<String> {
    line.split_whitespace().map(String::from).collect()
}

fn p(line: &str) -> Result<Parsed, String> {
    parse(&argv(line), &Env::default())
}

fn cmd(line: &str) -> Command {
    p(line).unwrap_or_else(|e| panic!("{line}: {e}")).command
}

fn pr(s: &str) -> Option<String> {
    Some(s.to_string())
}

fn s(x: &str) -> Option<String> {
    Some(x.to_string())
}

#[test]
fn cli1_every_command_parses() {
    // CLI-1
    let mut checks = Ask::new(Until::ChecksDone);
    checks.sha = s("1A2b3c4");
    checks.rerun_since = s("k3x9q2mz-400");
    checks.fail_fast = true;
    checks.since = s("k3x9q2mz-418");
    let mut changed = Ask::new(Until::Changed);
    changed.kind = s("activity,merged");

    let table: Vec<(&str, Command)> = vec![
        (
            "list",
            Command::List {
                all: false,
                since: None,
                json: false,
            },
        ),
        (
            "list --all --since k3x9q2mz-418 --json",
            Command::List {
                all: true,
                since: s("k3x9q2mz-418"),
                json: true,
            },
        ),
        (
            "show",
            Command::Show {
                pr: None,
                fresh: false,
                json: false,
            },
        ),
        (
            "show 12 --fresh --json",
            Command::Show {
                pr: pr("12"),
                fresh: true,
                json: true,
            },
        ),
        (
            "wait 12 --until checks-done --sha 1A2b3c4 --rerun-since k3x9q2mz-400 --fail-fast --since k3x9q2mz-418 --timeout 300 --json",
            Command::Wait {
                pr: pr("12"),
                cond: checks,
                timeout: Some(300),
                json: true,
            },
        ),
        (
            "wait --until changed --kind activity,merged",
            Command::Wait {
                pr: None,
                cond: changed,
                timeout: None,
                json: false,
            },
        ),
        (
            "wait yachiko/circitor#12 --until merged --timeout 0",
            Command::Wait {
                pr: pr("yachiko/circitor#12"),
                cond: Ask::new(Until::Merged),
                timeout: Some(0),
                json: false,
            },
        ),
        (
            "changes",
            Command::Changes {
                pr: None,
                since: None,
                json: false,
            },
        ),
        (
            "changes 12 --since k3x9q2mz-418 --json",
            Command::Changes {
                pr: pr("12"),
                since: s("k3x9q2mz-418"),
                json: true,
            },
        ),
        (
            "track 12 --json",
            Command::Track {
                pr: "12".into(),
                json: true,
            },
        ),
        (
            "untrack yachiko/circitor#12",
            Command::Untrack {
                pr: "yachiko/circitor#12".into(),
                json: false,
            },
        ),
        ("refresh --json", Command::Refresh { json: true }),
        ("status", Command::Status { json: false }),
        ("stop", Command::Stop),
        (
            "daemon",
            Command::Daemon {
                spawned: false,
                check_config: false,
            },
        ),
        (
            "daemon --spawned",
            Command::Daemon {
                spawned: true,
                check_config: false,
            },
        ),
        (
            "daemon --check-config",
            Command::Daemon {
                spawned: false,
                check_config: true,
            },
        ),
        ("guide", Command::Guide),
        ("version", Command::Version),
        ("--version", Command::Version),
        ("-V", Command::Version),
    ];
    for (line, want) in table {
        assert_eq!(cmd(line), want, "{line}");
    }
}

#[test]
fn ntf11_notify_parses() {
    // NTF-11
    let mut cond = Ask::new(Until::ChecksDone);
    cond.sha = s("1a2b3c4");
    assert_eq!(
        cmd("notify 12 --until checks-done --sha 1a2b3c4 --for 3600 --json"),
        Command::Notify {
            pr: pr("12"),
            what: NotifyWhat::Register {
                cond: cond.clone(),
                life: Some(3600)
            },
            json: true,
        }
    );
    assert_eq!(
        cmd("notify 12 --until checks-done --sha 1a2b3c4"),
        Command::Notify {
            pr: pr("12"),
            what: NotifyWhat::Register { cond, life: None },
            json: false,
        }
    );
    assert_eq!(
        cmd("notify --cancel"),
        Command::Notify {
            pr: None,
            what: NotifyWhat::Cancel,
            json: false
        }
    );
    assert_eq!(
        cmd("notify 12 --cancel --json"),
        Command::Notify {
            pr: pr("12"),
            what: NotifyWhat::Cancel,
            json: true
        }
    );
    for line in [
        "notify 12",
        "notify --cancel --until merged",
        "notify --cancel --for 60",
        "notify --until merged --for 59",
        "notify --until merged --for 86401",
        "notify --until merged --timeout 5",
        "notify --until merged --fail-fast",
    ] {
        one_line(line);
    }
}

fn one_line(line: &str) {
    let e = p(line).expect_err(line);
    assert!(!e.contains('\n'), "{line}: {e:?}");
    assert!(!e.contains('\x1b'), "{line}: {e:?}");
    assert!(e.ends_with("; see: circitor help"), "{line}: {e:?}");
}

#[test]
fn cli1_usage_errors() {
    // CLI-1, CLI-5
    for line in [
        "list --bogus",
        "stop --json",
        "guide --json",
        "wait 12",
        "wait 12 --until done",
        "wait 12 --until merged --timeout 3601",
        "wait 12 --until merged --timeout -1",
        "wait 12 --until checks-done --sha 12345",
        "wait 12 --until checks-done --sha zzzzzzz",
        "wait 12 --until merged --sha 1a2b3c4",
        "wait 12 --until merged --rerun-since k3x9q2mz-400",
        "wait 12 --until changed --fail-fast",
        "wait 12 --until checks-done --kind activity",
        "track",
        "untrack",
        "daemon --spawned --check-config",
        "",
        "frobnicate",
    ] {
        one_line(line);
    }
}

#[test]
fn cli1_no_spawn() {
    // CLI-7
    assert!(p("--no-spawn status").unwrap().no_spawn);
    assert!(p("status --no-spawn").unwrap().no_spawn);
    assert!(p("wait 12 --until merged --no-spawn").unwrap().no_spawn);
    assert!(!p("status").unwrap().no_spawn);
    let with = |v: &str| {
        let mut env = Env::default();
        env.vars.insert("CIRCITOR_NO_SPAWN".into(), v.into());
        parse(&argv("status"), &env).unwrap().no_spawn
    };
    assert!(with("1"));
    assert!(!with("0"));
    assert!(!with(""));
    assert!(!with("true"));
}

#[test]
fn cli12_help_is_plain() {
    // CLI-12
    for line in ["help", "--help", "-h"] {
        let Command::Help(text) = cmd(line) else {
            panic!("{line}");
        };
        assert!(!text.contains('\x1b'), "{line}");
        for name in [
            "list", "show", "wait", "notify", "changes", "track", "untrack", "refresh", "status",
            "stop", "daemon", "guide", "version", "help",
        ] {
            assert!(
                text.lines().any(|l| l.trim_start().starts_with(name)),
                "{line}: {name} missing from\n{text}"
            );
        }
    }
    let Command::Help(text) = cmd("wait --help") else {
        panic!("wait --help");
    };
    assert!(text.contains("--until") && !text.contains('\x1b'));
}
