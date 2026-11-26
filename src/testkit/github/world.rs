//! The scripted world (TST-3): what the fake GitHub answers from.
//!
//! # How a test scripts it
//!
//! A test builds the world with methods, never with JSON, and schedules changes on GitHub's own
//! clock with [`World::at`], [`World::after`] and [`World::every`]: each takes a closure that calls
//! the same methods. [`World::advance_to`] applies the changes due, in order, each at its own time,
//! and is the only thing that moves the clock. A method called directly acts at [`World::now`].
//!
//! ```ignore
//! let mut w = World::new(MARKER_LOGIN, start);
//! let repo = w.repo("yachiko", "experiment");
//! let pr = w.open_pr(repo);                       // updatedAt = now, no checks: rollup null
//! let slow = w.run(pr, "slow");                   // QUEUED
//! w.after(secs(8), move |w| w.start(slow));       // IN_PROGRESS at +8 s
//! w.after(secs(78), move |w| w.finish(slow, "SUCCESS"));
//! w.advance_to(start.plus(secs(10)));
//! ```
//!
//! # Defaults (docs/02 §4)
//!
//! - `statusCheckRollup` is `null` with no checks, `PENDING` while any runs, `FAILURE` at once when
//!   one fails though others still run, `SUCCESS` when all have passed (`SUCCESS`, `NEUTRAL`,
//!   `SKIPPED`). A conclusion it doesn't know, `FOO` included, is answered as given and counts as a
//!   failure.
//! - `updatedAt` moves to now with a push, a comment, a draft change, a close, a merge and a reopen;
//!   with a review or a thread reply only [`World::set_review_lag`] later (10 s by default); never
//!   with a check or a commit status, nor with a title or base edit (not measured: a test that wants
//!   it moved calls [`World::set_updated_at`]).
//! - `mergeStateStatus` is `DIRTY` when `CONFLICTING`, `UNKNOWN` when `UNKNOWN`, `UNSTABLE` while a
//!   check runs or has failed, `CLEAN` otherwise; a draft stays as it is (docs/02: `DRAFT` is never
//!   seen). A merged PR reads `mergeable` and `mergeStateStatus` `UNKNOWN`; a closed one keeps the
//!   values it had when it closed.
//! - A thread reply is a review of its own, `COMMENTED`, with one inline comment.
//! - A PR leaves `viewer.pullRequests(states: OPEN)` the moment it ends.
//!
//! # Markers (TST-11)
//!
//! Every title the world makes holds `MARKER_TITLE`, every check name it makes `MARKER_CHECK`, every
//! login it makes `MARKER_LOGIN`; the executor puts `MARKER_GITHUB_TEXT` in every error message. A
//! name, title or login a test gives is used exactly as given.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::clock::Time;
use crate::testkit::{MARKER_CHECK, MARKER_LOGIN, MARKER_TITLE};

/// A repository of the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RepoId(pub(super) usize);

/// A PR of the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PrId(pub(super) usize);

/// A check run on one commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RunId {
    pub(super) commit: usize,
    pub(super) index: usize,
}

/// The default delay before a review or a thread reply moves `updatedAt` (docs/02 §4: "about 10 s").
pub const REVIEW_LAG: Duration = Duration::from_secs(10);

pub(super) struct Repo {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrState::Open => "OPEN",
            PrState::Closed => "CLOSED",
            PrState::Merged => "MERGED",
        }
    }
}

/// What reads see during a lagged push: the PR as it was before it.
pub(super) struct Lag {
    pub history_len: usize,
    pub updated_at: Time,
    pub until: Time,
}

pub(super) struct Comment {
    pub database_id: u64,
    pub author: Option<String>,
    pub created_at: Time,
}

pub(super) struct Review {
    pub database_id: u64,
    pub author: Option<String>,
    pub state: String,
    pub submitted_at: Time,
}

pub(super) struct ReviewComment {
    pub database_id: u64,
    pub author: Option<String>,
    pub created_at: Time,
    /// The review it belongs to (an index into the PR's reviews).
    pub review: usize,
}

pub(super) struct Thread {
    pub comments: Vec<ReviewComment>,
}

pub(super) struct Pr {
    pub id: String,
    pub repo: usize,
    pub number: u64,
    pub title: String,
    pub author: Option<String>,
    pub state: PrState,
    pub draft: bool,
    pub base: String,
    pub head_ref: String,
    /// Commit indexes, oldest first; the last is the head.
    pub history: Vec<usize>,
    pub created_at: Time,
    pub updated_at: Time,
    pub closed_at: Option<Time>,
    pub merged_at: Option<Time>,
    pub merge_commit: Option<usize>,
    pub mergeable: String,
    pub merge_state: Option<String>,
    /// A closed PR's `mergeable` and `mergeStateStatus`, as they were when it closed.
    pub frozen: Option<(String, String)>,
    pub review_decision: Option<String>,
    pub comments: Vec<Comment>,
    pub reviews: Vec<Review>,
    pub threads: Vec<Thread>,
    pub hidden: bool,
    pub node_error: Option<String>,
    pub field_errors: Vec<(String, String)>,
    /// Reads left that answer `mergeable: UNKNOWN`; a read is one request that asks for it.
    pub unknown_reads: AtomicU32,
    pub lag: Option<Lag>,
}

