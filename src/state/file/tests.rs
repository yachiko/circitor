use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::clock::{TestClock, Time};
use crate::log::{BufferSink, Level, Logger};
use crate::state::{FileEntry, FileSource, Flush};
use crate::status::{WarningCode, Warnings};
use crate::testkit::{MARKER_TOKEN, ShortDir};

const DIR: &str = "/state";

fn state_path() -> PathBuf {
    PathBuf::from(DIR).join("state.json")
}

fn tmp() -> PathBuf {
    PathBuf::from(DIR).join("state.json.tmp")
}

fn bad() -> PathBuf {
    PathBuf::from(DIR).join("state.json.bad")
}

fn testdata(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/state")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn entry(n: u64) -> FileEntry {
    serde_json::from_value(json!({
        "pr": format!("o/r#{n}"), "id": format!("PR_kwDO{n}"), "source": "manual",
        "login": "octocat", "ended": null,
        "seen": {"comment": {"at": null, "ids": []}, "review": {"at": null, "ids": []}}
    }))
    .unwrap()
}

fn content(ns: &[u64]) -> FileContent {
    FileContent {
        prs: ns.iter().map(|n| entry(*n)).collect(),
    }
}

/// A `FileSource` of the test's own.
struct Source {
    content: Mutex<FileContent>,
    tx: tokio::sync::watch::Sender<u64>,
}

impl Source {
    fn new(c: FileContent) -> Arc<Source> {
        Arc::new(Source {
            content: Mutex::new(c),
            tx: tokio::sync::watch::channel(0).0,
        })
    }

    fn set(&self, c: FileContent) {
        *self.content.lock().unwrap() = c;
        self.tx.send_modify(|v| *v += 1);
    }
}

impl FileSource for Source {
    fn file_content(&self) -> FileContent {
        self.content.lock().unwrap().clone()
    }

    fn file_changed(&self) -> tokio::sync::watch::Receiver<u64> {
        self.tx.subscribe()
    }
}

struct Rig {
    clock: Arc<TestClock>,
    sink: BufferSink,
    logger: Logger,
    warnings: Warnings,
    fs: Arc<FaultFs>,
}

fn rig() -> Rig {
    let clock = TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap());
    let sink = BufferSink::default();
    let logger = Logger::new(clock.clone(), Arc::new(sink.clone()), Level::Debug);
    Rig {
        warnings: Warnings::new(clock.clone(), logger.clone()),
        clock,
        sink,
        logger,
        fs: Arc::new(FaultFs::new()),
    }
}

impl Rig {
    fn spawn_with(
        &self,
        fs: Arc<dyn StateFs>,
        path: PathBuf,
        source: &Arc<Source>,
        writable: bool,
    ) -> Arc<Saver> {
        Saver::spawn(SaverDeps {
            fs,
            path,
            source: source.clone(),
            clock: self.clock.clone(),
            logger: self.logger.clone(),
            warnings: self.warnings.clone(),
            activity: self.clock.activity(),
            writable,
        })
    }

    fn spawn(&self, source: &Arc<Source>) -> Arc<Saver> {
        self.spawn_with(self.fs.clone(), state_path(), source, true)
    }

    fn load(&self) -> Loaded {
        load(&*self.fs, &state_path(), &self.logger, &self.warnings)
    }

    fn loaded(&self) -> FileContent {
        match self.load() {
            Loaded::Content(c) => c,
            _ => panic!("the file didn't load as content"),
        }
    }

    fn unwritable(&self) -> bool {
        self.warnings
            .since(WarningCode::StateFileUnwritable, "")
            .is_some()
    }

    async fn advance(&self, secs: u64) {
        self.clock.advance(Duration::from_secs(secs)).await;
    }
}

fn writes(fs: &FaultFs) -> usize {
    fs.calls(Step::Rename)
}

