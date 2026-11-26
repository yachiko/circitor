//! The inner `main` and `deps_from`, which never reach `start` and never read the token: a hand-built
//! `Env` whose home is a `ShortDir`, and a `BufferSink`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use super::entry::{deps_from, run_main};
use crate::clock::SystemClock;
use crate::config::{Env, Platform, resolve};
use crate::log::{BufferSink, Level, Logger};
use crate::testkit::ShortDir;

fn env(home: &ShortDir, vars: &[(&str, &str)]) -> Env {
    Env {
        vars: vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<BTreeMap<_, _>>(),
        home: Some(home.path().to_path_buf()),
    }
}

fn write_config(env: &Env, text: &str) {
    let paths = resolve(env, Platform::current()).unwrap();
    std::fs::create_dir_all(paths.config_file.parent().unwrap()).unwrap();
    std::fs::write(&paths.config_file, text).unwrap();
}

/// Everything under the home but the config file's directories.
fn created(home: &ShortDir) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![home.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(home.path())
                .unwrap()
                .display()
                .to_string();
            if path.is_dir() {
                stack.push(path.clone());
            }
            if !rel.starts_with(".config") {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

fn lines(sink: &BufferSink) -> Vec<String> {
    sink.contents().lines().map(str::to_string).collect()
}

fn logger(sink: &BufferSink) -> Logger {
    Logger::new(SystemClock::new(), Arc::new(sink.clone()), Level::Info)
}

const LOOPBACK: (&str, &str) = ("CIRCITOR_TEST_GITHUB", "http://127.0.0.1:1/graphql");

// OPS-1: a config error stops the daemon before any file is touched.
#[test]
fn ops1_config_error_before_start() {
    let home = ShortDir::new();
    let e = env(&home, &[LOOPBACK]);
    write_config(&e, "interval = 30\nspeed = 3\n");
    let sink = BufferSink::default();
    assert_eq!(run_main(true, e.clone(), Arc::new(sink.clone())), 1);
    let lines = lines(&sink);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(" error "), "{lines:?}");
    assert!(lines[0].contains("speed"), "{lines:?}");
    let paths = resolve(&e, Platform::current()).unwrap();
    assert!(!paths.state_dir.exists());
    assert!(created(&home).is_empty(), "{:?}", created(&home));
}

// OPS-4: a socket path over 103 bytes stops the daemon, naming CIRCITOR_SOCKET, with nothing created.
#[test]
fn ops4_path_error_before_start() {
    let home = ShortDir::new();
    let long = format!("/tmp/{}", "a".repeat(99));
    assert_eq!(long.len(), 104);
    let e = env(&home, &[LOOPBACK, ("CIRCITOR_SOCKET", &long)]);
    let sink = BufferSink::default();
    assert_eq!(run_main(false, e, Arc::new(sink.clone())), 1);
    let lines = lines(&sink);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains(" error "), "{lines:?}");
    assert!(lines[0].contains("CIRCITOR_SOCKET"), "{lines:?}");
    assert!(created(&home).is_empty(), "{:?}", created(&home));
    assert!(!std::path::Path::new(&long).exists());
}

// OPS-9: an unknown CIRCITOR_LOG is `info`, with one warning line.
#[test]
fn ops9_unknown_log_level() {
    let home = ShortDir::new();
    let e = env(&home, &[LOOPBACK, ("CIRCITOR_LOG", "loud")]);
    write_config(&e, "speed = 3\n");
    let sink = BufferSink::default();
    assert_eq!(run_main(true, e.clone(), Arc::new(sink.clone())), 1);
    let lines = lines(&sink);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains(" warn log level unknown"), "{lines:?}");
    assert!(!lines[0].contains("loud"), "{lines:?}");
    assert!(lines[1].contains(" error "), "{lines:?}");

    // With a valid config, the level is `info`.
    write_config(&e, "interval = 30\n");
    let sink = BufferSink::default();
    let Ok(deps) = deps_from(true, &e, Arc::new(sink.clone()), &logger(&sink)) else {
        panic!("deps: {}", sink.contents());
    };
    assert_eq!(deps.level, Level::Info);
    assert!(created(&home).is_empty(), "{:?}", created(&home));
}

// TST-1: a test build without a loopback CIRCITOR_TEST_GITHUB stops at start, naming it.
#[cfg(feature = "test-endpoint")]
#[test]
fn tst1_endpoint_error_before_start() {
    for vars in [
        vec![],
        vec![("CIRCITOR_TEST_GITHUB", "http://example.com/graphql")],
    ] {
        let home = ShortDir::new();
        let e = env(&home, &vars);
        write_config(&e, "interval = 30\n");
        let sink = BufferSink::default();
        assert_eq!(run_main(true, e, Arc::new(sink.clone())), 1);
        let lines = lines(&sink);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(" error "), "{lines:?}");
        assert!(lines[0].contains("CIRCITOR_TEST_GITHUB"), "{lines:?}");
        assert!(created(&home).is_empty(), "{:?}", created(&home));
    }
}

// TST-1: CIRCITOR_TEST_STOP_DELAY seconds become the stop's delay.
#[cfg(feature = "test-endpoint")]
#[test]
fn tst1_stop_delay() {
    let home = ShortDir::new();
    let sink = BufferSink::default();
    let e = env(&home, &[LOOPBACK, ("CIRCITOR_TEST_STOP_DELAY", "3")]);
    let Ok(deps) = deps_from(false, &e, Arc::new(sink.clone()), &logger(&sink)) else {
        panic!("deps: {}", sink.contents());
    };
    assert_eq!(deps.stop_delay, Some(Duration::from_secs(3)));
    assert_eq!(deps.mode, crate::daemon::Mode::Foreground);
    assert!(deps.signals);

    let e = env(&home, &[LOOPBACK]);
    let Ok(deps) = deps_from(true, &e, Arc::new(sink.clone()), &logger(&sink)) else {
        panic!("deps: {}", sink.contents());
    };
    assert_eq!(deps.stop_delay, None);
    assert_eq!(deps.mode, crate::daemon::Mode::Spawned);
    assert!(sink.contents().is_empty(), "{}", sink.contents());
    assert!(created(&home).is_empty(), "{:?}", created(&home));
}

// STA-11: only a spawned daemon whose holder answers exits 0.
#[test]
fn sta11_codes() {
    use super::StartError;
    use std::path::PathBuf;
    let p = || PathBuf::from("/tmp/ct/circitor.sock");
    assert_eq!(StartError::AlreadyRunning(p()).code(), 0);
    for e in [
        StartError::HolderNotAnswering(p()),
        StartError::ForegroundHolder(p()),
        StartError::TakeoverTimedOut(p()),
        StartError::NotASocket(p()),
        StartError::NoHolder(p()),
    ] {
        assert_eq!(e.code(), 1, "{e}");
        assert!(e.to_string().contains("/tmp/ct/circitor.sock"), "{e}");
    }
}