pub(super) struct Run {
    pub database_id: u64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<Time>,
    pub completed_at: Option<Time>,
    /// Out-of-order reads to come: each request that reads this run takes one.
    pub misreads: Mutex<VecDeque<String>>,
}

pub(super) struct Status {
    pub context: String,
    pub state: String,
    pub created_at: Time,
}

pub(super) enum Check {
    Run(Run),
    Status(Status),
}

pub(super) struct Commit {
    pub oid: String,
    pub repo: usize,
    pub committed_date: Time,
    pub checks: Vec<Check>,
    /// The rollup's state as a test chose it, for a commit whose unread tail decides it (POL-8).
    pub rollup: Option<String>,
}

type Change = Box<dyn FnMut(&mut World) + Send>;

struct Scheduled {
    due: Time,
    seq: u64,
    period: Option<Duration>,
    change: Change,
}

/// The scripted world (TST-3). See the module's documentation for how to script it.
pub struct World {
    now: Time,
    login: String,
    remaining: u32,
    reset_at: Time,
    review_lag: Duration,
    pub(super) repos: Vec<Repo>,
    pub(super) prs: Vec<Pr>,
    pub(super) commits: Vec<Commit>,
    by_sha: HashMap<String, usize>,
    scheduled: Vec<Scheduled>,
    seq: u64,
    next_database_id: u64,
    next_name: u64,
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

impl World {
    /// An empty world whose viewer is `login`, its clock at `start`, with 5 000 points left and a
    /// reset an hour ahead. `MARKER_LOGIN` is the login a test passes unless it names its own.
    pub fn new(login: &str, start: Time) -> World {
        World {
            now: start,
            login: login.to_owned(),
            remaining: 5000,
            reset_at: start.plus(secs(3600)),
            review_lag: REVIEW_LAG,
            repos: Vec::new(),
            prs: Vec::new(),
            commits: Vec::new(),
            by_sha: HashMap::new(),
            scheduled: Vec::new(),
            seq: 0,
            next_database_id: 1000,
            next_name: 0,
        }
    }

    // ----- The clock and the schedule ---------------------------------------------------------

    /// GitHub's clock: only ever forward.
    pub fn now(&self) -> Time {
        self.now
    }

    /// Applies the scheduled changes due by `t`, in order of their times, and those due at one time
    /// in the order they were scheduled; each runs with [`World::now`] at its own time. Then the
    /// clock is `t`. An earlier `t` does nothing: GitHub's clock never goes back.
    pub fn advance_to(&mut self, t: Time) {
        loop {
            let next = self
                .scheduled
                .iter()
                .enumerate()
                .filter(|(_, s)| s.due <= t)
                .min_by_key(|(_, s)| (s.due, s.seq))
                .map(|(i, _)| i);
            let Some(i) = next else { break };
            let mut s = self.scheduled.swap_remove(i);
            if s.due > self.now {
                self.now = s.due;
            }
            (s.change)(self);
            if let Some(period) = s.period {
                s.due = s.due.plus(period);
                s.seq = self.next_seq();
                self.scheduled.push(s);
            }
        }
        if t > self.now {
            self.now = t;
        }
    }

    /// Schedules `change` at `t` on GitHub's clock. At or before now, it's applied at once.
    pub fn at(&mut self, t: Time, change: impl FnOnce(&mut World) + Send + 'static) {
        let mut once = Some(change);
        let change = move |w: &mut World| {
            if let Some(c) = once.take() {
                c(w);
            }
        };
        if t <= self.now {
            let mut change = change;
            change(self);
            return;
        }
        let seq = self.next_seq();
        self.scheduled.push(Scheduled {
            due: t,
            seq,
            period: None,
            change: Box::new(change),
        });
    }

    /// Schedules `change` at `d` after now.
    pub fn after(&mut self, d: Duration, change: impl FnOnce(&mut World) + Send + 'static) {
        let t = self.now.plus(d);
        self.at(t, change);
    }

