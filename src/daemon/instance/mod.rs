//! The lock, the socket, the directories (STA-11, OPS-4; T18).

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{Error, ErrorKind, Read};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};

use crate::api::StatusLite;
use crate::clock::{SharedClock, sleep};
use crate::config::check_socket_path;
use crate::transport::{Probe, SocketProbe};

#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub pid: u32,
    pub socket: PathBuf,
}

pub enum Acquire {
    Held(LockGuard),
    Busy(Option<Holder>),
}

/// STA-11.
pub const RETRY_EVERY: Duration = Duration::from_millis(100);
pub const PATIENCE: Duration = Duration::from_secs(6);
/// OPS-6.
pub const TAKEOVER_PATIENCE: Duration = Duration::from_secs(15);

/// The open descriptor. `Drop` releases the lock.
pub struct LockGuard {
    lock: Flock<File>,
    path: PathBuf,
}

/// STA-11: `flock(LOCK_EX | LOCK_NB)` on `path`, opened without truncating, tried every 100 ms of the
/// clock for up to `patience` (6 s; 15 s for a foreground daemon taking over). `Err`: the file can't be
/// opened or created; that's never retried.
pub async fn acquire(
    path: &Path,
    clock: &SharedClock,
    patience: Duration,
) -> std::io::Result<Acquire> {
    // STA-11: opened read-write, created 0600, never truncated (a refused try must leave the holder's
    // line). `File` opens close-on-exec, so `gh` never inherits the lock.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    let give_up = clock.mono().plus(patience);
    loop {
        // STA-11: `flock`, not a record lock: it belongs to the open file description.
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => {
                return Ok(Acquire::Held(LockGuard {
                    lock,
                    path: path.to_path_buf(),
                }));
            }
            Err((back, errno)) => {
                if errno != Errno::EWOULDBLOCK {
                    return Err(errno.into());
                }
                file = back;
            }
        }
        if clock.mono() >= give_up {
            return Ok(Acquire::Busy(read_holder(path)));
        }
        sleep(clock, RETRY_EVERY).await;
    }
}

impl LockGuard {
    /// Truncates and writes `<pid> <socket path>\n`.
    pub fn write_holder(&self, pid: u32, socket: &Path) -> std::io::Result<()> {
        let path = socket.as_os_str().as_bytes();
        if path.contains(&b'\n') {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the socket path holds a newline",
            ));
        }
        let mut line = format!("{pid} ").into_bytes();
        line.extend_from_slice(path);
        line.push(b'\n');
        let file: &File = &self.lock;
        file.set_len(0)?;
        file.write_all_at(&line, 0)
    }

    /// Whether the path still is this descriptor's file (same device and inode).
    pub fn still_own(&self) -> bool {
        // STA-11: the minute's check that the lock file is still the one held.
        let file: &File = &self.lock;
        match (file.metadata(), std::fs::metadata(&self.path)) {
            (Ok(held), Ok(now)) => (held.dev(), held.ino()) == (now.dev(), now.ino()),
            _ => false,
        }
    }
}

/// The holder a lock file names: one complete line, or `None`.
pub fn read_holder(path: &Path) -> Option<Holder> {
    let mut bytes = Vec::new();
    File::open(path).ok()?.read_to_end(&mut bytes).ok()?;
    // Only a line whose newline is there.
    let end = bytes.iter().position(|b| *b == b'\n')?;
    let line = &bytes[..end];
    let space = line.iter().position(|b| *b == b' ')?;
    let pid = std::str::from_utf8(&line[..space])
        .ok()?
        .parse::<u32>()
        .ok()?;
    let socket = &line[space + 1..];
    if socket.is_empty() {
        return None;
    }
    Some(Holder {
        pid,
        socket: PathBuf::from(OsStr::from_bytes(socket)),
    })
}

/// STA-11's test: take `LOCK_EX | LOCK_NB` on a descriptor of our own and let go at once.
pub fn is_held(path: &Path) -> std::io::Result<bool> {
    // Read-only and never creating: a missing file is "not held".
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(_taken) => Ok(false),
        Err((_, Errno::EWOULDBLOCK)) => Ok(true),
        Err((_, errno)) => Err(errno.into()),
    }
}

#[derive(Debug, PartialEq)]
pub enum SocketFile {
    Absent,
    RemovedStale,
    Live(StatusLite),
    /// Also `Probe::Denied`.
    NotAnswering,
    NotASocket,
}

