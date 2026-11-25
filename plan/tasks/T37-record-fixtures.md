---
id: T37
title: "Owner: record real responses from yachiko/experiment"
kind: manual
milestone: M5
size: S
status: todo
depends_on: [T07]
requirements: [TST-12]
owns: [testdata/github/recorded/]
shares: []
---

# T37 · Owner: record real responses from yachiko/experiment

## Outcome
`testdata/github/recorded/` holds real, scrubbed responses for the queries circitor sends, recorded under docs/05's rules with the owner's OK, and the decode tests pass on them.

## Context
circitor's decoder was written from docs/02 and from synthetic responses (T07, `testdata/github/responses/`). No real response is recorded yet (docs/05 says so). This task is **the owner's**: record the answers GitHub really gives to the queries circitor sends, scrub them, and put them where T07's decode test picks them up. It's a live measurement, so CLAUDE.md's rules apply: `yachiko/experiment` only, docs/05's rules, and the owner's OK before the first request.

## Read first
- `CLAUDE.md`, "Never break": the rules for live measurements, and how a fact changes.
- `docs/05-measurements.md`: "Rules for a new entry", and "Left behind in `yachiko/experiment`" (PRs #11 to #15 are still there to read).
- `specs/07-testing.md#what-is-tested`: TST-12. `specs/02-polling.md`: POL-4, POL-6, POL-8, POL-12, POL-16 (the five queries).
- `plan/contracts/github.md`: the queries T07 builds, `request_body`, and **the fixture file's format** (Semantics, "A fixture file").
- `docs/02-github-api.md` §4, §5 and §6: what's verified, and the open items a recording can close.
- `testdata/github/responses/`: T07's synthetic files: the same format, to copy the `request` part from.

## Scope
**TST-12 is shared three ways.** T07: the synthetic fixtures and the test that decodes every file in both directories. T26: the same shapes come back from the fake GitHub. **This task: the real recordings**, in `testdata/github/recorded/`. TST-12 is a (proposal); it's done as written.

- **What to record**, each as GitHub's response to the request body circitor itself builds (`github::client::request_body`, printed by T07's `dump_queries`):
  1. A poll (POL-4): the finding with the refresh of two or more PRs by node ID.
  2. A first refresh with the last 20 comments and reviews (POL-4, POL-12): #15, which has reviews, a thread reply and a comment.
  3. The record of a PR with checks (POL-6): #13: passed, failed, skipped and cancelled check runs and a commit status.
  4. A merged PR (#12) and a closed one (#11), by node ID.
  5. A PR with no checks: the rollup `null` (#14, which also reads `CONFLICTING`).
  6. One made-up node ID among real ones: `null` with `NOT_FOUND` (POL-21).
  7. By name (POL-16): a PR that exists, PR number 9999, and a repository that doesn't exist.
  8. A 401: the same request with a made-up `gho_` token, sent with `curl`, never with `gh`.
- **Not recorded, and why:** a later check page (POL-8: no commit there has over 100 checks), an exhausted or a secondary limit, a 5xx. They stay synthetic.
- **One file per response, in the format of `plan/contracts/github.md`:** one JSON object, `{"request": {"find", "nodes", "lookup"}, "status", "headers", "body"}`, named `recorded-<what>.json`. `request` says which query the response answers, so T07's test can decode it as that answer. `headers` keeps only `date`, `retry-after` and the `x-ratelimit-*` ones. No other file goes in the directory.
- **Scrubbed** (TST-12, docs/05): logins, owner and repository names, titles, URLs, branch names, node IDs, `databaseId`s and SHAs are replaced, each real value by the same stand-in in every file. Enum values, times, counts, `null`s and the shape stay exactly as GitHub sent them.
- **A dated entry in `docs/05-measurements.md`**: what was sent, the points spent, anything that differs from docs/02, and that every answer carried a `Date` header (or which didn't). MOD-6 leans on that header, which docs/02 doesn't record yet.

## Interfaces
- **Consumes:** `github::client::request_body` through T07's `#[ignore]`d test `dump_queries`; T07's decode test; the fixture format of `plan/contracts/github.md`.
- **Provides:** `testdata/github/recorded/*.json`, read by T07's decode test (`tst12_every_fixture_decodes`, which walks `responses/` and `recorded/`). Nothing else reads them: T26's share of TST-12 is the fake's answers, not these files.

## Out of scope
- Changing the decoder, the queries or the synthetic fixtures: T07. A recording that doesn't decode is a finding for T07, never a reason to edit the recording.
- Changing the fake GitHub to match: T14.
- Measuring required checks and reviews (D6): no task measures it.
- New facts in `docs/02`: they follow from the dated entry, on the owner's say, not from this checklist.
- The acceptance checks on real PRs (TST-13): T40.

## Acceptance
The owner ticks each line.

- [ ] **OK given** for a live measurement in `yachiko/experiment`, today's date noted.
- [ ] **No other use of the token** while recording, and the budget checked first: `gh api rate_limit --jq .resources.graphql`.
- [ ] **The query bodies come from the code:** `cargo test --features test-endpoint --lib github::client -- --ignored --nocapture dump_queries` prints each request body (`query`, `variables`). Save the ones for the eight cases as `body-<n>.json`, with the node IDs and numbers of the next step filled in.
- [ ] **Node IDs looked up**, read-only: `gh api graphql -f query='{repository(owner:"yachiko",name:"experiment"){pullRequests(last:10){nodes{id number state}}}}'`.
- [ ] **Each body sent** from a scratch directory outside the repository: `gh api graphql --input body-<n>.json -i > raw-<n>.txt`. `-i` adds the response's status line and headers; nothing prints the token. `gh` exits non-zero when the answer holds `errors` (cases 6 and 7): that's expected.
- [ ] **The 401:** `curl -s -i -H 'Authorization: bearer gho_0000000000000000000000000000000000000000' -d @body-1.json https://api.github.com/graphql > raw-401.txt`.
- [ ] **For case 1**, if no PR is open in `yachiko/experiment`: open two throwaway PRs there (one file each, no workflow), record, close them unmerged, delete their branches.
- [ ] **Assembled and scrubbed** with one script (`jq`) into `testdata/github/recorded/recorded-<what>.json`: `request` (as the matching synthetic file writes it, with the scrubbed IDs and names), `status`, the kept `headers`, and `body`. The finding of case 1 is cut down to the `yachiko/experiment` PRs first: it lists the open PRs of every repository.
- [ ] **Nothing real is left:** `grep -rniE 'yachiko|experiment|circitor-measure|gho_|PR_kw' testdata/github/recorded/` finds nothing, and neither does a grep for each real title and login.
- [ ] **The shape survived:** for each file, `jq -S 'paths|map(tostring)|join(".")'` of the raw body and of the file's `.body` are equal (but for the PRs cut from case 1), and `jq '.headers|keys'` shows only `date`, `retry-after` and `x-ratelimit-*`.
- [ ] **They decode:** `cargo test --features test-endpoint --lib github::client` passes; its `tst12_every_fixture_decodes` (T07's) is the test that reads `recorded/`. A failure is written down with the file and the field, and reported: it becomes a follow-up to T07.
- [ ] **`make verify` passes.**
- [ ] **`docs/05-measurements.md` has a new dated entry**: setup, the eight cases with the PRs used, points spent (`rateLimit.cost` of each answer), the headers, what differs from docs/02, and what was left behind in `yachiko/experiment`. Raw files go in `docs/measurements/<date>/` only once scrubbed the same way; otherwise they're deleted.
- [ ] **Told the orchestrator** what was recorded, so the task gets its Evidence.

## Traps
- **`viewer.pullRequests` lists the owner's open PRs everywhere**, private repositories' names included. Nothing unscrubbed is saved inside the repository, and CLAUDE.md's "experiment only" covers what's opened or changed, so open nothing anywhere else.
- **Record circitor's query, not a hand-written one.** A response to another query proves nothing about the decoder: GraphQL returns exactly the fields asked.
- **Don't fix a recording.** If it doesn't decode, or it contradicts docs/02, the recording is the evidence. A fact in docs/02 changes only with the dated entry (CLAUDE.md).
- **Scrub consistently.** A node ID appears in the finding and in `nodes`; a repository in `nameWithOwner` and in every URL. One map from real to stand-in, applied to every file.
- **The `request` part must name the query that was really sent.** A fixture whose `request` says "a poll" over a by-name answer fails the decode test for the wrong reason.
- **Keep GitHub's key order and `null`s.** `jq` keeps order unless `-S` is given: use `-S` for comparing only.
- **`gh api --verbose` prints request headers.** Don't use it. Don't paste raw output into a chat or a commit message.
- **A workflow file goes only on a PR branch that is closed unmerged** (docs/05). No case here needs a new workflow: #13's checks are still readable on the closed PR.
- A made-up token answered 401 in the first measurements; with no `Authorization` header at all the answer was a 403 about the IP address's limit, which isn't the case wanted.

## Open questions
- **TST-12 doesn't fix the fixtures' format** (docs/05 leaves it to the testing spec). The plan's format is `plan/contracts/github.md`'s; a spec line may follow.
- **Whether a closed PR's check rollup is still returned** months later isn't measured. If #13 reads `null`, a new PR with a workflow on its branch is needed: closed unmerged, under docs/05's rules, with the owner's OK for the Actions minutes.
- **`docs/05-measurements.md` and `docs/measurements/<date>/` aren't in this task's `owns`.** They're the owner's to write; say so to the orchestrator.
- D5 and D6 don't touch this task.
