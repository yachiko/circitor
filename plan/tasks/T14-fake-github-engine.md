---
id: T14
title: "Fake GitHub: the scripted world and the GraphQL engine that answers from it"
kind: test
milestone: M1
size: L
status: in-progress
depends_on: [T02, T41]
requirements: [TST-3:it-answers-the-queries-circitor-sends, TST-3:by-default-its-world-behaves, TST-3:it-parses-each-query-and]
owns: [src/testkit/github/, tests/cost/]
shares: []
---

# T14 · Fake GitHub: the scripted world and the GraphQL engine that answers from it

## Outcome
`testkit::github` holds a scripted world of PRs that changes at scripted times and behaves as docs/02 §4 records, and `execute`, which parses a query, checks it against GitHub's public schema, answers only the fields asked, and costs it by GitHub's formula: each row of the measured table gets its measured cost.

## Context
The fake GitHub is two tasks. This one is everything that decides **what** an answer says: the world, the GraphQL execution and the cost. T20 wraps it in an HTTP server with headers, transport faults and the rules that fail a test. T26, T28 and every scenario task (T30 to T34) script this world, so its building methods are an interface. `plan/contracts/testkit.md` declares what T20 relies on: `schema`, `World::new`, `set_rate`, `now`, `advance_to`, `Executed`, `Rejected`, `execute`, `describe`, `Asked` and `AskedOf`. This task writes all of them, in its own directory; the rest is of its design. T41 gives the schema file. This task needs no clock: time is a `Time` the caller passes.

