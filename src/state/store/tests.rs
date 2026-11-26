//! The store's tests (T15): on the test clock, filled through `reserve` and `apply` with
//! `github::fake::answers`.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::clock::{Clock, TestClock};
use crate::config::Config;
use crate::github::fake::answers::pr_read;
use crate::log::{BufferSink, Level, Logger};
use crate::model::{Event, EventKind, ItemKind, ItemRead, ItemsRead, ReadState, Seen, SeenKind};
use crate::state::{Cursor, Ended, FileEntry, FileSource, MAX_EVENTS};
use crate::testkit::MARKER_TITLE;

const MIN: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3_600);
const DAY: Duration = Duration::from_secs(86_400);
const SEC: Duration = Duration::from_secs(1);

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

struct Fix {
    clock: Arc<TestClock>,
    store: Arc<Store>,
    sink: BufferSink,
    seq: u64,
}

fn fix() -> Fix {
    fix_with(Config::default(), FileContent::default(), 1)
}

fn fix_with(config: Config, loaded: FileContent, seed: u64) -> Fix {
    let clock = TestClock::new(t0());
    let shared: SharedClock = clock.clone();
    let sink = BufferSink::default();
    let logger = Logger::new(shared.clone(), Arc::new(sink.clone()), Level::Info);
    let store = Store::new(
        StoreDeps {
            clock: shared,
            logger,
            config: Arc::new(config),
            seed,
        },
        loaded,
    );
    Fix {
        clock,
        store,
        sink,
        seq: 0,
    }
}

fn id(name: &str) -> NodeId {
    pr_read(name).id
}

fn name(s: &str) -> PrName {
    PrName::parse(s).unwrap()
}

fn merged(n: &str) -> PrRead {
    let mut r = pr_read(n);
    r.state = ReadState::Merged;
    r
}

fn closed(n: &str) -> PrRead {
    let mut r = pr_read(n);
    r.state = ReadState::Closed;
    r
}

fn no_items() -> ItemsRead {
    ItemsRead::default()
}

fn comment(id: u64, at: Time) -> ItemRead {
    ItemRead {
        kind: ItemKind::Comment,
        id,
        author: Some("someone".into()),
        at: Some(at),
        review_state: None,
        inline: None,
    }
}

impl Fix {
    fn meta(&mut self) -> ReadMeta {
        self.seq += 1;
        ReadMeta {
            seq: self.seq,
            sent_wall: self.clock.wall(),
            sent_mono: self.clock.mono(),
            github_date: None,
        }
    }

    /// Applies `read` with no items, read with login `someone`.
    fn put(&mut self, read: &PrRead) -> Applied {
        self.put_items(read, &no_items())
    }

    fn put_items(&mut self, read: &PrRead, items: &ItemsRead) -> Applied {
        let meta = self.meta();
        self.store.apply(
            &read.id,
            read,
            ItemsOutcome::Read(items),
            &meta,
            Some("someone"),
        )
    }

    /// Reserves and applies `read` as `source`.
    fn track(&mut self, read: &PrRead, source: Source) -> NodeId {
        self.store.reserve(&read.id, &read.pr, source).unwrap();
        assert!(matches!(self.put(read), Applied::Changed { .. }));
        read.id.clone()
    }

    fn has(&self, n: &str) -> bool {
        self.store.view().find(&name(n)).is_some()
    }

    fn last_event(&self) -> Event {
        self.store.view().events.last().cloned().unwrap()
    }

    fn record(&self, n: &str) -> Record {
        (*self.store.view().find(&name(n)).unwrap().record).clone()
    }

    fn timing(&self, n: &str) -> Timing {
        self.store.view().find(&name(n)).unwrap().timing.clone()
    }
}

fn saved(n: &str, source: Source, ended: Option<Ended>) -> FileEntry {
    FileEntry {
        pr: name(n),
        id: id(n),
        source,
        login: Some("someone".into()),
        ended,
        seen: Seen::default(),
    }
}

// STA-1: a PR tracked for two reasons keeps the stronger, and takes no new place.
#[tokio::test]
async fn sta1_stronger_source_kept() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    let free = f.store.places_free();
    assert_eq!(
        f.store
            .reserve(&a, &name("o/r#1"), Source::Asked)
            .map_err(|_| ()),
        Ok(())
    );
    assert_eq!(
        f.store.lookup(&name("o/r#1")).unwrap().source,
        Source::Manual
    );
    assert_eq!(f.store.places_free(), free);
    // And an `asked` one becomes `authored` when found.
    let b = f.track(&pr_read("o/r#2"), Source::Asked);
    f.store
        .reserve(&b, &name("o/r#2"), Source::Authored)
        .unwrap();
    assert_eq!(f.record("o/r#2").source, Source::Authored);
}

