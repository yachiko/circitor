//! Notify: subscriptions, the poster, the notices' words (`plan/contracts/notify.md`).
//! T02 declares, and builds `SessionToken` and `NoNotify` (NTF-9's share); T46 builds the rest.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Deserializer};

use crate::api::condition::{Condition, Progress};
use crate::api::{Ask, Reason};
use crate::clock::{Activity, SharedClock, SleepWatch, Time};
use crate::config::Config;
use crate::log::Logger;
use crate::model::{NodeId, PrName};
use crate::poller::PollerHandle;
use crate::state::{Cursor, Store};
use crate::status::Warnings;

#[cfg(any(test, feature = "test-endpoint"))]
pub mod fake;
mod notifier;
mod poster;
pub mod text;

pub use notifier::Notifier;
pub use poster::RealPoster;

/// NTF-2's `for`.
pub const DEFAULT_LIFE: Duration = Duration::from_secs(86_400);
pub const MIN_LIFE: Duration = Duration::from_secs(60);
pub const MAX_LIFE: Duration = Duration::from_secs(86_400);
/// NTF-2: at most 32 for one socket.
pub const PER_SOCKET: u32 = 32;
/// NTF-6: connecting and writing together. Never a second try.
pub const POST_LIMIT: Duration = Duration::from_secs(2);
/// NTF-10: for all the `stopped` notices together.
pub const STOP_LIMIT: Duration = Duration::from_secs(1);
/// POL-26's tick: every socket, `for` and `problem` is looked at.
pub const TICK: Duration = Duration::from_secs(10);
/// NTF-4's `problem`: 10 minutes in `auth`, from `PollerStatus::problem_since`.
pub const PROBLEM_AFTER: Duration = Duration::from_secs(600);
/// NTF-4: an expiry waits at most this for a refresh.
pub const CATCH_UP: Duration = Duration::from_secs(60);
// The 64 is `state::MAX_SUBSCRIPTIONS`, beside the other limits (`state.md`).

/// A session's token (NTF-9). As `token::Token`: no `Display`, no `Serialize`, `Debug` prints `SessionToken(…)`.
/// It lives in its subscription and in the poster's one write, and nowhere else.
#[derive(Clone)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new(value: String) -> SessionToken {
        SessionToken(value)
    }

    /// For the auth line of a post only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

// NTF-9: the value never leaves through `Debug`.
impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(…)")
    }
}

/// The route's body holds one, as a JSON string.
impl<'de> Deserialize<'de> for SessionToken {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<SessionToken, D::Error> {
        String::deserialize(d).map(SessionToken)
    }
}

/// Which socket a path names: its device and inode (NTF-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketId {
    pub dev: u64,
    pub ino: u64,
}

/// What one `stat` of a session's path saw (NTF-2, NTF-4, NTF-6). The texts are the system's (`io::Error`'s) or circitor's own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Looked {
    /// A socket owned by the user the daemon runs as.
    Socket(SocketId),
    /// Something else is there: a regular file, another user's socket. Which, in words.
    Other(SocketId, String),
    /// Nothing at the path.
    Missing,
    /// The `stat` failed otherwise.
    Failed(String),
}

/// How one post ended; the texts are the system's error (`io::Error`'s). `SessionGone` and `Failed` both end
/// the subscription, and nothing is tried again (NTF-6): they differ only in the log's `reason=`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Posted {
    /// The message line is written in full. Not "delivered": nobody can know that (docs/07).
    Done,
    /// A missing socket, a refused connection or a denied permission.
    SessionGone(String),
    /// A connect or a write that failed otherwise (a full queue's `EAGAIN` included), before the line was written in full.
    Failed(String),
}

/// The session's socket, seen from the daemon (NTF-2, NTF-4, NTF-6). The one door out of the daemon
/// besides `github::GitHub::send` (OPS-16).
#[async_trait::async_trait]
pub trait Poster: Send + Sync + 'static {
    /// A `stat`, never a connection.
    async fn look(&self, socket: &Path) -> Looked;
    /// One connection: connect; with a token, `{"type":"auth","token":"<token>"}` and a newline;
    /// `{"type":"user","message":{"role":"user","content":"<text>"}}` and a newline; close. Nothing is read.
    /// **It returns `Done` in the poll that writes the line's last byte, and awaits nothing after it**
    /// (the close is the drop). So a `post` that hasn't returned when the caller's timeout fires hasn't
    /// written the line in full, and the subscription ends (NTF-6). No timeout inside.
    async fn post(&self, socket: &Path, token: Option<&SessionToken>, text: &str) -> Posted;
}

