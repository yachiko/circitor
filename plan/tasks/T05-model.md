---
id: T05
title: "Model: checks summary, status, settling, kept mergeability, new items, and the merge that says what changed"
kind: code
milestone: M1
size: L
status: in-progress
depends_on: [T02]
requirements: [MOD-1, MOD-2, MOD-4, MOD-5, MOD-6, MOD-7, MOD-8, MOD-9, MOD-10, MOD-11, MOD-12, MOD-14, MOD-15, POL-12, STA-5, TST-15, U4, U5]
owns: [src/model/]
shares: []
---

# T05 · Model: checks summary, status, settling, kept mergeability, new items, and the merge that says what changed

## Outcome
`model::apply_read` merges one GitHub read into a record by every rule of 01-model and returns the event kinds of what visibly changed; `apply_gone`, `counts_rose`, name parsing and the item paths work; and the derivations are proven as tables (TST-15). Pure code: no clock, no I/O.

## Context
T02 left `src/model/` with every type of `plan/contracts/model.md`, their serde, and `PrName`; the functions are `unimplemented!("T05")`. This task builds them. The store (T15) calls `apply_read` for every refresh and records what it returns; the client (T07) builds the `PrRead` you take; cadence (T09), the routes (T22, T23) and the CLI (T10) read the `Record` and `Event`s you make.

## Read first
- `plan/conventions.md`; `plan/contracts/model.md`: the declarations, and **its Semantics section, which is half of this task's rules**.
- `specs/01-model.md`, all of it: MOD-1 to MOD-15. This brief cites it and adds only interpretation.
- `specs/03-state.md#the-tracked-set`: STA-5 (and STA-4 for what the caller does around you).
- `specs/02-polling.md#the-poll`: POL-12, for `more` and the kept counts; POL-8, for a read of several check pages.
- `specs/07-testing.md#what-is-tested`: TST-15, and the TST-8 rows named under Acceptance.
- `docs/02-github-api.md` §4 and `docs/measurements/2026-11-24/c1.log`: the timeline your table test replays.