    /// Schedules `change` at `first` and then every `period` for good: a change that repeats at
    /// every poll (26 PRs flipping `draft` every 30 s). `period` must not be zero.
    pub fn every(
        &mut self,
        first: Time,
        period: Duration,
        change: impl FnMut(&mut World) + Send + 'static,
    ) {
        assert!(!period.is_zero(), "a repeating change needs a period");
        let seq = self.next_seq();
        self.scheduled.push(Scheduled {
            due: first.max(self.now),
            seq,
            period: Some(period),
            change: Box::new(change),
        });
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    // ----- The viewer and the rate limit ------------------------------------------------------

    /// `viewer.login`.
    pub fn login(&self) -> &str {
        &self.login
    }

    /// Changes `viewer.login` (MOD-11). PRs keep their authors: the old login's PRs leave the
    /// viewer's list.
    pub fn set_login(&mut self, login: &str) {
        self.login = login.to_owned();
    }

    /// The `rateLimit` block's `remaining` and `resetAt`; `execute` answers them, T20 sets and spends them.
    pub fn set_rate(&mut self, remaining: u32, reset_at: Time) {
        self.remaining = remaining;
        self.reset_at = reset_at;
    }

    /// Spends `cost` points of `remaining` (T20, after it has answered a request).
    pub fn spend(&mut self, cost: u32) {
        self.remaining = self.remaining.saturating_sub(cost);
    }

    /// `remaining` and `resetAt` as they stand.
    pub fn rate(&self) -> (u32, Time) {
        (self.remaining, self.reset_at)
    }

    // ----- Repositories -----------------------------------------------------------------------

    /// A repository `owner/name`, made now; the same one again if it exists. Its node ID is kept
    /// through renames and transfers.
    pub fn repo(&mut self, owner: &str, name: &str) -> RepoId {
        if let Some(i) = self
            .repos
            .iter()
            .position(|r| r.owner == owner && r.name == name)
        {
            return RepoId(i);
        }
        let i = self.repos.len();
        self.repos.push(Repo {
            id: format!("R_fake{:04}", i + 1),
            owner: owner.to_owned(),
            name: name.to_owned(),
            hidden: false,
        });
        RepoId(i)
    }

    /// Renames the repository; its node ID and its PRs' IDs stay (MOD-1, assumed).
    pub fn rename_repo(&mut self, repo: RepoId, name: &str) {
        self.repos[repo.0].name = name.to_owned();
    }

    /// Transfers the repository to `owner`; node IDs stay.
    pub fn transfer_repo(&mut self, repo: RepoId, owner: &str) {
        self.repos[repo.0].owner = owner.to_owned();
    }

    /// Hides the repository, as lost access or a deletion would: `repository` is `NOT_FOUND` by
    /// name, and each of its PRs is off the open list and `NOT_FOUND` by node.
    pub fn hide_repo(&mut self, repo: RepoId) {
        self.repos[repo.0].hidden = true;
    }

    /// Brings a hidden repository back, as it was.
    pub fn show_repo(&mut self, repo: RepoId) {
        self.repos[repo.0].hidden = false;
    }

    /// `owner/name` as it stands.
    pub fn repo_name(&self, repo: RepoId) -> String {
        let r = &self.repos[repo.0];
        format!("{}/{}", r.owner, r.name)
    }

    // ----- PRs --------------------------------------------------------------------------------

    /// A PR opened now in `repo`: the next number in it, authored by the viewer, titled
    /// `MARKER-TITLE #n`, from `branch-n` into `main`, with a new head commit and no checks,
    /// `MERGEABLE`, `createdAt` and `updatedAt` now. A test changes any of it with the setters.
    pub fn open_pr(&mut self, repo: RepoId) -> PrId {
        let number = self.prs.iter().filter(|p| p.repo == repo.0).count() as u64 + 1;
        let i = self.prs.len();
        let head = self.new_commit(repo.0, None);
        let login = self.login.clone();
        self.prs.push(Pr {
            id: format!("PR_fake{:05}", i + 1),
            repo: repo.0,
            number,
            title: format!("{MARKER_TITLE} #{number}"),
            author: Some(login),
            state: PrState::Open,
            draft: false,
            base: "main".to_owned(),
            head_ref: format!("branch-{number}"),
            history: vec![head],
            created_at: self.now,
            updated_at: self.now,
            closed_at: None,
            merged_at: None,
            merge_commit: None,
            mergeable: "MERGEABLE".to_owned(),
            merge_state: None,
            frozen: None,
            review_decision: None,
            comments: Vec::new(),
            reviews: Vec::new(),
            threads: Vec::new(),
            hidden: false,
            node_error: None,
            field_errors: Vec::new(),
            unknown_reads: AtomicU32::new(0),
            lag: None,
        });
        PrId(i)
    }

    /// `n` open PRs in `repo`, the first the newest: PR `i`'s `updatedAt` (and `createdAt`) is
    /// `spacing × i` before now, so all are distinct (bulk worlds of 41, 150, 200, 205 PRs).
    pub fn open_prs(&mut self, repo: RepoId, n: usize, spacing: Duration) -> Vec<PrId> {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let pr = self.open_pr(repo);
            let t = self.now.minus(spacing.saturating_mul(i as u32));
            self.prs[pr.0].created_at = t;
            self.prs[pr.0].updated_at = t;
            out.push(pr);
        }
        out
    }