/// A subscription to be kept (NTF-1). The route has checked every value and evaluated once (NTF-2).
#[derive(Debug)]
pub struct NewSubscription {
    /// It holds its PR by node ID (NTF-1).
    pub id: NodeId,
    /// The name it was registered under: NTF-3's cancel matches it too.
    pub pr: PrName,
    /// `api::condition` (`api.md`).
    pub cond: Condition,
    /// From the route's one evaluation: `began` is the registration.
    pub progress: Progress,
    /// NTF-8's: `cond.since`, or, without one or with one too old on that view (let go, NTF-2), the
    /// cursor of the view the route evaluated on.
    pub cursor: Cursor,
    /// `for`.
    pub life: Duration,
    /// Whether the request held `for` (`Ask again` repeats it only then).
    pub life_given: bool,
    /// What `look` saw when registering.
    pub socket: PathBuf,
    pub socket_id: SocketId,
    pub token: Option<SessionToken>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Registered {
    pub expires_at: Time,
    pub replaced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// 64 are held, or 32 for this socket, and this isn't a replacement (NTF-2): API-3's `limit`.
    Limit,
    /// The confirmation wasn't posted: API-3's `no_session`, its message naming this, the system's error.
    NoSession(String),
    /// `stop` has begun: the connection is about to be closed without an answer (OPS-7).
    Stopping,
}

/// NTF-4's `problem`: what only the user can end. `as_str` is API-4's code, `auth`, or `pr_failing` (API-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemWhy {
    Auth,
    PrFailing,
}

impl ProblemWhy {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProblemWhy::Auth => "auth",
            ProblemWhy::PrFailing => "pr_failing",
        }
    }
}

/// How a subscription ends (NTF-4's table). `as_str` is the word the log line holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Satisfied,
    Ended(Reason),
    Expired,
    Stopped,
    Problem(ProblemWhy),
    Cancelled,
    Replaced,
    SessionGone,
}

impl End {
    pub fn as_str(&self) -> &'static str {
        match self {
            End::Satisfied => "satisfied",
            End::Ended(_) => "ended",
            End::Expired => "expired",
            End::Stopped => "stopped",
            End::Problem(_) => "problem",
            End::Cancelled => "cancelled",
            End::Replaced => "replaced",
            End::SessionGone => "session_gone",
        }
    }
}

/// What the API sees of the notifier, so the routes' shared test state needs no notifier (`api.md`).
#[async_trait::async_trait]
pub trait NotifyHandle: Send + Sync + 'static {
    /// `Poster::look`, for NTF-2's check of `socket` when registering: anything but `Socket` is `no_session`.
    async fn look(&self, socket: &Path) -> Looked;
    /// Ends, as `Replaced`, the subscription that is "the same request" (NTF-2) in any state, posting
    /// nothing. `true` if there was one. For a request whose answer is already known: nothing is
    /// registered, the earlier one still goes, and the answer has `registered: false` with
    /// `replaced: true` (NTF-2).
    fn drop_same(&self, id: &NodeId, socket: &Path, ask: &Ask) -> bool;
    /// Keeps it, in place of the one `drop_same` would end if there is one; posts the confirmation and
    /// returns once that post has ended (NTF-2).
    async fn register(&self, new: NewSubscription) -> Result<Registered, RegisterError>;
    /// NTF-3: drops that socket's subscriptions whose PR is `id`, or that were registered under `name`
    /// (without case), in any state. Their number. Posts nothing; never waits.
    fn cancel(&self, name: &PrName, id: Option<&NodeId>, socket: &Path) -> u32;
    /// The subscriptions held now, those whose confirmation or notice is being posted included. Equal to
    /// `store.view().subscriptions`, which `status` shows (API-5).
    fn count(&self) -> u32;
}

/// The handle behind `api::test_state`: no notifier. Built by T02.
pub struct NoNotify;

#[async_trait::async_trait]
impl NotifyHandle for NoNotify {
    async fn look(&self, _socket: &Path) -> Looked {
        Looked::Failed("no notifier".to_string())
    }