// STA-1: an `asked` PR leaves 60 minutes of wall time after the last request naming it ended.
#[tokio::test]
async fn sta1_asked_leaves_after_60_min() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Asked);
    let guard = f.store.name(&a);
    f.clock.advance(2 * HOUR).await;
    f.store.sweep();
    assert!(f.has("o/r#1"), "not while a guard is open");
    drop(guard);
    f.clock.advance(59 * MIN).await;
    f.store.sweep();
    assert!(f.has("o/r#1"));
    f.clock.advance(MIN).await;
    f.store.sweep();
    assert!(!f.has("o/r#1"));
    assert_eq!(
        f.last_event().kinds,
        vec![EventKind::Untracked {
            source: Source::Asked
        }]
    );

    // Not with a waiter.
    let b = f.track(&pr_read("o/r#2"), Source::Asked);
    let waiter = f.store.add_waiter(&b).unwrap();
    f.clock.advance(2 * HOUR).await;
    f.store.sweep();
    assert!(f.has("o/r#2"));
    drop(waiter);
    assert!(
        !f.has("o/r#2"),
        "the waiter's drop carries out the due drop"
    );

    // One never named leaves 60 minutes after it became `asked`.
    f.track(&pr_read("o/r#3"), Source::Asked);
    f.clock.advance(60 * MIN - SEC).await;
    f.store.sweep();
    assert!(f.has("o/r#3"));
    f.clock.advance(SEC).await;
    f.store.sweep();
    assert!(!f.has("o/r#3"));
}

// STA-1: a drop that fell due waits for the last waiter.
#[tokio::test]
async fn sta1_drop_waits_for_the_last_waiter() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    f.put(&merged("o/r#1"));
    let w1 = f.store.add_waiter(&a).unwrap();
    let w2 = f.store.add_waiter(&a).unwrap();
    f.clock.advance(DAY + HOUR).await;
    f.store.sweep();
    assert!(f.has("o/r#1"));
    drop(w1);
    assert!(f.has("o/r#1"), "one waiter is left");
    let rev = f.store.view().rev;
    drop(w2);
    assert!(!f.has("o/r#1"));
    let e = f.last_event();
    assert_eq!(e.rev, rev + 1);
    assert_eq!(
        e.kinds,
        vec![EventKind::Untracked {
            source: Source::Manual
        }]
    );
}

// STA-2, U7: `keep_done` by the wall clock from the read that first showed the end.
#[tokio::test]
async fn sta2_keep_done() {
    let mut f = fix();
    f.track(&pr_read("o/r#1"), Source::Manual);
    f.put(&merged("o/r#1"));
    f.clock.advance(DAY - SEC).await;
    f.store.sweep();
    assert!(f.has("o/r#1"));
    f.clock.advance(SEC).await;
    f.store.sweep();
    assert!(!f.has("o/r#1"));

    // Reopened, its clock is cleared.
    f.track(&pr_read("o/r#2"), Source::Manual);
    f.put(&merged("o/r#2"));
    f.clock.advance(HOUR).await;
    f.put(&pr_read("o/r#2"));
    f.clock.advance(DAY).await;
    f.store.sweep();
    assert!(f.has("o/r#2"));

    // Closed then merged: it starts again.
    f.track(&pr_read("o/r#3"), Source::Manual);
    f.put(&closed("o/r#3"));
    f.clock.advance(12 * HOUR).await;
    f.put(&merged("o/r#3"));
    f.clock.advance(12 * HOUR).await;
    f.store.sweep();
    assert!(f.has("o/r#3"));
    f.clock.advance(12 * HOUR).await;
    f.store.sweep();
    assert!(!f.has("o/r#3"));

    // A wall jump of 8 days drops every ended PR at one sweep, in the order of `pr`.
    let mut f = fix();
    for n in ["o/r#10", "o/r#9", "o/r#11"] {
        f.track(&pr_read(n), Source::Manual);
        f.put(&merged(n));
    }
    f.track(&pr_read("o/r#12"), Source::Manual);
    f.clock.sleep_wall_only(8 * DAY).await;
    let before = f.store.view().rev;
    f.store.sweep();
    let v = f.store.view();
    let dropped: Vec<(u64, String)> = v
        .events_after(before, None)
        .iter()
        .map(|e| (e.rev, e.pr.to_string()))
        .collect();
    assert_eq!(
        dropped,
        vec![
            (before + 1, "o/r#9".to_string()),
            (before + 2, "o/r#10".to_string()),
            (before + 3, "o/r#11".to_string())
        ]
    );
    assert!(f.has("o/r#12"), "an open PR stays");

    // The wall clock set back 1 hour drops nothing early.
    let mut f = fix();
    f.track(&pr_read("o/r#1"), Source::Manual);
    f.put(&merged("o/r#1"));
    f.clock.set_wall_back(HOUR).await;
    f.clock.advance(DAY).await;
    f.store.sweep();
    assert!(f.has("o/r#1"));
    f.clock.advance(HOUR).await;
    f.store.sweep();
    assert!(!f.has("o/r#1"));

    // With `keep_done` 0 the merged record is in the view after `apply`, and leaves at the next sweep.
    let mut f = fix_with(
        Config {
            keep_done: Duration::ZERO,
            ..Config::default()
        },
        FileContent::default(),
        1,
    );
    f.track(&pr_read("o/r#1"), Source::Manual);
    assert!(matches!(f.put(&merged("o/r#1")), Applied::Changed { .. }));
    assert_eq!(f.record("o/r#1").state, PrState::Merged);
    f.store.sweep();
    assert!(!f.has("o/r#1"));
}