    /// The PR's node ID.
    pub fn node_id(&self, pr: PrId) -> &str {
        &self.prs[pr.0].id
    }

    /// The PR's number.
    pub fn number(&self, pr: PrId) -> u64 {
        self.prs[pr.0].number
    }

    /// The PR's real head SHA (what a read sees after any lag).
    pub fn head(&self, pr: PrId) -> &str {
        let p = &self.prs[pr.0];
        let head = p.history.last().copied().unwrap_or_default();
        &self.commits[head].oid
    }

    /// Whether the PR is a draft.
    pub fn is_draft(&self, pr: PrId) -> bool {
        self.prs[pr.0].draft
    }

    /// The PR's real `updatedAt`.
    pub fn updated_at(&self, pr: PrId) -> Time {
        self.prs[pr.0].updated_at
    }

    /// Sets the title, as given. Doesn't move `updatedAt` (not measured).
    pub fn set_title(&mut self, pr: PrId, title: &str) {
        self.prs[pr.0].title = title.to_owned();
    }

    /// Sets the author's login, as given; `None` is a deleted account (`author: null`). Another
    /// author's PR isn't on the viewer's list.
    pub fn set_author(&mut self, pr: PrId, login: Option<&str>) {
        self.prs[pr.0].author = login.map(str::to_owned);
    }

    /// Sets the head branch's name (two PRs may share one).
    pub fn set_head_ref(&mut self, pr: PrId, name: &str) {
        self.prs[pr.0].head_ref = name.to_owned();
    }

    /// Changes the base branch. Doesn't move `updatedAt` or `mergeable` (not measured): a test
    /// that wants `UNKNOWN` with it calls [`World::unknown_for`].
    pub fn set_base(&mut self, pr: PrId, name: &str) {
        self.prs[pr.0].base = name.to_owned();
    }

    /// Sets `updatedAt` against GitHub's clock (`now - 2 min`, say). Nothing else moves.
    pub fn set_updated_at(&mut self, pr: PrId, t: Time) {
        self.prs[pr.0].updated_at = t;
    }

    /// Sets `createdAt`.
    pub fn set_created_at(&mut self, pr: PrId, t: Time) {
        self.prs[pr.0].created_at = t;
    }

    /// Sets `reviewDecision` (`APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED`, or `None`).
    pub fn set_review_decision(&mut self, pr: PrId, decision: Option<&str>) {
        self.prs[pr.0].review_decision = decision.map(str::to_owned);
    }

    /// Sets `mergeable` (`MERGEABLE`, `CONFLICTING`, `UNKNOWN`): `mergeStateStatus` follows
    /// (`DIRTY` with `CONFLICTING`) unless set with [`World::set_merge_state`].
    pub fn set_mergeable(&mut self, pr: PrId, mergeable: &str) {
        self.prs[pr.0].mergeable = mergeable.to_owned();
    }

    /// Sets `mergeStateStatus` whatever the rest says (`BLOCKED`, `BEHIND`); `None` derives it again.
    pub fn set_merge_state(&mut self, pr: PrId, state: Option<&str>) {
        self.prs[pr.0].merge_state = state.map(str::to_owned);
    }

    /// Sets the delay before a review or a thread reply moves `updatedAt` (10 s by default).
    pub fn set_review_lag(&mut self, lag: Duration) {
        self.review_lag = lag;
    }

    // ----- Actions that move `updatedAt` ------------------------------------------------------

    /// A push of a new commit: the head moves, the new head has no checks (rollup `null`), and
    /// `updatedAt` is now. Returns the new SHA.
    pub fn push(&mut self, pr: PrId) -> String {
        let repo = self.prs[pr.0].repo;
        let c = self.new_commit(repo, None);
        self.move_head(pr, c, None);
        self.commits[c].oid.clone()
    }

    /// A push of `sha`. If the world has that commit already (a force-push back to an earlier
    /// SHA), it comes back with the checks it had; otherwise it's a new commit with none.
    pub fn push_sha(&mut self, pr: PrId, sha: &str) {
        let repo = self.prs[pr.0].repo;
        let c = self.commit_by_sha(repo, sha);
        self.move_head(pr, c, None);
    }

