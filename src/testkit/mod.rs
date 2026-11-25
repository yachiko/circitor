//! The test kit (`plan/contracts/testkit.md`): only in a test build or under `cfg(test)` (TST-1).
//! T02's and final: T14 builds `github`, T20 `server`, T45 `session`, T29 `harness`.

use std::path::Path;

pub mod github;
pub mod golden;
pub mod harness;
pub mod server;
pub mod session;

/// The token every test uses (TST-11). Nothing but the `Authorization` header may ever hold it.
pub const MARKER_TOKEN: &str = "gho_CIRCITOR_TEST_MARKER_TOKEN_0000";
/// A second one, for "another token than the one refused" (POL-20) and TST-9's `GH_TOKEN` case. Swept like the first.
pub const MARKER_TOKEN_2: &str = "gho_CIRCITOR_TEST_MARKER_TOKEN_0002";
/// The version the harness gives the daemon and the CLI unless a test says otherwise, so golden files don't change with every commit (TST-7).
pub const TEST_VERSION: &str = "1.2.3+test";
/// The fake GitHub puts these in every title, every error `message` and the 401's body (TST-11).
pub const MARKER_TITLE: &str = "MARKER-TITLE";
pub const MARKER_GITHUB_TEXT: &str = "MARKER-GITHUB-TEXT";
/// A session's token in every test (TST-11, NTF-9): swept like `MARKER_TOKEN`. Only an auth line posted
/// to a session socket, and the request body that carries it to the daemon, may ever hold it.
/// 32 characters, as docs/07 measured.
pub const MARKER_SESSION_TOKEN: &str = "CIRCITOR_TEST_SESSION_MARKER_000";
/// In every check's name and every login the fake GitHub makes, unless a test names one itself (TST-11, D9):
/// no posted notice may hold either, nor `MARKER_TITLE` (NTF-8), and the log and the state file never hold
/// `MARKER_CHECK`. A login may be in the state file (STA-6) and the log.
pub const MARKER_CHECK: &str = "MARKER-CHECK";
pub const MARKER_LOGIN: &str = "marker-login";

/// A short directory under `/tmp`, removed when dropped (socket paths stay under 103 bytes, OPS-4).
/// Never under `std::env::temp_dir()`: macOS's is long.
pub struct ShortDir(tempfile::TempDir);

impl ShortDir {
    /// `/tmp/ct` and six random characters. Panics if `/tmp` can't be written: a test can't go on.
    pub fn new() -> ShortDir {
        let dir = tempfile::Builder::new()
            .prefix("ct")
            .rand_bytes(6)
            .tempdir_in("/tmp");
        ShortDir(dir.expect("a directory under /tmp"))
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Default for ShortDir {
    fn default() -> ShortDir {
        ShortDir::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_dir_is_short_and_removed() {
        let dir = ShortDir::new();
        let path = dir.path().to_path_buf();
        assert!(path.starts_with("/tmp"));
        assert!(path.is_dir());
        // Room for a socket's name inside it (OPS-4).
        assert!(path.as_os_str().len() <= 20, "{}", path.display());
        let other = ShortDir::default();
        assert_ne!(other.path(), path);
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn markers() {
        assert_eq!(MARKER_SESSION_TOKEN.len(), 32);
        assert_ne!(MARKER_TOKEN, MARKER_TOKEN_2);
        assert_eq!(TEST_VERSION, "1.2.3+test");
    }
}