// STA-3: 200 places, held by every class; a tracked ID is never refused.
#[tokio::test]
async fn sta3_places() {
    let mut f = fix();
    for i in 1..=crate::state::MAX_PLACES {
        let r = pr_read(&format!("o/r#{i}"));
        f.store.reserve(&r.id, &r.pr, Source::Authored).unwrap();
    }
    assert_eq!(f.store.places_free(), 0);
    let extra = pr_read("o/r#999");
    assert!(
        f.store
            .reserve(&extra.id, &extra.pr, Source::Asked)
            .is_err()
    );
    assert!(
        f.store
            .reserve(&id("o/r#1"), &name("o/r#1"), Source::Manual)
            .is_ok()
    );
    // `release` frees a `First` place.
    f.store.release(&id("o/r#2"));
    assert_eq!(f.store.places_free(), 1);
    assert!(!f.store.is_tracked(&id("o/r#2")));
    // And nothing else: an applied one keeps its place.
    f.put(&pr_read("o/r#3"));
    f.store.release(&id("o/r#3"));
    assert!(f.store.is_tracked(&id("o/r#3")));
    assert_eq!(f.store.places_free(), 1);
}

// STA-4: a read is applied only to a PR with a place; a visible change takes one revision.
#[tokio::test]
async fn sta4_apply() {
    let mut f = fix();
    let mut changed = f.store.changed();
    let stray = pr_read("o/r#5");
    assert_eq!(f.put(&stray), Applied::Discarded);
    assert!(f.store.view().events.is_empty());

    let r = pr_read("o/r#1");
    f.track(&r, Source::Authored);
    assert!(changed.has_changed().unwrap());
    changed.borrow_and_update();
    let n = f.store.view().rev;
    assert_eq!(f.put(&r), Applied::Unchanged);
    assert!(changed.has_changed().unwrap());
    changed.borrow_and_update();
    assert_eq!(f.put(&r), Applied::Unchanged);
    assert!(changed.has_changed().unwrap());
    changed.borrow_and_update();
    assert_eq!(f.record("o/r#1").rev, n);
    assert_eq!(f.store.view().rev, n);

    let mut pushed = r.clone();
    pushed.head_sha = "fedcba9876543210fedcba9876543210fedcba98".into();
    assert_eq!(f.put(&pushed), Applied::Changed { rev: n + 1 });
    assert_eq!(f.store.view().events_after(n, None).len(), 1);
    assert_eq!(f.record("o/r#1").rev, n + 1);
    assert_eq!(f.timing("o/r#1").fetched, Some(f.clock.mono()));
}

// MOD-13: over a seeded random run, every view's counter is its newest event's, and no record's
// `rev` is above it.
#[tokio::test]
async fn mod13_one_step() {
    let mut f = fix();
    let mut rng = fastrand::Rng::with_seed(13);
    let names: Vec<String> = (1..=8).map(|i| format!("o/r#{i}")).collect();
    let mut waiters = Vec::new();
    let mut subs = Vec::new();
    let mut named = Vec::new();
    for _ in 0..1_500 {
        let n = &names[rng.usize(..names.len())];
        let read = pr_read(n);
        match rng.u8(..14) {
            0 => {
                let _ = f.store.reserve(
                    &read.id,
                    &read.pr,
                    [Source::Asked, Source::Authored, Source::Manual][rng.usize(..3)],
                );
            }
            1 | 2 => {
                let mut r = read.clone();
                r.state = [ReadState::Open, ReadState::Merged, ReadState::Closed][rng.usize(..3)];
                r.draft = rng.bool();
                f.put(&r);
            }
            3 => {
                f.store.mark_gone(&read.id, f.clock.wall());
            }
            4 => f.store.sweep(),
            5 => {
                f.store.untrack(&read.pr);
            }
            6 => {
                f.store.set_manual(&read.id);
            }
            7 => f.store.demote_unlisted(std::slice::from_ref(&read.id)),
            8 => {
                if let Ok(w) = f.store.add_waiter(&read.id) {
                    waiters.push(w);
                }
            }
            9 => {
                if !waiters.is_empty() {
                    waiters.swap_remove(rng.usize(..waiters.len()));
                }
            }
            10 => {
                if let Ok(s) = f.store.add_subscription(&read.id) {
                    subs.push(s);
                }
                if rng.bool() && !subs.is_empty() {
                    subs.swap_remove(rng.usize(..subs.len()));
                }
            }
            11 => {
                named.push(f.store.name(&read.id));
                if rng.bool() && !named.is_empty() {
                    named.swap_remove(rng.usize(..named.len()));
                }
            }
            12 => f
                .store
                .attempt_failed(&read.id, Some("NOT_FOUND"), f.clock.wall()),
            _ => {
                f.clock
                    .sleep_wall_only(Duration::from_secs(rng.u64(..3 * 86_400)))
                    .await
            }
        }
        let v = f.store.view();
        assert_eq!(v.rev, v.events.last().map_or(0, |e| e.rev));
        for (a, b) in v.events.iter().zip(v.events.iter().skip(1)) {
            assert_eq!(a.rev + 1, b.rev, "every revision has its event");
        }
        assert!(v.events.len() <= MAX_EVENTS);
        for p in &v.prs {
            assert!(p.record.rev <= v.rev);
        }
    }
    drop(waiters);
    drop(subs);
    let v = f.store.view();
    assert_eq!(v.waiters, 0);
    assert_eq!(v.subscriptions, 0);
}