/// OPS-4, with the lock held: removes the file only if it's a socket that refuses a connection.
pub async fn clear_socket(path: &Path, probe: &dyn SocketProbe) -> std::io::Result<SocketFile> {
    // OPS-4: look before connecting: connecting to a regular file is refused like a dead socket.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(SocketFile::Absent),
        Err(e) => return Err(e),
    };
    if !meta.file_type().is_socket() {
        return Ok(SocketFile::NotASocket);
    }
    match probe.probe(path).await {
        Probe::NoDaemon => {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            Ok(SocketFile::RemovedStale)
        }
        Probe::Answers(status) => Ok(SocketFile::Live(status)),
        Probe::NotAnswering(_) | Probe::Denied(_) => Ok(SocketFile::NotAnswering),
    }
}

pub struct Bound {
    pub listener: tokio::net::UnixListener,
    /// Device and inode.
    id: (u64, u64),
    path: PathBuf,
}

/// Tries this many temporary names before giving up on `AddrInUse`.
const TEMP_TRIES: usize = 8;

/// A seed for the temporary names: the process and a count, never a clock (TST-2).
fn temp_rng() -> fastrand::Rng {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    fastrand::Rng::with_seed((u64::from(std::process::id()) << 32) ^ n.wrapping_mul(0x9E37_79B9))
}

/// A name as long as `name`: `.` and random `[a-z0-9]`, or one random character for a one-byte name.
fn temp_name(name: &OsStr, rng: &mut fastrand::Rng) -> OsString {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let len = name.len().max(1);
    let mut out = Vec::with_capacity(len);
    if len > 1 {
        out.push(b'.');
    }
    while out.len() < len {
        out.push(CHARS[rng.usize(..CHARS.len())]);
    }
    OsString::from_vec(out)
}