    fn drop_same(&self, _id: &NodeId, _socket: &Path, _ask: &Ask) -> bool {
        false
    }

    async fn register(&self, _new: NewSubscription) -> Result<Registered, RegisterError> {
        Err(RegisterError::Stopping)
    }

    fn cancel(&self, _name: &PrName, _id: Option<&NodeId>, _socket: &Path) -> u32 {
        0
    }

    fn count(&self) -> u32 {
        0
    }
}

pub struct NotifyDeps {
    pub store: Arc<Store>,
    pub clock: SharedClock,
    pub logger: Logger,
    pub activity: Activity,
    pub poster: Arc<dyn Poster>,
    /// `status()`: the `auth` problem with its `problem_since`, and the tier (`stale`).
    pub poller: Arc<dyn PollerHandle>,
    /// `for` counts the sleep noticed (POL-22).
    pub sleep: Arc<SleepWatch>,
    /// `pr_failing` for a PR (`log.md`, `since`).
    pub warnings: Warnings,
    /// `cadence::stale` reads it.
    pub config: Arc<Config>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Until;
    use crate::clock::Mono;
    use crate::testkit::MARKER_SESSION_TOKEN;
    use futures::FutureExt;

    fn subscription(token: SessionToken) -> NewSubscription {
        NewSubscription {
            id: NodeId("PR_1".into()),
            pr: PrName::parse("o/r#1").unwrap(),
            cond: Condition {
                ask: Ask::new(Until::Merged),
                since: None,
                rerun_since: None,
                kinds: None,
            },
            progress: Progress::new(Mono::ZERO),
            cursor: "k3x9a2bq-0".parse().unwrap(),
            life: DEFAULT_LIFE,
            life_given: false,
            socket: PathBuf::from("/tmp/s.sock"),
            socket_id: SocketId { dev: 1, ino: 2 },
            token: Some(token),
        }
    }

    // NTF-9: a session's token never leaves through `Debug`.
    #[test]
    fn ntf9_session_token_debug_holds_no_value() {
        let token = SessionToken::new(MARKER_SESSION_TOKEN.to_string());
        assert_eq!(format!("{token:?}"), "SessionToken(…)");
        assert_eq!(token.expose(), MARKER_SESSION_TOKEN);
        let new = subscription(token.clone());
        let shown = format!("{new:?} {new:#?}");
        assert!(!shown.contains(MARKER_SESSION_TOKEN), "{shown}");
        assert!(shown.contains("SessionToken(…)"));
        let json = format!("\"{MARKER_SESSION_TOKEN}\"");
        let back: SessionToken = serde_json::from_str(&json).unwrap();
        assert_eq!(back.expose(), MARKER_SESSION_TOKEN);
        let none: Option<SessionToken> = serde_json::from_str("null").unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn no_notify_does_nothing() {
        let n = NoNotify;
        let socket = Path::new("/tmp/s.sock");
        let id = NodeId("PR_1".into());
        assert!(matches!(
            n.look(socket).now_or_never(),
            Some(Looked::Failed(_))
        ));
        let registered = n
            .register(subscription(SessionToken::new("t".into())))
            .now_or_never();
        assert_eq!(registered, Some(Err(RegisterError::Stopping)));
        assert!(!n.drop_same(&id, socket, &Ask::new(Until::Merged)));
        assert_eq!(
            n.cancel(&PrName::parse("o/r#1").unwrap(), Some(&id), socket),
            0
        );
        assert_eq!(n.count(), 0);
    }

    #[test]
    fn end_and_problem_words() {
        let ends = [
            End::Satisfied,
            End::Ended(Reason::Closed),
            End::Expired,
            End::Stopped,
            End::Problem(ProblemWhy::Auth),
            End::Cancelled,
            End::Replaced,
            End::SessionGone,
        ];
        assert_eq!(
            ends.map(|e| e.as_str()),
            [
                "satisfied",
                "ended",
                "expired",
                "stopped",
                "problem",
                "cancelled",
                "replaced",
                "session_gone"
            ]
        );
        assert_eq!(
            [ProblemWhy::Auth, ProblemWhy::PrFailing].map(|w| w.as_str()),
            ["auth", "pr_failing"]
        );
    }
}