// TST-10: a write cut at any step leaves the old or the new file.
async fn cut_after(step: Step, power_cut: bool) -> FileContent {
    let r = rig();
    let old = content(&[1]);
    r.fs.put(&state_path(), &encode(&old));
    r.fs.stop_after(step);
    let source = Source::new(content(&[1, 2]));
    let saver = r.spawn(&source);
    r.clock.settle().await;
    saver.abort();
    if power_cut {
        r.fs.power_cut();
    } else {
        r.fs.revive();
    }
    let got = r.loaded();
    assert!(got == old || got == content(&[1, 2]), "{step:?}: {got:?}");
    got
}

// TST-10
#[tokio::test]
async fn tst10_cut_after_each_step() {
    let (old, new) = (content(&[1]), content(&[1, 2]));
    for power_cut in [false, true] {
        assert_eq!(cut_after(Step::WriteTmp, power_cut).await, old);
        assert_eq!(cut_after(Step::SyncFile, power_cut).await, old);
        let after_rename = cut_after(Step::Rename, power_cut).await;
        // Without a synced directory a power cut forgets the rename.
        assert_eq!(
            after_rename,
            if power_cut { old.clone() } else { new.clone() }
        );
        assert_eq!(cut_after(Step::SyncDir, power_cut).await, new);
    }
}

// TST-10: `FaultFs` is faithful enough to fail a write that skips a sync.
#[test]
fn tst10_fault_fs_loses_what_wasn_t_synced() {
    let fs = FaultFs::new();
    fs.put(&state_path(), b"old");
    fs.write_tmp(&tmp(), b"new").unwrap();
    fs.rename(&tmp(), &state_path()).unwrap();
    fs.sync_dir(Path::new(DIR)).unwrap();
    fs.power_cut();
    assert_eq!(fs.file(&state_path()), Some(Vec::new()));
    let fs = FaultFs::new();
    fs.put(&state_path(), b"old");
    fs.write_tmp(&tmp(), b"new").unwrap();
    fs.sync_file(&tmp()).unwrap();
    fs.rename(&tmp(), &state_path()).unwrap();
    fs.power_cut();
    assert_eq!(fs.file(&state_path()), Some(b"old".to_vec()));
    fs.stop_after(Step::Read);
    fs.read(&state_path()).unwrap();
    assert!(fs.read(&state_path()).is_err());
    fs.revive();
    fs.fail_at(Step::Read);
    assert!(fs.read(&state_path()).is_err());
    fs.heal();
    assert!(fs.read(&state_path()).is_ok());
}

// TST-10
#[tokio::test]
async fn tst10_tmp_left_by_a_crash() {
    let r = rig();
    r.fs.put(&state_path(), &encode(&content(&[1])));
    r.fs.put(&tmp(), b"\x00rubbish{");
    assert_eq!(r.loaded(), content(&[1]));
    let source = Source::new(content(&[1, 2]));
    let _saver = r.spawn(&source);
    r.clock.settle().await;
    assert_eq!(r.loaded(), content(&[1, 2]));
    assert!(!r.unwritable());

    // The same on the real file system.
    let dir = ShortDir::new();
    let path = dir.path().join("state.json");
    std::fs::write(dir.path().join("state.json.tmp"), b"rubbish").unwrap();
    let source = Source::new(content(&[3]));
    let saver = r.spawn_with(Arc::new(RealFs), path.clone(), &source, true);
    r.clock.settle().await;
    saver.shutdown().await.unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, encode(&content(&[3])));
}

// STA-7
#[tokio::test]
async fn sta7_spacing() {
    let r = rig();
    let source = Source::new(content(&[1]));
    let _saver = r.spawn(&source);
    r.clock.settle().await;
    assert_eq!(writes(&r.fs), 1);
    r.advance(2).await;
    source.set(content(&[1, 2]));
    r.clock.settle().await;
    assert_eq!(r.loaded(), content(&[1]));
    r.advance(2).await;
    assert_eq!(r.loaded(), content(&[1]));
    assert_eq!(writes(&r.fs), 1);
    r.advance(1).await;
    assert_eq!(r.loaded(), content(&[1, 2]));
    assert_eq!(writes(&r.fs), 2);
    // A change 9 s after the write is written at once.
    r.advance(9).await;
    source.set(content(&[1, 2, 3]));
    r.clock.settle().await;
    assert_eq!(r.loaded(), content(&[1, 2, 3]));
    assert_eq!(writes(&r.fs), 3);
}

