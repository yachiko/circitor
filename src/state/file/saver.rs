//! The saver (STA-7): one task owns every write of `state.json`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use super::{RETRY_AFTER, StateFs, WRITE_SPACING, encode, tmp_path};
use crate::clock::{Activity, Busy, Mono, SharedClock};
use crate::log::Logger;
use crate::state::{FileSource, Flush};
use crate::status::{WarningCode, Warnings};

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

enum Cmd {
    Flush(oneshot::Sender<()>),
    Shutdown(oneshot::Sender<std::io::Result<()>>),
}

pub struct Saver {
    commands: mpsc::UnboundedSender<Cmd>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Saver {
    /// Starts the task that writes when the content changes, at most once in 5 s, and retries a failed
    /// write at the next change or 60 s later (STA-7).
    ///
    /// The baseline is the file on disk, so content that differs from it is written at the first
    /// chance, and an unchanged load writes nothing.
    pub fn spawn(deps: SaverDeps) -> Arc<Saver> {
        let (commands, inbox) = mpsc::unbounded_channel();
        // Work in flight from the start: the task's first poll is not yet made.
        let busy = deps.activity.begin();
        let changes = deps.source.file_changed();
        let SaverDeps {
            fs,
            path,
            source,
            clock,
            logger: _,
            warnings,
            activity,
            writable,
        } = deps;
        let task = Task {
            tmp: tmp_path(&path),
            fs,
            path,
            source,
            clock,
            warnings,
            activity,
            writable,
            last: None,
            last_attempt: None,
            failed: false,
            changed_since_failure: false,
        };
        let handle = tokio::spawn(task.run(inbox, changes, busy));
        Arc::new(Saver {
            commands,
            task: Mutex::new(Some(handle)),
        })
    }

    /// OPS-7: the final write, without the spacing. `Err` if it failed. Stops the task.
    // OPS-7: the final write.
    pub async fn shutdown(&self) -> std::io::Result<()> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Cmd::Shutdown(reply)).is_err() {
            return Ok(());
        }
        answer.await.unwrap_or(Ok(()))
    }

    /// Stops the task without writing (`Running::crash`, TST-5).
    pub fn abort(&self) {
        let handle = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }
}

#[async_trait::async_trait]
impl Flush for Saver {
    // STA-7: a write now, whatever the spacing. The task does it, so a caller that goes away
    // doesn't cancel it.
    async fn flush(&self) {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Cmd::Flush(reply)).is_ok() {
            let _ = answer.await;
        }
    }
}

struct Task {
    fs: Arc<dyn StateFs>,
    path: PathBuf,
    tmp: PathBuf,
    source: Arc<dyn FileSource>,
    clock: SharedClock,
    warnings: Warnings,
    activity: Activity,
    writable: bool,
    /// The bytes last written, or on disk at the start.
    last: Option<Vec<u8>>,
    /// When the last write was begun (monotonic, POL-26).
    last_attempt: Option<Mono>,
    failed: bool,
    changed_since_failure: bool,
}

/// STA-7's four steps, one `StateFs` call each.
fn write_steps(
    fs: &dyn StateFs,
    path: &std::path::Path,
    tmp: &std::path::Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    fs.write_tmp(tmp, bytes)?;
    fs.sync_file(tmp)?;
    fs.rename(tmp, path)?;
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    fs.sync_dir(dir)
}

impl Task {
    fn current(&self) -> Vec<u8> {
        encode(&self.source.file_content())
    }

    /// When the pending write is due (STA-7): 5 s after the last one, or after a failure 60 s after
    /// it unless something changed since.
    fn due(&self) -> Mono {
        let Some(at) = self.last_attempt else {
            return self.clock.mono();
        };
        if self.failed && !self.changed_since_failure {
            at.plus(RETRY_AFTER)
        } else {
            at.plus(WRITE_SPACING)
        }
    }

    // STA-7: a failed write raises `state_file_unwritable`; a good one clears it.
    async fn write(&mut self, bytes: Vec<u8>) -> std::io::Result<()> {
        self.last_attempt = Some(self.clock.mono());
        let (fs, path, tmp, data) = (
            self.fs.clone(),
            self.path.clone(),
            self.tmp.clone(),
            bytes.clone(),
        );
        let result = tokio::task::spawn_blocking(move || write_steps(&*fs, &path, &tmp, &data))
            .await
            .unwrap_or_else(|_| Err(std::io::Error::other("the write panicked")));
        match &result {
            Ok(()) => {
                self.last = Some(bytes);
                self.failed = false;
                self.warnings.clear(WarningCode::StateFileUnwritable, "");
            }
            Err(e) => {
                self.failed = true;
                self.changed_since_failure = false;
                self.warnings.raise(
                    WarningCode::StateFileUnwritable,
                    "",
                    format!("state.json can't be written ({:?})", e.kind()),
                );
            }
        }
        result
    }

    async fn read_baseline(&self) -> Option<Vec<u8>> {
        let (fs, path) = (self.fs.clone(), self.path.clone());
        tokio::task::spawn_blocking(move || fs.read(&path))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
    }

    async fn run(
        mut self,
        mut inbox: mpsc::UnboundedReceiver<Cmd>,
        mut changes: tokio::sync::watch::Receiver<u64>,
        first: Busy,
    ) {
        let mut busy = Some(first);
        let mut changes_open = true;
        self.last = self.read_baseline().await;
        loop {
            // TST-4: work in flight from waking until parking again.
            let guard = busy.take().unwrap_or_else(|| self.activity.begin());
            if self.writable {
                let bytes = self.current();
                if self.last.as_deref() != Some(bytes.as_slice()) && self.clock.mono() >= self.due()
                {
                    let _ = self.write(bytes).await;
                    busy = Some(guard);
                    continue;
                }
            }
            let sleep = if self.writable && self.last.as_deref() != Some(self.current().as_slice())
            {
                Some(self.clock.sleep_until(self.due()))
            } else {
                None
            };
            drop(guard);
            let woke = tokio::select! {
                r = changes.changed(), if changes_open => {
                    if r.is_err() { changes_open = false; }
                    None
                }
                cmd = inbox.recv() => match cmd {
                    // The `Saver` is gone: nothing can ask for a write any more.
                    None => return,
                    Some(cmd) => Some(cmd),
                },
                () = async {
                    match sleep {
                        Some(s) => s.await,
                        None => std::future::pending().await,
                    }
                } => None,
            };
            let guard = self.activity.begin();
            self.changed_since_failure |= self.failed;
            if let Some(cmd) = woke {
                match cmd {
                    Cmd::Flush(reply) => {
                        if self.writable {
                            let bytes = self.current();
                            if self.last.as_deref() != Some(bytes.as_slice()) {
                                let _ = self.write(bytes).await;
                            }
                        }
                        let _ = reply.send(());
                    }
                    Cmd::Shutdown(reply) => {
                        let result = if self.writable {
                            let bytes = self.current();
                            self.write(bytes).await
                        } else {
                            Ok(())
                        };
                        let _ = reply.send(result);
                        return;
                    }
                }
            }
            busy = Some(guard);
        }
    }
}
