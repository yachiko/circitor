---
id: T20
title: "Fake GitHub: the HTTP server, faults, rate-limit controls, the rules that fail a test"
kind: test
milestone: M1
size: L
status: done
depends_on: [T04, T14]
requirements: [TST-3:it-can-be-told-to-misbehave, TST-3:the-test-sets-remaining-and, TST-3:it-fails-the-test-if, TST-3:it-records-every-request-with-its-time, TST-11]
owns: [src/testkit/server/, tests/fake_github.rs]
shares: []
---

# T20 · Fake GitHub: the HTTP server, faults, rate-limit controls, the rules that fail a test

## Outcome
`testkit::server::FakeGitHub` serves the world of T14 on the loopback with GitHub's headers, misbehaves on request in every way TST-3 lists, lets a test set `remaining` and the login, records every request, fails the test on a mutation or a broken POL-2, and tells the test clock when it's settled.

## Context
T14 gives the world, `execute`, the cost and the description of what a request asked. T04 gives the clocks this server runs on and is tested on (`TestClock`, `SystemClock`), `SettleProbe` and `Activity::excuse`. This task puts HTTP around them. T26 is its first user, with the real client and the engine; the harness (T29) hands it to every scenario, the quota tests (T34) read its request log, and the real-process tests (T36) run it on the system clock. The test clock asks it, as a `SettleProbe`, whether GitHub's side is quiet.

