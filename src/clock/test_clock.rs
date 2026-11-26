//! The test clock (TST-4). Only in a test build.
//!
//! Timers sit in a heap ordered by due time, then by the order they were set. The clock never moves
//! by itself: only `advance`, `sleep_wall_only`, `sleep_both` and `set_wall_back` move it, and each
//! settles (waits, in real time, until the daemon under test has nothing in flight) as it goes.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;

use super::{Activity, Clock, Mono, Time};

/// How long `settle` waits, in real time, before it panics naming what is still in flight.
const SETTLE_LIMIT: Duration = Duration::from_secs(10);
/// The real-time wait between two looks at an unsettled daemon.
const SETTLE_POLL: Duration = Duration::from_millis(1);
/// Settled is this many clean looks in a row, with a yield between them.
const CLEAN_LOOKS: usize = 3;

pub trait SettleProbe: Send + Sync {
    fn settled(&self) -> bool;
}

struct Timers {
    wall: Time,
    mono: Mono,
    next_seq: u64,
    /// Set and not yet fired: by due time, then by the order they were set.
    pending: BTreeMap<(Mono, u64), Waker>,
    /// Fired and not yet polled (work in flight, TST-4): the timer's sequence number and due time.
    fired: BTreeMap<u64, Mono>,
}

struct Shared {
    timers: Mutex<Timers>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Timers> {
        // Nothing panics while holding it; if something did, the timers are still consistent.
        self.timers.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The test clock (TST-4).
pub struct TestClock {
    shared: Arc<Shared>,
    activity: Activity,
    probes: Mutex<Vec<Arc<dyn SettleProbe>>>,
    /// Set while one of the four moves runs: a second one at the same time is a test's mistake.
    moving: AtomicBool,
    settle_limit_ms: AtomicU64,
}

/// Clears `moving` when the move ends, a panic included.
struct Moving<'a>(&'a AtomicBool);

impl Drop for Moving<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl TestClock {
    /// Wall time starts at `start` (TST-5: 2026-11-24T22:00:00Z), monotonic time at zero.
    pub fn new(start: Time) -> Arc<TestClock> {
        Arc::new(TestClock {
            shared: Arc::new(Shared {
                timers: Mutex::new(Timers {
                    wall: start,
                    mono: Mono::ZERO,
                    next_seq: 0,
                    pending: BTreeMap::new(),
                    fired: BTreeMap::new(),
                }),
            }),
            activity: Activity::default(),
            probes: Mutex::new(Vec::new()),
            moving: AtomicBool::new(false),
            settle_limit_ms: AtomicU64::new(SETTLE_LIMIT.as_millis() as u64),
        })
    }

    /// The daemon under test counts its work here; the clock treats it as a probe.
    pub fn activity(&self) -> Activity {
        self.activity.clone()
    }

    /// The fake GitHub adds itself.
    pub fn add_probe(&self, probe: Arc<dyn SettleProbe>) {
        self.probes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(probe);
    }

    /// Shortens (or lengthens) how long `settle` waits before it panics. 10 s unless set.
    #[cfg(test)]
    fn set_settle_limit(&self, limit: Duration) {
        let ms = u64::try_from(limit.as_millis()).unwrap_or(u64::MAX);
        self.settle_limit_ms.store(ms, Ordering::SeqCst);
    }

    fn start_moving(&self, what: &str) -> Moving<'_> {
        if self.moving.swap(true, Ordering::SeqCst) {
            panic!("TestClock::{what} called while the clock is already moving: a test's mistake");
        }
        Moving(&self.moving)
    }

    /// Fires the earliest timer due at or before `limit`, moving both clocks forward to its due time
    /// when it's ahead. False when there is none. The waker is called after the lock is let go.
    fn fire_next(&self, limit: Mono) -> bool {
        let waker = {
            let mut t = self.shared.lock();
            let Some((&(due, seq), _)) = t.pending.first_key_value() else {
                return false;
            };
            if due > limit {
                return false;
            }
            let Some(waker) = t.pending.remove(&(due, seq)) else {
                return false;
            };
            if due > t.mono {
                let step = due.since(t.mono);
                t.mono = due;
                t.wall = t.wall.plus(step);
            }
            // TST-4: a fired timer is work in flight until its future is polled or dropped.
            t.fired.insert(seq, due);
            waker
        };
        waker.wake();
        true
    }

