---
id: T13
title: "Poller auth: when the token is read, and the auth state"
kind: code
milestone: M1
size: M
status: done
depends_on: [T04, T43]
requirements: [POL-3, POL-20, OPS-5:the-token-is-read-after, OPS-5:a-read-that-ends-without]
owns: [src/poller/auth/]
shares: []
---

# T13 · Poller auth: when the token is read, and the auth state

## Outcome
`poller::auth::Auth` holds the token for the engine: the first read everything waits for, the read every 10 minutes, the read and one repeat on a 401, and the `auth` state with its retries and its message (POL-3, POL-20).

## Context
T02 declares `Auth`, `AuthProblem` and `AfterRefusal` with empty bodies, and the `TokenSource` trait. T43 builds `token::fake::FakeTokenSource`, which every test here uses. T04 gives the test clock and the logger. The engine's loop (T21, then T25) owns the one `Auth`: it asks it for a token before each request, tells it about every 401 and every success, and runs its periodic read. `gh` itself is run by T12, behind the trait.

## Read first
- `plan/conventions.md`
- `plan/contracts/poller.md`: "Inside: the token's life" (the signatures and their doc comments). `plan/contracts/github.md`: `Token`, `TokenRead`, `GhFailure`, `TokenSourceKind`, `TokenSource`, and `FakeTokenSource`. `plan/contracts/clock.md`: `Clock`, `TestClock`, "`Activity`: who counts what". `plan/contracts/log.md`: "Who logs what".
- `specs/02-polling.md#requests` POL-3; `#errors` POL-20, all five parts.
- `specs/06-operations.md#the-token` OPS-5: "The token is read after…", "A read that ends without a token…", "A read has three outcomes".
- `specs/04-api.md#basics` API-3, the `auth` row: what the message must say.
- `docs/02-github-api.md` §1 (gh rotates tokens; the variables), §5 (the 401).

