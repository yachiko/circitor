use super::GUIDE;
use crate::cli::args::{Command, parse};
use crate::config::Env;

const SKILL: &str = include_str!("../../../skills/circitor/SKILL.md");
const SECTION: &str = include_str!("../../../AGENTS.section.md");

#[test]
fn cli11_guide_is_one_text() {
    // CLI-11, OPS-12
    let body = SKILL
        .strip_prefix("---\n")
        .and_then(|r| r.split_once("\n---\n"))
        .map(|(front, body)| {
            assert!(
                front.starts_with("name: circitor\ndescription: "),
                "{front}"
            );
            body
        })
        .expect("frontmatter");
    assert_eq!(body.strip_prefix('\n').unwrap_or(body), GUIDE);
    let rest = SECTION.strip_prefix("## circitor\n").expect("heading");
    assert_eq!(rest.strip_prefix('\n').unwrap_or(rest), GUIDE);
    assert!(
        GUIDE.lines().count() < 120,
        "{} lines",
        GUIDE.lines().count()
    );
}

#[test]
fn cli11_guide_says_it_all() {
    // CLI-11, U8
    for word in [
        "list",
        "show",
        "wait",
        "notify",
        "changes",
        "track",
        "untrack",
        "refresh",
        "status",
        "stop",
        "--sha",
        "--rerun-since",
        "--fail-fast",
        "--kind",
        "show --fresh",
        "--no-spawn",
        "notify --cancel",
        "100 s",
        "`gh` doesn't help",
    ] {
        assert!(GUIDE.contains(word), "guide lacks {word:?}");
    }
    for code in 0..=11 {
        let prefix = format!("{code} ");
        assert!(
            GUIDE.lines().any(|l| l.starts_with(&prefix)),
            "guide lacks exit code {code}"
        );
    }
}

#[test]
fn cli11_examples_parse() {
    // CLI-11
    let mut n = 0;
    for line in GUIDE.lines().filter(|l| l.starts_with("circitor ")) {
        let cmd = line.split(" # ").next().unwrap();
        let args: Vec<String> = cmd.split_whitespace().skip(1).map(String::from).collect();
        let parsed = parse(&args, &Env::default()).unwrap_or_else(|e| panic!("{line}: {e}"));
        assert!(!matches!(parsed.command, Command::Help(_)), "{line}");
        n += 1;
    }
    assert!(n >= 12, "{n} examples");
}