    /// A push whose reads lag: for `lag` after now, reads see the PR as before the push (the old
    /// head with its old checks and rollup, the old `updatedAt`), then the new head with no checks
    /// (docs/02 §4: the old head at +0.4 s, the new one at +3 s). `sha` is the new head's, or a
    /// made-up one when `None`. Returns the new SHA.
    pub fn push_lagged(&mut self, pr: PrId, sha: Option<&str>, lag: Duration) -> String {
        let repo = self.prs[pr.0].repo;
        let c = match sha {
            Some(sha) => self.commit_by_sha(repo, sha),
            None => self.new_commit(repo, None),
        };
        let p = &self.prs[pr.0];
        let before = Lag {
            history_len: p.history.len(),
            updated_at: p.updated_at,
            until: self.now.plus(lag),
        };
        self.move_head(pr, c, Some(before));
        self.commits[c].oid.clone()
    }

    fn move_head(&mut self, pr: PrId, commit: usize, lag: Option<Lag>) {
        let now = self.now;
        let p = &mut self.prs[pr.0];
        p.history.push(commit);
        p.updated_at = now;
        p.lag = lag;
    }

    /// An issue comment now, by a made-up login; `updatedAt` is now. Returns its `databaseId`.
    pub fn comment(&mut self, pr: PrId) -> u64 {
        let login = self.made_up_login();
        self.comment_by(pr, Some(&login))
    }

    /// An issue comment now by `login` (`None`: a deleted account). Returns its `databaseId`.
    pub fn comment_by(&mut self, pr: PrId, login: Option<&str>) -> u64 {
        let id = self.next_database_id();
        let now = self.now;
        let p = &mut self.prs[pr.0];
        p.comments.push(Comment {
            database_id: id,
            author: login.map(str::to_owned),
            created_at: now,
        });
        p.updated_at = now;
        id
    }

    /// Converts to draft (`true`) or marks ready (`false`); `updatedAt` is now.
    pub fn set_draft(&mut self, pr: PrId, draft: bool) {
        let now = self.now;
        let p = &mut self.prs[pr.0];
        p.draft = draft;
        p.updated_at = now;
    }

    /// Closes it unmerged: `CLOSED`, `closedAt` and `updatedAt` now, off the open list, keeping
    /// the `mergeable` and `mergeStateStatus` it has now.
    pub fn close(&mut self, pr: PrId) {
        let frozen = self.live_mergeability(pr.0);
        let now = self.now;
        let p = &mut self.prs[pr.0];
        p.state = PrState::Closed;
        p.closed_at = Some(now);
        p.updated_at = now;
        p.frozen = Some(frozen);
    }

    /// Merges it: `MERGED`, `merged`, `mergedAt` equal to `closedAt`, a merge commit, `mergeable`
    /// and `mergeStateStatus` `UNKNOWN`, off the open list, `updatedAt` now.
    pub fn merge(&mut self, pr: PrId) {
        let repo = self.prs[pr.0].repo;
        let mc = self.new_commit(repo, None);
        let now = self.now;
        let p = &mut self.prs[pr.0];
        p.state = PrState::Merged;
        p.closed_at = Some(now);
        p.merged_at = Some(now);
        p.merge_commit = Some(mc);
        p.updated_at = now;
        p.frozen = None;
    }

    /// Reopens a closed (or, for a test's sake, merged) PR: `OPEN`, on the open list again,
    /// `closedAt`, `mergedAt` and the merge commit cleared, `updatedAt` now (assumed, as a close).
    pub fn reopen(&mut self, pr: PrId) {
        let now = self.now;
        let p = &mut self.prs[pr.0];
        p.state = PrState::Open;
        p.closed_at = None;
        p.merged_at = None;
        p.merge_commit = None;
        p.updated_at = now;
        p.frozen = None;
    }

    // ----- Reviews ----------------------------------------------------------------------------

    /// A review now by a made-up login, in `state` (`COMMENTED`, `APPROVED`, `CHANGES_REQUESTED`),
    /// with `inline` comments on the diff, each opening a thread. `reviews.totalCount` and
    /// `reviewThreads.totalCount` are right at once; `updatedAt` moves to now only after the review
    /// lag (docs/02 §4). Returns the review's `databaseId`.
    pub fn review(&mut self, pr: PrId, state: &str, inline: u32) -> u64 {
        let login = self.made_up_login();
        self.review_by(pr, Some(&login), state, inline)
    }

    /// The same by `login` (`None`: a deleted account).
    pub fn review_by(&mut self, pr: PrId, login: Option<&str>, state: &str, inline: u32) -> u64 {
        let review = self.add_review(pr, login, state);
        for _ in 0..inline {
            let id = self.next_database_id();
            let now = self.now;
            let p = &mut self.prs[pr.0];
            p.threads.push(Thread {
                comments: vec![ReviewComment {
                    database_id: id,
                    author: login.map(str::to_owned),
                    created_at: now,
                    review,
                }],
            });
        }
        self.prs[pr.0].reviews[review].database_id
    }