/// Binds so that the socket's mode is 0600 before it's at its path (OPS-4, D10): at a temporary name in the
/// same directory, set to 0600, then renamed over `path`. The umask is never touched. Checks the length first.
pub fn bind(path: &Path) -> std::io::Result<Bound> {
    check_socket_path(path).map_err(|e| Error::new(ErrorKind::InvalidInput, e.to_string()))?;
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(Error::new(ErrorKind::InvalidInput, "no file name"));
    };
    let mut rng = temp_rng();
    let mut tries = 0;
    let (listener, tmp) = loop {
        // The temporary name is as long as the socket's, so the 103 bytes hold for it too.
        let tmp = dir.join(temp_name(name, &mut rng));
        match tokio::net::UnixListener::bind(&tmp) {
            Ok(listener) => break (listener, tmp),
            Err(e) if e.kind() == ErrorKind::AddrInUse && tries < TEMP_TRIES => tries += 1,
            Err(e) => return Err(e),
        }
    };
    let finish = || -> std::io::Result<(u64, u64)> {
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&tmp, path)?;
        let meta = std::fs::symlink_metadata(path)?;
        Ok((meta.dev(), meta.ino()))
    };
    match finish() {
        Ok(id) => Ok(Bound {
            listener,
            id,
            path: path.to_path_buf(),
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

impl Bound {
    pub fn still_own(&self) -> bool {
        match std::fs::symlink_metadata(&self.path) {
            Ok(m) => m.file_type().is_socket() && (m.dev(), m.ino()) == self.id,
            Err(_) => false,
        }
    }

    /// OPS-7, STA-11: only a socket it made.
    pub fn remove_if_own(&self) {
        if self.still_own() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, TestClock, Time};
    use crate::testkit::ShortDir;
    use std::sync::Arc;

    struct FakeProbe(Probe);

    #[async_trait::async_trait]
    impl SocketProbe for FakeProbe {
        async fn probe(&self, _path: &Path) -> Probe {
            self.0.clone()
        }
    }

    fn clock() -> (Arc<TestClock>, SharedClock) {
        let test = TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap());
        let shared: SharedClock = test.clone();
        (test, shared)
    }

    fn held(a: Acquire) -> LockGuard {
        match a {
            Acquire::Held(g) => g,
            Acquire::Busy(_) => panic!("busy"),
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    // STA-11
    #[tokio::test]
    async fn sta11_lock_is_exclusive() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.lock");
        let (test, shared) = clock();
        let guard = held(acquire(&path, &shared, PATIENCE).await.unwrap());
        assert!(is_held(&path).unwrap());

        let p = path.clone();
        let c = shared.clone();
        let waiting = tokio::spawn(async move { acquire(&p, &c, PATIENCE).await.unwrap() });
        // About every 100 ms: still trying one try before the end.
        test.advance(Duration::from_millis(5900)).await;
        assert!(!waiting.is_finished());
        test.advance(Duration::from_millis(100)).await;
        assert!(matches!(waiting.await.unwrap(), Acquire::Busy(None)));

        drop(guard);
        assert!(!is_held(&path).unwrap());
        assert!(matches!(
            acquire(&path, &shared, PATIENCE).await.unwrap(),
            Acquire::Held(_)
        ));
    }

    // STA-11
    #[tokio::test]
    async fn sta11_lock_freed_while_waiting() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.lock");
        let (test, shared) = clock();
        let guard = held(acquire(&path, &shared, PATIENCE).await.unwrap());
        let p = path.clone();
        let c = shared.clone();
        let waiting = tokio::spawn(async move { acquire(&p, &c, PATIENCE).await.unwrap() });
        test.advance(Duration::from_secs(3)).await;
        assert!(!waiting.is_finished());
        drop(guard);
        test.advance(RETRY_EVERY).await;
        assert!(matches!(waiting.await.unwrap(), Acquire::Held(_)));
        assert!(test.mono().since(crate::clock::Mono::ZERO) < Duration::from_secs(6));
    }

    // STA-11
    #[tokio::test]
    async fn sta11_open_error_not_retried() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.lock");
        std::fs::create_dir(&path).unwrap();
        let (test, shared) = clock();
        let before = test.mono();
        assert!(acquire(&path, &shared, PATIENCE).await.is_err());
        assert_eq!(test.mono(), before);
    }

    // STA-11
    #[tokio::test]
    async fn sta11_not_truncated_before_the_lock() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.lock");
        let (test, shared) = clock();
        let guard = held(acquire(&path, &shared, PATIENCE).await.unwrap());
        guard
            .write_holder(4711, Path::new("/tmp/x/s.sock"))
            .unwrap();
        let p = path.clone();
        let c = shared.clone();
        let waiting =
            tokio::spawn(async move { acquire(&p, &c, Duration::from_millis(300)).await.unwrap() });
        test.advance(Duration::from_millis(300)).await;
        let Acquire::Busy(holder) = waiting.await.unwrap() else {
            panic!("held");
        };
        let want = Holder {
            pid: 4711,
            socket: PathBuf::from("/tmp/x/s.sock"),
        };
        assert_eq!(holder, Some(want.clone()));
        assert_eq!(read_holder(&path), Some(want));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    // STA-11
    #[tokio::test]
    async fn sta11_holder_line() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.lock");
        assert_eq!(read_holder(&path), None);
        let (_test, shared) = clock();
        let guard = held(acquire(&path, &shared, PATIENCE).await.unwrap());
        assert_eq!(read_holder(&path), None, "empty file");
        let long = Path::new("/tmp/x/Application Support/circitor/circitor.sock");
        guard.write_holder(4711, long).unwrap();
        assert_eq!(
            read_holder(&path),
            Some(Holder {
                pid: 4711,
                socket: long.to_path_buf()
            })
        );
        guard.write_holder(7, Path::new("/tmp/s")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"7 /tmp/s\n");
        assert!(guard.write_holder(1, Path::new("/tmp/a\nb")).is_err());
        for (text, what) in [
            (&b"7 /tmp/s"[..], "no newline"),
            (b"abc /p\n", "pid not a number"),
            (b"12\n", "no space"),
            (b"12 \n", "empty path"),
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(read_holder(&path), None, "{what}");
        }
    }

    // STA-11
    #[test]
    fn sta11_is_held_creates_nothing() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.lock");
        assert!(!is_held(&path).unwrap());
        assert!(!path.exists());
    }

    // STA-11
    #[tokio::test]
    async fn sta11_still_own() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.lock");
        let (_test, shared) = clock();
        let guard = held(acquire(&path, &shared, PATIENCE).await.unwrap());
        assert!(guard.still_own());
        std::fs::remove_file(&path).unwrap();
        assert!(!guard.still_own());
        std::fs::write(&path, b"").unwrap();
        assert!(!guard.still_own(), "a new file at the path");

        let sock = dir.path().join("circitor.sock");
        let bound = bind(&sock).unwrap();
        assert!(bound.still_own());
        std::fs::remove_file(&sock).unwrap();
        assert!(!bound.still_own());
        let other = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        assert!(!bound.still_own());
        bound.remove_if_own();
        assert!(sock.exists(), "the other socket is left");
        drop(other);
    }

    // OPS-4
    #[tokio::test]
    async fn ops4_clear_socket() {
        let dir = ShortDir::new();
        let path = dir.path().join("circitor.sock");
        let no_daemon = FakeProbe(Probe::NoDaemon);
        assert_eq!(
            clear_socket(&path, &no_daemon).await.unwrap(),
            SocketFile::Absent
        );

        std::fs::write(&path, b"mine").unwrap();
        assert_eq!(
            clear_socket(&path, &no_daemon).await.unwrap(),
            SocketFile::NotASocket
        );
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();

        let make = || drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        make();
        assert_eq!(
            clear_socket(&path, &no_daemon).await.unwrap(),
            SocketFile::RemovedStale
        );
        assert!(!path.exists());

        let lite = StatusLite {
            version: "1.2.3+test".into(),
            mode: "spawned".into(),
        };
        make();
        assert_eq!(
            clear_socket(&path, &FakeProbe(Probe::Answers(lite.clone())))
                .await
                .unwrap(),
            SocketFile::Live(lite)
        );
        assert!(path.exists());
        for probe in [
            Probe::NotAnswering("timed out".into()),
            Probe::Denied("eperm".into()),
        ] {
            assert_eq!(
                clear_socket(&path, &FakeProbe(probe)).await.unwrap(),
                SocketFile::NotAnswering
            );
            assert!(path.exists());
        }
    }

    /// The process's umask, read without changing it (Linux); `None` elsewhere.
    fn current_umask() -> Option<String> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        status
            .lines()
            .find_map(|l| l.strip_prefix("Umask:"))
            .map(|v| v.trim().to_string())
    }

    // OPS-4
    #[tokio::test]
    async fn ops4_mode_0600() {
        let dir = ShortDir::new();
        let before = current_umask();
        let bound = bind(&dir.path().join("a.sock")).unwrap();
        let mode = std::fs::symlink_metadata(dir.path().join("a.sock"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o600);
        assert_eq!(current_umask(), before);
        drop(bound);

        let threads: Vec<_> = ["b.sock", "c.sock"]
            .into_iter()
            .map(|name| {
                let path = dir.path().join(name);
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt.block_on(async {
                        let _bound = bind(&path).unwrap();
                        std::fs::symlink_metadata(&path)
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o7777
                    })
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), 0o600);
        }
        assert_eq!(current_umask(), before);

        // Exactly 103 bytes binds: the temporary name is as long as it.
        let fits = dir
            .path()
            .join("y".repeat(103 - dir.path().as_os_str().len() - 1));
        assert_eq!(fits.as_os_str().len(), 103);
        let bound = bind(&fits).unwrap();
        assert_eq!(
            std::fs::symlink_metadata(&fits)
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
        drop(bound);
        std::fs::remove_file(&fits).unwrap();
        // A one-byte name too.
        let one = dir.path().join("z");
        let bound = bind(&one).unwrap();
        assert!(bound.still_own());
        bound.remove_if_own();

        let long = dir
            .path()
            .join("x".repeat(104 - dir.path().as_os_str().len() - 1));
        assert_eq!(long.as_os_str().len(), 104);
        let before_names = names(dir.path());
        assert!(bind(&long).is_err());
        assert_eq!(names(dir.path()), before_names);
    }

    // STA-12
    #[tokio::test]
    async fn sta12_only_its_files() {
        let dir = ShortDir::new();
        let (_test, shared) = clock();
        let lock = dir.path().join("circitor.lock");
        let sock = dir.path().join("circitor.sock");
        let guard = held(acquire(&lock, &shared, PATIENCE).await.unwrap());
        guard.write_holder(1, &sock).unwrap();
        assert_eq!(names(dir.path()), ["circitor.lock"]);
        let bound = bind(&sock).unwrap();
        assert_eq!(
            names(dir.path()),
            ["circitor.lock", "circitor.sock"],
            "no temporary name"
        );
        bound.remove_if_own();
        assert_eq!(names(dir.path()), ["circitor.lock"]);
    }

    // OPS-4, D10
    #[test]
    fn d10_umask_untouched() {
        let dir = ShortDir::new();
        let sock = dir.path().join("circitor.sock");
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let binder = {
            let (sock, stop) = (sock.clone(), stop.clone());
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async {
                    while !stop.load(Ordering::SeqCst) {
                        let bound = bind(&sock).unwrap();
                        bound.remove_if_own();
                    }
                })
            })
        };
        for i in 0..300 {
            let made = dir.path().join(format!("d{i}"));
            std::fs::create_dir(&made).unwrap();
            std::fs::write(made.join("f"), b"x").unwrap();
        }
        stop.store(true, Ordering::SeqCst);
        binder.join().unwrap();
    }
}
