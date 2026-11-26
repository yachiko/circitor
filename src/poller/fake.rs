//! `FakePoller` (T43): a `PollerHandle` for route and waiter tests.
//!
//! Its defaults: tier active, no problem, first poll `Finished`, `refresh_pr` `Refreshed::NotSent`,
//! `read_through` and `track` a `GithubUnavailable` refusal, `poll_now` `Polled` at cost 0.
//! The contracts say nothing of `idle_since`: it's `Mono::ZERO`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::Mono;
use crate::model::{NodeId, PrName};
use crate::poller::{
    FirstPoll, PollOutcome, PollReport, PollerHandle, PollerStatus, Refreshed, Refusal,
    RefusalCode, Tier,
};
use crate::token::TokenSourceKind;

type RefreshFn = Box<dyn FnMut(&NodeId) -> Refreshed + Send + 'static>;
type PollFn = Box<dyn FnMut() -> PollReport + Send + 'static>;
type NameFn = Box<dyn FnMut(&PrName) -> Result<NodeId, Refusal> + Send + 'static>;

#[derive(Default)]
struct Counts {
    client_requests: AtomicUsize,
    read_throughs: AtomicUsize,
    tracks: AtomicUsize,
    refreshes: AtomicUsize,
    polls: AtomicUsize,
    nudges: AtomicUsize,
}

pub struct FakePoller {
    status: Mutex<PollerStatus>,
    first: tokio::sync::watch::Sender<FirstPoll>,
    refresh: Mutex<Option<RefreshFn>>,
    read_through: Mutex<Option<NameFn>>,
    track: Mutex<Option<NameFn>>,
    poll: Mutex<Option<PollFn>>,
    counts: Counts,
}

/// A std lock, taken and released inside one function; a poisoned one still holds its value.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn unavailable() -> Refusal {
    Refusal {
        code: RefusalCode::GithubUnavailable,
        message: "GitHub is unavailable (FakePoller's default)".to_string(),
        until: None,
    }
}

impl FakePoller {
    pub fn new() -> Arc<FakePoller> {
        Arc::new(FakePoller {
            status: Mutex::new(PollerStatus {
                tier: Tier::Active,
                last_poll_at: None,
                last_poll: None,
                problem: None,
                problem_since: None,
                cost_last_hour: 0,
                remaining: None,
                reset_at: None,
                login: None,
                token_source: TokenSourceKind::Gh,
                gh_path: "gh".to_string(),
            }),
            first: tokio::sync::watch::channel(FirstPoll::Finished).0,
            refresh: Mutex::new(None),
            read_through: Mutex::new(None),
            track: Mutex::new(None),
            poll: Mutex::new(None),
            counts: Counts::default(),
        })
    }

    pub fn set_status(&self, status: PollerStatus) {
        *lock(&self.status) = status;
    }

    pub fn set_first_poll(&self, first: FirstPoll) {
        self.first.send_replace(first);
    }

    pub fn on_refresh(&self, f: impl FnMut(&NodeId) -> Refreshed + Send + 'static) {
        *lock(&self.refresh) = Some(Box::new(f));
    }

    /// The test's closure fills the real store through `reserve` and `apply`.
    pub fn on_read_through(
        &self,
        f: impl FnMut(&PrName) -> Result<NodeId, Refusal> + Send + 'static,
    ) {
        *lock(&self.read_through) = Some(Box::new(f));
    }

    pub fn on_track(&self, f: impl FnMut(&PrName) -> Result<NodeId, Refusal> + Send + 'static) {
        *lock(&self.track) = Some(Box::new(f));
    }

    pub fn on_poll(&self, f: impl FnMut() -> PollReport + Send + 'static) {
        *lock(&self.poll) = Some(Box::new(f));
    }

    // ---- counters, one per call ----

    pub fn client_requests(&self) -> usize {
        self.counts.client_requests.load(Ordering::SeqCst)
    }

    pub fn read_throughs(&self) -> usize {
        self.counts.read_throughs.load(Ordering::SeqCst)
    }

    pub fn tracks(&self) -> usize {
        self.counts.tracks.load(Ordering::SeqCst)
    }

    pub fn refreshes(&self) -> usize {
        self.counts.refreshes.load(Ordering::SeqCst)
    }

    pub fn polls(&self) -> usize {
        self.counts.polls.load(Ordering::SeqCst)
    }