// STA-7
#[tokio::test]
async fn sta7_failed_write() {
    for step in [Step::WriteTmp, Step::SyncFile, Step::Rename, Step::SyncDir] {
        for at_next_change in [false, true] {
            let r = rig();
            let old = content(&[1]);
            r.fs.put(&state_path(), &encode(&old));
            r.fs.fail_at(step);
            let source = Source::new(content(&[1, 2]));
            let _saver = r.spawn(&source);
            r.clock.settle().await;
            assert!(r.unwritable(), "{step:?}");
            if step != Step::SyncDir {
                assert_eq!(r.loaded(), old, "{step:?}: the old file still loads");
            }
            r.fs.heal();
            if at_next_change {
                r.advance(1).await;
                source.set(content(&[1, 2, 3]));
                r.clock.settle().await;
                assert!(r.unwritable());
                r.advance(4).await;
                assert!(!r.unwritable(), "{step:?}: written at the next change");
                assert_eq!(r.loaded(), content(&[1, 2, 3]));
            } else {
                r.advance(59).await;
                assert!(r.unwritable(), "{step:?}: not before 60 s");
                r.advance(1).await;
                assert!(!r.unwritable(), "{step:?}: written 60 s later");
                assert_eq!(r.loaded(), content(&[1, 2]));
            }
            // The task is still running: the next change is written.
            r.advance(5).await;
            source.set(content(&[9]));
            r.clock.settle().await;
            assert_eq!(r.loaded(), content(&[9]));
        }
    }
}

// STA-7
#[tokio::test]
async fn sta7_flush_ignores_spacing() {
    let r = rig();
    let source = Source::new(content(&[1]));
    let saver = r.spawn(&source);
    r.clock.settle().await;
    r.advance(1).await;
    source.set(content(&[1, 2]));
    r.clock.settle().await;
    assert_eq!(writes(&r.fs), 1);
    saver.flush().await;
    assert_eq!(writes(&r.fs), 2);
    assert_eq!(r.loaded(), content(&[1, 2]));
    // With no change it writes nothing.
    saver.flush().await;
    assert_eq!(writes(&r.fs), 2);
    // A failed write: flush returns after it.
    r.fs.fail_at(Step::Rename);
    source.set(content(&[7]));
    saver.flush().await;
    assert!(r.unwritable());
    // The flush of a caller that went away still writes.
    r.fs.heal();
    let flush = saver.flush();
    drop(flush);
    r.clock.settle().await;
}

