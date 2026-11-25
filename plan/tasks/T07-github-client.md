---
id: T07
title: "GitHub client: queries, the HTTP request, decoding, error classes, synthetic fixtures"
kind: code
milestone: M1
size: L
status: todo
depends_on: [T02, T41]
requirements: [POL-1, POL-6, POL-25, POL-24, MOD-3, MOD-2:a-value-github-sends-that, OPS-16, TST-12, TST-1]
owns: [src/github/client/, testdata/github/responses/]
shares: []
---

# T07 · GitHub client: queries, the HTTP request, decoding, error classes, synthetic fixtures

## Outcome
`github::client::Client` implements `GitHub::send`: it builds the one GraphQL query a `Query` describes, posts it to api.github.com (or, in a test build, to the loopback address of TST-1), and decodes the answer into `Answer` or a classified `Failure`, with partial errors kept per node. Every query it can build is valid against GitHub's schema, and every fixture decodes.

## Context
T02 gives the `GitHub` trait, `Query`, `Answer`, `Failure`, `Token` and the model's read types; T41 gives GitHub's schema. The engine's gate (T21, T25) is the only caller of `send`. The fake GitHub (T14, T20) reads these queries against the same schema but shares no type with this code: T26 is where the two first meet. T37 adds recorded responses beside the synthetic ones written here.

