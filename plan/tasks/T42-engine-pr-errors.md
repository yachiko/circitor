---
id: T42
title: "Poller engine: a PR's errors: gone, pr_failing, and its one log line"
kind: code
milestone: M2
size: M
status: done
depends_on: [T25]
requirements: [POL-21, POL-24, MOD-11]
owns: [src/poller/engine/]
shares: []
---

# T42 · Poller engine: a PR's errors: gone, pr_failing, and its one log line

## Outcome
The engine follows each PR's run of errors: a PR that answers `NOT_FOUND` on every refresh for 5 minutes and at least three of them becomes `gone` under MOD-11's conditions (in a poll, under the same login, not when every PR of the poll is `NOT_FOUND`), a PR whose refresh keeps failing for 5 minutes raises `pr_failing`, and a PR's error is logged once while it lasts.

## Context
T21 already keeps a failing PR's record as it was and marks it stale: for a node that answers `NotFound` or `Error` it calls `store.attempt_failed` and nothing more. It left three empty calls marked `// T42`: one per node result, one per finished poll, one per tick. T25 passes the node results of hot, forced and follow-up refreshes through the same call, marked as not a poll. This task fills those calls, in the same directory. The store (T15) turns the record `gone`; the model (T05) derives the `status`. The scenarios `repo-gone`, `pr-error` and `gone-returns` (T32, T31) run on it, and the golden CLI output (T35) shows `gone`: those three wait for this task directly. The harness (T29) doesn't.

## Read first
- `plan/conventions.md`
- `plan/contracts/poller.md`: "Inside: the engine", the lines for T21 and T42. `plan/contracts/state.md`: `attempt_failed`, `login_of`, `mark_gone` (also for a `Loaded` entry), `is_tracked`, `targets`, `Target`, `Class`. `plan/contracts/log.md`: `Warnings` (`raise`, `clear`, the `key`), the `pr error` word, and "Messages are built by circitor's own code". `plan/contracts/github.md`: `NodeAnswer::NotFound` and `Error`, `TokenSourceKind`.
- `src/poller/engine/` as T21 and T25 left it: the three calls, what each is given, and where they're made from.
- `specs/02-polling.md#errors` POL-21, POL-24; `specs/01-model.md#status` MOD-11 ("`gone` is circitor's own state", "`gone` needs the same login", "A gone PR that GitHub returns"); `#identity-and-fields` MOD-2 (`error`); `#revisions-and-events` MOD-16.
- `specs/03-state.md#the-tracked-set` STA-2 (when the retention clock starts); `#the-state-file` STA-8 ("its `NOT_FOUND`s count towards `gone` as for its saved state, under its saved login").
- `specs/04-api.md#basics` API-5: the `pr_failing` row.
- `specs/07-testing.md#what-is-tested` TST-8's rows `repo-gone`, `pr-error`, `gone-returns`.
- `docs/02-github-api.md` §5: what a missing node looks like, and that lost access is only assumed to look the same.

## Scope
- **Per PR, by node ID, in the engine's memory:** when its run of errors began and whether it's been logged; and its run of `NOT_FOUND`s in polls: when the first one was sent, and how many.
- **POL-21's share here: `gone`.** T21 has "keeps its last values and is stale". This task has the run: a PR becomes `gone` when its `NOT_FOUND`s have run for 5 minutes, on the monotonic clock from the first one's send time to the latest one's, and number at least three.
- **MOD-11's share here: which `NOT_FOUND` counts.** The derivation of `status` is T05's and the record's change is T15's. This task has the part "`gone` needs the same login":
  - only one from a poll counts; one from a hot, forced or follow-up refresh **neither counts nor ends the run**
  - only while the poll's login equals `store.login_of(id)`, without regard to case; a poll whose finding failed has no login, and nothing in it counts
  - not when the poll refreshed two or more tracked PRs and every one answered `NOT_FOUND`: so **decide at the end of the poll**, never per response
  - a `NOT_FOUND` that doesn't count for one of these reasons leaves the run as it was (this brief's reading)