## Scope
Suggested files: `derive.rs` (MOD-5, MOD-7, MOD-11), `merge.rs` (`apply_read`), `diff.rs` (MOD-14, MOD-15), `items.rs` (MOD-12, STA-5), `name.rs`. **Pure code: times arrive as `Time` and `Mono` values in `ReadMeta`.** Each (proposal) value is one named constant: 20 s, 60 s, the 5 s margin, MOD-8's 60 s, POL-12's 20, the passing results, the blocked merge states.
- **Names (MOD-1).** `PrName` is T02's work and now yours: check it against MOD-1 and the contract, and fill any gap. A rename arrives as `read.pr`: the record takes it, with `edited` naming `pr`. A change of case alone isn't a change (names compare without case): the record takes GitHub's spelling, no kind.
- **The record (MOD-2, MOD-10).** Field for field from `PrRead`, already mapped and lowered by T07 (MOD-3 and MOD-2's unlisted values are T07's share). `fetched_at`, `error`, `stale`, `source` and `rev` as the contract's Semantics say. **A newer read wins:** `meta.seq < carry.applied_seq` is `Merged::Older`; anything else is applied and `applied_seq` becomes `meta.seq`.
- **Checks (MOD-4, MOD-5, MOD-7).** The read's list replaces the record's: each read is believed. Matching (a run by `id`, a status by `name`; a run with no `id` by `name`) is only for MOD-14's "which items are in the list". `failed` counts completed items whose result isn't `success`, `skipped` or `neutral`, also while others run. Truncated is `items.len() < total`; then MOD-7's three rules in its order, on `checks.rollup`.
- **Settling (MOD-6).** `carry.settle_start` is `meta.sent_mono` of the read that first read the record (`New` and `Loaded` alike), first showed this `head_sha` (also back to an earlier one), or first showed it `open` again after `closed` or `gone`. That read leaves `settled` false. A later applied read sets it when `sent_mono - settle_start` is at least 20 s, or 60 s for the list **as this read shows it** when it's empty. **Once true it stays true** until the next settle start. The shortcut is for `New` and `Loaded` only: settled at once when `github_date - updated_at` is **more than** 25 s (65 s with no checks); without `github_date` or `updated_at`, never. The local clock plays no part in it.
- **Mergeability (MOD-8, MOD-9).** `carry.known` holds the last `yes` or `no` with its `merge_state`, head, base and `meta.sent_wall`. An `UNKNOWN` read keeps both values while the PR is open, head and base are the same, and `sent_wall.since(known.read_at) <= 60 s`; otherwise the record reads what GitHub sent and `known` is cleared. Only `yes` and `no` are kept: any other value is taken as read. A merged PR reads `unknown` for both, with no `mergeable` kind beside `merged`.
- **Status (MOD-11).** The table, first row that applies. **An unlisted value is read on the safe side, for `status` only** (MOD-11): an unlisted `mergeable` counts as `unknown`, an unlisted `review` as `review_required`; the record still shows the value as GitHub sent it (MOD-2). `blocked` and `behind` are built as written (D6). **`apply_gone`** turns an open record `gone` and returns `status` and `gone` kinds; every other field stays, `fetched_at` and `error` included; `None` for a record that isn't open. Deciding that a PR is gone (the login, the five minutes) is T21's. A gone record read again takes GitHub's state with `reopened`, `merged` or `closed`.
- **Items and newness (MOD-12, STA-5: the rule; storing `Carry` is T15's).** **POL-12's share here:** the newest 20 of a kind are what an `activity` reports, `more` says when there were others, and the old counts are kept when the items failed. Asking for the items, batching them and holding a refresh until they answer are T21's.
  - An `Item` is an `ItemRead` plus MOD-12's paths, built from the record's current name. A review always has `inline`; `fetch_inline` only above 0. An item with no time is ignored (MOD-12).
  - New is STA-5's rule against `carry.seen`, per kind. After a read, `seen` moves to the newest `at` read and the IDs at it; it adds IDs at an equal `at`, and **never moves back**.
  - `Before::New` with items read: a baseline, no `activity`. `Before::Loaded`: `activity` when at least one item is new. `Before::Known`: `activity` exactly when `counts.comments` or `counts.reviews` rose, with the new items, which may be none.
  - `ItemsOutcome::Failed`, or `NotAsked` with a rise: the record keeps its old `comments` and `reviews` counts (POL-12). `New` and `Loaded` always come with `Read`; given anything else, apply the record, leave `seen` alone and make no `activity`.
  - **`more`** (POL-12): for `Loaded`, true when all 20 read of a kind are new; for `Known`, when all 20 are new **and** that kind's count rose by more than 20.
  - Items in an `activity` are ordered by `at`, oldest first; then comments before reviews; then by `id` (MOD-15).
- **What changed (MOD-14, MOD-15).** Compare the old and new record on MOD-14's list only, and return kinds in `EventKind`'s declared order. `checks` whenever any of MOD-14's checks fields or the item set changed. `New` is exactly `[tracked]`; `Loaded` is `merged` or `closed` (when `saved_open` and it has ended), `activity`, `tracked`. One call makes one event's kinds: the revision, the event and its `at` are T15's (MOD-13).
- **`counts_rose` (POL-12) and `ChecksRead::append_page` (POL-8)** as the contract says. `short_sha` is T02's, built for real because T10 needs it first: leave it as it is.
- **U4 and U5** are met by the record and the item as built: say in your report which tests show them.

## Interfaces
- Consumes: the types of `src/model/` and `clock::{Time, Mono}` (T02).
- Provides: `apply_read`, `apply_gone`, `counts_rose`, `ChecksRead::append_page`, and `PrName` and `short_sha` as T02 built them, exactly as `plan/contracts/model.md` declares them. You may add private helpers and `pub(crate)` derivation functions; a changed or new public signature goes through the orchestrator.

## Out of scope
- Mapping GitHub's JSON to `PrRead`, lowering, MOD-3: T07.
- The revision counter, storing records and events, retention, `source` changes, `untracked`: T15 (MOD-13, STA-1 to STA-4).
- `stale` (MOD-16): T09. When a PR is gone (POL-21), joining check pages and asking for items (POL-8, POL-12): T21.
- Saving `seen` (STA-6): T16. Rendering events: T10. The golden JSON files of `testdata/api/` and `testdata/model/`: T41's; don't read them in your tests, and if one disagrees with the spec, report it.

## Acceptance
Unit tests in `src/model/`, as tables where the spec has a table (TST-15), no clock and no fake GitHub:
- `tst15_checks_state`: one case per row of MOD-5, with `startup_failure` and an unlisted `foo` reading `failing`, `skipped` and `neutral` passing, `cancelled` failing (TST-8 `skipped-and-cancelled`, `unlisted-values`).
- `tst15_truncated_state`: 400 checks with the 300 read all passed: rollup `pending` and `expected` read `pending`, `success` `passing`, `failure` `failing`; a read item not completed reads `pending` whatever the rollup; `total` is 400 and `completed` 300. 300 of 300 isn't truncated, 300 of 301 is (MOD-7, TST-8 `many-checks`).
- `tst15_status`: one case per row of MOD-11, and one per pair of adjacent rows whose conditions can hold together (gone with a conflict; conflict with failing checks; pending checks with changes requested; changes requested on a draft; a draft needing review; review required and blocked; blocked with `mergeable` unknown), each reading the earlier word. A failing draft reads `checks_failing`. `none` and `passing` unsettled read `checks_pending`.
- `mod5_pr_c_timeline`: the six samples of `c1.log` in order. At 11.1 s: `pending`, total 4, completed 3, failed 1 (`fail`), though the rollup says `failure`. At 57.5 s: total 5, completed 4, failed 2. At 88.3 s: `failing`, completed 5, failed 3 (`fail`, `slow`, `circitor/manual`), and `status` `checks_failing`.
- `mod2_an_older_read_is_not_applied`; `mod2_the_same_read_twice_makes_no_kinds` (a second read, same content, next `seq`, same send times).
- `mod4_queued_and_in_progress_swap_makes_no_kind`; `mod4_a_step_back_from_completed_is_a_checks_kind`; `mod4_a_rerun_with_a_new_id_is_a_checks_kind`; `mod14_a_url_change_alone_makes_no_kind`.
- `mod6_settles_at_20_s_with_checks_and_60_s_without`: false at 19.999 s and true at 20 s after the settle start; an empty list false at 59.999 s, true at 60 s; the read that sets the start is false; and it changes `status` from `checks_pending` to `ready` with a `status` kind (TST-8 `no-ci`).
- `mod6_shortcut`: `updated_at` 2 minutes before `github_date` is settled at once for `New` and `Loaded`; 25 s isn't, 26 s is; 65 s isn't with no checks, 66 s is; no `github_date`, never; never for `Known` (TST-8 `settled-at-once`).
- `mod6_a_new_head_an_old_head_back_and_a_reopening_start_again` (TST-8 `head-back`, `reopened`).
- `mod8_unknown_keeps_yes_for_60_s`: at 60 s kept, no kind; at 61 s `unknown` with a `mergeable` kind; a changed head or base reads `unknown` at once; with `sent_wall` earlier than `read_at` (a clock set back) kept (TST-8 `mergeable-unknown`, `base-changed`). `mod9_merge_state_follows`. `mod8_merged_reads_unknown_with_no_mergeable_kind`.
- `mod11_unlisted_values_read_on_the_safe_side`: a record that would read `ready` reads `unknown` with `mergeable` `foo`, and `review_required` with `review` `foo`; `mergeable` and `review` in the record are still `foo`.
- `mod11_gone`: `apply_gone` on open, merged and closed records; a gone record read open, merged and closed gives `reopened`, `merged`, `closed`.
- `sta5_new_is_a_baseline`: 5 comments on a `New` read make `[tracked]` only, and `seen.comment` holds the newest time and its IDs (TST-8 `baseline`). `sta5_same_time_other_id_is_new`. `sta5_nothing_seen_makes_every_item_new`. `sta5_loaded_reports_what_came_meanwhile`: saved open, read merged with 2 comments after `seen`: kinds `merged`, `activity` (2 items), `tracked`, in that order (TST-8 `restart`). `sta5_more`: both rules of `more` above.
- `mod12_paths`: a comment's `fetch`, a review's, `fetch_inline` only with `inline` above 0; `u5_an_item_has_no_text`: an item's JSON keys are MOD-12's and no others.
- `pol12_failed_items_keep_the_old_counts`: no `activity`, and `counts_rose` is still true for the next read.
- `mod15_edited_names_pr_title_base_in_that_order`; `mod1_names_compare_without_case`; `mod15_kinds_keep_the_declared_order`.
- You change no serde attribute and no field of a type: T02's unit tests in `src/model/` still pass, and so does `cargo test --features test-endpoint --test golden_types` where T41 is already merged.
- `cargo test --features test-endpoint --lib model::` and `make verify` pass.

## Traps
- **Never take `checks.state` from the rollup** outside MOD-7: it reads `FAILURE` while checks run (docs/02 §4, 78 s in `c1.log`).
- **`settled` moves only inside `apply_read`**, never by the clock, and by itself is no change: only the `status` it moves is.
- **Settling uses `sent_mono`, MOD-8 uses `sent_wall`, the shortcut uses GitHub's `Date`.** Three clocks, never compared with each other (POL-26).
- **`mergeable` going `yes` to `unknown` inside MOD-8's 60 s isn't a change:** diff the record after keeping, not the read.
- **A count that falls, or `review_threads` alone, makes no kind**, but the record takes the new numbers, so a later rise is seen from the lower value.
- **Don't glob-import `EventKind`:** its variants `Status`, `Checks`, `Mergeable` and `Review` share names with model types.
- `Record` derives `PartialEq` over every field: don't use `==` on whole records to decide what's visible.
- `record.rev` in `Merged::Applied` is the old one. Don't count revisions here.
- An unlisted PR `state` never reaches you: T07 turns that node into an error (MOD-2).

## Open questions
- **D6:** `blocked` and `behind` are built as (proposal), in one constant; a check's `required` isn't read (MOD-3). A measurement may change that row later.
- **`more`** (POL-12) and **the order of items** (MOD-15) were this brief's interpretations; the specs now say the same. Build them as written above.
- **A check run without `databaseId`, re-runs, `EXPECTED`, reopening** aren't measured (MOD-3, MOD-4, MOD-6; docs/02 §6). Build the spec as written.
- MOD-1, MOD-2, MOD-4, MOD-6 to MOD-9, MOD-11's order and MOD-12 carry (proposal): values in constants, no other hedge.