// STA-5: `seen` is carried between reads, into the file, and from the file into the first event.
#[tokio::test]
async fn sta5_seen_is_carried() {
    let mut f = fix();
    let r = pr_read("o/r#1");
    f.store.reserve(&r.id, &r.pr, Source::Manual).unwrap();
    let items = ItemsRead {
        comments: vec![comment(7, t0())],
        reviews: Vec::new(),
    };
    f.put_items(&r, &items);
    let content = f.store.file_content();
    assert_eq!(
        content.prs[0].seen.comment,
        SeenKind {
            at: Some(t0()),
            ids: vec![7]
        }
    );

    // A loaded entry's `seen` makes only later items `activity` in its first event.
    let mut entry = saved("o/r#1", Source::Manual, None);
    entry.seen.comment = SeenKind {
        at: Some(t0()),
        ids: vec![7],
    };
    let mut f = fix_with(Config::default(), FileContent { prs: vec![entry] }, 1);
    let mut r = pr_read("o/r#1");
    r.counts.comments = 2;
    let items = ItemsRead {
        comments: vec![comment(7, t0()), comment(8, t0().plus(MIN))],
        reviews: Vec::new(),
    };
    f.put_items(&r, &items);
    let kinds = f.last_event().kinds;
    let ids: Vec<u64> = kinds
        .iter()
        .find_map(|k| match k {
            EventKind::Activity { items, .. } => Some(items.iter().map(|i| i.id).collect()),
            _ => None,
        })
        .unwrap();
    assert_eq!(ids, vec![8]);
    assert!(kinds.contains(&EventKind::Tracked {
        source: Source::Manual
    }));
}

// STA-8: a loaded PR keeps its saved entry until its first refresh is applied.
#[tokio::test]
async fn sta8_loaded_entry_unchanged() {
    let ended = Ended {
        state: PrState::Merged,
        seen_at: t0().minus(HOUR),
    };
    let loaded = FileContent {
        prs: vec![
            saved("o/r#1", Source::Manual, Some(ended.clone())),
            saved("o/r#2", Source::Authored, None),
        ],
    };
    let mut f = fix_with(Config::default(), loaded.clone(), 1);
    assert_eq!(f.store.file_content(), loaded);
    assert_eq!(f.store.login_of(&id("o/r#2")).as_deref(), Some("someone"));
    f.store
        .mark_sent(&[id("o/r#1"), id("o/r#2")], f.clock.mono());
    f.store
        .attempt_failed(&id("o/r#1"), Some("NOT_FOUND"), f.clock.wall());
    f.store.attempt_failed(&id("o/r#2"), None, f.clock.wall());
    f.clock.advance(MIN).await;
    f.store.sweep();
    assert_eq!(f.store.file_content(), loaded);
    assert_eq!(f.store.view().pending, vec![name("o/r#1"), name("o/r#2")]);

    // Its first refresh keeps the saved source, and a confirmed `merged` the saved time.
    f.put(&merged("o/r#1"));
    assert_eq!(f.record("o/r#1").source, Source::Manual);
    let content = f.store.file_content();
    assert_eq!(content.prs[0].ended, Some(ended));
    assert_eq!(content.prs[0].source, Source::Manual);
    // ... so it leaves 24 hours from the saved time.
    f.clock.advance(DAY - HOUR - MIN).await;
    f.store.sweep();
    assert!(!f.has("o/r#1"));
}

// STA-8: entries past their `keep_done` are dropped on loading, without an event.
#[tokio::test]
async fn sta8_expired_dropped_on_load() {
    let loaded = FileContent {
        prs: vec![
            saved(
                "o/r#1",
                Source::Manual,
                Some(Ended {
                    state: PrState::Closed,
                    seen_at: t0().minus(DAY),
                }),
            ),
            saved("o/r#2", Source::Manual, None),
        ],
    };
    let f = fix_with(Config::default(), loaded, 1);
    let v = f.store.view();
    assert!(v.events.is_empty());
    assert_eq!(v.cursor().to_string(), format!("{}-0", v.epoch));
    assert!(!f.store.is_tracked(&id("o/r#1")));
    assert!(f.store.is_tracked(&id("o/r#2")));
}

// POL-5: a loaded `authored` entry outside `owners` becomes `asked`, leaves the file, starts its hour.
#[tokio::test]
async fn pol5_loaded_outside_owners() {
    let loaded = FileContent {
        prs: vec![
            saved("o/r#1", Source::Authored, None),
            saved("mine/r#2", Source::Authored, None),
        ],
    };
    let config = Config {
        owners: vec!["MINE".into()],
        ..Config::default()
    };
    let f = fix_with(config, loaded, 1);
    assert_eq!(
        f.store.lookup(&name("o/r#1")).unwrap().source,
        Source::Asked
    );
    let content = f.store.file_content();
    assert_eq!(content.prs.len(), 1);
    assert_eq!(content.prs[0].pr, name("mine/r#2"));
    f.clock.advance(HOUR).await;
    f.store.sweep();
    assert!(!f.store.is_tracked(&id("o/r#1")));
    assert!(f.store.is_tracked(&id("mine/r#2")));
}