## Read first
- `plan/conventions.md`
- `plan/contracts/testkit.md`, whole: "what T20 gives everyone" is the list to build, word for word; "Faults that change an answer are the world's"; the harness section shows how your methods get used. `plan/contracts/clock.md`: `TestClock`, `SettleProbe`, `Activity::excuse`, "`Activity`: who counts what", and "Don't assume". `plan/contracts/github.md`: `Failure`'s classes, to know what each fault must look like on the wire; `Client::for_tests`.
- `specs/07-testing.md#test-doubles` TST-3 (four parts are this task's; the first paragraph's clock rules too) and TST-4 ("Settled means…"); `#what-is-tested` TST-11, TST-6 (what the log is for), TST-9 (the second token, the per-process rules), and TST-8's rows `github-down`, `token-rotated`, `forced`, `crash`, `starting`, `sleep`, `clock-back`, `settled-at-once`.
- `specs/02-polling.md#requests` POL-1, POL-2; `#errors` POL-25 (the answers to play).
- `docs/02-github-api.md` §2 (the headers seen, the lagging counters), §5 (the 401's body).
- `src/testkit/github/` as T14 left it: the world's methods and their rustdoc.

## Scope
- **Two ways to start.** `FakeGitHub::start(clock: Arc<TestClock>)` binds `127.0.0.1:0` (real TCP), serves `POST /graphql` through `testkit::github::execute`, and adds itself to the clock as a probe. `FakeGitHub::start_real(clock: SharedClock)` is the same server on the system clock, for T36: the world's clock and the POL-2 rule run on real time, there's no probe and no excuse, and the in-flight and 1 s rules are held **per process, by the request's `x-circitor-test-pid` header** (TST-3, TST-1: a test build's client sends its process ID; requests without the header count as one process). Run your own accept loop with `hyper-util`'s connection builder: you need each connection's life.
- **Headers on every answer** (TST-3's first part, this task's share): `Date` from the world's clock, which a test can leave out; `x-ratelimit-limit`, `-remaining`, `-used`, `-reset`, and `x-ratelimit-resource: graphql`.
- **GitHub's clock (TST-3's first paragraph).** Before each request, and before each `world(|w| …)`, the world advances to the clock's wall time plus the skew, and never back.
- **TST-3's part "it can be told to misbehave".** Each for a number of requests or until cleared: a 5xx; a hang; a dropped connection; a 401; a 200 without `data`; a body that isn't JSON; a body cut short; a redirect, which counts the hits on its target (OPS-16); a refused connection; an exhausted limit, with and without `x-ratelimit-reset`; a secondary limit as a 403 and as a 200 with an error, each with and without `Retry-After`. **A fault can be aimed:** at requests with a finding, at requests without one, at one node, at a by-name lookup (an error `type` on `repository.pullRequest`). The answer-changing items are T14's world (`testkit.md` lists the five): expose each under the same style of control.
- **Holding an answer.** `hold_next()` makes the next request wait until the returned handle is released, and then answers it from the world as it is then. A hang never answers; a held request does.
- **The 401.** A REST-style body, as docs/02 §5 measured. Besides the counted fault, let a test name the tokens the fake accepts (by default any): another one gets the 401 (`MARKER_TOKEN` against `MARKER_TOKEN_2`).
- **TST-3's part "the test sets `remaining` and the reset time".** `set_rate(remaining, reset_at)` through `World::set_rate`, apart from what circitor spends; each answer's cost then comes off it. A lag: answers report a value some requests old (docs/02 §2). `set_login` (MOD-11).
- **TST-3's part "it records every request".** Every request, faulted ones too, each with exactly what `testkit.md` lists: the clock's monotonic and wall time at arrival; its cost (0 when nothing was executed); whether it held a finding, and that page's cursor; the node IDs and what was asked of each; a lookup's owner, repository and number; the `Authorization` value. `requests()` returns them; `points()` sums the costs.
- **TST-3's part "it fails the test if".** Four rules, each recorded with the offending request and raised as a panic at `check()` and at drop (not while already panicking): a mutation (POL-1); a query the schema refuses or the fake can't resolve; a request while another is in flight; a request less than 1 s after the last one ended, on the clock's monotonic time (POL-2). A request ends when it's answered or its connection closes. `new_daemon()` starts the in-flight and 1 s checks afresh.
- **The settle probe (TST-4).** Settled: every connection accepted is answered, closed, or held by script. **While it holds an answer by script, a hang or `hold_next`, it also holds `clock.activity().excuse()`**: the engine counts its request as work for as long as it awaits the answer, and the excuse is what lets the clock move under it.
- **`url()` and `client()`.** `client()` returns `github::client::Client::for_tests(&self.url())`. Until T07 is merged that's T02's stub: build `client()`, and call it in no test of this task.
- **TST-11's share:** the markers this layer adds. The 401's body and every fault body that carries text (a 5xx page, both limit messages, the redirect's body) hold `MARKER_GITHUB_TEXT`. Titles, check names, logins and GraphQL error messages are T14's; the sweep is T29's.

## Interfaces
- **Consumes:** `testkit::github::{World, execute, Executed, Rejected}`, the world's methods and its description of a request (T14); `clock::{TestClock, SharedClock, SettleProbe, Time, Mono}`; `testkit::{MARKER_GITHUB_TEXT, MARKER_TOKEN, MARKER_TOKEN_2}`; `axum`, `hyper`, `hyper-util`.
- **Provides:** `testkit::server::FakeGitHub` with everything `plan/contracts/testkit.md` lists. The names are this task's: put one rustdoc line on each, because eight later tasks read them.

## Out of scope
- The world, GraphQL execution, the cost, paging, the answer-changing faults: T14. Fix nothing there: report it.
- The real client and its decoding: T07. No test here uses it.
- The engine obeying POL-2 and reacting to each fault: T21, T25, T42. The first run of both together: T26.
- The harness, when `new_daemon()` is called, and TST-11's sweep: T29. TST-6's figures: T34. Real processes: T36.

## Acceptance
`tests/fake_github.rs`, on `TestClock` unless a test says otherwise, with a plain `reqwest` client and hand-written queries, each `#[tokio::test]`.
- `tst3_headers`: an answer has a `Date` equal to the clock's wall time, and the five `x-ratelimit-*` headers; after `without_date_header`, none.
- `tst3_githubs_clock`: after `advance(90 s)` the `Date` is 90 s later; after `set_wall_back(1 h)` it hasn't moved back; with a skew of 5 minutes it's 5 minutes ahead of the wall time; a change scheduled in the world shows once the clock passes it.
- `tst3_faults`, one case each: a 500; a hang that ends only when the client drops the request; a dropped connection (the client sees no status); a 401 whose body has `"status": "401"`; a 200 with no `data`; a body that isn't JSON; a body shorter than its `Content-Length`; a 302 whose target counts 0 hits, and 1 once the test follows it by hand; a refused connection, then listening again on the same port; exhausted with `x-ratelimit-remaining: 0`, with and without `x-ratelimit-reset`, and no `data`; a secondary limit as a 403 and as a 200, with `Retry-After: 30` and without, the message naming a secondary rate limit. A fault set for 2 requests is gone at the third.
- `tst3_aimed_faults`: a 500 aimed at requests with a finding leaves a `nodes`-only request answered, and the other way round; `NOT_FOUND` aimed at one node leaves the other node of the same request; `FORBIDDEN` aimed at a lookup puts the error on `["repository","pullRequest"]`.
- `tst3_hold_next`: the next request doesn't answer while its handle is held and answers when it's released, with a change made to the world meanwhile; the request after it isn't held.
- `tst3_tokens`: with `MARKER_TOKEN` the only accepted token, `MARKER_TOKEN_2` gets the 401; `requests()` shows the `Authorization` value of each.
- `tst3_rate_controls`: after `set_rate(400, t)`, the next answer's header and `rateLimit.remaining` are 400 less its cost, and `resetAt` is `t`; with a lag of 2, an answer reports the value from two requests before; `set_login` changes `viewer.login`.
- `tst3_request_log`: a finding with two nodes, a second page, an items request, a check page and a lookup are each logged with both times, the cost, the finding and its cursor, each node's ID and what was asked of it, and the lookup's name; `points()` is the sum; a faulted request is in the log with cost 0.
- `tst3_rules`, each `#[should_panic]` naming the rule: a mutation; a field the schema lacks; a second request while a hung one is open; a request 0.9 s after the last one ended.
- `tst3_rules_hold`: requests exactly 1 s apart pass; after a hung request's client drops, a request 1 s later passes; after `new_daemon()`, a request straight away passes.
- `tst3_real_processes_are_told_apart_by_pid`: with `start_real`, two requests 0.1 s apart with different `x-circitor-test-pid` values break no rule; two with the same value do; `requests()` gives each request's value.
- `tst4_probe_and_excuse`: `settled()` is true with no connection and while a request hangs or is held. With the test holding a `Busy` from `clock.activity()` around a held request, as the engine does, `advance(5 s)` returns; after release, answer and drop, `in_flight()` is 0. The same with **no** `Busy` taken: `advance` returns and the count never goes below 0.
- `tst9_start_real`: on `SystemClock`, two clients on two connections may overlap without breaking a rule; two requests 0.2 s apart on one connection break the 1 s rule; `Date` is the real time.
- `tst11_markers`: the 401 body and both limit messages hold `MARKER_GITHUB_TEXT`.
- `cargo test --features test-endpoint --test fake_github` and `make verify` pass.

## Traps
- **Hold the excuse exactly as long as the answer is held.** Take it when the hold begins, drop it when the client goes away, the handle is released or the fault is cleared. One left behind hides real work from the clock for the rest of the test; one taken late stops `advance` for 10 s and then panics.
- **`Excused` must not take the count below zero** when nothing counted the held request, as in this task's own tests with a plain client. T04 builds it that way; the last probe test proves it from this side.
- A connection accepted and not yet answered must never read as settled, a script's hold apart.
- **Measure POL-2 on the clock's monotonic time**, not on real time: under the test clock, real time between two requests is microseconds. Under `start_real` the two are the same clock.
- **A hung answer must notice its client going away**: the engine drops the request at 20 s, and the next one comes 1 s later. If the hang still counts as in flight, the rule fires wrongly.
- A refused connection means dropping the listener. Bind it again on the same port with `SO_REUSEADDR`; a test's URL must stay valid.
- A panic inside a handler task is swallowed by the runtime: record the broken rule and panic from `check()` and from `Drop`, on the test's own thread.
- `execute` needs the world behind a lock: never hold it across an `.await`.
- The exhausted and secondary answers carry no `data` at all, not `"data": null` beside a result.
- `tst9_start_real` is the one test here that waits on real time: keep it under a second. The file starts with `#![cfg(feature = "test-endpoint")]`.

## Open questions
- **The exhausted and secondary answers are `assumed`** (docs/02 §2, §6 item 6): play them as POL-25 words them. The exhausted answer's status isn't known: offer it as a 403 with a REST-style body and as a 200 with only `errors`.
- **A 5xx's shape isn't measured** (docs/02 §5): a 500 and a 502 with a short text body.
- **The settle seam is first exercised with real I/O in T26.** If the clock moves under a request there, or `advance` won't return under a hold, the fix is here, in T04 or in T21, and the orchestrator decides.
- **`Executed` carries only the body, the cost and `node_ids`**; the log's other fields come from T14's description of the request. If T14 didn't provide one, report it: don't parse GraphQL here.

## Evidence
- **2026-11-26**, branch `t20-fake-github-server`, executor on **Opus, medium**; reviewed and merged by the orchestrator.
- `src/testkit/server/` (`mod.rs`, new `fault.rs`, `answer.rs`, `log.rs`, `serve.rs`) and `tests/fake_github.rs` (starts with the feature's `cfg`). Binds loopback only. `cargo test --features test-endpoint --test fake_github`: 17 passed; the executor ran it 30 times, then 40 times in 8 parallel loops under load, without a failure. `make verify` passes on main after the merge (469 lib tests, 17 fake_github, 9 golden), the macOS check included.
- Choices: hyper's `http1::Builder` in its own accept loop, with hyper's own `Date` header off; `refuse()`/`listen()` close open connections and aren't counted as requests; a cleared hang closes its connection unanswered; `pr_error`/`field_error` set and clear the world's node or field error per request; the exhausted answer comes as 403 and as 200 (`LimitStatus`); additions `Hold::{arrived, is_holding}`, `held()`, `until_held(n)`, `broken()`, `take_broken()`, `addr()`, `with_date_header`, `accept_any`, `set_skew_behind`, `Request.{pid, fault, rejected, index}`; `Request`'s `Debug` hides the `Authorization` value.
- **Open, for T26:** the end of a hung request is recorded when hyper sees the client close; settle doesn't wait for that (a hang is held by script), so a clock advanced before the close is seen could falsely break POL-2's gap. Not seen in the stress runs. If T26 hits it, the fix is here or in T04.
