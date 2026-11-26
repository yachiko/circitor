use super::*;
use crate::clock::Time;
use crate::model::tests::sample_record;

#[test]
fn cli2_pr_arg() {
    // CLI-2
    let full = PrName::parse("yachiko/circitor#12").unwrap();
    for a in [
        "yachiko/circitor#12",
        "https://github.com/yachiko/circitor/pull/12",
        "https://github.com/yachiko/circitor/pull/12/files",
    ] {
        assert_eq!(pr_arg(Some(a)), Ok(PrArg::Full(full.clone())), "{a}");
    }
    assert_eq!(pr_arg(Some("12")), Ok(PrArg::Number(12)));
    assert_eq!(pr_arg(None), Ok(PrArg::Bare));
    let e = pr_arg(Some("#12")).unwrap_err();
    assert!(e.contains("write 12"), "{e}");
    for a in [
        "0",
        "-3",
        "abc",
        "yachiko/circitor",
        "",
        "99999999999999999999999",
    ] {
        let e = pr_arg(Some(a)).expect_err(a);
        assert!(e.contains("owner/repo#12"), "{a}: {e}");
        assert!(!e.contains('\n'));
    }
}

#[test]
fn cli2_repo_of_origin() {
    // CLI-2
    let want = Some(("yachiko".to_string(), "circitor".to_string()));
    for u in [
        "https://github.com/yachiko/circitor",
        "https://github.com/yachiko/circitor.git",
        "https://github.com/yachiko/circitor/",
        "https://GitHub.com/yachiko/circitor.git",
        "git@github.com:yachiko/circitor",
        "git@github.com:yachiko/circitor.git",
        "ssh://git@github.com/yachiko/circitor",
        "ssh://git@github.com/yachiko/circitor.git",
    ] {
        assert_eq!(repo_of_origin(u), want, "{u}");
    }
    for u in [
        "https://gitlab.com/a/b.git",
        "https://github.example.com/a/b",
        "/srv/git/r.git",
        "https://github.com/a",
        "https://github.com/a/b/c",
        "",
    ] {
        assert_eq!(repo_of_origin(u), None, "{u}");
    }
}

fn t(s: &str) -> Time {
    Time::parse(s).unwrap()
}

fn rec(name: &str, state: PrState, closed: Option<&str>) -> Record {
    let mut r = sample_record();
    r.pr = PrName::parse(name).unwrap();
    r.head = "feature".into();
    r.author = Some("Yachiko".into());
    r.state = state;
    r.closed_at = closed.map(t);
    r.merged_at = None;
    r.fetched_at = t("2026-11-20T00:00:00Z");
    r
}

#[test]
fn cli2_match_branch() {
    // CLI-2
    let name = |s: &str| PrName::parse(s).unwrap();
    let m = |rs: &[Record], ended, login| {
        match_branch(rs, "yachiko", "circitor", "feature", login, ended)
    };
    let open = rec("yachiko/circitor#12", PrState::Open, None);
    assert_eq!(
        m(std::slice::from_ref(&open), false, Some("yachiko")),
        BranchMatch::Open(name("yachiko/circitor#12"))
    );
    // A fork contributor's PR on the same branch isn't matched.
    let mut fork = open.clone();
    fork.pr = name("yachiko/circitor#13");
    fork.author = Some("someone".into());
    assert_eq!(
        m(&[open.clone(), fork], false, Some("yachiko")),
        BranchMatch::Open(name("yachiko/circitor#12"))
    );
    // Login and repository without case, the branch with it.
    assert_eq!(
        match_branch(
            std::slice::from_ref(&open),
            "YACHIKO",
            "Circitor",
            "feature",
            Some("YACHIKO"),
            false
        ),
        BranchMatch::Open(name("yachiko/circitor#12"))
    );
    assert_eq!(
        match_branch(
            std::slice::from_ref(&open),
            "yachiko",
            "circitor",
            "Feature",
            Some("yachiko"),
            false
        ),
        BranchMatch::None
    );
    // Two open ones, in PrName's order.
    let mut second = open.clone();
    second.pr = name("yachiko/circitor#9");
    assert_eq!(
        m(&[open.clone(), second], false, Some("yachiko")),
        BranchMatch::Several(vec![
            name("yachiko/circitor#9"),
            name("yachiko/circitor#12")
        ])
    );
    // Ended: the later closed_at wins; open comes before ended.
    let merged = rec(
        "yachiko/circitor#5",
        PrState::Merged,
        Some("2026-11-10T00:00:00Z"),
    );
    let closed = rec(
        "yachiko/circitor#6",
        PrState::Closed,
        Some("2026-11-11T00:00:00Z"),
    );
    assert_eq!(
        m(&[merged.clone(), closed.clone()], true, Some("yachiko")),
        BranchMatch::Ended(name("yachiko/circitor#6"))
    );
    assert_eq!(
        m(
            &[merged.clone(), closed.clone(), open.clone()],
            true,
            Some("yachiko")
        ),
        BranchMatch::Open(name("yachiko/circitor#12"))
    );
    // A gone one counts by fetched_at.
    let mut gone = rec("yachiko/circitor#7", PrState::Gone, None);
    gone.fetched_at = t("2026-11-15T00:00:00Z");
    assert_eq!(
        m(
            &[merged.clone(), closed.clone(), gone.clone()],
            true,
            Some("yachiko")
        ),
        BranchMatch::Ended(name("yachiko/circitor#7"))
    );
    // A missing closed_at counts as merged_at.
    let mut no_close = rec("yachiko/circitor#8", PrState::Merged, None);
    no_close.merged_at = Some(t("2026-11-12T00:00:00Z"));
    assert_eq!(
        m(&[closed, no_close], true, Some("yachiko")),
        BranchMatch::Ended(name("yachiko/circitor#8"))
    );
    // An ended one is None unless `ended_too`.
    assert_eq!(m(&[merged], false, Some("yachiko")), BranchMatch::None);
    // No login matches nothing.
    assert_eq!(m(&[open], true, None), BranchMatch::None);
}
