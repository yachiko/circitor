//! The engine's tests: `ScriptedGitHub` answering from a small world, the real `Store`, `TestClock`,
//! `FakeTokenSource`, a `BufferSink`, OPS-2's defaults.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{Engine, EngineDeps};
use crate::clock::{Clock, Mono, SharedClock, SleepWatch, TestClock, Time};
use crate::config::Config;
use crate::github::fake::{ScriptedGitHub, answers};
use crate::github::{
    Answer, Failure, Found, FoundPr, LookupAnswer, NodeAnswer, NodeQuery, Query, Rate, Unavailable,
    Want,
};
use crate::log::{BufferSink, Level, Logger};
use crate::model::{
    CheckItem, CheckKind, ChecksRead, Event, ItemsRead, NodeId, Phase, PrName, PrRead, ReadState,
    Record,
};
use crate::poller::PollerHandle;
use crate::state::{FileContent, Store, StoreDeps, Timing};
use crate::status::Warnings;
use crate::token::fake::FakeTokenSource;

mod api;
mod cadence;
mod demand;
mod errors;
mod poll;

pub(super) fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

pub(super) fn at(n: u64) -> Mono {
    Mono::ZERO.plus(s(n))
}

pub(super) fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

pub(super) fn id(name: &str) -> NodeId {
    answers::pr_read(name).id
}

