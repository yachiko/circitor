use std::collections::HashSet;
use std::time::Duration;

use super::*;
use crate::model::PrName;
use crate::model::tests::sample_record;
use crate::poller::QuotaState;

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn at(n: u64) -> Mono {
    Mono::ZERO.plus(s(n))
}

fn ms(n: u64) -> Mono {
    Mono::ZERO.plus(Duration::from_millis(n))
}

fn policy() -> Policy {
    Policy {
        state: QuotaState::Normal,
        poll_every: Some(s(30)),
        hot_every: Some(s(10)),
        demand: true,
        pr_window: s(5),
        poll_window: s(10),
        follow_up: true,
    }
}

fn target(id: &str, class: Class, source: Source) -> Target {
    Target {
        id: NodeId(id.into()),
        pr: PrName::parse("o/r#1").unwrap(),
        source,
        class,
        status: Some(Status::Ready),
        mergeable_unknown: false,
        updated_at: None,
        timing: Timing::default(),
    }
}

fn sent(mut t: Target, n: u64) -> Target {
    t.timing.last_sent = Some(at(n));
    t
}

fn id(x: &str) -> NodeId {
    NodeId(x.into())
}

// POL-13
#[test]
fn pol13_tier() {
    let cfg = Config::default();
    assert_eq!(tier(None, at(5000), &cfg), Tier::Idle);
    assert_eq!(tier(Some(at(1)), at(600), &cfg), Tier::Active); // 599 s
    assert_eq!(tier(Some(at(0)), at(600), &cfg), Tier::Idle); // 600 s
}

// POL-14
#[test]
fn pol14_hot() {
    let now = at(1000);
    let named = |ago: u64| Timing {
        named: Some(at(1000 - ago)),
        ..Timing::default()
    };
    let waiter = Timing {
        waiters: 1,
        ..Timing::default()
    };
    assert!(hot(Some(Status::Merged), &waiter, now));
    assert!(hot(Some(Status::ChecksPending), &named(119), now));
    assert!(hot(Some(Status::Unknown), &named(119), now));
    assert!(!hot(Some(Status::Ready), &named(119), now));
    assert!(!hot(Some(Status::ChecksPending), &named(120), now));
    assert!(!hot(None, &Timing::default(), now));
    assert!(hot(None, &waiter, now));
}

// POL-10
#[test]
fn pol10_period() {
    use Class::*;
    use Source::*;
    use Tier::*;
    let c = Config::default();
    let p = |class, source, hot, sub, tier| period(class, source, hot, sub, tier, &c);
    assert_eq!(p(Ended, Authored, true, false, Idle), s(10));
    assert_eq!(p(Ended, Authored, false, false, Active), s(300));
    assert_eq!(p(Ended, Authored, false, false, Idle), s(300));
    assert_eq!(p(Open, Asked, false, false, Active), s(300));
    assert_eq!(p(Open, Asked, false, false, Idle), s(300));
    for src in [Authored, Manual] {
        assert_eq!(p(Open, src, false, false, Active), s(30));
        assert_eq!(p(Open, src, false, false, Idle), s(300));
    }
    assert_eq!(p(Open, Asked, false, true, Active), s(30));
    assert_eq!(p(Open, Asked, false, true, Idle), s(300));
    assert_eq!(p(Ended, Asked, false, true, Active), s(300));
    assert_eq!(p(Open, Asked, true, true, Active), s(10));
}

// NTF-5
#[test]
fn ntf5_last_ask() {
    let cfg = Config::default();
    let now = at(10_000);
    assert_eq!(last_ask(None, 0, now), None);
    assert_eq!(last_ask(None, 1, now), Some(now));
    assert_eq!(last_ask(Some(at(5)), 3, now), Some(now));
    assert_eq!(last_ask(Some(at(100)), 0, now), Some(at(100)));
    // One open: active two hours after the last request.
    let later = at(7200 + 5);
    assert_eq!(
        tier(last_ask(Some(at(5)), 1, later), later, &cfg),
        Tier::Active
    );
    // None open: the last request decides, whatever just ended.
    let n = at(1000);
    assert_eq!(tier(last_ask(Some(at(401)), 0, n), n, &cfg), Tier::Active);
    assert_eq!(tier(last_ask(Some(at(400)), 0, n), n, &cfg), Tier::Idle);
}