    /// Settles first. Then moves both clocks by `d`: fires each timer at its own due time, in order (same
    /// instant: in the order they were set), and settles after each before time moves on (TST-4).
    pub async fn advance(&self, d: Duration) {
        let _moving = self.start_moving("advance");
        // TST-4: work started just before the call sets its timers first.
        self.settle().await;
        let target = self.shared.lock().mono.plus(d);
        // Timers set while settling join the same loop.
        while self.fire_next(target) {
            self.settle().await;
        }
        {
            let mut t = self.shared.lock();
            let step = target.since(t.mono);
            t.mono = target;
            t.wall = t.wall.plus(step);
        }
        self.settle().await;
    }

    /// A suspend the monotonic clock doesn't count: wall time jumps by `d` in one step. Then settles.
    pub async fn sleep_wall_only(&self, d: Duration) {
        let _moving = self.start_moving("sleep_wall_only");
        self.settle().await;
        {
            let mut t = self.shared.lock();
            t.wall = t.wall.plus(d);
        }
        self.settle().await;
    }

    /// A suspend the monotonic clock counts: both jump by `d` in one step; every timer that fell due
    /// fires afterwards, in the order of due time. Then settles.
    pub async fn sleep_both(&self, d: Duration) {
        let _moving = self.start_moving("sleep_both");
        self.settle().await;
        let target = {
            let mut t = self.shared.lock();
            t.mono = t.mono.plus(d);
            t.wall = t.wall.plus(d);
            t.mono
        };
        // One at a time, a settle after each; the clock doesn't move while they fire.
        while self.fire_next(target) {
            self.settle().await;
        }
        self.settle().await;
    }

    /// Sets wall time back by `d`, alone. Then settles.
    pub async fn set_wall_back(&self, d: Duration) {
        let _moving = self.start_moving("set_wall_back");
        self.settle().await;
        {
            let mut t = self.shared.lock();
            t.wall = t.wall.minus(d);
        }
        self.settle().await;
    }

    /// What keeps the daemon from being settled, or `None` when it is.
    fn unsettled(&self) -> Option<String> {
        let mut why = Vec::new();
        let in_flight = self.activity.in_flight();
        if in_flight > 0 {
            let (busy, excused) = self.activity.counts();
            why.push(format!(
                "work in flight: {in_flight} ({busy} begun, {excused} excused)"
            ));
        }
        let fired: Vec<Mono> = self.shared.lock().fired.values().copied().collect();
        if !fired.is_empty() {
            let dues: Vec<String> = fired
                .iter()
                .map(|m| format!("{:?}", m.as_duration()))
                .collect();
            why.push(format!(
                "timers fired and not polled: {} (due at {})",
                fired.len(),
                dues.join(", ")
            ));
        }
        let probes: Vec<Arc<dyn SettleProbe>> = self
            .probes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let unsettled: Vec<String> = probes
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.settled())
            .map(|(i, _)| format!("#{i}"))
            .collect();
        if !unsettled.is_empty() {
            why.push(format!(
                "probes not settled: {} (of {})",
                unsettled.join(", "),
                probes.len()
            ));
        }
        if why.is_empty() {
            None
        } else {
            Some(why.join("; "))
        }
    }

    /// Waits, in real time, until settled: `Activity::in_flight() == 0`, no timer fired but not yet
    /// polled, and every probe settled, on three checks in a row with a yield between them.
    pub async fn settle(&self) {
        let limit = Duration::from_millis(self.settle_limit_ms.load(Ordering::SeqCst));
        // TST-4: real time on purpose, the fake GitHub and the sockets are real I/O.
        let began = Instant::now();
        let mut clean = 0;
        loop {
            tokio::task::yield_now().await;
            match self.unsettled() {
                None => {
                    clean += 1;
                    if clean >= CLEAN_LOOKS {
                        return;
                    }
                }
                Some(why) => {
                    clean = 0;
                    if began.elapsed() >= limit {
                        panic!("TestClock: not settled after {limit:?}: {why}");
                    }
                    tokio::time::sleep(SETTLE_POLL).await;
                }
            }
        }
    }

    /// Timers set and not yet fired.
    #[cfg(test)]
    fn pending_timers(&self) -> usize {
        self.shared.lock().pending.len()
    }
}