## Scope
- **The first read (OPS-5's part "the token is read after…", this task's share).** `token()` runs it if none has ended, and returns only when it has: that's how every request that needs GitHub waits. Until it ends `problem()` is `None` and `first_read_done()` false. Reading only after the socket is bound is T28's share.
- **A read that ends without a token (OPS-5's part, whole; POL-20).** No login, or a failure with no token in hand: the `auth` state, at once, with `first_read_done()` true. The daemon still runs: `token()` returns the problem, nothing panics.
- **The periodic read (POL-3).** `due()` is 10 minutes after the last read ended, on the monotonic clock; before any read, `Mono::ZERO`. `periodic()` reads once. A token: it replaces the one in hand, silently. A failure keeps the token in hand and logs one `warn` for a run of failures. No login drops it and enters `auth`.
- **A 401 (POL-20).** `unauthorized(refused, false)` reads again and returns `Retry` with the token now in hand, the same one included, and with the kept one when the read fails. `unauthorized(_, true)`, and a 401 on a try in `auth`, return `InAuth` with no read. No login on that read is `InAuth` too.
- **In `auth` (POL-20's parts "the token is read again every 60 s" and "a read that gives another token…").**
  - `due()` is 60 s after the last read. `token(true)` reads again first when the last read ended 5 s ago or more; `token(false)` never reads.
  - **One try.** A read that gives another token than the one refused, or any token when none was in hand, opens one try: `periodic()` returns `true`, and the next `token()` returns `Ok`. The same token as the one refused opens a try only when its last try was 10 minutes ago or more.
  - `accepted(used)` ends `auth`. A 401 on the try goes back to `auth` at once.
  - **`problem()` stays `Some` during a try**, until `accepted` (`contracts/poller.md`), so `daemon.problem` doesn't flicker.
- **`last_read_ended()` (POL-20's 5 s, the engine's side).** The monotonic time the last read ended, of any kind: the first, a periodic one, the one after a 401, one by `token(true)`. `None` until the first has ended. The engine publishes it so `client_request` can apply POL-20's 5 s without waiting on the loop (T21).
- **The message (API-3's `auth` row, POL-20's part "a request that can't be answered…").** One sentence, built here, by cause and by `source.kind()`: no login names `gh auth login`; a failure names `gh auth token`, its exit status or the 10 s timeout, and `source.gh_path()`, never a login; a refused token from `GH_TOKEN` or `GITHUB_TOKEN` names the variable and says to fix or unset it and restart the daemon; a refused token from gh's login says GitHub refused it and names `gh auth login` (this brief's reading: the specs don't give this case).
- **POL-20's other parts are someone else's:** answering tracked PRs from memory and `daemon.problem` (T21, T22), the `auth` error of a read-through or `track` (T25), waiters staying (T23). This task gives them `problem()`.
- **Logging (OPS-9), with `contracts/log.md`'s event words.** `problem begin code=auth` (`warn`) and `problem end code=auth` (`info`), once each; `token read failed` (`warn`) once per run of failed periodic reads. A line holds the cause and the exit status, never `gh`'s output or the token.
- 10 minutes, 60 s and 5 s are one named constant each (POL-20 is a proposal).

## Interfaces
- **Consumes:** `token::{TokenSource, TokenRead, GhFailure, TokenSourceKind, Token}`, `clock::{SharedClock, Mono}`, `log::Logger`. `Auth::new` takes no `Activity`.
- **Provides:** `poller::auth::{Auth, AuthProblem, AfterRefusal}` exactly as `plan/contracts/poller.md` declares them.

## Out of scope
- Running `gh`, its 10 s limit, finding it, telling its outcomes apart, sharing one run: T12.
- Sending a request, repeating it after `Retry`, starting a poll after `periodic()` says `true`, publishing the status: T21. Calling `token(true)` for a read-through, a `track` or a forced refresh: T25.
- The `auth` error's HTTP shape and exit code: T22, T10.
- The order of start (the socket before the token): T28.

## Acceptance
Unit tests in `src/poller/auth/`, on `TestClock` with `token::fake::FakeTokenSource` and a `BufferSink` logger, each `#[tokio::test]`.
- `ops5_first_read_is_waited_for`: with a read delayed 8 s, `token(false)` is pending, `problem()` is `None` and `first_read_done()` false; after `advance(8 s)` it's `Ok`, and `reads() == 1`. Two more `token()` calls read nothing.
- `pol20_no_login_at_start`: `NoLogin` gives `Err`; the message holds `gh auth login`; `first_read_done()` is true.
- `ops5_failure_message`: `Failed(Exit(3))` at start names `gh auth token`, status 3 and the `gh_path`; `Failed(Timeout)` names the timeout; neither holds a login name or `gh auth login`.
- `pol3_periodic`: `due()` is 600 s after the first read ended. A periodic `Failed` keeps the token (`token(false)` is `Ok`), and two failures in a row log one `token read failed` line. A periodic read of another token makes `token()` return it. A periodic `NoLogin` drops it: `problem()` is `Some`.
- `pol20_one_repeat`: `unauthorized(&t, false)` reads once more and returns `Retry`; `unauthorized(&t, true)` returns `InAuth` and reads nothing; `problem()` is then `Some`, and the log has one `problem begin code=auth` line.
- `pol20_failed_reread_repeats_with_the_kept_token`: the read after a 401 fails: `Retry` holds the token in hand.
- `pol20_reads_in_auth`: `due()` is 60 s after the last read. Three `token(true)` calls inside 5 s make one read; `token(false)` makes none.
- `pol20_another_token_gets_one_try`: in `auth` after a refused token, a periodic read of the same token returns `false`; of another (`testkit::MARKER_TOKEN_2`), `true`, and `token(false)` is then `Ok` with it, with `problem()` still `Some`. `unauthorized` on that try is `InAuth` with no read. `accepted` clears `problem()`, logs `problem end code=auth` once, and puts `due()` back to 10 minutes.
- `pol20_same_token_every_10_minutes`: the refused token, read again at each 60 s, opens no try before 600 s and one at the first read after.
- `pol20_last_read_ended`: `None` before any read and while the first (delayed 8 s) runs; the read's end once it's done. It moves to each later read's end (`periodic`, `unauthorized(_, false)`, `token(true)` in `auth`), and not on `token(false)` or `unauthorized(_, true)`.
- `pol20_login_after_no_login`: in `auth` with no token, a read that gives a token opens a try at once (`token(true)` 5 s later is `Ok`).
- `api3_message_per_source`: with a `TokenSource` of the test's own whose `kind()` is `GhTokenVar`, a refused token's message names `GH_TOKEN` and a restart; with `Gh`, `gh auth login`.
- `ops5_no_token_anywhere`: after every case above, neither the log buffer nor the `Debug` of any `AuthProblem` holds `MARKER_TOKEN` or `MARKER_TOKEN_2`.
- `cargo test --features test-endpoint --lib poller::auth::` and `make verify` pass.

## Traps
- **`Auth` counts no work in flight** (`contracts/clock.md`, "`Activity`: the rule"): it has no `Activity`. The real source counts its own `gh` run (T12); the fake's delay is a timer, which nobody may hold a guard across. The engine drops its own guard before it calls in here (T21).
- **Compare tokens with `Token::same_as`**, never through a `String`. No `format!` may see `expose()`.
- **A failed read never drops a token** (POL-3); only `NoLogin` does. A failure with no token in hand is `auth`; with a refused token in hand the message stays the refused one.
- A 401 on a try must not read again: a bad token would otherwise run `gh` at every request.
- The 5 s and 60 s count from when the last read **ended**, on the monotonic clock; a read can take 10 s.
- Durations go by `Mono` (POL-26). Don't use wall time anywhere here.
- `FakeTokenSource::kind()` is fixed: the message test for the variables needs a few lines of `TokenSource` of its own.

## Open questions
- **POL-20 is a (proposal)**; the answer to an expired token is `assumed` (docs/02 §5), and `gh auth token` from a service on macOS isn't checked (docs/04 §8 item 4). Build the spec as written.
- **Readings this brief makes, to report if they don't fit:** a failed re-read after a 401 repeats with the kept token; the message for a refused token from gh's login; `due()` before any read is `Mono::ZERO`, so the engine's first `periodic()` or `token()` is the first read.

## Evidence
- **2026-11-26**, branch `t13-poller-auth`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/poller/auth/mod.rs` and a new `tests.rs`; no public signature changed; every test the Acceptance names exists; `expose()` appears only in test comparisons; the log words are `contracts/log.md`'s. `cargo test --features test-endpoint --lib poller::auth::`: 13 passed. `make verify` passes on main after the merge, the macOS check included.
- Choices: `api3_message_per_source` uses T43's `FakeTokenSource::set_kind` (the brief's "kind() is fixed" predates it); two tests move `Auth` into a spawned task to see a pending read; `token read failed cause=<word>` once per run of failures of any read; `accepted(used)` also sets the token in hand; `NoLogin` clears the refused token; `token(true)` during an open try still reads after 5 s, and a same-token read during a try is ignored. The refused token from gh's login reads "GitHub refused the token from gh's login: run `gh auth login`."