    /// A reply in the PR's last thread (a new thread if it has none), by a made-up login: a review
    /// of its own in state `COMMENTED` with one inline comment (docs/02 §4). `updatedAt` moves after
    /// the review lag. Returns the new review's `databaseId`.
    pub fn thread_reply(&mut self, pr: PrId) -> u64 {
        let login = self.made_up_login();
        let review = self.add_review(pr, Some(&login), "COMMENTED");
        let id = self.next_database_id();
        let now = self.now;
        let p = &mut self.prs[pr.0];
        let comment = ReviewComment {
            database_id: id,
            author: Some(login),
            created_at: now,
            review,
        };
        match p.threads.last_mut() {
            Some(t) => t.comments.push(comment),
            None => p.threads.push(Thread {
                comments: vec![comment],
            }),
        }
        p.reviews[review].database_id
    }

    fn add_review(&mut self, pr: PrId, login: Option<&str>, state: &str) -> usize {
        let id = self.next_database_id();
        let now = self.now;
        let p = &mut self.prs[pr.0];
        p.reviews.push(Review {
            database_id: id,
            author: login.map(str::to_owned),
            state: state.to_owned(),
            submitted_at: now,
        });
        let index = p.reviews.len() - 1;
        // docs/02 §4: a review moves `updatedAt` to its own time, but late.
        let lag = self.review_lag;
        let at = now;
        self.after(lag, move |w| {
            let p = &mut w.prs[pr.0];
            if p.updated_at < at {
                p.updated_at = at;
            }
        });
        index
    }

    // ----- Checks -----------------------------------------------------------------------------

    /// A check run named `name` on the PR's head, `QUEUED`, with a new `databaseId`. Moves nothing.
    pub fn run(&mut self, pr: PrId, name: &str) -> RunId {
        let commit = self.head_commit(pr);
        self.add_run(commit, name.to_owned(), "QUEUED", None)
    }

    /// A check run with a made-up name holding `MARKER_CHECK`.
    pub fn unnamed_run(&mut self, pr: PrId) -> RunId {
        let name = self.made_up_check();
        let commit = self.head_commit(pr);
        self.add_run(commit, name, "QUEUED", None)
    }

    /// `n` check runs with made-up names on the PR's head, each with `status` and `conclusion`
    /// (100, 101, 187, 300, 301, 400 checks; a tail in another state is a second call).
    pub fn runs(
        &mut self,
        pr: PrId,
        n: usize,
        status: &str,
        conclusion: Option<&str>,
    ) -> Vec<RunId> {
        let commit = self.head_commit(pr);
        (0..n)
            .map(|_| {
                let name = self.made_up_check();
                let run = self.add_run(commit, name, "QUEUED", None);
                self.set_run(run, status, conclusion);
                run
            })
            .collect()
    }

    fn add_run(
        &mut self,
        commit: usize,
        name: String,
        status: &str,
        conclusion: Option<&str>,
    ) -> RunId {
        let id = self.next_database_id();
        let c = &mut self.commits[commit];
        c.checks.push(Check::Run(Run {
            database_id: id,
            name,
            status: status.to_owned(),
            conclusion: conclusion.map(str::to_owned),
            started_at: None,
            completed_at: None,
            misreads: Mutex::new(VecDeque::new()),
        }));
        RunId {
            commit,
            index: c.checks.len() - 1,
        }
    }

    /// Sets a run's `status` and `conclusion` as given, any string (`FOO` too). `startedAt` is set
    /// the first time it leaves `QUEUED`, `completedAt` when it's `COMPLETED`.
    pub fn set_run(&mut self, run: RunId, status: &str, conclusion: Option<&str>) {
        let now = self.now;
        if let Some(r) = self.run_mut(run) {
            r.status = status.to_owned();
            r.conclusion = conclusion.map(str::to_owned);
            if status != "QUEUED" && r.started_at.is_none() {
                r.started_at = Some(now);
            }
            r.completed_at = (status == "COMPLETED").then_some(now);
        }
    }

    /// `IN_PROGRESS`.
    pub fn start(&mut self, run: RunId) {
        self.set_run(run, "IN_PROGRESS", None);
    }

    /// `COMPLETED` with `conclusion` (`SUCCESS`, `FAILURE`, `SKIPPED`, `CANCELLED`, `FOO`…).
    pub fn finish(&mut self, run: RunId, conclusion: &str) {
        self.set_run(run, "COMPLETED", Some(conclusion));
    }

    /// A re-run: the same run, same `databaseId`, back to `QUEUED` on the same head with no
    /// conclusion (assumed: a re-run isn't measured, docs/02 §6).
    pub fn rerun(&mut self, run: RunId) {
        if let Some(r) = self.run_mut(run) {
            r.status = "QUEUED".to_owned();
            r.conclusion = None;
            r.started_at = None;
            r.completed_at = None;
        }
    }

