//! The harness's spawner (TST-5): `FakeSpawner::starts` with a closure that starts the in-process
//! daemon on a task and returns a child that follows it.

use std::io::Write;
use std::sync::{Arc, Mutex};

use crate::cli::Spawned;
use crate::cli::fake::FakeSpawner;
use crate::config::Env;
use crate::daemon::Mode;
use crate::testkit::TEST_VERSION;

use super::{Shared, lock};

/// Where a spawned start is.
#[derive(Clone, Copy)]
enum Child {
    Starting,
    /// `start` failed with this status (0: another daemon answers, STA-11).
    Failed(i32),
    /// Running as this start's number.
    Started(u64),
}

struct Follow {
    shared: Arc<Shared>,
    state: Arc<Mutex<Child>>,
}

impl Spawned for Follow {
    fn exited(&mut self) -> Option<i32> {
        match *lock(&self.state) {
            Child::Starting => None,
            Child::Failed(code) => Some(code),
            Child::Started(n) => self.shared.exit_of(n),
        }
    }
}

/// The harness's default spawner: a daemon it starts is `spawned`, at `TEST_VERSION` (the binary the
/// CLI is), on the harness's directories, fakes and current config.
pub(super) fn starter(shared: &Arc<Shared>) -> Arc<FakeSpawner> {
    let shared = shared.clone();
    FakeSpawner::starts(move |mut log, _env: &Env| {
        let state = Arc::new(Mutex::new(Child::Starting));
        let (sh, st) = (shared.clone(), state.clone());
        tokio::spawn(async move {
            // TST-4: the start is work in flight. In the paths a test takes it awaits no timer (the
            // lock is free, a stale socket refuses at once).
            let _busy = sh.clock.activity().begin();
            match sh.launch(TEST_VERSION.to_string(), Mode::Spawned).await {
                Ok(n) => *lock(&st) = Child::Started(n),
                Err(e) => {
                    // What the real daemon prints on stderr, which goes to the log (CLI-7).
                    let _ = writeln!(log, "{e}");
                    *lock(&st) = Child::Failed(e.code());
                }
            }
        });
        Box::new(Follow {
            shared: shared.clone(),
            state,
        })
    })
}