// NTF-5
#[test]
fn ntf5_a_subscription_is_never_hot() {
    let now = at(1000);
    let mut t = target("a", Class::Open, Source::Authored);
    t.status = Some(Status::ChecksPending);
    t.mergeable_unknown = true;
    t.timing.subscriptions = 3;
    assert!(!hot(t.status, &t.timing, now));
    let due = hot_due(std::slice::from_ref(&t), &policy(), now);
    assert!(due.now.is_empty());
    assert_eq!(due.next, None);
    assert!(!wants_follow_up(&t, false, &policy(), now));
}

// MOD-16
#[test]
fn mod16_stale() {
    let c = Config::default();
    let now = at(10_000);
    let rec = |state: PrState, source: Source, status: Status| {
        let mut r = sample_record();
        r.state = state;
        r.source = source;
        r.status = status;
        r
    };
    let fetched = |ago: u64| Timing {
        fetched: Some(at(10_000 - ago)),
        ..Timing::default()
    };
    let open = rec(PrState::Open, Source::Authored, Status::Ready);
    // gone: never, flags or not.
    let gone = rec(PrState::Gone, Source::Authored, Status::Gone);
    let both = Timing {
        attempt_failed: true,
        slept: true,
        ..fetched(5000)
    };
    assert!(!stale(&gone, &both, Tier::Active, &c, now));
    let failed = Timing {
        attempt_failed: true,
        ..fetched(1)
    };
    let slept = Timing {
        slept: true,
        ..fetched(1)
    };
    assert!(stale(&open, &failed, Tier::Active, &c, now));
    assert!(stale(&open, &slept, Tier::Active, &c, now));
    // Open, active: 60 s fresh, 61 s stale.
    assert!(!stale(&open, &fetched(60), Tier::Active, &c, now));
    assert!(stale(&open, &fetched(61), Tier::Active, &c, now));
    // Idle: 600 / 601.
    assert!(!stale(&open, &fetched(600), Tier::Idle, &c, now));
    assert!(stale(&open, &fetched(601), Tier::Idle, &c, now));
    // Hot: 20 / 21.
    let mut hot_t = fetched(20);
    hot_t.waiters = 1;
    assert!(!stale(&open, &hot_t, Tier::Active, &c, now));
    hot_t = fetched(21);
    hot_t.waiters = 1;
    assert!(stale(&open, &hot_t, Tier::Active, &c, now));
    // Ended: 600 / 601 in both tiers.
    for tier in [Tier::Active, Tier::Idle] {
        for state in [PrState::Merged, PrState::Closed] {
            let r = rec(state, Source::Authored, Status::Merged);
            assert!(!stale(&r, &fetched(600), tier, &c, now));
            assert!(stale(&r, &fetched(601), tier, &c, now));
        }
    }
}

// POL-13
#[test]
fn pol13_poll_due() {
    let p = policy();
    assert_eq!(poll_due(None, &p), Some(Mono::ZERO));
    assert_eq!(poll_due(Some(at(100)), &p), Some(at(130)));
    let none = Policy {
        poll_every: None,
        ..policy()
    };
    assert_eq!(poll_due(Some(at(100)), &none), None);
    assert_eq!(poll_due(None, &none), None);
}