    /// An out-of-order read: the next request that reads this run sees it with `status` (and no
    /// conclusion), and the one after sees it as it is (docs/02 §4: `IN_PROGRESS`, then `QUEUED`).
    /// Called again, the reads queue up.
    pub fn misread(&mut self, run: RunId, status: &str) {
        if let Some(r) = self.run_mut(run) {
            r.misreads
                .get_mut()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(status.to_owned());
        }
    }

    /// The run's `databaseId`.
    pub fn run_database_id(&self, run: RunId) -> u64 {
        match self.commits[run.commit].checks.get(run.index) {
            Some(Check::Run(r)) => r.database_id,
            _ => 0,
        }
    }

    fn run_mut(&mut self, run: RunId) -> Option<&mut Run> {
        match self.commits[run.commit].checks.get_mut(run.index) {
            Some(Check::Run(r)) => Some(r),
            _ => None,
        }
    }

    /// Posts a commit status `context` in `state` (`PENDING`, `SUCCESS`, `FAILURE`, `ERROR`) on the
    /// PR's head, in the same list as its runs. Posted again, the status takes the new state in
    /// place (a pending again reads pending). Moves nothing.
    pub fn status(&mut self, pr: PrId, context: &str, state: &str) {
        let commit = self.head_commit(pr);
        let now = self.now;
        let c = &mut self.commits[commit];
        let existing = c.checks.iter_mut().find_map(|ch| match ch {
            Check::Status(s) if s.context == context => Some(s),
            _ => None,
        });
        match existing {
            Some(s) => {
                s.state = state.to_owned();
                s.created_at = now;
            }
            None => c.checks.push(Check::Status(Status {
                context: context.to_owned(),
                state: state.to_owned(),
                created_at: now,
            })),
        }
    }

    /// Sets the head commit's rollup state whatever its checks say (for many checks whose unread
    /// tail decides it); `None` derives it again.
    pub fn set_rollup(&mut self, pr: PrId, state: Option<&str>) {
        let commit = self.head_commit(pr);
        self.commits[commit].rollup = state.map(str::to_owned);
    }

    // ----- Misbehaviours that change an answer (TST-3) ----------------------------------------

    /// `mergeable` (and `mergeStateStatus`) read `UNKNOWN` for the next `reads` requests that ask
    /// for it, on an open PR; then the value.
    pub fn unknown_for(&mut self, pr: PrId, reads: u32) {
        self.prs[pr.0].unknown_reads.store(reads, Ordering::Relaxed);
    }

    /// The same for every PR there is (a list of idle PRs; docs/02 §4).
    pub fn unknown_for_all(&mut self, reads: u32) {
        for p in &self.prs {
            p.unknown_reads.store(reads, Ordering::Relaxed);
        }
    }

    /// The PR's node answers `null` with an error of `type_` (`FORBIDDEN`, say) by node and by
    /// name, until cleared with `None`. The other nodes are answered.
    pub fn node_error(&mut self, pr: PrId, type_: Option<&str>) {
        self.prs[pr.0].node_error = type_.map(str::to_owned);
    }

    /// The PR's field `field` (as the schema names it: `commits`, `mergeable`) answers an error of
    /// `type_` whose path goes into the node, until cleared with `None`. GraphQL's null rule
    /// applies: a non-null field's error empties the node.
    pub fn field_error(&mut self, pr: PrId, field: &str, type_: Option<&str>) {
        let p = &mut self.prs[pr.0];
        p.field_errors.retain(|(f, _)| f != field);
        if let Some(t) = type_ {
            p.field_errors.push((field.to_owned(), t.to_owned()));
        }
    }

    /// Hides the PR (deleted, or access lost): off the open list, `NOT_FOUND` by node and by name.
    pub fn hide(&mut self, pr: PrId) {
        self.prs[pr.0].hidden = true;
    }

    /// Brings a hidden PR back with the same node ID, in whatever state it now has (a test merges
    /// it first to bring it back merged).
    pub fn unhide(&mut self, pr: PrId) {
        self.prs[pr.0].hidden = false;
    }

    // ----- What reads see ---------------------------------------------------------------------

    pub(super) fn pr_visible(&self, i: usize) -> bool {
        let p = &self.prs[i];
        !p.hidden && !self.repos[p.repo].hidden
    }

    /// The PR's history as reads see it now (shorter during a lagged push).
    pub(super) fn view_history(&self, i: usize) -> &[usize] {
        let p = &self.prs[i];
        match &p.lag {
            Some(l) if self.now < l.until => &p.history[..l.history_len.min(p.history.len())],
            _ => &p.history,
        }
    }

    pub(super) fn view_head(&self, i: usize) -> usize {
        self.view_history(i).last().copied().unwrap_or_default()
    }

