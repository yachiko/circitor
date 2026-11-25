---
id: T41
title: "Golden files: the API's and the model's JSON, and GitHub's public schema"
kind: infra
milestone: M1
size: M
status: todo
depends_on: [T02]
requirements: [API-19]
owns: [testdata/api/, testdata/model/, testdata/github/schema.graphql, tests/golden_types.rs]
shares: []
---

# T41 · Golden files: the API's and the model's JSON, and GitHub's public schema

## Outcome
`testdata/api/` and `testdata/model/` hold one hand-written JSON file per response body, error and record, written from 04-api and 01-model, and a test keeps them equal to T02's types, key for key and in order; `testdata/github/schema.graphql` is GitHub's public schema. The CLI's rendering, the routes and the fake GitHub are built against these files instead of against each other.

## Context
T02 wrote the API's and the model's types from `plan/contracts/`. This task writes the same JSON a second time, **by hand from the specs**, and a test that holds the two against each other: that's the check that the types say what 04-api and 01-model say. The files then serve as the fake daemon for the CLI's tasks (T10, T24, T27, T35) and the expected shapes for the routes (T22, T23); the schema is what T07's queries and T14's fake GitHub are checked against.

## Read first
- `plan/conventions.md`: "JSON names", "Tests" (golden files; every file under `tests/` starts with the feature's `cfg`).
- `plan/contracts/api.md`: the body types, and in "Semantics" the list of files to write. `plan/contracts/model.md`: `Record`, `Checks`, `Item`, `Event`, `EventKind`. `plan/contracts/testkit.md`: `golden::check`.
- `specs/04-api.md`: API-2 to API-5 and API-19 in `#basics`; then each route's section for what its answer holds (API-6 to API-12, API-15, API-16). `specs/08-notify.md#subscriptions`: NTF-2 and NTF-3, for the two notify routes' answers.
- `specs/01-model.md`: MOD-2, MOD-3, MOD-5, MOD-8, MOD-11, MOD-12, MOD-15.
- `specs/03-state.md#cursors` (STA-9's form), `specs/06-operations.md#shipping` (OPS-11's version), `specs/README.md` "Conventions" (times, names).
- `docs/02-github-api.md` §4: values a real record holds (a merged PR's `mergeable`, `mergedAt` equal to `closedAt`).

## Scope
- **`testdata/api/`: every file `plan/contracts/api.md` lists** (API-19: this task's share is the files and the test; the types are T02's). One whole response body per file, as the daemon sends it: the envelope's `version`, `cursor` and `daemon`, then the route's keys; for an error, API-3's body. One `error-<code>.json` per code, named by the code as written (`error-unknown_route.json`).
- **Written by hand, from the spec's text.** Not serialised from the types, not produced by the update switch: the types are what's under test.
  - Layout: what `serde_json::to_string_pretty` writes (two spaces, one key a line, `[]` and `{}` for empty ones), and a final newline.
  - Key order: the spec's where it gives one (the envelope's three first; API-19's row; API-5's list; MOD-2's, MOD-3's and MOD-12's tables; an event as `rev`, `pr`, `at`, `kinds`), the type's elsewhere.
  - Values: times in whole seconds near 2026-11-24T22:00:00Z; a cursor of STA-9's form whose `rev` isn't below any `rev` in the body; a version of OPS-11's form; `octo/app#12`-style names; a 40-character `head_sha`; plain titles and messages of your own wording. **No marker string, no real login, no real repository.**
  - **Each record is one that could exist:** its `status` is MOD-11's word for its fields, its `checks` counts follow from its items (MOD-5), a merged one reads `unknown` twice (MOD-8) with `merged_at` equal to `closed_at`.
- **What each file must show** (beyond being a valid answer of its route):
  - `list.json`: two records in API-6's order, **without `checks.items`** and with `failed_items`; `untracked` and `pending` empty, `reset` false. `list-reset.json`: `reset` true, a name in `pending`.
  - `show.json`: a record with `checks.items`: a run with its `id`, a commit status with `id` `null`, one of them failed. `track.json`: `source` `manual`. `untrack-null.json`: `record` `null`.
  - `changes.json`: events that between them hold **every one of MOD-15's fourteen kinds**, each with what MOD-15 says it carries; one `activity` with a comment and a review of `inline` 1 with its `fetch_inline` (MOD-12); `records` and `pending` empty.
  - `wait-satisfied.json`: `outcome` `satisfied`, `reason` `null`, the record with `checks.items` (API-19: every single `record` has them), one event. `wait-timeout-no-record.json`: `timeout`, `record` `null`, every list empty. `wait-reset.json`: `reset` true, `events` empty, and `records` holding the record **without `checks.items`** (API-19: a reset leaves them out), beside `record`, which has them.
  - `notify-registered.json` (NTF-2): `registered` true, `replaced` false, `expires_at` a day after the answer's times, `outcome` and `reason` `null`, a record with a running check, every list empty. `notify-already.json`: `registered` false, `expires_at` `null`, `outcome` `satisfied`, a record whose one check failed (`checks.state` `failing`, settled), one event. `notify-cancelled.json`: `cancelled` 1. **No file holds a `socket` or a `token`:** no answer does (NTF-9).
  - `refresh.json`: `polled` and a cost. `status.json`: API-5's keys, `subscriptions` and `gh_path` among them, no problem, `warnings` empty. `status-problem.json`: a `daemon.problem` with its `message` and `until`, and two warnings of API-5's table.
  - The errors: `until` set for `github_unavailable` and `rate_limited`, `null` for the rest (API-3). `error-no_session.json`'s message names a system error of your own wording (`No such file or directory (os error 2)`).
- **`testdata/model/`**: `record-open.json` (open, checks with items, one failed) and `record-merged.json`, each a bare record.
- **`testdata/github/schema.graphql`**: GitHub's public schema, downloaded once, with no token and no header of your own: `curl -fsSL https://docs.github.com/public/fpt/schema.docs.graphql -o testdata/github/schema.graphql` (plan decision P6; about 1.5 MB). **If the download fails, report it and stop that part. Never write, trim or patch a schema by hand.** Put the date and the file's `sha256sum` in your report.
- **`tests/golden_types.rs`**: the tests under Acceptance, with one table of `(file, type)` that every test walks.

## Interfaces
- Consumes: `api::types::{Envelope, ListBody, RecordBody, ChangesBody, WaitBody, NotifyBody, CancelBody, RefreshBody, StatusBody, ErrorBody, ErrorCode}`, `model::Record`, `testkit::golden::compare` (T02); `apollo-compiler` (T01's dependency set).
- Provides: the files, at the paths api.md names. T10, T24, T27 and T48 feed them to the CLI; T22, T23 and T47 compare their answers' keys with them; T07 and T14 read the schema. **After this task a changed file is a contract change.**

## Out of scope
- The types, `golden::check`, anything under `src/`: T02's. **A file and a type that disagree are not yours to reconcile** (see Traps).
- Rendered text for these answers: T10 (`tests/golden/render/`). Its extra inputs (each stale mark, each outcome line, odd titles) are T10's own, under its directory.
- `testdata/config/`: T06. `testdata/state/`: T16. `testdata/github/responses/` and `recorded/`: T07, T37. `tests/cost/`: T14.
- Parsing queries against the schema, and `testkit::github::schema()`: T07, T14.

## Acceptance
`tests/golden_types.rs`, starting with `#![cfg(feature = "test-endpoint")]`:
- `api19_files_and_types_agree`: each file decodes into its type, and the type's pretty JSON plus a newline equals the file, through `golden::compare`, which never writes. A string comparison: it proves every key, every value and the order.
- `api19_every_file_has_its_rows_keys`: read as plain JSON, each file's top-level keys are the envelope's three plus exactly API-19's row for its route (for `status.json`, API-5's keys; for the two `notify-` files, the row's ten, `replaced` among them); an error file has `version` and `error` with `code`, `message`, `until`.
- `api19_empty_lists_and_flags_are_written`: `list.json` has `untracked: []`, `pending: []`, `reset: false`; `wait-timeout-no-record.json` has `reason: null`, `record: null`, `events: []`, `records: []`, `more_events: false`, `reset: false`; `untrack-null.json` has `record: null`; `notify-registered.json` has `outcome: null`, `reason: null`, `events: []`, `records: []`, `more_events: false`, `reset: false`, and `notify-already.json` has `expires_at: null`; `status.json` has `subscriptions: 0`, `warnings: []` and `daemon.problem`, `message`, `until` all `null`.
- `api3_one_error_file_per_code`: the nine files exist, each one's `error.code` is the code in its name, and `until` is set only for `github_unavailable` and `rate_limited`.
- `mod15_changes_holds_every_kind`: the fourteen `kind` words each appear in `changes.json`; no event has an `id` key; the review item has `inline` 1 and a `fetch_inline` ending `/comments`, the comment item has neither `inline` nor `review_state`.
- `api6_list_leaves_items_out_and_show_has_them`: no record of `list.json` has `checks.items`, every one has `failed_items`; `show.json`, `wait-satisfied.json`, `notify-registered.json` and `notify-already.json` have `items` in `record`; `wait-reset.json` has them in `record` and not in `records` (API-19).
- `ntf9_no_answer_holds_a_session`: no file under `testdata/api/` has a key `socket` or `token` at any depth.
- `every_file_is_in_the_table`: a file in `testdata/api/` or `testdata/model/` that the table doesn't name fails the test, and so does a row of the table with no file. The table holds every name of api.md's list.
- `p6_schema_validates`: the downloaded file validates as a schema with `apollo-compiler`, and has the type `PullRequest`, the field `statusCheckRollup` on `Commit`, and `nodes` and `viewer` on `Query`.
- **By hand, said in your report:** each file read once more against its spec section, naming the section per file.
- `cargo test --features test-endpoint --test golden_types` and `make verify` pass.

## Traps
- **These files are never rewritten from the types.** Use `golden::compare`, which doesn't write, not `golden::check`: under `CIRCITOR_UPDATE_GOLDEN=1` the latter would write what the types say over what the spec says, and the check would be gone.
- **When a file and a type disagree, find out which one the spec backs.** Your slip: fix the file. The type's (a missing key, another name, a key left out when empty, an order the spec gives and the type breaks): **leave both as they are**, mark that file's case `#[ignore = "T02: <what differs>"]`, and report it as a contract change. Don't bend the file to pass, and don't touch `src/`.
- **An order the spec doesn't give isn't a disagreement.** Follow the type's order and say where you did.
- `serde_json::Value` sorts its keys here (no `preserve_order`): compare key **sets** on values, and leave order to the string comparison.
- **`checks.items` and an item's `review_state`, `inline` and `fetch_inline` are the only keys that may be absent.** Every other `null`, `[]` and `false` is written out (API-19).
- A review item always has `inline`, also at 0; `fetch_inline` only above 0. A commit status has `"id": null`, not a missing `id`.
- Don't put the schema through a formatter, and don't open it in an editor that rewrites line ends: the file is evidence of what GitHub publishes.
- The download goes to docs.github.com, not to the API: no token, no `gh`, nothing about this project in the request.

## Open questions
- **API-19's key names, API-5's names and codes, and MOD-15's `kinds` as objects tagged `kind`** are (proposal). Write them as the specs and the contracts have them today; a later decision shows here as a diff.
- **The schema is a snapshot.** GitHub changes it without notice; a refresh is a new plan decision, never a quiet re-download.
- D5 doesn't touch this task. **D6:** no file needs a `blocked` record; if you write one, it's MOD-11's row as it stands.