pub(super) fn name(n: &str) -> PrName {
    PrName::parse(n).unwrap()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the scripted GitHub holds.
pub(super) struct World {
    pub prs: BTreeMap<NodeId, PrRead>,
    /// Open, yet not in the open list.
    pub unlisted: HashSet<NodeId>,
    pub login: String,
    pub items: HashMap<NodeId, ItemsRead>,
    /// A PR's number of checks on its last commit.
    pub checks: HashMap<NodeId, u32>,
    /// What a `Record` or `RecordAndItems` node answers instead of the PR.
    pub errors: HashMap<NodeId, NodeAnswer>,
    /// `ChecksPage` nodes that answer an error.
    pub page_errors: HashSet<NodeId>,
    /// `Items` nodes that answer an error.
    pub item_errors: HashSet<NodeId>,
    /// One per request, from the front: `Some` fails it as a whole, `None` answers it.
    pub script: VecDeque<Option<Failure>>,
    pub rate: Option<Rate>,
    /// T25: what a lookup of this name answers instead of the PR.
    pub lookup_errors: HashMap<PrName, LookupAnswer>,
    /// T25: a lookup of this name answers this PR, under this name (a renamed repository, MOD-1).
    pub aliases: HashMap<PrName, NodeId>,
    /// T25: PRs whose checks are pending.
    pub pending: HashSet<NodeId>,
}

impl Default for World {
    fn default() -> World {
        World {
            prs: BTreeMap::new(),
            unlisted: HashSet::new(),
            login: "octo".to_string(),
            items: HashMap::new(),
            checks: HashMap::new(),
            errors: HashMap::new(),
            page_errors: HashSet::new(),
            item_errors: HashSet::new(),
            script: VecDeque::new(),
            rate: Some(answers::rate(1, 4_999)),
            lookup_errors: HashMap::new(),
            aliases: HashMap::new(),
            pending: HashSet::new(),
        }
    }
}

impl World {
    /// An open PR, updated `secs` after t0.
    pub fn open(&mut self, n: &str, secs: u64) -> NodeId {
        let mut r = answers::pr_read(n);
        r.updated_at = Some(t0().plus(s(secs)));
        let id = r.id.clone();
        self.prs.insert(id.clone(), r);
        id
    }

    pub fn pr(&mut self, n: &str) -> &mut PrRead {
        self.prs.get_mut(&id(n)).expect("a PR of the world")
    }

    pub fn set_state(&mut self, n: &str, state: ReadState) {
        self.pr(n).state = state;
    }

    pub fn fail(&mut self, f: Failure) {
        self.script.push_back(Some(f));
    }

    pub fn pass(&mut self) {
        self.script.push_back(None);
    }

    fn answer(&mut self, q: &Query) -> Result<Answer, Failure> {
        if let Some(Some(f)) = self.script.pop_front() {
            return Err(f);
        }
        let found = q.find.as_ref().map(|f| self.find(f.after.as_deref()));
        let nodes = q.nodes.iter().map(|n| self.node(n)).collect();
        let lookup = q.lookup.as_ref().map(|n| self.lookup(n));
        Ok(Answer {
            rate: self.rate.clone(),
            date: None,
            found,
            nodes,
            lookup,
        })
    }

    /// POL-16: one PR by name, as `RecordAndItems`.
    fn lookup(&self, name: &PrName) -> LookupAnswer {
        if let Some(e) = self.lookup_errors.get(name) {
            return e.clone();
        }
        let read = match self.aliases.get(name) {
            Some(id) => self.prs.get(id).map(|r| {
                let mut r = r.clone();
                r.pr = name.clone();
                r
            }),
            None => self.prs.values().find(|r| r.pr == *name).cloned(),
        };
        let Some(mut read) = read else {
            return LookupAnswer::NotFound;
        };
        read.checks = self.checks_page(&read.id, 0);
        LookupAnswer::Found {
            items: self.items.get(&read.id).cloned().unwrap_or_default(),
            read: Box::new(read),
        }
    }

    fn find(&self, after: Option<&str>) -> Found {
        let mut open: Vec<&PrRead> = self
            .prs
            .values()
            .filter(|r| r.state == ReadState::Open && !self.unlisted.contains(&r.id))
            .collect();
        open.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        let from: usize = after.map_or(0, |a| a[1..].parse().unwrap());
        let to = (from + 100).min(open.len());
        Found {
            login: self.login.clone(),
            prs: open[from..to]
                .iter()
                .map(|r| FoundPr {
                    id: r.id.clone(),
                    pr: r.pr.clone(),
                    updated_at: r.updated_at,
                })
                .collect(),
            next: (to < open.len()).then(|| format!("o{to}")),
        }
    }

    fn checks_page(&self, pr: &NodeId, from: u32) -> ChecksRead {
        let pending = self.pending.contains(pr);
        let total = self
            .checks
            .get(pr)
            .copied()
            .unwrap_or(0)
            .max(u32::from(pending));
        let to = (from + 100).min(total);
        ChecksRead {
            items: (from..to)
                .map(|i| CheckItem {
                    kind: CheckKind::Run,
                    id: Some(u64::from(i) + 1),
                    name: format!("check-{i}"),
                    phase: if pending {
                        Phase::InProgress
                    } else {
                        Phase::Completed
                    },
                    result: (!pending).then(|| "success".to_string()),
                    url: None,
                })
                .collect(),
            total,
            rollup: (total > 0).then(|| if pending { "pending" } else { "success" }.to_string()),
            next_page: (to < total).then(|| format!("k{to}")),
        }
    }

    fn node(&self, n: &NodeQuery) -> NodeAnswer {
        match &n.want {
            Want::Record | Want::RecordAndItems => {
                if let Some(e) = self.errors.get(&n.id) {
                    return e.clone();
                }
                let Some(read) = self.prs.get(&n.id) else {
                    return NodeAnswer::NotFound;
                };
                let mut read = read.clone();
                read.checks = self.checks_page(&n.id, 0);
                let items = (n.want == Want::RecordAndItems)
                    .then(|| self.items.get(&n.id).cloned().unwrap_or_default());
                NodeAnswer::Record {
                    read: Box::new(read),
                    items,
                }
            }
            Want::Items => {
                if self.item_errors.contains(&n.id) {
                    return NodeAnswer::Error {
                        type_: "INTERNAL".to_string(),
                    };
                }
                NodeAnswer::Items(self.items.get(&n.id).cloned().unwrap_or_default())
            }
            Want::ChecksPage { after } => {
                if self.page_errors.contains(&n.id) {
                    return NodeAnswer::Error {
                        type_: "INTERNAL".to_string(),
                    };
                }
                NodeAnswer::ChecksPage(self.checks_page(&n.id, after[1..].parse().unwrap()))
            }
        }
    }
}

pub(super) fn unavailable(status: u16) -> Failure {
    Failure::Unavailable {
        kind: Unavailable::Http,
        status: Some(status),
    }
}

pub(super) struct Setup {
    pub config: Config,
    pub loaded: FileContent,
    pub world: World,
    pub level: Level,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup {
            config: Config::default(),
            loaded: FileContent::default(),
            world: World::default(),
            level: Level::Debug,
        }
    }
}

pub(super) struct Rig {
    pub clock: Arc<TestClock>,
    pub github: Arc<ScriptedGitHub>,
    pub world: Arc<Mutex<World>>,
    pub token: Arc<FakeTokenSource>,
    pub store: Arc<Store>,
    pub sink: BufferSink,
    pub warnings: Warnings,
    pub sleep: Arc<SleepWatch>,
    pub engine: Arc<Engine>,
    times: Arc<Mutex<Vec<Mono>>>,
}

