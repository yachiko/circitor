#![allow(unused_variables, dead_code)] // stub: T16 removes this
//! `state.json` (T16).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::clock::{Activity, SharedClock};
use crate::log::Logger;
use crate::state::{FileContent, FileSource, Flush};
use crate::status::Warnings;

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

pub struct RealFs;

impl StateFs for RealFs {
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        unimplemented!("T16")
    }

    fn write_tmp(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        unimplemented!("T16")
    }

    fn sync_file(&self, path: &Path) -> std::io::Result<()> {
        unimplemented!("T16")
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        unimplemented!("T16")
    }

    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        unimplemented!("T16")
    }
}

pub enum Loaded {
    /// Also for a missing file: empty.
    Content(FileContent),
    /// Not JSON or no integer `version`: renamed to `state.json.bad`; start empty.
    SetAside,
    /// A newer `version`: left alone, never written; start empty.
    Newer,
}

/// STA-8. Skips and logs entries that don't decode; raises `state_file_set_aside` or `state_file_newer`.
pub fn load(fs: &dyn StateFs, path: &Path, logger: &Logger, warnings: &Warnings) -> Loaded {
    unimplemented!("T16")
}

pub struct SaverDeps {
    pub fs: Arc<dyn StateFs>,
    pub path: PathBuf,
    pub source: Arc<dyn FileSource>,
    pub clock: SharedClock,
    pub logger: Logger,
    pub warnings: Warnings,
    pub activity: Activity,
    /// `false` after `Loaded::Newer`.
    pub writable: bool,
}

pub struct Saver {
    deps: SaverDeps,
}

impl Saver {
    /// Starts the task that writes when the content changes, at most once in 5 s, and retries a failed
    /// write at the next change or 60 s later (STA-7).
    pub fn spawn(deps: SaverDeps) -> Arc<Saver> {
        unimplemented!("T16")
    }

    /// OPS-7: the final write, without the spacing. `Err` if it failed. Stops the task.
    pub async fn shutdown(&self) -> std::io::Result<()> {
        unimplemented!("T16")
    }

    /// Stops the task without writing (`Running::crash`, TST-5).
    pub fn abort(&self) {
        unimplemented!("T16")
    }
}

#[async_trait::async_trait]
impl Flush for Saver {
    async fn flush(&self) {
        unimplemented!("T16")
    }
}