// STA-7
#[tokio::test]
async fn sta7_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig();
    let dir = ShortDir::new();
    let path = dir.path().join("state.json");
    // A temporary file left with a wider mode is made 0600 too.
    std::fs::write(dir.path().join("state.json.tmp"), b"x").unwrap();
    std::fs::set_permissions(
        dir.path().join("state.json.tmp"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let source = Source::new(content(&[1]));
    let saver = r.spawn_with(Arc::new(RealFs), path.clone(), &source, true);
    saver.shutdown().await.unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert!(!dir.path().join("state.json.tmp").exists());
}

// OPS-7
#[tokio::test]
async fn ops7_shutdown_writes() {
    let r = rig();
    let source = Source::new(FileContent::default());
    let saver = r.spawn(&source);
    r.clock.settle().await;
    saver.shutdown().await.unwrap();
    assert_eq!(r.loaded(), FileContent::default());
    assert!(r.fs.file(&state_path()).is_some());
    // Shutdown without the spacing, with a change just made.
    let r = rig();
    let source = Source::new(content(&[1]));
    let saver = r.spawn(&source);
    r.clock.settle().await;
    source.set(content(&[2]));
    saver.shutdown().await.unwrap();
    assert_eq!(r.loaded(), content(&[2]));
    // And a failure is the result.
    let r = rig();
    let source = Source::new(content(&[1]));
    let saver = r.spawn(&source);
    r.clock.settle().await;
    r.fs.fail_at(Step::SyncFile);
    assert!(saver.shutdown().await.is_err());
    // The task has ended: a second shutdown and a flush return.
    assert!(saver.shutdown().await.is_ok());
    saver.flush().await;
}

// TST-5
#[tokio::test]
async fn abort_writes_nothing() {
    let r = rig();
    let source = Source::new(content(&[1]));
    let saver = r.spawn(&source);
    r.clock.settle().await;
    r.advance(1).await;
    source.set(content(&[1, 2]));
    r.clock.settle().await;
    saver.abort();
    r.advance(30).await;
    assert_eq!(r.loaded(), content(&[1]));
    source.set(content(&[5]));
    r.advance(30).await;
    assert_eq!(r.loaded(), content(&[1]));
    assert_eq!(writes(&r.fs), 1);
    saver.abort();
}

// STA-8
#[tokio::test]
async fn sta8_missing() {
    let r = rig();
    assert_eq!(r.loaded(), FileContent::default());
    assert!(r.warnings.list().is_empty());
    assert_eq!(r.sink.contents(), "");
    // An unchanged load writes nothing.
    r.fs.put(&state_path(), &encode(&content(&[1])));
    let source = Source::new(content(&[1]));
    let _saver = r.spawn(&source);
    r.clock.settle().await;
    assert_eq!(writes(&r.fs), 0);
}

fn set_aside_case(name: &str) -> Rig {
    let r = rig();
    r.fs.put(&state_path(), &testdata(name));
    r.fs.put(&bad(), b"an earlier one");
    assert!(matches!(r.load(), Loaded::SetAside));
    assert_eq!(r.fs.file(&state_path()), None);
    assert_eq!(r.fs.file(&bad()), Some(testdata(name)));
    let w = r.warnings.list();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].code, WarningCode::StateFileSetAside);
    r
}

// STA-8
#[test]
fn sta8_not_json() {
    let r = set_aside_case("bad-not-json.json");
    assert!(r.sink.contents().contains("warn state file set aside\n"));
    // Truncated JSON, a bare number and an empty file are not JSON objects with a version.
    for bytes in [
        &b"{\"version\": 1, \"prs\": ["[..],
        b"7",
        b"",
        b"{\"version\": \"1\"}",
    ] {
        let r = rig();
        r.fs.put(&state_path(), bytes);
        assert!(matches!(r.load(), Loaded::SetAside));
    }
}

// STA-8
#[test]
fn sta8_no_version() {
    let r = set_aside_case("bad-no-version.json");
    assert!(r.sink.contents().contains("state file set aside"));
}

// STA-8
#[tokio::test]
async fn sta8_newer() {
    let r = rig();
    let bytes = testdata("bad-version-2.json");
    r.fs.put(&state_path(), &bytes);
    assert!(matches!(r.load(), Loaded::Newer));
    assert_eq!(r.fs.file(&state_path()), Some(bytes.clone()));
    assert_eq!(r.fs.file(&bad()), None);
    let w = r.warnings.list();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].code, WarningCode::StateFileNewer);
    assert!(r.sink.contents().contains("warn state file newer"));
    // A saver with `writable: false` never writes, `shutdown` included.
    let source = Source::new(content(&[1]));
    let saver = r.spawn_with(r.fs.clone(), state_path(), &source, false);
    r.clock.settle().await;
    source.set(content(&[2]));
    saver.flush().await;
    r.advance(100).await;
    saver.shutdown().await.unwrap();
    assert_eq!(r.fs.calls(Step::WriteTmp), 0);
    assert_eq!(r.fs.file(&state_path()), Some(bytes));
}

