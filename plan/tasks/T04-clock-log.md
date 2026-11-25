---
id: T04
title: "Clock, test clock, work in flight, sleep detection, logger and warnings"
kind: code
milestone: M1
size: M
status: todo
depends_on: [T02]
requirements: [TST-4, TST-2, OPS-9, POL-26, POL-22]
owns: [src/clock/, src/log/, src/status/]
shares: []
---

# T04 · Clock, test clock, work in flight, sleep detection, logger and warnings

## Outcome
`SystemClock`, and a `TestClock` that fires timers in order and settles after each (TST-4); `Activity`; `SleepWatch` (POL-22's check); the `Logger` with OPS-9's format, levels and its rule that logging never stops the daemon; and the `Warnings` registry. Every later task tests on this clock and logs through this logger.

## Context
T02 left `src/clock/`, `src/log/` and `src/status/` with their declarations: `Time`, `Mono`, `clock::sleep`, `clock::timeout` and `Activity` (with `excuse`) work; the rest are `unimplemented!("T04")`, unless T02 wrote a minimal logger for its own tests. This task builds them. **Every later task tests on your `TestClock`**, and the whole test suite's determinism rests on its settling, so its behaviour matters more than its size.

## Read first
- `plan/conventions.md`: "Time", "Work in flight", "Logging", "Tests" (the runtime).
- `plan/contracts/clock.md` and `plan/contracts/log.md`: the declarations you fill and their semantics. They're the interface; this brief only adds how.
- `specs/07-testing.md#test-doubles`: TST-4, word by word. `specs/07-testing.md#rules`: TST-2.
- `specs/02-polling.md#errors`: POL-22 (its first two sentences) and POL-26.
- `specs/06-operations.md#the-daemons-life`: OPS-9.
- `docs/04-platform.md` §8 item 6: what isn't known about timers across a suspend.
- `src/clock/`, `src/log/`, `src/status/` as T02 left them.

## Scope
- **`SystemClock`** is a struct with a private field, the `Instant` it was made at (clock.md), made by `SystemClock::new() -> Arc<SystemClock>`; T02 left a placeholder field, which you replace. `wall()` is the system's UTC time; `mono()` is the time since an `Instant` taken at creation; `sleep_until` uses `tokio::time::sleep` for what's left, and is ready at once for a time already reached. This directory is the only place those calls may appear (TST-2).
- **`TestClock` (TST-4, whole).** Timers sit in a heap ordered by due time, then by the order they were set.
  - `sleep_until(at)` with `at` already reached is ready at its first poll and never enters the heap. A dropped timer leaves the heap.
  - `advance(d)`: **`settle()` first**, so work started just before the call has set its timers. Then take the target, `mono + d`. While the earliest timer is due at or before the target: set `mono` to its due time and move `wall` by the same amount, fire it, `settle()`. Timers set while settling join the same loop. Then set both clocks to the target and `settle()`. A second `advance` while one runs is a test's mistake: panic.
  - **A fired timer is work in flight** from the moment it fires until its future is polled or dropped.
  - `settle()`: yield to the runtime; while `Activity::in_flight() > 0`, a fired timer is unpolled, or a probe isn't settled, wait 1 ms of real time and look again. **Settled is three clean looks in a row with a `yield_now` between.** After 10 s of real time it panics, naming what was still in flight (the count, how much is excused, the unpolled timers, which probe), so a hung test says why. Keep that limit in a value a unit test can shorten.
  - `sleep_wall_only(d)`: `wall` jumps, nothing fires, settle. `sleep_both(d)`: both jump in one step, then every timer now due fires in order of due time, one at a time with a settle after each. `set_wall_back(d)`: `wall` alone, settle.
  - **The clock never moves by itself.**
- **`Activity`, `Busy`, `excuse` and `Excused`** are T02's work and now yours: check them against clock.md's "`Activity`: the rule" and "who counts what", and test them on the test clock. `in_flight()` leaves out what a held `Excused` covers and never goes below zero; if T02 left `excuse` a stub, build it. The same goes for any minimal `Logger` or `BufferSink` T02 left: replace it freely, keeping the declared signatures.
- **`SleepWatch` (POL-22: the check only).** It remembers the wall and monotonic times of the last check. `check()` computes the gap, wall's move minus mono's move. **A gap over 30 s is a sleep:** it returns the gap, adds it to `total` and tells subscribers. Exactly 30 s isn't. A negative gap (the wall clock set back) is never a sleep. Every check moves the baseline. Concurrent calls see one sleep once. **What happens on a sleep is T21's** (stale records, the backoff, the poll) **and T23's** (waiters' deadlines).
- **POL-26's share: the two clocks stay apart.** No function turns a `Time` into a `Mono` or back; `Time::since` gives zero for an earlier time that is ahead (a wall time ahead of now counts as now). T02 wrote the arithmetic; you own it and prove it. The tick is T21's, the sliding hour T08's.
- **`Logger`, `StderrSink`, `BufferSink` (OPS-9: the logger itself).**
  - A line is `<time> <level> <message>`: `Time::rfc3339()` of the clock's wall time, the level as `error`, `warn`, `info` or `debug`. A newline or carriage return inside a message becomes a space: one event, one line.
  - The level is checked **before** the message is formatted. `level_from_env`: unset or empty is `Info`; exactly `error`, `warn`, `info` or `debug` is that level; anything else (other case included) is `Info` and the value handed back for T28's warning line.
  - **Logging never stops the daemon:** a line the sink refuses is dropped, no panic, no error. `failing_since()` is the wall time of the first refused line, until a line is written again. The logger doesn't raise a warning itself: T22 shows `log_unwritable` while `failing_since()` is set (plan decision P10: OPS-9 asks for the warning, API-5 has no row).
  - **Never logged** is kept by construction as far as a logger can: it has no method that takes a record, a response or a `Token`, only text a caller built. The marker sweep that proves it is T29's (TST-11).
  - Of OPS-9's other parts: rotation is T24's, the "Logged" list is spread as `plan/contracts/log.md` says (yours is the row "every warning raised and cleared"), the `debug` line per request is T21's.
