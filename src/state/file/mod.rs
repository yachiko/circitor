//! `state.json` (T16): its format (STA-6), loading (STA-8) and the saver that writes it (STA-7).
//!
//! The file on disk is `{"version": 1, "prs": [...]}`. `FileContent` holds `prs` only: the
//! `version` key and the object around it are this module's own (`OnDisk`, `read_version`).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::state::FileContent;

mod load;
mod saver;

#[cfg(any(test, feature = "test-endpoint"))]
mod fault;
#[cfg(test)]
mod tests;

pub use load::{Loaded, load};
pub use saver::{Saver, SaverDeps};

#[cfg(any(test, feature = "test-endpoint"))]
pub use fault::{FaultFs, Step};

/// The version this binary writes (STA-6).
pub const VERSION: u64 = 1;

/// STA-7: a change within this long of the last write waits until it has passed.
pub(crate) const WRITE_SPACING: std::time::Duration = std::time::Duration::from_secs(5);
/// STA-7: a failed write is tried again after this long, if nothing changes first.
pub(crate) const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// The file's name in the state directory, and its two companions (STA-7, STA-8).
pub(crate) fn tmp_path(path: &Path) -> PathBuf {
    sibling(path, ".tmp")
}

pub(crate) fn bad_path(path: &Path) -> PathBuf {
    sibling(path, ".bad")
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// The object around `prs` (STA-6): `version` first, then the content's own keys.
#[derive(Serialize)]
struct OnDisk<'a> {
    version: u64,
    #[serde(flatten)]
    content: &'a FileContent,
}

/// STA-6: the bytes of a file holding `content`, in the order the source gave the entries.
// STA-6: nothing but STA-6's keys can be written, because `FileContent` has no field for anything else.
pub(crate) fn encode(content: &FileContent) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&OnDisk {
        version: VERSION,
        content,
    })
    // A `FileContent` is strings, numbers and lists: it always serialises.
    .unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// Every file operation of STA-7, one call per step, so a test can stop after any of them (TST-10).
pub trait StateFs: Send + Sync + 'static {
    /// `None`: no such file.
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>>;
    /// Create or replace, mode 0600.
    fn write_tmp(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;
    fn sync_file(&self, path: &Path) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()>;
}

/// The real file system.
pub struct RealFs;

impl StateFs for RealFs {
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    // STA-7: mode 0600, replacing a file left by a crash.
    fn write_tmp(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        // A file left by a crash may have another mode: `mode` applies only to a new one.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.flush()
    }

    fn sync_file(&self, path: &Path) -> std::io::Result<()> {
        std::fs::File::open(path)?.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::File::open(dir)?.sync_all()
    }
}