// STA-8
#[test]
fn sta8_bad_entries() {
    let r = rig();
    r.fs.put(&state_path(), &testdata("bad-entry-key-missing.json"));
    let got = r.loaded();
    let ids: Vec<&str> = got.prs.iter().map(|e| e.id.0.as_str()).collect();
    assert_eq!(ids, ["PR_kwDOAbc1", "PR_kwDOAbc3"]);
    let log = r.sink.contents();
    assert_eq!(log.matches("state file entry skipped").count(), 1);
    assert!(log.contains("state file entry skipped index=1\n"));

    let r = rig();
    r.fs.put(&state_path(), &testdata("bad-duplicate-id.json"));
    let got = r.loaded();
    assert_eq!(got.prs.len(), 2);
    assert_eq!(got.prs[0].pr.number, 1);
    assert_eq!(got.prs[1].pr.number, 3);
    assert!(
        r.sink
            .contents()
            .contains("state file entry skipped index=1\n")
    );

    // An unknown source, `asked`, and an `open` end don't decode either.
    let mut v: serde_json::Value = serde_json::from_slice(&testdata("v1.json")).unwrap();
    let list = v["prs"].as_array_mut().unwrap();
    let mut a = list[1].clone();
    a["source"] = json!("asked");
    a["id"] = json!("PR_a");
    let mut b = list[1].clone();
    b["source"] = json!("elsewhere");
    b["id"] = json!("PR_b");
    let mut c = list[1].clone();
    c["ended"] = json!({"state": "open", "seen_at": "2026-11-24T18:00:00Z"});
    c["id"] = json!("PR_c");
    list.extend([a, b, c, json!(7)]);
    let r = rig();
    r.fs.put(&state_path(), &serde_json::to_vec(&v).unwrap());
    assert_eq!(r.loaded().prs.len(), 4);
    assert_eq!(r.sink.contents().matches("entry skipped").count(), 4);

    // `prs` that isn't a list: no entries, one warn line.
    let r = rig();
    r.fs.put(&state_path(), br#"{"version": 1, "prs": 5}"#);
    assert_eq!(r.loaded(), FileContent::default());
    assert_eq!(r.sink.contents().lines().count(), 1);

    // An unreadable file: one error line, no entries.
    let r = rig();
    r.fs.put(&state_path(), b"{}");
    r.fs.fail_at(Step::Read);
    assert_eq!(r.loaded(), FileContent::default());
    assert!(r.sink.contents().contains(" error "));
}

// OPS-9
#[tokio::test]
async fn ops9_lines() {
    let r = rig();
    r.fs.put(&state_path(), &testdata("bad-not-json.json"));
    r.load();
    assert!(r.sink.contents().contains("warn state file set aside"));

    let r = rig();
    r.fs.put(&state_path(), &testdata("bad-version-2.json"));
    r.load();
    assert!(r.sink.contents().contains("warn state file newer"));

    let r = rig();
    let source = Source::new(content(&[1]));
    r.fs.fail_at(Step::Rename);
    let _saver = r.spawn(&source);
    r.clock.settle().await;
    r.fs.heal();
    r.advance(60).await;
    let log = r.sink.contents();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 2, "{log}");
    assert!(lines[0].ends_with("warn warning raised code=state_file_unwritable"));
    assert!(lines[1].ends_with("info warning cleared code=state_file_unwritable"));
}

// TST-10
#[test]
fn tst10_every_version_loads() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/state");
    let mut found = 0;
    for file in std::fs::read_dir(&dir).unwrap() {
        let file = file.unwrap();
        let name = file.file_name().into_string().unwrap();
        if !(name.starts_with('v') && name.ends_with(".json")) {
            continue;
        }
        found += 1;
        let r = rig();
        r.fs.put(&state_path(), &std::fs::read(file.path()).unwrap());
        let got = r.loaded();
        let raw: serde_json::Value = serde_json::from_slice(&testdata(&name)).unwrap();
        assert_eq!(
            got.prs.len(),
            raw["prs"].as_array().unwrap().len(),
            "{name}"
        );
        assert_eq!(r.sink.contents(), "", "{name}");
        // Written back and loaded again, the content is the same.
        r.fs.put(&state_path(), &encode(&got));
        assert_eq!(r.loaded(), got);
    }
    assert!(found >= 1);

    let r = rig();
    r.fs.put(&state_path(), &testdata("v1.json"));
    let got = r.loaded();
    let sources: Vec<_> = got.prs.iter().map(|e| e.source).collect();
    use crate::model::{PrState, Source as S};
    assert!(sources.contains(&S::Authored) && sources.contains(&S::Manual));
    let ended: Vec<_> = got
        .prs
        .iter()
        .map(|e| e.ended.as_ref().map(|x| x.state))
        .collect();
    assert_eq!(
        ended,
        [
            Some(PrState::Merged),
            None,
            Some(PrState::Closed),
            Some(PrState::Gone)
        ]
    );
    // STA-6's own example entry, field for field.
    let first = &got.prs[0];
    assert_eq!(first.pr.to_string(), "o/r#12");
    assert_eq!(first.id.0, "PR_kwDOAbc123");
    assert_eq!(first.source, S::Manual);
    assert_eq!(first.login.as_deref(), Some("octocat"));
    let ended = first.ended.as_ref().unwrap();
    assert_eq!(ended.state, PrState::Merged);
    assert_eq!(ended.seen_at, Time::parse("2026-11-24T22:09:23Z").unwrap());
    assert_eq!(
        first.seen.comment.at,
        Some(Time::parse("2026-11-24T21:00:00Z").unwrap())
    );
    assert_eq!(first.seen.comment.ids, [918273]);
    assert_eq!(first.seen.review.at, None);
    assert!(first.seen.review.ids.is_empty());
}