    pub(super) fn view_updated_at(&self, i: usize) -> Time {
        let p = &self.prs[i];
        match &p.lag {
            Some(l) if self.now < l.until => l.updated_at,
            _ => p.updated_at,
        }
    }

    // TST-3: docs/02 §4's rollup: `null` with no checks, `FAILURE` at once though others run.
    /// The rollup's state: `None` with no checks.
    pub(super) fn rollup_state(&self, commit: usize) -> Option<String> {
        let c = &self.commits[commit];
        if c.checks.is_empty() {
            return None;
        }
        if let Some(s) = &c.rollup {
            return Some(s.clone());
        }
        let passed = |s: &str| matches!(s, "SUCCESS" | "NEUTRAL" | "SKIPPED");
        let mut failing = false;
        let mut running = false;
        for ch in &c.checks {
            match ch {
                Check::Run(r) if r.status == "COMPLETED" => {
                    failing |= !r.conclusion.as_deref().is_some_and(passed);
                }
                Check::Run(_) => running = true,
                Check::Status(s) => match s.state.as_str() {
                    "PENDING" | "EXPECTED" => running = true,
                    "SUCCESS" => {}
                    _ => failing = true,
                },
            }
        }
        Some(
            if failing {
                "FAILURE"
            } else if running {
                "PENDING"
            } else {
                "SUCCESS"
            }
            .to_owned(),
        )
    }

    /// `mergeable` and `mergeStateStatus` of an open PR from its fields, without read counters.
    fn live_mergeability(&self, i: usize) -> (String, String) {
        let p = &self.prs[i];
        let m = p.mergeable.clone();
        let s = match &p.merge_state {
            Some(s) => s.clone(),
            None => match m.as_str() {
                "CONFLICTING" => "DIRTY".to_owned(),
                "UNKNOWN" => "UNKNOWN".to_owned(),
                _ => match self.rollup_state(self.view_head(i)).as_deref() {
                    Some("PENDING" | "FAILURE" | "ERROR") => "UNSTABLE".to_owned(),
                    _ => "CLEAN".to_owned(),
                },
            },
        };
        (m, s)
    }

    /// What a read answers for `mergeable` and `mergeStateStatus` now.
    pub(super) fn mergeability(&self, i: usize) -> (String, String) {
        let p = &self.prs[i];
        match p.state {
            PrState::Merged => ("UNKNOWN".to_owned(), "UNKNOWN".to_owned()),
            PrState::Closed => p
                .frozen
                .clone()
                .unwrap_or_else(|| self.live_mergeability(i)),
            PrState::Open if p.unknown_reads.load(Ordering::Relaxed) > 0 => {
                ("UNKNOWN".to_owned(), "UNKNOWN".to_owned())
            }
            PrState::Open => self.live_mergeability(i),
        }
    }

    /// After a request that read `mergeable` of these PRs and these runs: one read is spent.
    pub(super) fn spend_reads(&self, prs: &[usize], runs: &[(usize, usize)]) {
        for &i in prs {
            let _ =
                self.prs[i]
                    .unknown_reads
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
        }
        for &(c, i) in runs {
            if let Some(Check::Run(r)) = self.commits[c].checks.get(i) {
                r.misreads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .pop_front();
            }
        }
    }

    // ----- Helpers ----------------------------------------------------------------------------

    fn head_commit(&self, pr: PrId) -> usize {
        self.prs[pr.0].history.last().copied().unwrap_or_default()
    }

    fn commit_by_sha(&mut self, repo: usize, sha: &str) -> usize {
        match self.by_sha.get(sha) {
            Some(&c) => c,
            None => self.new_commit(repo, Some(sha)),
        }
    }

    fn new_commit(&mut self, repo: usize, sha: Option<&str>) -> usize {
        let i = self.commits.len();
        let oid = match sha {
            Some(s) => s.to_owned(),
            None => made_up_sha(i as u64 + 1),
        };
        self.by_sha.insert(oid.clone(), i);
        self.commits.push(Commit {
            oid,
            repo,
            committed_date: self.now,
            checks: Vec::new(),
            rollup: None,
        });
        i
    }

    fn next_database_id(&mut self) -> u64 {
        self.next_database_id += 1;
        self.next_database_id
    }

    // TST-11: a login or a check name the world makes up holds its marker.
    fn made_up_login(&mut self) -> String {
        self.next_name += 1;
        format!("{MARKER_LOGIN}-{}", self.next_name)
    }

    fn made_up_check(&mut self) -> String {
        self.next_name += 1;
        format!("{MARKER_CHECK}-{}", self.next_name)
    }
}

/// 40 hex characters from a counter, the same every run (TST-2).
fn made_up_sha(n: u64) -> String {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut out = String::with_capacity(40);
    while out.len() < 40 {
        x ^= x >> 31;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        out.push_str(&format!("{:016x}", x));
    }
    out.truncate(40);
    out
}