- **`Warnings`.** First `raise` of a `(code, key)`: `since` is the clock's wall time, one `warn` line. A repeat replaces the message and logs nothing. `clear` removes it with one `info` line, and does nothing when it isn't raised. `list()` orders by `since`, then by the code's place in `WarningCode`, then by key. **The key is never logged and never in a `Warning`:** for `pr_failing` it's a node ID, which is never shown (MOD-1). The two lines' wording is yours, fixed by your tests.
- **TST-2's share: the guard.** A unit test in `src/clock/` walks every `.rs` file under `src/` (from `CARGO_MANIFEST_DIR`) and fails, naming file and line, on: `Instant::now`, `SystemTime::now`, `Utc::now`, `Local::now` or `tokio::time::` outside `src/clock/`; `thread::sleep` anywhere; and unseeded randomness anywhere (`fastrand::` followed by a lower-case letter, or `Rng::new(`). Comment lines are skipped. Seeding from `Deps.seed` is T15's and T28's: T28's `main` seeds from the clock's wall milliseconds XOR the process ID, so nothing needs an unseeded call and the `fastrand::` pattern stays.

## Interfaces
- Consumes: T02's `Time`, `Mono`, `Clock`, `clock::sleep`, `clock::timeout`, and the stubs it left in your three directories.
- Provides: everything `plan/contracts/clock.md` and `plan/contracts/log.md` declare, unchanged in name and signature. A sink that fails on demand stays in your tests.

## Out of scope
- Reacting to a sleep, the 10 s tick, the gate's timeouts: T21. A waiter's deadline: T23.
- The fake GitHub's `SettleProbe`: T20. The harness: T29.
- Who logs which event: the tasks in `plan/contracts/log.md`'s table. Log rotation and opening the log file: T24. Reading `CIRCITOR_LOG`: T28.
- The `log_unwritable` warning in `status`: T22.

