//! `RecordingPoster` (T46): a `Poster` that records every post.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::{Activity, SharedClock, Time};

use super::{Looked, Posted, Poster, SessionToken, SocketId};

/// One post asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub socket: PathBuf,
    pub token: Option<String>,
    pub text: String,
    pub at: Time,
}

#[derive(Clone)]
enum Seen {
    Id(SocketId),
    Gone,
    Fails,
}

#[derive(Default)]
struct State {
    posts: Vec<Post>,
    answer: Option<Posted>,
    next: VecDeque<Posted>,
    stall: Option<(Activity, tokio::sync::watch::Receiver<bool>)>,
    paths: HashMap<PathBuf, Seen>,
    next_ino: u64,
}

/// `look` answers `Looked::Socket` with one fixed `SocketId` per path, until `gone(path)` (`Missing`),
/// `swapped(path)` (another `SocketId`) or `stat_fails(path)` (`Failed`).
pub struct RecordingPoster {
    clock: SharedClock,
    state: Mutex<State>,
}

impl RecordingPoster {
    pub fn new(clock: SharedClock) -> Arc<RecordingPoster> {
        Arc::new(RecordingPoster {
            clock,
            state: Mutex::new(State::default()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every post asked for, in order, a stalled one included from the moment it's asked.
    pub fn posts(&self) -> Vec<Post> {
        self.lock().posts.clone()
    }

    /// The answer to every post from now on.
    pub fn answer(&self, posted: Posted) {
        self.lock().answer = Some(posted);
    }

    /// The answer to the next post.
    pub fn answer_next(&self, posted: Posted) {
        self.lock().next.push_back(posted);
    }

    /// Every post waits until the `Release` is dropped, excused from the work in flight meanwhile.
    pub fn stall(&self, activity: &Activity) -> Release {
        let (tx, rx) = tokio::sync::watch::channel(false);
        self.lock().stall = Some((activity.clone(), rx));
        Release { inner: Some(tx) }
    }

    pub fn gone(&self, path: &Path) {
        self.lock().paths.insert(path.to_path_buf(), Seen::Gone);
    }

    pub fn swapped(&self, path: &Path) {
        let mut s = self.lock();
        let id = fresh_id(&mut s);
        s.paths.insert(path.to_path_buf(), Seen::Id(id));
    }

    pub fn stat_fails(&self, path: &Path) {
        self.lock().paths.insert(path.to_path_buf(), Seen::Fails);
    }
}

fn fresh_id(s: &mut State) -> SocketId {
    s.next_ino += 1;
    SocketId {
        dev: 1,
        ino: s.next_ino,
    }
}

#[async_trait::async_trait]
impl Poster for RecordingPoster {
    async fn look(&self, socket: &Path) -> Looked {
        let mut s = self.lock();
        let seen = match s.paths.get(socket) {
            Some(seen) => seen.clone(),
            None => {
                let id = fresh_id(&mut s);
                s.paths.insert(socket.to_path_buf(), Seen::Id(id));
                Seen::Id(id)
            }
        };
        match seen {
            Seen::Id(id) => Looked::Socket(id),
            Seen::Gone => Looked::Missing,
            Seen::Fails => Looked::Failed("stat failed (RecordingPoster)".to_string()),
        }
    }

    async fn post(&self, socket: &Path, token: Option<&SessionToken>, text: &str) -> Posted {
        let (answer, stall) = {
            let mut s = self.lock();
            s.posts.push(Post {
                socket: socket.to_path_buf(),
                token: token.map(|t| t.expose().to_string()),
                text: text.to_string(),
                at: self.clock.wall(),
            });
            let answer = s
                .next
                .pop_front()
                .or_else(|| s.answer.clone())
                .unwrap_or(Posted::Done);
            // A stall whose `Release` is gone is over.
            if s.stall
                .as_ref()
                .is_some_and(|(_, rx)| rx.has_changed().is_err())
            {
                s.stall = None;
            }
            (answer, s.stall.clone())
        };
        if let Some((activity, mut rx)) = stall {
            // TST-4: a stalled post isn't work in flight while the test holds it.
            let _excused = activity.excuse();
            // The sender never sends: this ends when the `Release` is dropped.
            while rx.changed().await.is_ok() {}
        }
        answer
    }
}

/// Dropping it lets the stalled posts go on.
pub struct Release {
    inner: Option<tokio::sync::watch::Sender<bool>>,
}

impl Drop for Release {
    fn drop(&mut self) {
        // Dropping the sender ends every stalled post's wait.
        self.inner.take();
    }
}