// POL-10
#[test]
fn pol10_in_poll() {
    let c = Config::default();
    let now = at(1000);
    let ids =
        |ts: &[Target], listed: Option<&HashSet<NodeId>>, tier| in_poll(ts, listed, tier, &c, now);

    let loaded = target("loaded", Class::Loaded, Source::Authored);
    let first = sent(target("first", Class::First, Source::Authored), 0);
    let never = target("never", Class::Open, Source::Authored);
    assert_eq!(
        ids(&[loaded, first, never], None, Tier::Active),
        vec![id("loaded"), id("never")]
    );

    let open = |ago: u64| sent(target("o", Class::Open, Source::Authored), 1000 - ago);
    assert!(ids(&[open(29)], None, Tier::Active).is_empty());
    assert_eq!(ids(&[open(30)], None, Tier::Active), vec![id("o")]);
    assert!(ids(&[open(30)], None, Tier::Idle).is_empty());

    let merged = |ago: u64| sent(target("m", Class::Ended, Source::Authored), 1000 - ago);
    assert!(ids(&[merged(299)], None, Tier::Active).is_empty());
    assert_eq!(ids(&[merged(300)], None, Tier::Active), vec![id("m")]);

    let asked = |ago: u64| sent(target("a", Class::Open, Source::Asked), 1000 - ago);
    assert!(ids(&[asked(299)], None, Tier::Active).is_empty());
    assert_eq!(ids(&[asked(300)], None, Tier::Active), vec![id("a")]);

    let mut h = open(10);
    h.timing.waiters = 1;
    assert_eq!(ids(&[h], None, Tier::Active), vec![id("o")]);

    // Listed: a merged and a gone PR are due at 30 s.
    let gone = sent(target("g", Class::Ended, Source::Authored), 970);
    let m = merged(30);
    let listed: HashSet<NodeId> = [id("m"), id("g")].into();
    assert_eq!(
        ids(&[m.clone(), gone.clone()], Some(&listed), Tier::Active),
        vec![id("m"), id("g")]
    );
    // Not listed: still by their own period.
    let empty = HashSet::new();
    assert!(ids(&[m, gone], Some(&empty), Tier::Active).is_empty());
    // A tracked open PR missing from `listed` is due at its period.
    assert_eq!(ids(&[open(30)], Some(&empty), Tier::Active), vec![id("o")]);
}

// POL-13
#[test]
fn pol13_an_active_hour() {
    let c = Config::default();
    let p = policy();
    for (tier, expect) in [(Tier::Active, 120), (Tier::Idle, 12)] {
        let pol = Policy {
            poll_every: Some(if tier == Tier::Active { s(30) } else { s(300) }),
            ..p.clone()
        };
        let mut targets: Vec<Target> = (0..10)
            .map(|i| target(&format!("p{i}"), Class::Open, Source::Authored))
            .collect();
        let mut last: Option<Mono> = None;
        let mut polls = 0;
        for sec in 0..3600u64 {
            let now = at(sec);
            if poll_due(last, &pol).is_some_and(|d| d <= now) {
                let due = in_poll(&targets, None, tier, &c, now);
                assert_eq!(due.len(), 10, "at {sec}");
                for t in &mut targets {
                    t.timing.last_sent = Some(now);
                }
                last = Some(now);
                polls += 1;
            }
        }
        assert_eq!(polls, expect);
    }
}

// POL-14
#[test]
fn pol14_hot_due() {
    let mut t = sent(target("h", Class::Open, Source::Authored), 100);
    t.timing.waiters = 1;
    let p = policy();
    let d = hot_due(std::slice::from_ref(&t), &p, at(109));
    assert!(d.now.is_empty());
    assert_eq!(d.next, Some(at(110)));
    let d = hot_due(std::slice::from_ref(&t), &p, at(110));
    assert_eq!(d.now, vec![id("h")]);
    let slow = Policy {
        hot_every: Some(s(20)),
        ..policy()
    };
    assert!(
        hot_due(std::slice::from_ref(&t), &slow, at(119))
            .now
            .is_empty()
    );
    assert_eq!(
        hot_due(std::slice::from_ref(&t), &slow, at(120)).now,
        vec![id("h")]
    );
    let off = Policy {
        hot_every: None,
        ..policy()
    };
    let d = hot_due(std::slice::from_ref(&t), &off, at(500));
    assert!(d.now.is_empty());
    assert_eq!(d.next, None);
    // A First one is skipped; one never sent is due; not hot isn't in.
    let mut f = target("f", Class::First, Source::Authored);
    f.timing.waiters = 1;
    let mut n = target("n", Class::Open, Source::Authored);
    n.timing.waiters = 1;
    let cold = target("c", Class::Open, Source::Authored);
    assert_eq!(hot_due(&[f, n, cold], &p, at(5)).now, vec![id("n")]);
}

