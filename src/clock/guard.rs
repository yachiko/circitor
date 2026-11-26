//! TST-2's guard: all time comes from the clock, no thread sleeps, and randomness is seeded.
//!
//! The patterns are built from pieces so this file doesn't hold them as they'd appear in code.

use std::path::Path;

/// Calls that read or wait on the system's time: only `src/clock/` may hold them.
fn time_patterns() -> Vec<String> {
    [
        ["Instant", "::now"],
        ["SystemTime", "::now"],
        ["Utc", "::now"],
        ["Local", "::now"],
        ["tokio::", "time::"],
    ]
    .iter()
    .map(|p| p.concat())
    .collect()
}

/// Patterns no file may hold, `src/clock/` included.
fn everywhere_patterns() -> Vec<String> {
    [["thread::", "sleep"], ["Rng", "::new("]]
        .iter()
        .map(|p| p.concat())
        .collect()
}

/// `fastrand::` followed by a lower-case letter: a call on the global, unseeded generator.
fn unseeded_fastrand(line: &str) -> bool {
    let prefix = ["fast", "rand::"].concat();
    line.match_indices(&prefix).any(|(i, _)| {
        line[i + prefix.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
    })
}

/// The violations in one file, `rel` its path under `src/` with `/` between the parts.
fn check_source(rel: &str, text: &str) -> Vec<String> {
    let in_clock = rel.starts_with("clock/");
    let time = time_patterns();
    let everywhere = everywhere_patterns();
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let at = format!("src/{rel}:{}", n + 1);
        if !in_clock {
            for p in time.iter().filter(|p| line.contains(p.as_str())) {
                found.push(format!("{at}: {p} outside src/clock/"));
            }
        }
        for p in everywhere.iter().filter(|p| line.contains(p.as_str())) {
            found.push(format!("{at}: {p}"));
        }
        if unseeded_fastrand(line) {
            found.push(format!("{at}: unseeded fastrand"));
        }
    }
    found
}

/// Every `.rs` file under `root`, checked.
fn scan(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let text = std::fs::read_to_string(&path).unwrap();
                found.extend(check_source(&rel, &text));
            }
        }
    }
    found
}

// TST-2: only the clock reads time; nothing sleeps a thread; randomness is seeded.
#[test]
fn tst2_only_the_clock_reads_time() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let found = scan(&src);
    assert!(found.is_empty(), "TST-2:\n{}", found.join("\n"));

    // And it fails on a file that holds each pattern.
    let dir = crate::testkit::ShortDir::new();
    let root = dir.path();
    std::fs::create_dir_all(root.join("poller")).unwrap();
    std::fs::create_dir_all(root.join("clock")).unwrap();
    let mut bad: Vec<String> = time_patterns()
        .iter()
        .chain(everywhere_patterns().iter())
        .map(|p| format!("    let x = {p}"))
        .collect();
    bad.push(format!(
        "    let n = {}u32(..10);",
        ["fast", "rand::"].concat()
    ));
    bad.push(format!(
        "    // {} in a comment",
        ["thread::", "sleep"].concat()
    ));
    bad.push(format!(
        "    let r = {}Rng::with_seed(7);",
        ["fast", "rand::"].concat()
    ));
    let text = bad.join("\n");
    std::fs::write(root.join("poller").join("bad.rs"), &text).unwrap();
    std::fs::write(root.join("clock").join("ok.rs"), &text).unwrap();
    let found = scan(root);
    // Outside the clock: every pattern, one line each (5 time, 2 everywhere, 1 fastrand).
    let outside: Vec<_> = found
        .iter()
        .filter(|f| f.starts_with("src/poller/bad.rs"))
        .collect();
    assert_eq!(outside.len(), 8, "{found:#?}");
    for n in 1..=8 {
        assert!(
            outside.iter().any(|f| f.contains(&format!("bad.rs:{n}:"))),
            "line {n}: {found:#?}"
        );
    }
    // In the clock: only the thread sleep, the unseeded generator and the fastrand call.
    let inside: Vec<_> = found
        .iter()
        .filter(|f| f.starts_with("src/clock/ok.rs"))
        .collect();
    assert_eq!(inside.len(), 3, "{found:#?}");
}