    pub fn nudges(&self) -> usize {
        self.counts.nudges.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl PollerHandle for FakePoller {
    fn client_request(&self) {
        self.counts.client_requests.fetch_add(1, Ordering::SeqCst);
    }

    fn idle_since(&self) -> Mono {
        Mono::ZERO
    }

    async fn read_through(&self, name: &PrName) -> Result<NodeId, Refusal> {
        self.counts.read_throughs.fetch_add(1, Ordering::SeqCst);
        match lock(&self.read_through).as_mut() {
            Some(f) => f(name),
            None => Err(unavailable()),
        }
    }

    async fn track(&self, name: &PrName) -> Result<NodeId, Refusal> {
        self.counts.tracks.fetch_add(1, Ordering::SeqCst);
        match lock(&self.track).as_mut() {
            Some(f) => f(name),
            None => Err(unavailable()),
        }
    }

    async fn refresh_pr(&self, id: &NodeId) -> Refreshed {
        self.counts.refreshes.fetch_add(1, Ordering::SeqCst);
        match lock(&self.refresh).as_mut() {
            Some(f) => f(id),
            None => Refreshed::NotSent,
        }
    }

    async fn poll_now(&self) -> PollReport {
        self.counts.polls.fetch_add(1, Ordering::SeqCst);
        match lock(&self.poll).as_mut() {
            Some(f) => f(),
            None => PollReport {
                outcome: PollOutcome::Polled,
                cost: 0,
            },
        }
    }

    fn nudge(&self) {
        self.counts.nudges.fetch_add(1, Ordering::SeqCst);
    }

    fn first_poll(&self) -> tokio::sync::watch::Receiver<FirstPoll> {
        self.first.subscribe()
    }

    fn status(&self) -> PollerStatus {
        lock(&self.status).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_poller_defaults_and_counts() {
        let poller = FakePoller::new();
        let handle: Arc<dyn PollerHandle> = poller.clone();
        let name = PrName::parse("o/r#1").unwrap();
        let id = NodeId("A".into());

        let status = handle.status();
        assert_eq!(status.tier, Tier::Active);
        assert!(status.problem.is_none());
        assert_eq!(*handle.first_poll().borrow(), FirstPoll::Finished);
        assert_eq!(handle.refresh_pr(&id).await, Refreshed::NotSent);
        for r in [handle.read_through(&name).await, handle.track(&name).await] {
            assert_eq!(r.unwrap_err().code, RefusalCode::GithubUnavailable);
        }
        assert_eq!(
            handle.poll_now().await,
            PollReport {
                outcome: PollOutcome::Polled,
                cost: 0
            }
        );
        handle.client_request();
        handle.client_request();
        handle.nudge();
        assert_eq!(handle.idle_since(), Mono::ZERO);
        assert_eq!(
            (
                poller.client_requests(),
                poller.read_throughs(),
                poller.tracks(),
                poller.refreshes(),
                poller.polls(),
                poller.nudges()
            ),
            (2, 1, 1, 1, 1, 1)
        );

        // Each closure runs.
        poller.on_refresh(|_| Refreshed::Applied);
        poller.on_read_through(|n| Ok(NodeId(format!("RT {n}"))));
        poller.on_track(|n| Ok(NodeId(format!("TR {n}"))));
        poller.on_poll(|| PollReport {
            outcome: PollOutcome::Auth,
            cost: 4,
        });
        assert_eq!(handle.refresh_pr(&id).await, Refreshed::Applied);
        assert_eq!(handle.read_through(&name).await.unwrap().0, "RT o/r#1");
        assert_eq!(handle.track(&name).await.unwrap().0, "TR o/r#1");
        assert_eq!(handle.poll_now().await.cost, 4);
        assert_eq!(
            (
                poller.read_throughs(),
                poller.tracks(),
                poller.refreshes(),
                poller.polls()
            ),
            (2, 2, 2, 2)
        );

        // The first poll and the status can be set, and a receiver sees it.
        let mut rx = handle.first_poll();
        poller.set_first_poll(FirstPoll::Running);
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), FirstPoll::Running);
        let mut s = handle.status();
        s.tier = Tier::Idle;
        poller.set_status(s);
        assert_eq!(handle.status().tier, Tier::Idle);
    }
}