- **What ends the run:** an applied refresh of that PR, or another error `type` for it in a poll. A request that fails as a whole says nothing about the PR and changes nothing.
- **Making it gone.** `store.mark_gone(id, seen_at)`, with the wall send time of the request whose `NOT_FOUND` completed the run (STA-2). Then forget the PR's runs. A `Loaded` entry answers `Unchanged`: that's right (STA-8), don't retry it.
- **POL-21's share here: `pr_failing`.** When a PR's errors, of any type and from any kind of refresh, have lasted 5 minutes with no applied refresh between, raise `WarningCode::PrFailing` with the node ID as `key`. The message names the PR, the latest error `type` and the token source in use (`TokenSourceKind::as_str`). Clear it when a refresh of the PR is applied, when it becomes `gone`, and, at the tick, when it's no longer tracked (API-5's row).
- **POL-24's share here: the log line.** `pr error pr=<name> type=<type>` at `warn`, once when a PR's run of errors begins; nothing more while it lasts, whatever the types. "Only `NOT_FOUND` counts towards `gone`" is the rule above. T21 has POL-24's whole-request half and "the rest of the response is applied".
- **A PR that's already `gone`** keeps answering `NOT_FOUND` at its slower rate: no run, no warning, no log line for it.
- 5 minutes and three are one named constant each (proposal).

## Interfaces
- **Consumes:** T21's three calls and what they carry; `Store::{login_of, mark_gone, is_tracked, targets}`; `status::Warnings`; `log::Logger`; `token::TokenSource::kind`; `clock::{Mono, Time}`.
- **Provides:** nothing new outside the engine. The bodies of the three calls; no signature of `PollerHandle` or of the store changes.

## Out of scope
- Marking a failed attempt, applying the other nodes, whole-request failures and the backoff: T21. Passing hot, forced and follow-up results through the call: T25. If a result doesn't reach the call, or reaches it without its kind or its send time, report it: don't re-plumb the poll.
- The record's change to `gone`, its event, its retention clock, a gone PR returning (`reopened`, `merged`, `closed`): T15, T05.
- `stale` (MOD-16): T09. `error` in the record (MOD-2): T15, fed by T21.
- Showing a warning in `status`, and `show`'s `error FORBIDDEN`: T22, T10. The scenarios: T31, T32.

## Acceptance
Unit tests in `src/poller/engine/`, set up as T21's (`ScriptedGitHub`, the real `Store`, `TestClock`, OPS-2's defaults). The tier is kept active by `client_request` unless a test says idle.
- `pol21_gone_after_five_minutes`: one of two tracked PRs answers `NOT_FOUND` at every poll, 30 s apart. It's still open and stale after the polls at 0 to 270 s, and `gone` at the poll sent 300 s after the first: one event with the kind `gone`, and the view's record keeps its other fields. The other PR is applied throughout.
- `pol21_idle_tier`: with polls 300 s apart, the first two `NOT_FOUND`s (0 and 300 s) don't make it gone, though 5 minutes have passed; the third does.
- `pol21_three_in_a_minute`: three `NOT_FOUND`s 30 s apart, then an applied refresh: open, not stale, and the run starts again from nothing.
- `mod11_same_login`: with the finding's login changed from the one the PR was last read with, 10 minutes of `NOT_FOUND` leave it open and stale.
- `mod11_not_every_pr`: two tracked PRs both `NOT_FOUND` at every poll for 10 minutes: neither becomes gone. A lone tracked PR in the same case does.
- `mod11_only_polls_count`: `NOT_FOUND` on hot refreshes alone, every 10 s for 6 minutes, never makes the PR gone; and a run begun in polls isn't ended by a hot `NOT_FOUND` between them.
- `mod11_no_login_no_count`: polls whose finding fails while the node answers `NOT_FOUND` count nothing.
- `pol21_other_type_ends_the_run`: `NOT_FOUND` for 4 minutes, one `FORBIDDEN`, `NOT_FOUND` again: gone only 5 minutes and three polls after the `FORBIDDEN`.
- `sta8_loaded_entry`: a loaded PR saved open, under its saved login, answers `NOT_FOUND` for 5 minutes: `mark_gone` is called once, there's no event, and the store's file content says `gone` with that request's send time.
- `sta2_seen_at`: the gone PR's retention runs from the send time of the request that completed the run, not from the first `NOT_FOUND`.
- `pol24_one_log_line`: a node answering `FORBIDDEN` at every poll for 10 minutes makes exactly one `pr error` line, with the PR's name and `FORBIDDEN`; after an applied refresh and a new failure, a second.
- `pol21_pr_failing`: the same PR has no warning at 270 s and `pr_failing` from the failing refresh at 300 s; its message holds the PR's name, `FORBIDDEN` and `gh`, and not the node ID. It never becomes gone. The first applied refresh clears it. A type that changes to `NOT_FOUND` keeps the warning's `since` and updates its message.
- `pol21_pr_failing_cleared`: when the PR becomes gone, and when it's untracked while failing (by the next tick).
- `pol21_already_gone`: a gone PR's later `NOT_FOUND`s add no log line, no warning and no second `mark_gone`.
- `ops9_no_text`: with titles set in `pr_read`, no log line and no warning message holds one.
- T21's and T25's tests still pass unchanged.
- `cargo test --features test-endpoint --lib poller::engine::` and `make verify` pass.