// STA-6
#[test]
fn sta6_no_title_no_token() {
    let r = rig();
    r.fs.put(&state_path(), &testdata("v1.json"));
    let every_shape = r.loaded();
    assert_eq!(every_shape.prs.len(), 4);
    for c in [FileContent::default(), content(&[1, 2]), every_shape] {
        let bytes = encode(&c);
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("\"title\""));
        assert!(!text.contains(MARKER_TOKEN));
        assert!(text.starts_with("{\"version\":1,\"prs\":["), "{text}");
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 2);
        let r = rig();
        r.fs.put(&state_path(), &bytes);
        assert_eq!(r.loaded(), c);
    }
}

/// A clock whose `mono()` moves 1 ms forward at every read, and counts the reads.
struct MovingClock {
    inner: Arc<TestClock>,
    reads: std::sync::atomic::AtomicU64,
}

impl crate::clock::Clock for MovingClock {
    fn wall(&self) -> Time {
        self.inner.wall()
    }
    fn mono(&self) -> crate::clock::Mono {
        use std::sync::atomic::Ordering::SeqCst;
        let n = self.reads.fetch_add(1, SeqCst) + 1;
        self.inner.mono().plus(Duration::from_millis(n))
    }
    fn sleep_until(&self, at: crate::clock::Mono) -> futures::future::BoxFuture<'static, ()> {
        self.inner.sleep_until(at)
    }
}

// STA-7, OPS-17
#[tokio::test]
async fn sta7_writes_on_a_moving_clock() {
    use std::sync::atomic::Ordering::SeqCst;
    let r = rig();
    let moving = Arc::new(MovingClock {
        inner: r.clock.clone(),
        reads: Default::default(),
    });
    let source = Source::new(content(&[1]));
    let _saver = Saver::spawn(SaverDeps {
        fs: r.fs.clone(),
        path: state_path(),
        source: source.clone(),
        clock: moving.clone(),
        logger: r.logger.clone(),
        warnings: r.warnings.clone(),
        activity: r.clock.activity(),
        writable: true,
    });
    r.clock.settle().await;
    assert_eq!(writes(&r.fs), 1);
    assert_eq!(r.loaded(), content(&[1]));
    // Parked: a handful of clock reads, not thousands of turns.
    assert!(
        moving.reads.load(SeqCst) <= 10,
        "{}",
        moving.reads.load(SeqCst)
    );
    // A change inside the spacing waits for its due time, still without spinning.
    source.set(content(&[1, 2]));
    r.clock.settle().await;
    assert_eq!(writes(&r.fs), 1);
    assert!(
        moving.reads.load(SeqCst) <= 20,
        "{}",
        moving.reads.load(SeqCst)
    );
    // The clock runs a few ms ahead of the test clock, so go past the 5 s.
    r.advance(6).await;
    assert_eq!(r.loaded(), content(&[1, 2]));
    assert!(
        moving.reads.load(SeqCst) <= 30,
        "{}",
        moving.reads.load(SeqCst)
    );
}