// STA-9: the epoch's form, from the seed; too old for another epoch, above the counter, past history.
#[tokio::test]
async fn sta9_cursor() {
    let a = fix_with(Config::default(), FileContent::default(), 7);
    let epoch = a.store.view().epoch;
    assert_eq!(epoch.len(), 8);
    assert!(
        epoch
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    );
    let same = fix_with(Config::default(), FileContent::default(), 7);
    assert_eq!(same.store.view().epoch, epoch);
    let other = fix_with(Config::default(), FileContent::default(), 8);
    assert_ne!(other.store.view().epoch, epoch);
    let parsed: Cursor = a.store.view().cursor().to_string().parse().unwrap();
    assert_eq!(parsed, a.store.view().cursor());

    let mut f = fix();
    let r = pr_read("o/r#1");
    f.track(&r, Source::Manual);
    let v = f.store.view();
    assert!(v.too_old(&Cursor {
        epoch: other.store.view().epoch,
        rev: 1
    }));
    assert!(v.too_old(&Cursor {
        epoch: v.epoch.clone(),
        rev: v.rev + 1
    }));
    // 501 events: `rev` 0 is too old, `rev` 1 isn't.
    for i in 0..500 {
        let mut d = r.clone();
        d.draft = i % 2 == 0;
        assert!(matches!(f.put(&d), Applied::Changed { .. }));
    }
    let v = f.store.view();
    assert_eq!(v.rev, 501);
    let at = |rev| Cursor {
        epoch: v.epoch.clone(),
        rev,
    };
    assert!(v.too_old(&at(0)));
    assert!(!v.too_old(&at(1)));
}

// STA-10: the last 500 events, across all PRs.
#[tokio::test]
async fn sta10_500_events() {
    let mut f = fix();
    let a = pr_read("o/r#1");
    let b = pr_read("o/r#2");
    f.track(&a, Source::Manual);
    f.track(&b, Source::Manual);
    for i in 0..600 {
        let mut d = if i % 2 == 0 { a.clone() } else { b.clone() };
        d.draft = (i / 2) % 2 == 0;
        f.put(&d);
    }
    let v = f.store.view();
    assert_eq!(v.rev, 602);
    assert_eq!(v.events.len(), MAX_EVENTS);
    assert_eq!(v.events[0].rev, 103);
    assert_eq!(v.events.last().unwrap().rev, 602);
}

// API-14: at most 64 waiters in all; a dropped guard frees one.
#[tokio::test]
async fn api14_64_waiters() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    let b = f.track(&pr_read("o/r#2"), Source::Manual);
    let mut held: Vec<Waiting> = (0..32).map(|_| f.store.add_waiter(&a).unwrap()).collect();
    held.extend((0..32).map(|_| f.store.add_waiter(&b).unwrap()));
    assert!(f.store.add_waiter(&a).is_err());
    assert_eq!(f.store.view().waiters, 64);
    assert_eq!(f.timing("o/r#1").waiters, 32);
    held.pop();
    assert!(f.store.add_waiter(&a).is_ok());
}

// OPS-8: when the last waiter ended, on the monotonic clock.
#[tokio::test]
async fn ops8_last_waiter_ended() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    assert_eq!(f.store.last_waiter_ended(), None);
    let w1 = f.store.add_waiter(&a).unwrap();
    let w2 = f.store.add_waiter(&a).unwrap();
    f.clock.advance(5 * SEC).await;
    drop(w1);
    assert_eq!(f.store.last_waiter_ended(), Some(Mono::ZERO.plus(5 * SEC)));
    f.clock.advance(5 * SEC).await;
    drop(w2);
    assert_eq!(f.store.last_waiter_ended(), Some(Mono::ZERO.plus(10 * SEC)));
}

// NTF-4: a PR with a subscription stays tracked; its end counts as the end of a naming request.
#[tokio::test]
async fn ntf4_a_subscription_keeps_the_pr() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    f.put(&merged("o/r#1"));
    let b = f.track(&pr_read("o/r#2"), Source::Asked);
    let sa = f.store.add_subscription(&a).unwrap();
    let sb = f.store.add_subscription(&b).unwrap();
    f.clock.advance(DAY + HOUR).await;
    f.store.sweep();
    assert!(f.has("o/r#1"), "an ended PR past `keep_done`");
    assert!(f.has("o/r#2"), "an asked PR past its hour");
    drop(sa);
    drop(sb);
    f.clock.advance(59 * MIN).await;
    f.store.sweep();
    assert!(f.has("o/r#1"));
    assert!(f.has("o/r#2"));
    let before = f.store.view().rev;
    f.clock.advance(MIN).await;
    f.store.sweep();
    assert!(!f.has("o/r#1"));
    assert!(!f.has("o/r#2"));
    let v = f.store.view();
    let kinds: Vec<_> = v
        .events_after(before, None)
        .iter()
        .map(|e| e.kinds.clone())
        .collect();
    assert_eq!(
        kinds,
        vec![
            vec![EventKind::Untracked {
                source: Source::Manual
            }],
            vec![EventKind::Untracked {
                source: Source::Asked
            }]
        ]
    );

    // `untrack` of a `manual` PR with a guard makes it `asked`.
    let mut theirs = pr_read("x/r#3");
    theirs.author = Some("other".into());
    let c = f.track(&theirs, Source::Manual);
    let _s = f.store.add_subscription(&c).unwrap();
    let out = f.store.untrack(&name("x/r#3"));
    assert!(out.file_changed);
    assert_eq!(
        f.store.lookup(&name("x/r#3")).unwrap().source,
        Source::Asked
    );
}