## Traps
- **"Every PR of the poll" spans all its requests**, and a poll can have five. Collect per-PR answers through the poll and judge once, in the per-poll call.
- **A lone tracked PR must be able to go**: the "every one" rule applies only from two refreshed PRs up.
- **Two clocks.** The 5 minutes are `Mono`; `seen_at` is the request's wall send time. Don't derive one from the other.
- **The warning's `key` is the node ID and is never shown** (`contracts/log.md`); the message names the PR by `owner/repo#number`, taken from the store, since a failing node brings no name.
- **Forget a PR's runs when it's no longer tracked**, or the map grows for as long as the daemon lives. The tick's call is the place.
- A token that sees less looks like a deleted repository (docs/02 §5, assumed): that's why the login and the "every one" rules exist. Don't loosen them to make a test shorter.
- Logins compare without case, as owners do.
- Hold no `Busy` across an await here: these calls don't await at all.

## Open questions
- **POL-21 and MOD-11's `gone` rule are (proposal)**, and that lost access answers `NOT_FOUND` is `assumed` (docs/02 §5). Build the spec as written.
- **Readings this brief makes, to report if they don't fit:** a `NOT_FOUND` that doesn't count (wrong login, every PR missing, no login) leaves the run as it was, as POL-21 says of a hot one; another error `type` in a poll ends it; the 5 minutes span the first counted `NOT_FOUND`'s send to the latest one's; `pr_failing` counts a PR's own errors from any kind of refresh, not whole-request failures; a check page whose node answers an error counts as that PR's error.
- **`contracts/log.md`'s table still names T21 for `pr error`**; the line is built here, with that word.

## Evidence
- **2026-11-26**, branch `t42-engine-pr-errors`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- `src/poller/engine/pr_errors.rs`, new `tests/pr_errors.rs`, one line in `tests/mod.rs`. **T21's `pol21_node_errors_stay_with_the_pr`:** its ending had pinned the pre-T42 engine on purpose ("that's T42"); the executor first ignored it, and on the orchestrator's instruction changed only its final assertion (the lone `NOT_FOUND` PR `gone`, the `FORBIDDEN` one open with its error, one `pr_failing`), with a comment. No test ignored.
- `cargo test --features test-endpoint --lib poller::engine::`: 70 passed, 0 ignored. `make verify` passes on main after the merge (530 lib tests), the macOS check included.
- **Watch:** `transport::tests::no_daemon` failed once in one of the executor's `make verify` runs, under load, and passed alone and in 5 more runs of `transport::`. Not reproduced; T17's to look at if it shows again.
- Choices: a PR no longer tracked when its error arrives starts no run; PRs made `gone` here sit in an engine-side set (a `Loaded` entry has no status), cleared by an applied refresh or by the tick once untracked.