## Acceptance
Unit tests beside the code, on `#[tokio::test]`, the clock starting at 2026-11-24T22:00:00Z:
- `tst4_advance_fires_timers_at_their_due_time_in_order`: timers set for 30 s, 10 s and 20 s; under `advance(60 s)` each wakes with `mono()` reading exactly its own due time and `wall()` the start plus the same; afterwards `mono()` is 60 s and `wall()` 22:01:00Z.
- `tst4_same_instant_fires_in_the_order_set`: three timers due at 10 s.
- `tst4_settles_after_each_timer`: a task woken at 10 s takes a `Busy`, awaits something another task completes several yields later, then records; a timer at 11 s records after it, never before.
- `tst4_a_loop_rearmed_with_absolute_times_fires_120_times_in_an_hour`: every 30 s under `advance(3 600 s)` (TST-6's active hour). The same loop under `sleep_both(7 200 s)` fires once, not 240 times.
- `tst4_advance_settles_first`: a task spawned just before `advance(30 s)` sets a 10 s timer at its first poll, and wakes with `mono()` at 10 s, not 30 s.
- `tst4_an_excused_hang_does_not_hold_the_clock`: with a `Busy` held for good and an `Excused` beside it, `advance(20 s)` returns; once the `Excused` is dropped, the next `settle()` (limit shortened) panics. **Catch that `settle()`'s panic alone** (`futures::FutureExt::catch_unwind` around it, or a spawned task's `JoinError::is_panic`), not with `#[should_panic]`, which would also pass if `advance` itself hung and panicked.
- `tst4_advance_waits_for_a_probe`, `tst4_a_fired_timer_counts_until_polled`, `tst4_a_dropped_timer_never_fires`.
- `tst4_timeout_runs_on_the_clock`: `clock::timeout` of 20 s around a future that never ends is still pending after `advance(19.999 s)` and `Err(Elapsed)` one millisecond later (POL-2's value; the same with OPS-5's 10 s).
- `tst4_sleep_wall_only_fires_nothing`, `tst4_set_wall_back_moves_wall_alone`, `tst4_the_clock_never_moves_by_itself`.
- `tst4_settle_names_what_is_in_flight`: with a `Busy` never dropped and the limit shortened, `settle()` panics and the message says so.
- `pol22_a_gap_over_30_s_is_a_sleep`: wall alone forward 31 s gives `Some(31 s)`; 30 s gives `None`; `sleep_wall_only(2 h)` gives `Some(2 h)`; `sleep_both(2 h)` gives `None` (both ways of TST-4).
- `pol22_a_clock_set_back_is_never_a_sleep`: back 1 h gives `None`; after that check, wall alone forward 31 s gives `Some(31 s)`.
- `pol22_total_adds_up_and_subscribers_hear`: sleeps of 2 h and 40 s make `total()` 2 h 40 s; two `check()` calls after one jump report it once.
- `pol26_time_since_never_goes_negative`.
- `ops9_line_format`: `info("started")` writes exactly `2026-11-24T22:00:00Z info started`; one test per level word.
- `ops9_level_is_checked_before_formatting`: at `Info`, a `debug` call writes nothing and never calls the message's `Display`.
- `ops9_level_from_env`: `None` and `Some("")` give `(Info, None)`; `Some("debug")` gives `(Debug, None)`; `Some("loud")` and `Some("DEBUG")` give `Info` and the value back.
- `ops9_a_message_with_newlines_is_one_line`.
- `ops9_a_failing_sink_never_stops_logging`: no panic; `failing_since()` is the time of the first refused line and stays it while time moves on; after the sink works again a line is written and it's `None`.
- `ops9_a_warning_logs_once_raised_and_once_cleared`: three raises and two clears of one `(code, key)` make exactly one `warn` and one `info` line; `since` is the first raise's time, the message the last one's; the key is in neither line.
- `status_list_order`: by `since`, then code, then key.
- `tst2_only_the_clock_reads_time`: the guard, passing on the tree as it is, and failing on a temporary file that holds each pattern.
- `cargo test --features test-endpoint --lib clock::`, `… --lib log::`, `… --lib status::` and `make verify` pass.

## Traps
- **Never hold a `Busy` across a wait that only the clock can end** (clock.md, "`Activity`: the rule"). `settle()` waits for the count to reach zero, and the timer fires only after `settle()`: that's a deadlock, and your 10 s panic is how it shows. In your own tests, a hang by script is held with `excuse()`.
- **Don't use tokio's paused time** (`start_paused`, `tokio::time::pause`): the fake GitHub and the socket are real I/O. The 1 ms waits inside `settle()` are real time on purpose.
- **Don't hold the clock's own lock across an `.await`**, and fire a timer's waker after letting go of it: the woken task may set a new timer at once.
- **Don't test `SystemClock` by sleeping.** Test that its `mono()` never goes back and that a time already reached is ready at once. Real timers are exercised by T36.
- **The guard must not trip on itself:** build its patterns so the test file doesn't hold them as they'd appear in code.
- `Level` derives `Ord` in the order `Error < Warn < Info < Debug`: a line is written when its level is at or below the logger's.
- An `io::Error`'s text is safe to log; text built from a response isn't (log.md, "Don't assume"). The logger can't tell: it's the caller's rule, so don't add a method that takes an error from another module.

## Open questions
- **POL-22 and POL-26 are (proposal)**, and whether tokio's timers and the monotonic clock count a suspend isn't checked on either system (docs/04 §8 item 6). Build as written: the 30 s gap in one constant, and nothing that assumes `mono()` stops or runs during a suspend. TST-13 step 6 checks it on real machines.
- D5 and D6 don't touch this task.