// NTF-2: at most 64 subscriptions, apart from the waiters; none is in the file.
#[tokio::test]
async fn ntf2_64_subscriptions() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    let b = f.track(&pr_read("o/r#2"), Source::Manual);
    let content = f.store.file_content();
    let mut file_rx = f.store.file_changed();
    assert_eq!(f.store.last_subscription_ended(), None);
    let _waiters: Vec<Waiting> = (0..64).map(|_| f.store.add_waiter(&a).unwrap()).collect();
    let mut subs: Vec<Subscribed> = (0..40)
        .map(|_| f.store.add_subscription(&a).unwrap())
        .collect();
    subs.extend((0..24).map(|_| f.store.add_subscription(&b).unwrap()));
    assert!(f.store.add_subscription(&a).is_err());
    let v = f.store.view();
    assert_eq!(f.store.subscriptions(), 64);
    assert_eq!(v.subscriptions, 64);
    assert_eq!(
        v.prs.iter().map(|p| p.timing.subscriptions).sum::<u32>(),
        64
    );
    assert_eq!(f.timing("o/r#1").subscriptions, 40);
    f.clock.advance(3 * SEC).await;
    subs.pop();
    assert_eq!(
        f.store.last_subscription_ended(),
        Some(Mono::ZERO.plus(3 * SEC))
    );
    assert!(f.store.add_subscription(&b).is_ok());
    assert_eq!(f.store.file_content(), content);
    assert!(!file_rx.has_changed().unwrap());
    let _ = file_rx.borrow_and_update();
}

// NTF-5: only subscriptions on open PRs count for the tier.
#[tokio::test]
async fn ntf5_open_subscriptions() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    let b = f.track(&pr_read("o/r#2"), Source::Manual);
    f.put(&merged("o/r#2"));
    let _sa = f.store.add_subscription(&a).unwrap();
    let _sb = f.store.add_subscription(&b).unwrap();
    assert_eq!(f.store.open_subscriptions(), 1);
    assert_eq!(f.store.subscriptions(), 2);
    f.put(&merged("o/r#1"));
    assert_eq!(f.store.open_subscriptions(), 0);
    assert_eq!(f.store.subscriptions(), 2);
    let c = f.track(&pr_read("o/r#3"), Source::Manual);
    let sc = f.store.add_subscription(&c).unwrap();
    assert_eq!(f.store.open_subscriptions(), 1);
    assert_eq!(f.store.subscriptions(), 3);
    drop(sc);
    assert_eq!(f.store.open_subscriptions(), 0);
    assert_eq!(f.store.subscriptions(), 2);
    // NTF-5: a subscription is in `Timing::subscriptions` and nowhere else.
    let t = f.timing("o/r#1");
    assert_eq!(t.named, None);
    assert_eq!(t.waiters, 0);
}

// STA-1, NTF-2: `name_quiet` counts for the hour only, never for `Timing::named`.
#[tokio::test]
async fn ntf2_name_quiet() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Asked);
    let g = f.store.name_quiet(&a);
    assert_eq!(f.timing("o/r#1").named, None);
    drop(g);
    f.clock.advance(5 * SEC).await;
    drop(f.store.name(&a));
    assert_eq!(f.timing("o/r#1").named, Some(Mono::ZERO.plus(5 * SEC)));
    f.clock.advance(30 * MIN).await;
    let g = f.store.name_quiet(&a);
    assert_eq!(f.timing("o/r#1").named, Some(Mono::ZERO.plus(5 * SEC)));
    f.clock.advance(MIN).await;
    drop(g);
    // The quiet guard's drop restarted the hour.
    f.clock.advance(59 * MIN).await;
    f.store.sweep();
    assert!(f.has("o/r#1"));
    f.clock.advance(MIN).await;
    f.store.sweep();
    assert!(!f.has("o/r#1"));
}

// API-15: a dropped PR's record has the `rev` of its `untracked` event.
#[tokio::test]
async fn api15_untracked_record_rev() {
    let mut f = fix();
    let mut r = pr_read("o/r#1");
    r.author = Some("other".into());
    f.track(&r, Source::Manual);
    let out = f.store.untrack(&name("o/r#1"));
    let rec = out.record.unwrap();
    let e = f.last_event();
    assert_eq!(rec.rev, e.rev);
    assert_eq!(
        e.kinds,
        vec![EventKind::Untracked {
            source: Source::Manual
        }]
    );
    assert!(out.file_changed);
}