impl Clock for TestClock {
    fn wall(&self) -> Time {
        self.shared.lock().wall
    }

    fn mono(&self) -> Mono {
        self.shared.lock().mono
    }

    fn sleep_until(&self, at: Mono) -> BoxFuture<'static, ()> {
        Box::pin(TestSleep {
            shared: self.shared.clone(),
            at,
            state: SleepState::New,
        })
    }
}

enum SleepState {
    /// Not polled yet: not in the heap.
    New,
    /// In the heap, or fired and waiting for this poll.
    Set(u64),
    Done,
}

struct TestSleep {
    shared: Arc<Shared>,
    at: Mono,
    state: SleepState,
}

impl Future for TestSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let mut t = this.shared.lock();
        match this.state {
            SleepState::New => {
                // A time already reached is ready at the first poll and never enters the heap.
                if this.at <= t.mono {
                    this.state = SleepState::Done;
                    return Poll::Ready(());
                }
                let seq = t.next_seq;
                t.next_seq += 1;
                t.pending.insert((this.at, seq), cx.waker().clone());
                this.state = SleepState::Set(seq);
                Poll::Pending
            }
            SleepState::Set(seq) => {
                if t.fired.remove(&seq).is_some() {
                    this.state = SleepState::Done;
                    return Poll::Ready(());
                }
                // Only firing makes it ready: in `sleep_both`, timers fire one at a time.
                if let Some(w) = t.pending.get_mut(&(this.at, seq)) {
                    if !w.will_wake(cx.waker()) {
                        *w = cx.waker().clone();
                    }
                    return Poll::Pending;
                }
                // Neither set nor fired can't happen; don't hang on it.
                this.state = SleepState::Done;
                Poll::Ready(())
            }
            SleepState::Done => Poll::Ready(()),
        }
    }
}