// POL-15
#[test]
fn pol15_windows() {
    let p = policy();
    let timing = |last| Timing {
        last_sent: last,
        ..Timing::default()
    };
    let now = at(100);
    assert!(!may_force_pr(&timing(Some(at(95))), &p, now));
    assert!(may_force_pr(&timing(Some(ms(94_999))), &p, now));
    assert!(may_force_pr(&timing(None), &p, now));
    let slow = Policy {
        pr_window: s(30),
        poll_window: s(30),
        ..policy()
    };
    assert!(!may_force_pr(&timing(Some(at(70))), &slow, now));
    assert!(may_force_pr(&timing(Some(ms(69_999))), &slow, now));
    // The poll: from the last forced poll.
    assert!(!may_force_poll(Some(at(90)), &p, now));
    assert!(may_force_poll(Some(ms(89_999)), &p, now));
    assert!(may_force_poll(None, &p, now));
    assert!(!may_force_poll(Some(at(70)), &slow, now));
}

// POL-11
#[test]
fn pol11_follow_up() {
    let now = at(1000);
    let p = policy();
    let mut t = target("t", Class::Open, Source::Authored);
    t.mergeable_unknown = true;
    assert!(!wants_follow_up(&t, false, &p, now));
    assert!(wants_follow_up(&t, true, &p, now));
    let mut w = t.clone();
    w.timing.waiters = 1;
    assert!(wants_follow_up(&w, false, &p, now));
    let mut n = t.clone();
    n.timing.named = Some(at(941));
    assert!(wants_follow_up(&n, false, &p, now));
    n.timing.named = Some(at(940));
    assert!(!wants_follow_up(&n, false, &p, now));
    let mut e = w.clone();
    e.class = Class::Ended;
    assert!(!wants_follow_up(&e, true, &p, now));
    let off = Policy {
        follow_up: false,
        ..policy()
    };
    assert!(!wants_follow_up(&w, true, &off, now));
    let mut k = w.clone();
    k.mergeable_unknown = false;
    assert!(!wants_follow_up(&k, true, &p, now));
}

// POL-7
#[test]
fn pol7_batches() {
    let v = |n: usize| (0..n).collect::<Vec<_>>();
    assert!(batches(&v(0)).is_empty());
    assert_eq!(batches(&v(40)).len(), 1);
    let b = batches(&v(41));
    assert_eq!((b[0].len(), b[1].len()), (40, 1));
    assert_eq!(batches(&v(200)).len(), 5);
    assert_eq!(batches(&v(41)).concat(), v(41));
}

// POL-19, D12
#[test]
fn d12_a_failed_pr_is_due_at_the_next_poll() {
    let c = Config::default();
    let now = at(1000);
    let ids = |t: Target| in_poll(&[t], None, Tier::Active, &c, now);
    let failed = |mut t: Target| {
        t.timing.attempt_failed = true;
        t
    };
    let open = || sent(target("p", Class::Open, Source::Authored), 999);
    assert_eq!(ids(failed(open())), vec![id("p")]);
    assert!(ids(open()).is_empty());
    let merged = || sent(target("p", Class::Ended, Source::Authored), 999);
    assert_eq!(ids(failed(merged())), vec![id("p")]);
    let first = sent(target("p", Class::First, Source::Authored), 999);
    assert!(ids(failed(first)).is_empty());
}