// API-15: every case of `untrack`.
#[tokio::test]
async fn api15_untrack_table() {
    let mut f = fix_with(
        Config {
            owners: vec!["o".into(), "x".into()],
            ..Config::default()
        },
        FileContent {
            prs: vec![
                saved("o/r#20", Source::Manual, None),
                saved("o/r#21", Source::Manual, None),
            ],
        },
        1,
    );

    // Not tracked: nothing, `record: null`.
    let out = f.store.untrack(&name("o/r#99"));
    assert!(out.record.is_none() && !out.file_changed);

    // `authored` and `asked`: nothing changes.
    f.track(&pr_read("o/r#1"), Source::Authored);
    f.track(&pr_read("o/r#2"), Source::Asked);
    for (n, s) in [("o/r#1", Source::Authored), ("o/r#2", Source::Asked)] {
        let rev = f.store.view().rev;
        let out = f.store.untrack(&name(n));
        assert_eq!(out.record.unwrap().source, s);
        assert!(!out.file_changed);
        assert_eq!(f.store.view().rev, rev);
    }

    // `manual`, the user's own (without case), open, owner in `owners`: `authored`.
    let mut own = pr_read("O/r#3");
    own.author = Some("SomeOne".into());
    f.track(&own, Source::Manual);
    let rev = f.store.view().rev;
    let out = f.store.untrack(&name("o/r#3"));
    assert_eq!(out.record.unwrap().source, Source::Authored);
    assert!(out.file_changed);
    assert_eq!(f.store.view().rev, rev, "a source change takes no revision");

    // Owner not in `owners`: dropped.
    f.track(&pr_read("z/r#4"), Source::Manual);
    let out = f.store.untrack(&name("z/r#4"));
    assert!(out.file_changed);
    assert!(!f.store.is_tracked(&id("z/r#4")));
    assert_eq!(out.record.unwrap().rev, f.last_event().rev);

    // Not the user's own: dropped.
    let mut theirs = pr_read("o/r#5");
    theirs.author = Some("other".into());
    f.track(&theirs, Source::Manual);
    assert!(f.store.untrack(&name("o/r#5")).file_changed);
    assert!(!f.store.is_tracked(&theirs.id));

    // No login to compare with: not the user's own.
    let r6 = pr_read("o/r#6");
    f.store.reserve(&r6.id, &r6.pr, Source::Manual).unwrap();
    let meta = f.meta();
    f.store
        .apply(&r6.id, &r6, ItemsOutcome::Read(&no_items()), &meta, None);
    f.store.untrack(&name("o/r#6"));
    assert!(!f.store.is_tracked(&r6.id));

    // The user's own but merged: dropped.
    f.track(&pr_read("o/r#7"), Source::Manual);
    f.put(&merged("o/r#7"));
    f.store.untrack(&name("o/r#7"));
    assert!(!f.store.is_tracked(&id("o/r#7")));

    // With a waiter: `asked`, out of the file, and its hour starts.
    f.track(&theirs, Source::Manual);
    let w = f.store.add_waiter(&theirs.id).unwrap();
    let out = f.store.untrack(&name("o/r#5"));
    assert!(out.file_changed);
    assert_eq!(out.record.unwrap().source, Source::Asked);
    assert!(
        f.store
            .file_content()
            .prs
            .iter()
            .all(|e| e.pr != name("o/r#5"))
    );
    drop(w);
    assert!(f.store.is_tracked(&theirs.id));

    // With a subscription: `asked`.
    f.track(&pr_read("z/r#8"), Source::Manual);
    let s = f.store.add_subscription(&id("z/r#8")).unwrap();
    f.store.untrack(&name("z/r#8"));
    assert_eq!(
        f.store.lookup(&name("z/r#8")).unwrap().source,
        Source::Asked
    );
    drop(s);

    // A loaded `manual` entry with no record: dropped with `untracked`, `record: null`.
    let rev = f.store.view().rev;
    let out = f.store.untrack(&name("o/r#20"));
    assert!(out.record.is_none() && out.file_changed);
    assert!(!f.store.is_tracked(&id("o/r#20")));
    assert_eq!(f.store.view().rev, rev + 1);
    assert_eq!(f.last_event().pr, name("o/r#20"));
    // ... or `asked` while it has a waiter.
    let _w = f.store.add_waiter(&id("o/r#21")).unwrap();
    let out = f.store.untrack(&name("o/r#21"));
    assert!(out.record.is_none() && out.file_changed);
    assert_eq!(
        f.store.lookup(&name("o/r#21")).unwrap().source,
        Source::Asked
    );

    // Idempotent: a second `untrack` of an `authored` PR changes nothing.
    assert!(!f.store.untrack(&name("o/r#3")).file_changed);
}

// POL-5: `demote_unlisted` changes only open `authored` PRs in its slice.
#[tokio::test]
async fn pol5_demote_unlisted() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Authored);
    let m = f.track(&pr_read("o/r#2"), Source::Manual);
    let d = f.track(&pr_read("o/r#3"), Source::Authored);
    f.put(&merged("o/r#3"));
    let kept = f.track(&pr_read("o/r#4"), Source::Authored);
    let mut changed = f.store.changed();
    let _ = changed.borrow_and_update();
    f.store.demote_unlisted(&[a.clone(), m, d]);
    assert!(changed.has_changed().unwrap());
    assert_eq!(f.record("o/r#1").source, Source::Asked);
    assert_eq!(f.record("o/r#2").source, Source::Manual);
    assert_eq!(f.record("o/r#3").source, Source::Authored);
    assert_eq!(f.record("o/r#4").source, Source::Authored);
    let content = f.store.file_content();
    assert!(content.prs.iter().all(|e| e.id != a));
    assert!(content.prs.iter().any(|e| e.id == kept));
    f.clock.advance(HOUR).await;
    f.store.sweep();
    assert!(!f.store.is_tracked(&a));
    assert!(f.store.is_tracked(&kept));
}

