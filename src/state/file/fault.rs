//! `FaultFs` (TST-10): an in-memory `StateFs` that keeps what was written apart from what was synced,
//! and can fail or stop at any step.
//!
//! The model: a file's data (an inode) is written by `write_tmp` and made durable by `sync_file`;
//! a name (a directory entry) is made or moved by `write_tmp` and `rename`, and made durable by
//! `sync_dir` on its directory. `power_cut` drops what wasn't durable: an inode's unsynced data
//! (the file is then empty) and every name change whose directory wasn't synced.
//!
//! Controls:
//! - `put(path, bytes)`: a file already there, durable (a file from an earlier run).
//! - `file(path)`: what a read would give now.
//! - `fail_at(step)`: every call of that step returns an error, until `heal()`.
//! - `stop_after(step)`: the next call of that step is carried out, and then every call, of any
//!   step, fails: a process that died. `revive()` is the next process; `power_cut()` revives too.
//! - `power_cut()`: drops what wasn't synced.
//! - `calls(step)`: how many calls of that step were made, failed ones too.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::StateFs;

/// One call of `StateFs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    Read,
    WriteTmp,
    SyncFile,
    Rename,
    SyncDir,
}

#[derive(Default)]
struct Inode {
    written: Vec<u8>,
    synced: Option<Vec<u8>>,
}

#[derive(Default)]
struct Inner {
    next_inode: u64,
    inodes: HashMap<u64, Inode>,
    names: BTreeMap<PathBuf, u64>,
    durable_names: BTreeMap<PathBuf, u64>,
    failing: Vec<Step>,
    stop_after: Option<Step>,
    dead: bool,
    calls: HashMap<Step, usize>,
}

#[derive(Default)]
pub struct FaultFs {
    inner: Mutex<Inner>,
}

fn injected(what: &str) -> std::io::Error {
    std::io::Error::other(format!("FaultFs: {what}"))
}

impl FaultFs {
    pub fn new() -> FaultFs {
        FaultFs::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A file that's already there, durable.
    pub fn put(&self, path: &Path, bytes: &[u8]) {
        let mut fs = self.lock();
        let id = fs.new_inode(bytes.to_vec(), true);
        fs.names.insert(path.to_path_buf(), id);
        fs.durable_names.insert(path.to_path_buf(), id);
    }

    /// What a read of `path` gives now.
    pub fn file(&self, path: &Path) -> Option<Vec<u8>> {
        let fs = self.lock();
        let id = fs.names.get(path)?;
        fs.inodes.get(id).map(|i| i.written.clone())
    }

    pub fn fail_at(&self, step: Step) {
        let mut fs = self.lock();
        if !fs.failing.contains(&step) {
            fs.failing.push(step);
        }
    }

    /// Ends every `fail_at`.
    pub fn heal(&self) {
        self.lock().failing.clear();
    }

    pub fn stop_after(&self, step: Step) {
        self.lock().stop_after = Some(step);
    }

    /// A new process: calls work again. What was written stays.
    pub fn revive(&self) {
        let mut fs = self.lock();
        fs.dead = false;
        fs.stop_after = None;
    }

    /// Drops what wasn't synced, and revives.
    pub fn power_cut(&self) {
        let mut fs = self.lock();
        fs.dead = false;
        fs.stop_after = None;
        fs.names = fs.durable_names.clone();
        for inode in fs.inodes.values_mut() {
            inode.written = inode.synced.clone().unwrap_or_default();
        }
        let live: Vec<u64> = fs.names.values().copied().collect();
        fs.inodes.retain(|id, _| live.contains(id));
    }

    pub fn calls(&self, step: Step) -> usize {
        self.lock().calls.get(&step).copied().unwrap_or(0)
    }
}

impl Inner {
    fn new_inode(&mut self, bytes: Vec<u8>, synced: bool) -> u64 {
        self.next_inode += 1;
        let id = self.next_inode;
        let synced = synced.then(|| bytes.clone());
        self.inodes.insert(
            id,
            Inode {
                written: bytes,
                synced,
            },
        );
        id
    }

    /// Counts the call and says whether it may go ahead.
    fn enter(&mut self, step: Step) -> std::io::Result<()> {
        *self.calls.entry(step).or_default() += 1;
        if self.dead {
            return Err(injected("the process is dead"));
        }
        if self.failing.contains(&step) {
            return Err(injected("injected failure"));
        }
        Ok(())
    }

    /// After a call that was carried out.
    fn leave(&mut self, step: Step) {
        if self.stop_after == Some(step) {
            self.dead = true;
        }
    }
}

impl StateFs for FaultFs {
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        let mut fs = self.lock();
        fs.enter(Step::Read)?;
        let found = fs
            .names
            .get(path)
            .and_then(|id| fs.inodes.get(id))
            .map(|i| i.written.clone());
        fs.leave(Step::Read);
        Ok(found)
    }

    fn write_tmp(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut fs = self.lock();
        fs.enter(Step::WriteTmp)?;
        let id = fs.new_inode(bytes.to_vec(), false);
        fs.names.insert(path.to_path_buf(), id);
        fs.leave(Step::WriteTmp);
        Ok(())
    }

    fn sync_file(&self, path: &Path) -> std::io::Result<()> {
        let mut fs = self.lock();
        fs.enter(Step::SyncFile)?;
        let id = *fs
            .names
            .get(path)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        if let Some(inode) = fs.inodes.get_mut(&id) {
            inode.synced = Some(inode.written.clone());
        }
        fs.leave(Step::SyncFile);
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        let mut fs = self.lock();
        fs.enter(Step::Rename)?;
        let id = fs
            .names
            .remove(from)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        fs.names.insert(to.to_path_buf(), id);
        fs.leave(Step::Rename);
        Ok(())
    }

    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        let mut fs = self.lock();
        fs.enter(Step::SyncDir)?;
        fs.durable_names.retain(|p, _| p.parent() != Some(dir));
        let here: Vec<(PathBuf, u64)> = fs
            .names
            .iter()
            .filter(|(p, _)| p.parent() == Some(dir))
            .map(|(p, id)| (p.clone(), *id))
            .collect();
        fs.durable_names.extend(here);
        fs.leave(Step::SyncDir);
        Ok(())
    }
}