## Read first
- `plan/conventions.md`
- `plan/contracts/github.md`: every declaration, "Semantics" and "Don't assume". This task is its producer.
- `plan/contracts/model.md`: "what the GitHub client hands over" (`PrRead`, `ChecksRead`, `ItemRead`, `ItemsRead`), and `Mergeable`, `Review`, `CheckItem`, `Phase`.
- `specs/02-polling.md#requests` POL-1; `#the-poll` POL-4, POL-6, POL-8, POL-12, and `#cadence` POL-16: the fields each `Want` asks; `#errors` POL-21, POL-24, POL-25.
- `specs/01-model.md#identity-and-fields` MOD-2 (the "From GitHub" column, and "A value GitHub sends…"); `#checks` MOD-3; `#whats-new` MOD-12 (an item's fields).
- `specs/06-operations.md#shipping` OPS-16; `#the-daemons-life` OPS-9 ("Never logged"); `specs/07-testing.md#rules` TST-1; `#what-is-tested` TST-12.
- `docs/02-github-api.md` §1 (the endpoint), §2 (the cost formula), §4 (every value a field takes), §5 (error shapes).
- `docs/measurements/2026-11-24/frag.graphql` (the measured fragment), and `c1.log`, `a2.log`, `b1.log` for the fixtures' values. `testdata/github/schema.graphql` (T41).

## Scope
- **The builder.** One function turns a `Query` into the POST body `{query, variables}`. Hand-written strings with shared fragments, no code generation. Every query asks `rateLimit { cost remaining resetAt }`.
  - `find` (POL-4's finding): as `contracts/github.md` writes it, plus `pageInfo { hasNextPage endCursor }` and `after` when given. Per PR: `id number updatedAt repository { nameWithOwner }`.
  - `Want::Record` (**POL-6**): each GitHub field of MOD-2's table, `repository { nameWithOwner }`, the three `totalCount`s, and `commits(last: 1)` down to `statusCheckRollup { state contexts(first: 100) { totalCount pageInfo { hasNextPage endCursor } nodes { __typename … } } }` with MOD-3's fields for `CheckRun` (its `databaseId` included) and `StatusContext`. No comment or review IDs.
  - `Want::RecordAndItems` and `Want::Items` (POL-12's fields): `comments(last: 20)` and `reviews(last: 20)` with MOD-12's fields, a review's `comments { totalCount }` for `inline`.
  - `Want::ChecksPage` (POL-8): only the rollup's `state` and the `contexts` page after the cursor.
  - `lookup` (POL-16): `repository(owner:, name:) { pullRequest(number:) }` with the `RecordAndItems` selection.
  - **Nodes go through `nodes(ids:)` with one shared fragment per want**, each want under its own alias: the form POL-7's point was measured for. A `ChecksPage` has its own cursor, so each is its own aliased field.
- **POL-1.** The builder is the only code in circitor that writes a GraphQL document, and every document is one `query`.
- **MOD-3, whole.** The mapping table is this task's. Its last part too: no query asks `isRequired` (D6).
- **The request (OPS-16).** `POST`, `Authorization: bearer <token.expose()>`, `User-Agent: circitor/<version>` (the constructor's argument; `version::version()` in `for_tests`), a JSON body. `redirect::Policy::none()`. No timeout, retry or sleep of its own (the gate's, POL-2). The release endpoint is the constant `https://api.github.com/graphql`.
- **TST-1's share:** the endpoint. In a test build `Client::from_env` takes it from `CIRCITOR_TEST_GITHUB` and accepts only `http://127.0.0.1:<port>/graphql`; without the variable, or with anything else, it fails with an `EndpointError` naming the variable or the host. In a release build `env` isn't read. `for_tests` takes a URL the test made, uses no proxy, and panics on anything but that loopback form: no client of a test build can reach another host. T01 has the feature, T28 the daemon's other hooks.
- **Classification (POL-25, POL-24's share here),** in this order: HTTP 401 is `Unauthorized`; no `data` and `x-ratelimit-remaining: 0` is `Exhausted`, with `x-ratelimit-reset` (epoch seconds) when present; a 403, or a 200 with an error, whose message contains `secondary rate limit` (any case) is `Secondary`, with `Retry-After` in seconds; any other status but 200 is `Unavailable { Http }`; a body that isn't complete JSON is `BadBody`; a 200 without `data` is `NoData`; a connection or TLS failure is `Network`. `status` is `Some` whenever a status line arrived. **How the engine reacts** to each is T21's half of POL-24, and how long a limit holds is T08's half of POL-25.
- **Partial errors (POL-21's shape, POL-24).** A 200 with `data` is `Ok`. Match each error's `path` to its part by alias and index: a `null` node with `NOT_FOUND` is `NotFound`; a `null` node with another type, or a node with an error whose path points inside it, is `Error { type_ }` with GitHub's `type` as written. The other parts are returned. `viewer` and `rateLimit` follow `contracts/github.md`.
- **Decoding (MOD-2's part "a value GitHub sends that…").** This task's share is the mapping and lowering: unlisted enum values pass through lowered, a check's `phase` is always one of three, and one odd node never fails the answer. T05's share is what the derivations make of an unlisted value. The plan's words (P10, `contracts/github.md`): an unlisted PR `state` is `Error { type_: "UNLISTED_STATE" }`; a node of another type is `NOT_A_PULL_REQUEST`; a node that can't be decoded (a missing field, a wrong type, `null` with no error) is `UNDECODABLE`; an error without a `type` is `UNTYPED`.
- **Headers.** `Answer.date` from `Date`; `Rate.reset_at` from `resetAt`, else from `x-ratelimit-reset`.
- **Fixtures (TST-12's share here).** Synthetic responses in `testdata/github/responses/`, each `synthetic-*.json`, written by hand from docs/02 §4, §5 and the logs. One file has the layout `contracts/github.md` fixes: `request` says which `Query` the response answers (`find`: whether; `nodes`: one of `record`, `record_and_items`, `items`, `checks_page` per node, in order; `lookup`: whether), then `status`, `headers` (only `date`, `retry-after` and the `x-ratelimit-*` ones) and `body`, a string when it isn't JSON. One test decodes every file there and in `testdata/github/recorded/` (T37's, absent today). T26 has the end-to-end share.

## Interfaces
- **Consumes:** `github::{GitHub, Query, Find, NodeQuery, Want, Answer, Rate, Found, FoundPr, NodeAnswer, LookupAnswer, Failure, Unavailable}`, `token::Token`, `model::{PrRead, ChecksRead, CheckItem, ItemRead, ItemsRead, Mergeable, Review, PrName, NodeId}`, `clock::Time`, `config::Env`, `version::version`.
- **Provides:** `impl GitHub for Client`, `Client::from_env`, `Client::for_tests` and `request_body`, as `plan/contracts/github.md` declares them ("The real ones' constructors"); the inside of `EndpointError` is this task's, and never holds a token. `send` posts exactly `request_body(query)`.
- **For T37:** the fixture layout above, and `dump_queries` (Acceptance): the owner records real responses with the bodies it prints.

## Out of scope
- When and what to send, the 1 s gap, the 20 s timeout, request numbers, the repeat after a 401, and the reaction to each `Failure`: T21, T25. Quota states and the backoff: T08.
- `checks.state`, `status`, settling, kept mergeability, which items are new: T05.
- Reading the token: T12, T13. Building the daemon's client in `daemon::run::main`: T28.
- The fake GitHub and the cost test: T14, T20. Recorded fixtures: T37. `ScriptedGitHub` and `github::fake::answers`: T02.

## Acceptance
Unit tests in `src/github/client/`. `send` is tested against a few lines of axum on `127.0.0.1:0` inside the test; decoding is a pure function over status, headers and body, tested on the fixtures.
- `pol1_every_query_validates_and_is_a_query`: every shape the builder can make (each `Want` alone and all four mixed, 1 and 40 nodes, `find` with and without `after`, `lookup`, all three parts at once) parses and validates against `testdata/github/schema.graphql` with `apollo-compiler`, and is one `query` operation.
- `pol6_record_selection`: the `Record` selection names every GitHub field of MOD-2 and MOD-3, has `contexts(first: 100)`, and has no `isRequired` and no `nodes` under `comments` or `reviews`.
- **The request:** the stub sees `POST /graphql`, `authorization: bearer <testkit::MARKER_TOKEN>`, a `user-agent` starting `circitor/`, and a body equal to `request_body(query)`.
- **POL-25, POL-24, each a case:** 401 with `{"message":"Bad credentials","status":"401"}` is `Unauthorized`. A 403 and a 200 without `data`, with `x-ratelimit-remaining: 0` and `x-ratelimit-reset: 1795561200`, are `Exhausted { reset_at: 2026-11-24T23:00:00Z }`; without the reset header, `None`. A 403 saying `You have exceeded a secondary rate limit` with `Retry-After: 30` is `Secondary { Some(30 s) }`; a 200 whose error says `SECONDARY RATE LIMIT`, no header, is `Secondary { None }`. A plain 403, a 500 and a 502 are `Http` with their status. A 302 is `Http` with 302, and its `Location` (a second stub route) gets no request. A closed port is `Network`. `{"errors":[…]}` with `remaining` above 0 is `NoData`. `not json`, and a body cut before its `Content-Length`, are `BadBody`.
- **Fixtures, each decoded and asserted:**
  - a poll with two PRs: `found.login`, two `FoundPr`, `next: None`, two `Record`s, `rate.cost == 1`, `date` set
  - PR C at 22:26:36 (`c1.log`): five items, `pass` completed `success`, `fail` completed `failure`, `skipped` completed `skipped`, `slow` `in_progress` with no result, `circitor/manual` a status, `in_progress`, `id: None`; `rollup: Some("failure")`, `total: 5`, `mergeable: Yes`, `merge_state: "unstable"`, `head_sha` starting `180b372`
  - PR B merged (`b1.log`): `Merged`, `mergeable: Unknown`, `merge_state: "unknown"`, `merged_at == closed_at`, `rollup: None`, `total: 0`
  - §5's unknown ID among two nodes: `[Record, NotFound]`. A `FORBIDDEN` error with a path inside node 0: `[Error { "FORBIDDEN" }, Record]`
  - lookup: `NOT_FOUND` on `["repository","pullRequest"]`, and on `["repository"]`, are `LookupAnswer::NotFound`; `FORBIDDEN` there is `Error`
  - a `null` rollup; a checks page with `hasNextPage: true` (100 items, `next_page: Some`); items with a review carrying `comments.totalCount: 1` (`inline: Some(1)`, `review_state: Some("commented")`) and one with `submittedAt: null` (`at: None`)
  - an error on `viewer` (`found: None`, nodes kept); on `rateLimit` (`rate: None`); `resetAt` missing (the header is used)
- `mod2_unlisted_values`: `mergeStateStatus: HAS_HOOKS` is `has_hooks`; `mergeable: FOO` is `Other("foo")`; a run's `status: WAITING` is `Queued`; `conclusion: STARTUP_FAILURE` and `FOO` are `startup_failure` and `foo`; a status `EXPECTED` is `InProgress`, `ERROR` is completed `error`; `state: FOO` is `Error { "UNLISTED_STATE" }`; `__typename: Issue` is `NOT_A_PULL_REQUEST`; `author: null` is `None`; `reviewDecision: null` is `Review::None`. The other nodes of each answer decode.
- `tst12_every_fixture_decodes`: walks both directories; a missing `recorded/` is fine.
- `dump_queries`, marked `#[ignore]`: prints the request body the builder makes for each shape the owner records (a poll with a finding and two records; a first refresh with items; the record by ID; a check page; items alone; a lookup by name), each under a line naming it and the fixture `request` that goes with it. It asserts nothing; `cargo test … dump_queries -- --ignored --nocapture` must run.
- `tst1_endpoint`: in a test build, `from_env` with no variable is an error naming `CIRCITOR_TEST_GITHUB`; `http://127.0.0.1:4567/graphql` works; `https://api.github.com/graphql`, `http://localhost:4567/graphql`, `http://127.0.0.1/graphql` and `http://127.0.0.1:4567/other` are refused. `for_tests("https://api.github.com/graphql")` panics.
- `ops9_no_text_kept`: with `MARKER_GITHUB_TEXT` in the stub's 401 body and in every error `message`, the `Debug` of each `Failure` and `Answer` holds neither it nor `MARKER_TOKEN`.
- `cargo test --features test-endpoint --lib github::client::` and `make verify` pass.

## Traps
- **`comments { totalCount }` and `comments(last: 20) { … }` under one response key don't validate.** Alias the item connections. The same for `reviews`.
- **An error's `path` starts with the alias**, not with `nodes`. Map alias and index back to the `NodeQuery`'s place: `Answer.nodes` is always one per `Query::nodes`, in order.
- **The secondary-limit check is the only place an error `message` is read.** Compare it and drop it. Never keep, log or return a body or a message (OPS-9): `Error { type_ }` only.
- A check run's `result` is `None` until its phase is `completed`, whatever `conclusion` says. A context of an unknown `__typename` is skipped, not an error.
- The rollup is `null` with no checks (docs/02 §4): that's `total: 0`, `rollup: None`, never a failure.
- `frag.graphql` is the measured fragment, not circitor's: no `reviews(last: 10)`, `contexts(first: 100)`, a run's `databaseId`.
- `make verify` also lints without the feature: the test endpoint's code must be behind `cfg`, and nothing may be unused in the release configuration.
- The schema is about 1.5 MB: parse it once per test process.

## Open questions
- **POL-25's answers are `assumed`** (docs/02 §2, §6 item 6), and a 401 for an expired token too (§5). Build POL-25 as written; keep the secondary-limit phrase in one constant.
- **A check run's `databaseId` isn't measured** (MOD-4, docs/02 §6): it's in the schema. `EXPECTED` and results beyond the four verified ones are (proposal).
- **Proxies** (review H2-13, parked): the daemon's client keeps reqwest's default; OPS-16 doesn't say. `for_tests` uses none.
- **`UNLISTED_STATE`, `UNDECODABLE` and `UNTYPED`** are plan decision P10; docs/02 §5 records no such answer. Say in your report if a fixture needs another word.
- MOD-2's pass-through rule and MOD-3's `EXPECTED` are (proposal): built as written.
