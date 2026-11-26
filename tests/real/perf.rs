//! OPS-17's figures (proposal), measured on the test build, with 200 tracked PRs on the fake
//! GitHub. Each test prints what it measured.

use std::time::Duration;

use crate::fixture::{Fixture, real};
use crate::{ms, s};

/// `interval = 10`: two polls fit in a test's 30 s.
const CONFIG_PERF: &str = "exit_after = 60\ninterval = 10\n";

/// 199 more PRs beside `o/r#1`: 200 tracked.
fn two_hundred(fx: &Fixture) {
    fx.gh.world(|w| {
        let r = w.repo("o", "big");
        w.open_prs(r, 199, s(60));
    });
}

/// `list --json` until it answers with all 200 records and nothing pending: answered from memory.
async fn listed(fx: &Fixture) {
    let start = fx.mono();
    loop {
        let out = fx.cli(&["list", "--json"]).await;
        if out.code == Some(0) {
            let json: serde_json::Value = serde_json::from_str(&out.stdout).expect("list --json");
            let records = json["records"].as_array().map_or(0, Vec::len);
            let pending = json["pending"].as_array().map_or(0, Vec::len);
            if records == 200 && pending == 0 {
                return;
            }
        } else {
            // API-18: `starting` while the first poll runs.
            assert_eq!(out.code, Some(9), "{out:?}");
        }
        assert!(fx.mono().since(start) < s(20), "never listed 200 PRs");
        fx.sleep(ms(200)).await;
    }
}

fn last_poll(st: &serde_json::Value) -> String {
    st["last_poll"]["at"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Waits for the poll after the one `status` last showed; at most 15 s.
async fn next_poll(fx: &Fixture) {
    let before = last_poll(&fx.status().await);
    let start = fx.mono();
    loop {
        fx.sleep(ms(200)).await;
        if last_poll(&fx.status().await) != before {
            return;
        }
        assert!(fx.mono().since(start) < s(15), "no poll after {before}");
    }
}

/// `utime + stime` of `pid`, in clock ticks (Linux).
fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("/proc/<pid>/stat");
    let rest = stat.rsplit_once(')').expect("the command's parenthesis").1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // Fields 14 and 15 of proc(5); `rest` starts at field 3.
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    utime + stime
}

/// The resident memory of `pid`, in kB.
async fn resident_kb(fx: &Fixture, pid: u32) -> u64 {
    if cfg!(target_os = "macos") {
        let mut ps = tokio::process::Command::new("/bin/ps");
        ps.args(["-o", "rss=", "-p", &pid.to_string()]);
        let out = fx.run_to_end(&mut ps, "ps").await;
        out.stdout.trim().parse().expect("ps's rss")
    } else {
        let status =
            std::fs::read_to_string(format!("/proc/{pid}/status")).expect("/proc/<pid>/status");
        status
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .expect("VmRSS")
    }
}

// TST-9:
// OPS-17: from a cold start to the first `list` answer within 5 s.
#[tokio::test(flavor = "multi_thread")]
async fn ops17_cold_start_within_5_s() {
    real(CONFIG_PERF, |fx| async move {
        let out = fx.cli(&["list"]).await;
        assert_eq!(out.code, Some(0), "{out:?}");
        eprintln!(
            "OPS-17: cold start to the first list answer: {:?}",
            out.took
        );
        assert!(out.took < s(5), "the first list took {:?}", out.took);
    })
    .await;
}

// TST-9:
// OPS-17: with 200 tracked PRs, a `list` answered from memory returns within 50 ms: the median of
// 11 runs of the CLI process, start to exit.
#[tokio::test(flavor = "multi_thread")]
async fn ops17_list_from_memory_within_50_ms() {
    real(CONFIG_PERF, |fx| async move {
        two_hundred(&fx);
        listed(&fx).await;
        let mut took: Vec<Duration> = Vec::new();
        for _ in 0..11 {
            let out = fx.cli(&["list"]).await;
            assert_eq!(out.code, Some(0), "{out:?}");
            took.push(out.took);
        }
        took.sort();
        let median = took[5];
        eprintln!("OPS-17: list from memory, 200 PRs: median {median:?} of {took:?}");
        assert!(median <= ms(50), "the median list took {median:?}");
    })
    .await;
}

// TST-9:
// OPS-17: with 200 tracked PRs, the daemon's resident memory is under 30 MB after two polls.
#[tokio::test(flavor = "multi_thread")]
async fn ops17_resident_memory_under_30_mb() {
    real(CONFIG_PERF, |fx| async move {
        two_hundred(&fx);
        listed(&fx).await;
        next_poll(&fx).await;
        let pid = fx.daemon_pid();
        let kb = resident_kb(&fx, pid).await;
        eprintln!("OPS-17: resident memory with 200 PRs after two polls: {kb} kB");
        assert!(kb < 30 * 1024, "resident {kb} kB");
    })
    .await;
}

// TST-9:
// OPS-17: on Linux, between two polls the daemon's CPU time grows by at most 2 clock ticks over 5 s.
#[tokio::test(flavor = "multi_thread")]
async fn ops17_no_cpu_between_polls() {
    if cfg!(target_os = "macos") {
        eprintln!("OPS-17: no /proc on macOS: the CPU check is Linux's");
        return;
    }
    real(CONFIG_PERF, |fx| async move {
        two_hundred(&fx);
        listed(&fx).await;
        next_poll(&fx).await;
        let pid = fx.daemon_pid();
        let poll = last_poll(&fx.status().await);
        let before = cpu_ticks(pid);
        fx.sleep(s(5)).await;
        let after = cpu_ticks(pid);
        assert_eq!(
            last_poll(&fx.status().await),
            poll,
            "a poll ran inside the 5 s: the window wasn't between two polls"
        );
        eprintln!(
            "OPS-17: CPU between two polls, 5 s: {} ticks",
            after - before
        );
        assert!(after - before <= 2, "{} ticks in 5 s", after - before);
    })
    .await;
}