/// Builds everything; `before` runs on the token source before the engine starts.
pub(super) async fn start_with(setup: Setup, before: impl FnOnce(&FakeTokenSource)) -> Rig {
    let clock = TestClock::new(t0());
    let shared: SharedClock = clock.clone();
    let sink = BufferSink::default();
    let logger = Logger::new(shared.clone(), Arc::new(sink.clone()), setup.level);
    let config = Arc::new(setup.config);
    let store = Store::new(
        StoreDeps {
            clock: shared.clone(),
            logger: logger.clone(),
            config: config.clone(),
            seed: 7,
        },
        setup.loaded,
    );
    let world = Arc::new(Mutex::new(setup.world));
    let w = world.clone();
    let times = Arc::new(Mutex::new(Vec::new()));
    let (tm, c) = (times.clone(), clock.clone());
    let github = ScriptedGitHub::new(move |q| {
        lock(&tm).push(c.mono());
        lock(&w).answer(q)
    });
    let token = FakeTokenSource::new(shared.clone());
    before(&token);
    let warnings = Warnings::new(shared.clone(), logger.clone());
    let sleep = SleepWatch::new(shared.clone());
    let engine = Engine::spawn(EngineDeps {
        clock: shared,
        github: github.clone(),
        token_source: token.clone(),
        store: store.clone(),
        config,
        logger,
        warnings: warnings.clone(),
        activity: clock.activity(),
        sleep: sleep.clone(),
    });
    clock.settle().await;
    Rig {
        clock,
        github,
        world,
        token,
        store,
        sink,
        warnings,
        sleep,
        engine,
        times,
    }
}

pub(super) async fn start(setup: Setup) -> Rig {
    start_with(setup, |_| {}).await
}

/// A world of open PRs, updated in the order given (the first oldest).
pub(super) fn world_of(names: &[&str]) -> World {
    let mut w = World::default();
    for (i, n) in names.iter().enumerate() {
        w.open(n, i as u64);
    }
    w
}

impl Rig {
    pub fn world(&self) -> MutexGuard<'_, World> {
        lock(&self.world)
    }

    pub fn calls(&self) -> Vec<Query> {
        self.github.calls()
    }

    pub fn n_calls(&self) -> usize {
        self.github.calls().len()
    }

    pub async fn advance(&self, secs: u64) {
        self.clock.advance(s(secs)).await;
    }

    pub fn now(&self) -> Mono {
        self.clock.mono()
    }

    /// Lines holding `word` (event words are at the start of the message).
    pub fn lines(&self, word: &str) -> usize {
        self.sink
            .contents()
            .lines()
            .filter(|l| l.contains(word))
            .count()
    }

    pub fn record(&self, n: &str) -> Option<Record> {
        let view = self.store.view();
        view.get(&id(n)).map(|v| (*v.record).clone())
    }

    pub fn timing(&self, n: &str) -> Timing {
        let i = id(n);
        self.store
            .targets()
            .into_iter()
            .find(|t| t.id == i)
            .map(|t| t.timing)
            .expect("a tracked PR")
    }

    pub fn events(&self, n: &str) -> Vec<Event> {
        let i = id(n);
        self.store
            .view()
            .events
            .iter()
            .filter(|e| e.id == i)
            .cloned()
            .collect()
    }

    pub fn tracked(&self, n: &str) -> bool {
        self.store.is_tracked(&id(n))
    }

    /// Client requests every `every` seconds for `secs` seconds.
    pub async fn keep_active(&self, secs: u64, every: u64) {
        let mut done = 0;
        while done < secs {
            self.engine.client_request();
            let step = every.min(secs - done);
            self.advance(step).await;
            done += step;
        }
    }

    /// The send times of every request, in order.
    pub fn send_times(&self) -> Vec<Mono> {
        lock(&self.times).clone()
    }
}

/// The IDs a query names.
pub(super) fn named(q: &Query) -> Vec<NodeId> {
    q.nodes.iter().map(|n| n.id.clone()).collect()
}

pub(super) fn has_find(q: &Query) -> bool {
    q.find.is_some()
}

impl Rig {
    pub fn handle(&self) -> &dyn PollerHandle {
        self.engine.as_ref()
    }

    /// `poll_now` on a task of its own, so the test can move the clock meanwhile.
    pub fn spawn_poll_now(&self) -> tokio::task::JoinHandle<crate::poller::PollReport> {
        let engine = self.engine.clone();
        tokio::spawn(async move { engine.poll_now().await })
    }
}