// STA-2, POL-21: `mark_gone` starts the retention clock at `seen_at`.
#[tokio::test]
async fn gone_starts_retention() {
    let mut f = fix();
    let a = f.track(&pr_read("o/r#1"), Source::Manual);
    f.clock.advance(HOUR).await;
    let seen = t0().plus(30 * MIN);
    let rev = f.store.view().rev;
    assert_eq!(
        f.store.mark_gone(&a, seen),
        Applied::Changed { rev: rev + 1 }
    );
    assert!(f.last_event().kinds.contains(&EventKind::Gone));
    assert_eq!(f.record("o/r#1").state, PrState::Gone);
    assert_eq!(f.record("o/r#1").rev, rev + 1);
    assert_eq!(
        f.store.file_content().prs[0].ended,
        Some(Ended {
            state: PrState::Gone,
            seen_at: seen
        })
    );
    // Not open any more: nothing.
    assert_eq!(f.store.mark_gone(&a, seen), Applied::Unchanged);
    f.clock.advance(DAY - 30 * MIN - SEC).await;
    f.store.sweep();
    assert!(f.has("o/r#1"));
    f.clock.advance(SEC).await;
    f.store.sweep();
    assert!(!f.has("o/r#1"));
}

// STA-8: `mark_gone` on a loaded entry saved open changes the saved entry only.
#[tokio::test]
async fn gone_on_a_loaded_entry() {
    let f = fix_with(
        Config::default(),
        FileContent {
            prs: vec![saved("o/r#1", Source::Authored, None)],
        },
        1,
    );
    let mut file_rx = f.store.file_changed();
    let seen = t0().minus(MIN);
    assert_eq!(f.store.mark_gone(&id("o/r#1"), seen), Applied::Unchanged);
    assert!(f.store.view().events.is_empty());
    assert!(file_rx.has_changed().unwrap());
    let _ = file_rx.borrow_and_update();
    assert_eq!(
        f.store.file_content().prs[0].ended,
        Some(Ended {
            state: PrState::Gone,
            seen_at: seen
        })
    );
    f.clock.advance(DAY - 2 * MIN).await;
    f.store.sweep();
    assert!(f.store.is_tracked(&id("o/r#1")));
    f.clock.advance(MIN).await;
    f.store.sweep();
    assert!(!f.store.is_tracked(&id("o/r#1")));
}

// STA-7: `file_changed` moves only when the file's content would differ.
#[tokio::test]
async fn file_changed_only_on_content() {
    let mut f = fix();
    let r = pr_read("o/r#1");
    let mut rx = f.store.file_changed();
    f.store.reserve(&r.id, &r.pr, Source::Authored).unwrap();
    assert!(!rx.has_changed().unwrap(), "a `First` PR isn't in the file");
    f.put(&r);
    assert!(rx.has_changed().unwrap());
    let _ = rx.borrow_and_update();
    let mut checks = r.clone();
    checks.checks.items.push(crate::model::CheckItem {
        kind: crate::model::CheckKind::Run,
        id: Some(1),
        name: "ci".into(),
        phase: crate::model::Phase::InProgress,
        result: None,
        url: None,
    });
    checks.checks.total = 1;
    assert!(matches!(f.put(&checks), Applied::Changed { .. }));
    assert!(!rx.has_changed().unwrap());
    f.put(&merged("o/r#1"));
    assert!(rx.has_changed().unwrap());
}

// OPS-9: tracked, untracked and gone are logged, by name; never the title.
#[tokio::test]
async fn ops9_lines() {
    let mut f = fix();
    let mut r = pr_read("o/r#1");
    r.title = MARKER_TITLE.into();
    f.track(&r, Source::Manual);
    let mut r2 = pr_read("o/r#2");
    r2.title = MARKER_TITLE.into();
    f.track(&r2, Source::Manual);
    f.store.mark_gone(&r2.id, f.clock.wall());
    let mut other = r.clone();
    other.author = Some("other".into());
    f.put(&other);
    f.store.untrack(&name("o/r#1"));
    let log = f.sink.contents();
    assert!(log.contains("info tracked pr=o/r#1 source=manual"), "{log}");
    assert!(log.contains("info gone pr=o/r#2 source=manual"), "{log}");
    assert!(
        log.contains("info untracked pr=o/r#1 source=manual"),
        "{log}"
    );
    assert!(!log.contains(MARKER_TITLE));
}

// U3: the user's own PRs and those added by hand are tracked and counted by source.
#[tokio::test]
async fn u3_view_counts_by_source() {
    let mut f = fix();
    f.track(&pr_read("o/r#1"), Source::Authored);
    f.track(&pr_read("o/r#2"), Source::Manual);
    f.track(&pr_read("o/r#3"), Source::Asked);
    let r = pr_read("o/r#4");
    f.store.reserve(&r.id, &r.pr, Source::Authored).unwrap();
    let v = f.store.view();
    assert_eq!(v.tracked, [2, 1, 1]);
    assert_eq!(v.prs.len(), 3);
    assert_eq!(v.pending, vec![name("o/r#4")]);
    let targets = f.store.targets();
    assert_eq!(targets.len(), 4);
    assert_eq!(targets[3].class, Class::First);
    assert_eq!(targets[0].class, Class::Open);
}