impl Drop for TestSleep {
    fn drop(&mut self) {
        // A dropped timer leaves the heap, and a fired one stops counting as work.
        if let SleepState::Set(seq) = self.state {
            let mut t = self.shared.lock();
            t.pending.remove(&(self.at, seq));
            t.fired.remove(&seq);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Elapsed, SharedClock, timeout};
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;
    use std::sync::atomic::AtomicUsize;

    fn start() -> Time {
        Time::parse("2026-11-24T22:00:00Z").unwrap()
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn at(s: u64) -> Mono {
        Mono::ZERO.plus(secs(s))
    }

    fn panic_text(e: Box<dyn std::any::Any + Send>) -> String {
        match e.downcast::<String>() {
            Ok(s) => *s,
            Err(e) => e
                .downcast::<&str>()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        }
    }

    type Log<T> = Arc<Mutex<Vec<T>>>;

    fn push<T>(log: &Log<T>, v: T) {
        log.lock().unwrap().push(v);
    }

    // TST-4: an advance fires each timer at its own due time, in order.
    #[tokio::test]
    async fn tst4_advance_fires_timers_at_their_due_time_in_order() {
        let clock = TestClock::new(start());
        let log: Log<(u64, Mono, Time)> = Log::default();
        for s in [30, 10, 20] {
            let (c, log) = (clock.clone(), log.clone());
            tokio::spawn(async move {
                c.sleep_until(at(s)).await;
                push(&log, (s, c.mono(), c.wall()));
            });
        }
        clock.advance(secs(60)).await;
        let want: Vec<_> = [10, 20, 30]
            .into_iter()
            .map(|s| (s, at(s), start().plus(secs(s))))
            .collect();
        assert_eq!(*log.lock().unwrap(), want);
        assert_eq!(clock.mono(), at(60));
        assert_eq!(clock.wall().rfc3339(), "2026-11-24T22:01:00Z");
    }

    // TST-4: timers due at the same instant fire in the order they were set.
    #[tokio::test]
    async fn tst4_same_instant_fires_in_the_order_set() {
        let clock = TestClock::new(start());
        let log: Log<u32> = Log::default();
        for n in [1, 2, 3] {
            let (c, log) = (clock.clone(), log.clone());
            tokio::spawn(async move {
                c.sleep_until(at(10)).await;
                push(&log, n);
            });
            // Each sets its timer before the next is spawned.
            clock.settle().await;
        }
        assert_eq!(clock.pending_timers(), 3);
        clock.advance(secs(10)).await;
        assert_eq!(*log.lock().unwrap(), vec![1, 2, 3]);
    }

    // TST-4: the clock settles after each timer before time moves on.
    #[tokio::test]
    async fn tst4_settles_after_each_timer() {
        let clock = TestClock::new(start());
        let log: Log<(&'static str, Mono)> = Log::default();
        {
            let (c, log, activity) = (clock.clone(), log.clone(), clock.activity());
            tokio::spawn(async move {
                c.sleep_until(at(10)).await;
                let _busy = activity.begin();
                let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                tokio::spawn(async move {
                    for _ in 0..20 {
                        tokio::task::yield_now().await;
                    }
                    let _ = tx.send(());
                });
                let _ = rx.await;
                push(&log, ("a", c.mono()));
            });
        }
        {
            let (c, log) = (clock.clone(), log.clone());
            tokio::spawn(async move {
                c.sleep_until(at(11)).await;
                push(&log, ("c", c.mono()));
            });
        }
        clock.advance(secs(20)).await;
        assert_eq!(*log.lock().unwrap(), vec![("a", at(10)), ("c", at(11))]);
    }

    async fn ticker(clock: Arc<TestClock>, count: Arc<AtomicUsize>) {
        // Absolute due times, from the time read on waking.
        let mut next = clock.mono().plus(secs(30));
        loop {
            clock.sleep_until(next).await;
            count.fetch_add(1, Ordering::SeqCst);
            next = clock.mono().plus(secs(30));
        }
    }

    // TST-4 (TST-6's active hour): a loop rearmed every 30 s fires 120 times in an hour, once on a jump.
    #[tokio::test]
    async fn tst4_a_loop_rearmed_with_absolute_times_fires_120_times_in_an_hour() {
        let clock = TestClock::new(start());
        let count = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(ticker(clock.clone(), count.clone()));
        clock.advance(secs(3600)).await;
        assert_eq!(count.load(Ordering::SeqCst), 120);
        clock.sleep_both(secs(7200)).await;
        assert_eq!(count.load(Ordering::SeqCst), 121);
        task.abort();
    }

    // TST-4: advance settles first, so work started just before it sets its timers at the start time.
    #[tokio::test]
    async fn tst4_advance_settles_first() {
        let clock = TestClock::new(start());
        let woke: Log<Mono> = Log::default();
        {
            let (c, woke) = (clock.clone(), woke.clone());
            tokio::spawn(async move {
                let now = c.mono();
                c.sleep_until(now.plus(secs(10))).await;
                push(&woke, c.mono());
            });
        }
        clock.advance(secs(30)).await;
        assert_eq!(*woke.lock().unwrap(), vec![at(10)]);
    }

    // TST-4: a hang held by script is excused; once the excuse goes, the clock can't settle.
    #[tokio::test]
    async fn tst4_an_excused_hang_does_not_hold_the_clock() {
        let clock = TestClock::new(start());
        let activity = clock.activity();
        let busy = activity.begin();
        let excused = activity.excuse();
        assert_eq!(activity.in_flight(), 0);
        clock.advance(secs(20)).await;
        assert_eq!(clock.mono(), at(20));
        drop(excused);
        clock.set_settle_limit(Duration::from_millis(50));
        let r = AssertUnwindSafe(clock.settle()).catch_unwind().await;
        let text = panic_text(r.expect_err("settle must panic with work in flight"));
        assert!(text.contains("work in flight: 1"), "{text}");
        drop(busy);
        clock.settle().await;
    }

    struct Flag(AtomicBool);
    impl SettleProbe for Flag {
        fn settled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    // TST-4: settled includes every probe settled.
    #[tokio::test]
    async fn tst4_advance_waits_for_a_probe() {
        let clock = TestClock::new(start());
        let probe = Arc::new(Flag(AtomicBool::new(false)));
        clock.add_probe(probe.clone());
        let done = Arc::new(AtomicBool::new(false));
        {
            let (probe, done) = (probe.clone(), done.clone());
            tokio::spawn(async move {
                for _ in 0..50 {
                    tokio::task::yield_now().await;
                }
                done.store(true, Ordering::SeqCst);
                probe.0.store(true, Ordering::SeqCst);
            });
        }
        clock.advance(secs(1)).await;
        assert!(done.load(Ordering::SeqCst));

        // An unsettled probe is named when the limit runs out.
        probe.0.store(false, Ordering::SeqCst);
        clock.set_settle_limit(Duration::from_millis(50));
        let r = AssertUnwindSafe(clock.settle()).catch_unwind().await;
        let text = panic_text(r.expect_err("settle must panic on an unsettled probe"));
        assert!(text.contains("probes not settled: #0"), "{text}");
    }

    // TST-4: a fired timer is work in flight from the moment it fires until its future is polled.
    #[tokio::test]
    async fn tst4_a_fired_timer_counts_until_polled() {
        let clock = TestClock::new(start());
        clock.set_settle_limit(Duration::from_millis(50));
        let mut timer = clock.sleep_until(at(10));
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        // The noop waker means nothing polls it: the advance can't settle after firing it.
        let r = AssertUnwindSafe(clock.advance(secs(20)))
            .catch_unwind()
            .await;
        let text = panic_text(r.expect_err("advance must not settle on an unpolled timer"));
        assert!(text.contains("timers fired and not polled: 1"), "{text}");
        assert_eq!(clock.mono(), at(10));
        assert!(timer.as_mut().poll(&mut cx).is_ready());
        clock.settle().await;
        clock.advance(secs(10)).await;
        assert_eq!(clock.mono(), at(20));

        // Dropped after firing, it stops counting too.
        let mut timer = clock.sleep_until(at(30));
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        assert!(clock.fire_next(at(30)));
        assert!(clock.unsettled().is_some());
        drop(timer);
        assert!(clock.unsettled().is_none());
    }

    // TST-4: a dropped timer leaves the heap and never fires.
    #[tokio::test]
    async fn tst4_a_dropped_timer_never_fires() {
        let clock = TestClock::new(start());
        clock.set_settle_limit(Duration::from_millis(50));
        let mut timer = clock.sleep_until(at(10));
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        assert_eq!(clock.pending_timers(), 1);
        drop(timer);
        assert_eq!(clock.pending_timers(), 0);
        clock.advance(secs(20)).await;
        assert_eq!(clock.mono(), at(20));

        // A timeout whose value came first drops its timer too.
        let shared: SharedClock = clock.clone();
        assert_eq!(timeout(&shared, secs(5), async { 3 }).await.unwrap(), 3);
        assert_eq!(clock.pending_timers(), 0);
    }

    // TST-4: every timeout runs on the clock: POL-2's 20 s and OPS-5's 10 s cost no real time.
    #[tokio::test]
    async fn tst4_timeout_runs_on_the_clock() {
        for limit in [20, 10] {
            let clock = TestClock::new(start());
            let result: Arc<Mutex<Option<Result<(), Elapsed>>>> = Arc::default();
            {
                let (shared, result): (SharedClock, _) = (clock.clone(), result.clone());
                tokio::spawn(async move {
                    let r = timeout(&shared, secs(limit), futures::future::pending::<()>()).await;
                    *result.lock().unwrap() = Some(r);
                });
            }
            clock.advance(secs(limit) - Duration::from_millis(1)).await;
            assert!(result.lock().unwrap().is_none());
            clock.advance(Duration::from_millis(1)).await;
            assert!(matches!(*result.lock().unwrap(), Some(Err(Elapsed))));
        }
    }

    // TST-4: a suspend the monotonic clock doesn't count moves the wall time alone and fires nothing.
    #[tokio::test]
    async fn tst4_sleep_wall_only_fires_nothing() {
        let clock = TestClock::new(start());
        let fired = Arc::new(AtomicBool::new(false));
        {
            let (c, fired) = (clock.clone(), fired.clone());
            tokio::spawn(async move {
                c.sleep_until(at(10)).await;
                fired.store(true, Ordering::SeqCst);
            });
        }
        clock.sleep_wall_only(secs(3600)).await;
        assert!(!fired.load(Ordering::SeqCst));
        assert_eq!(clock.mono(), Mono::ZERO);
        assert_eq!(clock.wall(), start().plus(secs(3600)));
        clock.advance(secs(10)).await;
        assert!(fired.load(Ordering::SeqCst));
        assert_eq!(clock.wall(), start().plus(secs(3610)));
    }

    // TST-4: the wall time can be set back alone.
    #[tokio::test]
    async fn tst4_set_wall_back_moves_wall_alone() {
        let clock = TestClock::new(start());
        clock.advance(secs(5)).await;
        let fired = Arc::new(AtomicBool::new(false));
        {
            let (c, fired) = (clock.clone(), fired.clone());
            tokio::spawn(async move {
                c.sleep_until(at(10)).await;
                fired.store(true, Ordering::SeqCst);
            });
        }
        clock.set_wall_back(secs(3600)).await;
        assert_eq!(clock.mono(), at(5));
        assert_eq!(clock.wall(), start().plus(secs(5)).minus(secs(3600)));
        assert!(!fired.load(Ordering::SeqCst));
        assert_eq!(clock.pending_timers(), 1);
    }

    // TST-4: the clock never moves by itself.
    #[tokio::test]
    async fn tst4_the_clock_never_moves_by_itself() {
        let clock = TestClock::new(start());
        let fired = Arc::new(AtomicBool::new(false));
        {
            let (c, fired) = (clock.clone(), fired.clone());
            tokio::spawn(async move {
                c.sleep_until(at(1)).await;
                fired.store(true, Ordering::SeqCst);
            });
        }
        clock.settle().await;
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        clock.settle().await;
        assert_eq!(clock.mono(), Mono::ZERO);
        assert_eq!(clock.wall(), start());
        assert!(!fired.load(Ordering::SeqCst));
        assert_eq!(clock.pending_timers(), 1);
    }

    // TST-4: a hung test says why: settle's panic names what is still in flight.
    #[tokio::test]
    async fn tst4_settle_names_what_is_in_flight() {
        let clock = TestClock::new(start());
        clock.set_settle_limit(Duration::from_millis(50));
        let activity = clock.activity();
        let busy = activity.begin();
        let r = AssertUnwindSafe(clock.settle()).catch_unwind().await;
        let text = panic_text(r.expect_err("settle must panic"));
        assert!(text.contains("not settled after 50ms"), "{text}");
        assert!(
            text.contains("work in flight: 1 (1 begun, 0 excused)"),
            "{text}"
        );
        drop(busy);
        clock.settle().await;
    }

    // TST-4: a time already reached is ready at the first poll and never enters the heap.
    #[tokio::test]
    async fn tst4_a_time_reached_is_ready_at_once() {
        let clock = TestClock::new(start());
        clock.advance(secs(10)).await;
        assert!(clock.sleep_until(at(10)).now_or_never().is_some());
        assert!(clock.sleep_until(at(3)).now_or_never().is_some());
        assert_eq!(clock.pending_timers(), 0);
    }

    // TST-4: a second move while one runs is a test's mistake.
    #[tokio::test]
    async fn tst4_a_second_advance_while_one_runs_panics() {
        let clock = TestClock::new(start());
        let activity = clock.activity();
        let busy = activity.begin();
        let first = clock.advance(secs(1));
        let second = async {
            tokio::task::yield_now().await;
            let r = AssertUnwindSafe(clock.advance(secs(1)))
                .catch_unwind()
                .await;
            drop(busy);
            r
        };
        let ((), r) = futures::join!(first, second);
        let text = panic_text(r.expect_err("a second advance must panic"));
        assert!(text.contains("already moving"), "{text}");
        assert_eq!(clock.mono(), at(1));
    }

    // TST-4: work in flight on the test clock: in_flight leaves out what is excused, never below zero.
    #[tokio::test]
    async fn tst4_activity_on_the_test_clock() {
        let clock = TestClock::new(start());
        let activity = clock.activity();
        let excused = activity.excuse();
        assert_eq!(activity.in_flight(), 0);
        let busy = activity.begin();
        assert_eq!(activity.in_flight(), 0);
        clock.advance(secs(1)).await;
        drop(excused);
        assert_eq!(activity.in_flight(), 1);
        drop(busy);
        assert_eq!(activity.in_flight(), 0);
        clock.advance(secs(1)).await;
    }
}