## Read first
- `plan/conventions.md`
- `plan/contracts/testkit.md`: the markers, "what T14 gives T20", **"What the world must be able to play (T14)"** (a checklist: every line is a requirement on your building methods), "Semantics" and "Don't assume". `plan/contracts/clock.md`: `Time` only.
- `specs/07-testing.md#test-doubles` TST-3, every part: three are this task's, and the others say what T20 will ask of the world. `#what-is-tested` TST-8: read every row for what a test must be able to script; TST-11 for the markers.
- `specs/02-polling.md#the-poll` POL-4, POL-6, POL-8, POL-12 and `#cadence` POL-16: the queries circitor sends.
- `docs/02-github-api.md` §2 (the formula and "Measured cost"), §3, §4 (every table: this is the world's default behaviour), §5 (error shapes).
- `docs/05-measurements.md`, 2026-11-24: PR A to PR E's timelines. `docs/measurements/2026-11-24/frag.graphql`, `c1.log`, `a2.log`.
- `testdata/github/schema.graphql` (T41).

## Scope
- **`execute` (TST-3's part "it answers the queries circitor sends").** Parse the POST body (`query`, `variables`), validate against the schema with `apollo-compiler`, then walk the validated selection sets and resolve each field from the world. Resolvers for what circitor's queries reach: `Query.viewer`, `rateLimit`, `nodes`, `node`, `repository`; `User.login`, `pullRequests`; `Repository.pullRequest`, `nameWithOwner`; `PullRequest` and its connections; `Commit.statusCheckRollup`; `CheckRun`, `StatusContext`; `IssueComment`, `PullRequestReview`. Fragments, inline fragments, aliases, `__typename`, variables and literal arguments all work. The headers of that part (`Date`, `x-ratelimit-*`) are T20's.
- **TST-3's part "it parses each query and…".** A document that doesn't parse is `Unparsable`; one the schema refuses is `NotInSchema`, naming the field; a mutation is `Mutation`. The answer holds only the fields asked, under the keys asked. A field the schema has but no resolver here is `NoResolver`, naming type and field, so a new query field fails loudly.
- **The cost.** docs/02 §2's formula, read as the planner checked it against every measured row: one request for each top-level connection, plus, for each parent node asked for (`first` or `last`, or the number of IDs given), one for each connection nested in it; a connection with neither `first` nor `last` isn't counted; divide by 100, round, at least 1. Offer it alone as well (`cost_of(request)`), because the `search` rows are costed, not answered.
- **`tests/cost/`:** one `.graphql` file per row of "Measured cost" (15 rows), `frag.graphql` among them, copied from the measurements. The test that reads them is a unit test in `src/testkit/github/`: nothing in `Cargo.toml` builds a test from `tests/cost/`.
- **The world (TST-3's part "by default its world behaves…").** PRs with every field of POL-6, checks (runs with a `databaseId`, commit statuses), comments, reviews and threads, in repositories with owners, under one login. Defaults from docs/02 §4, each row a behaviour: the rollup `null` with no checks, `PENDING` while any runs, `FAILURE` at once when one fails though others run, `SUCCESS` when all pass; `updatedAt` moved by a push, a comment, a draft change, a close or a merge, late by a review, never by a check; a merged PR's `mergeable` and `mergeStateStatus` `UNKNOWN`; a closed one keeps its last; `mergeStateStatus` `UNSTABLE` while a check runs or has failed; a thread reply is a review of its own; a PR leaves `viewer.pullRequests(states: OPEN)` the moment it ends.
- **Scripting.** Changes take effect at times on the world's clock: `advance_to` applies those due, in order. Design the building methods so a test plays every row of docs/02 §4 and every TST-8 scenario **without touching JSON**, and document each in rustdoc: they're what eight later tasks read. **`contracts/testkit.md`'s checklist is the minimum, line by line**; add draft changes, a title edit and a PR's creation at a time, which TST-8 also needs.
- **Three behaviours of TST-3's "misbehave" list are built here,** because they change the answer and not the transport: `mergeable` `UNKNOWN` for a number of reads, the old head served for a while after a push, a check read out of order. So are a PR's node answering `null` with `NOT_FOUND` or another error `type`, and an error with a path inside the node (docs/02 §5's shapes, for `nodes` and for `repository.pullRequest`). T20 exposes them with the rest of the list.
- **What T20 needs from the world:** the login (settable); `World::set_rate` and a way to spend a cost, with `execute` answering `rateLimit`'s `remaining` and `resetAt` from them and computing `cost`; `Executed.node_ids`; and, for its request log, what a request asked: whether it held a finding and that page's cursor, what was asked of each node (the record, the items, a check page), and a by-name lookup's owner, repository and number. That's `describe(request) -> Asked`, declared in testkit.md beside `execute`.
- **Paging** is by cursor over the world as it is at this call, so a change between two pages shifts them (TST-3). `first: 100` and `last: 20` are honoured, `totalCount` is the true count.
- **Markers (TST-11's fake side).** Every title holds `MARKER_TITLE`; every error `message` holds `MARKER_GITHUB_TEXT`; **every check's name and every login the world makes up holds `MARKER_CHECK` and `MARKER_LOGIN`** (TST-11, D9), the viewer's login included. A name or login a test gives is used exactly as given.

## Interfaces
- **Consumes:** `clock::Time`; `testkit::{MARKER_TITLE, MARKER_GITHUB_TEXT, MARKER_CHECK, MARKER_LOGIN}`; `apollo-compiler`; `testdata/github/schema.graphql` (T41).
- **Provides:** `testkit::github::{schema, World, Executed, Rejected, execute, describe, Asked, AskedOf}` as `plan/contracts/testkit.md` declares them; and, of this task's design, `cost_of` and the world's building methods.

## Out of scope
- The HTTP server, headers, transport faults, the rate-limit controls, the request log, the rules that fail a test, the settle probe: T20.
- Anything from `src/github/` or `src/model/`: the fake shares no type with them. A mismatch with T07's queries shows in T26, and is reported there.
- The harness: T29. The scenarios themselves: T30 to T33. TST-6's figures: T34.
- Behaviour docs/02 doesn't record (auto-merge, the merge queue, edited or deleted comments, `isRequired`): not in the fake until a measurement adds it.

## Acceptance
Unit tests in `src/testkit/github/`, with hand-written queries.
- `tst3_measured_costs`: each file of `tests/cost/` gets its row's cost. `viewer.pullRequests` with the full record at 10, 20, 30, 50, 100: **1, 1, 1, 2, 3**. `search`, full record, 50: **2**. The light list at 100: **1**. `id` and `updatedAt` only at 100: **1**. `nodes(ids:)` with the full record, 2 and 40 PRs: **1, 1**. The IDs-only list with `nodes` for 2 PRs: **1**. 40 PRs plus the last 10 comments' IDs: **2**; plus the last 10 reviews' IDs: **2**; plus the last 10 threads with 3 comments each: **6**; the full record plus `timelineItems(last: 20)`: **2**.
- `tst3_circitor_shapes_cost`: a list of 100 with 40 records (`contexts(first: 100)`, no review nodes) costs 1; 40 records with the last 20 comments and reviews cost 2 (POL-4); 40 PRs' items alone cost 1 (POL-12).
- `tst3_schema_is_checked`: `PullRequest.nope` is `NotInSchema` naming it; a valid mutation is `Mutation`; `{ viewer {` is `Unparsable`; a schema field with no resolver (`viewer { bio }`) is `NoResolver` naming it.
- `tst3_only_what_is_asked`: `{ viewer { login } }` answers exactly `{"data":{"viewer":{"login":…}}}`; an alias is the key; `__typename` is right for a run and a status.
- `tst3_defaults_follow_docs02`: PR C's timeline from `c1.log`: rollup `null`; then `PENDING` with `skipped` already `COMPLETED`/`SKIPPED`; then `FAILURE` with `slow` still `IN_PROGRESS`; a commit status `circitor/manual` in the same list as `PENDING`, then `FAILURE`; `slow` `CANCELLED`; `updatedAt` unchanged throughout. PR B merged: `MERGED`, `UNKNOWN`, `UNKNOWN`, gone from the open list, `mergedAt == closedAt`. A push, a comment, a draft change and a close each move `updatedAt`; a check doesn't.
- `tst3_scripted_times`: a change scheduled 3 s ahead isn't visible at 2.9 s and is after `advance_to` 3 s; two changes due together apply in the order scripted.
- `tst3_world_misbehaviours`: `UNKNOWN` for exactly 3 reads, then the value; after a push the old head and its old rollup for 3 s, then the new head with a `null` rollup (PR A, `dce9dbe` to `cf54ec4`); a run read `IN_PROGRESS`, then `QUEUED`, then `IN_PROGRESS`.
- `tst3_error_shapes`: an unknown ID among two gives `null` at its index, one `NOT_FOUND` whose `path` is the field's key and the index, and the other node; a PR number and a repository that don't exist give docs/02 §5's two paths; a scripted `FORBIDDEN` on one node leaves the others; a scripted error inside a node has a path that goes into it.
- `tst3_paging`: 150 open PRs give 100 with `hasNextPage` and then 50, newest `updatedAt` first; a PR closed between the two calls shifts the second page; 187 checks are two pages with `totalCount` 187; `comments(last: 20)` of 25 gives the newest 20.
- `tst3_checklist`, one small case per line of `contracts/testkit.md`'s checklist, built only with the world's methods. Among them: a force-push back to an earlier SHA shows its old checks; a run concluding `FOO` is answered as `FOO`; 100 checks are one page and 101 two, 300 and 301 and 400 report their `totalCount`, with the tail's state and the rollup as chosen; `updatedAt` 2 minutes behind the world's clock; `UNKNOWN` for 3 reads on every PR at once; a renamed and a transferred repository keep their node IDs; a hidden PR leaves the open list, answers `NOT_FOUND` by node and by name, and comes back with its ID, open and merged; an error `type` on one node until cleared; worlds of 41, 150, 200 and 205 PRs with distinct `updatedAt`; two PRs on one head branch, one by another author; a change set to repeat, 26 PRs flipping `draft` every 30 s.
- `tst3_what_was_asked`: for a poll's request, the description says it held a finding with no cursor and names each node with what was asked of it; for a lookup, the owner, repository and number.
- `tst11_markers`: every title in an answer holds `MARKER_TITLE`, every error message `MARKER_GITHUB_TEXT`, every check's name and login left to the world `MARKER_CHECK` and `MARKER_LOGIN`; a check the test names `build` is `build`.
- `cargo test --features test-endpoint --lib testkit::github::` and `make verify` pass.

## Traps
- **Cost follows the page size asked for, not the rows returned** (docs/02 §2): cost the query's arguments, never the world's size.
- **`nodes(ids:)` isn't a connection**, but each ID is a parent: 40 IDs with two nested connections are 80 requests, which is 1 point.
- `comments { totalCount }` without `first` or `last` costs nothing; `comments(last: 20)` costs one per parent.
- **The table's node counts say what each row's query held.** 3 280 for "plus the last 10 reviews' IDs" is a second, aliased `reviews(last: 10)`; 4 480 for the threads row is the record, the comments' IDs and the threads, without that second `reviews`. Write each file to match; only its cost is asserted.
- **`execute` takes `&World`.** Behaviours that count reads (`UNKNOWN` for N reads, an out-of-order read) need interior counters; `advance_to` is the only thing that moves time.
- An error's `path` uses the response key (the alias), as GitHub's does.
- The schema is about 1.5 MB: parse it once per process (`schema()`), never per call.
- The rollup reads `FAILURE` while checks still run: don't "fix" it to `PENDING`. That's the fact MOD-5 exists for.
- GitHub's clock only moves forward: `advance_to` with an earlier time does nothing.
- Keep the resolvers to what circitor asks. The schema has thousands of fields; a generic executor is out of size.

## Open questions
- **The formula is `documented`, its fit to the rows `observed`** (docs/02 §2). Whether exactly half a point rounds up isn't measured: round half up, in one place.
- **Deletion, lost access, a rename's node ID, a re-run and a review's late `updatedAt` aren't measured** (docs/02 §5, §6; MOD-1, MOD-4). Play them as the requirement that cites them assumes: `NOT_FOUND`, the same ID, a run back to `QUEUED` on the same head, about 10 s late.
- This is the plan's least certain size (plan README, "Risks"). If the world's builders won't fit beside the executor, say which part you'd split off; don't thin the acceptance.
