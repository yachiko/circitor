# Plan file formats

Contents:
1. Task files (`plan/tasks/Tnn-slug.md`): frontmatter, sections, a worked example
2. `plan/README.md`: the orchestrator's guide, with the generated blocks
3. `plan/conventions.md`: the rules every executor follows
4. `plan/contracts/`: one file per seam (how to design them: contracts.md)
5. The executor report

## 1. Task files

One file per task, named `T<nn>-<slug>.md`: `T07-state-store.md`. IDs run T01, T02, … roughly in the order tasks can start, and are never reused. A dropped task keeps its file with `status: dropped`.

### Frontmatter

`plan.py` reads a strict subset of YAML: `key: value`, flow lists `[a, b]` (they may wrap onto following lines), block lists (`key:` then `  - item` lines), and `#` comments after a space. Quote a value that contains `: ` or ` #`.

| Key | Required | Values |
|---|---|---|
| `id` | yes | `T07`; the file name starts with it |
| `title` | yes | Short and specific: "State store: revisions, the tracked set, merged TTL and the state file" |
| `kind` | yes | `infra` (scaffold, contracts, tooling), `code` (production code and its unit tests), `test` (test infrastructure and scenario suites), `integration` (wiring at a checkpoint, end-to-end checks), `manual` (the owner does it: a Mac, the live GitHub API, decisions) |
| `milestone` | no | `M1` … the checkpoint it serves |
| `size` | yes in practice | `S` (under ~300 lines with tests), `M` (~300–800), `L` (~800–1,500). Bigger: split it |
| `status` | yes | `todo`, `in-progress`, `review`, `done`, `blocked`, `dropped`. The orchestrator changes it with `plan.py status` |
| `depends_on` | yes | Task IDs that must be merged first; `[]` for none |
| `requirements` | yes | Requirement IDs (`STA-6`) or parts (`TST-4:state-file`; `plan.py show ID` lists an ID's parts). Every ID here is mentioned in the body |
| `owns` | yes | Paths the task creates or changes; a directory ends with `/`. No globs. Tasks that may run at the same time never overlap |
| `shares` | no | Files the task changes but doesn't own: an index line, a wiring call. The owner must come earlier in the graph; the body says exactly what changes |

### Sections

In this order, as `##` headings. `plan.py check` warns when a required one is missing.

| Section | What goes in it |
|---|---|
| **Outcome** (required) | One or two sentences: what works when this is merged, and at which boundary. Not a list of files. |
| **Context** | Where it sits: what the tasks before it give it, who uses what it builds. Two to four lines. |
| **Read first** (required) | Exactly what to read, each with why: `plan/conventions.md`, the `plan/contracts/` files it touches, spec files and sections (link with `#anchor`), sections of the facts docs, fixtures. Nothing "for background". |
| **Scope** (required) | What to build, as bullets that cite requirement IDs. Don't restate a rule the spec already states; say where the task interprets one, and how. Where a requirement is shared with another task, say which share is this task's. |
| **Interfaces** | Consumes: what it uses from contracts and earlier tasks. Provides: what later tasks use, matching `plan/contracts/`. A change goes through the orchestrator. |
| **Out of scope** (required) | The neighbouring work it must leave alone, with the task ID that does it. |
| **Acceptance** (required) | The tests to write and what each proves, with the spec's concrete values and the failure cases; the commands that must pass. Checkable in the worktree, without asking anyone. |
| **Traps** | What an executor would get wrong without being told: from the specs, the reviews, GitHub's behaviour and the platform. |
| **Open questions** | Open decisions, open review findings and unverified facts that touch the task, and what to build meanwhile. |
| **Evidence** | Added by the orchestrator on acceptance: date, commands and results, branch or PR. Not written by the planner. |

A body over ~150 lines is restating the spec, or it's two tasks.

### Worked example

Illustrative: the requirement IDs, the neighbours (T02, T05, T09, T11, T12, T14), the crate layout and the contract names are invented for the example. Check every ID against the current specs.

````markdown
---
id: T07
title: "State store: revisions, the tracked set, merged TTL and the state file"
kind: code
milestone: M1
size: L
status: todo
depends_on: [T02]
requirements: [STA-1, STA-2, STA-3, STA-4, STA-5, STA-6, STA-7, MOD-9, TST-4:state-file, TST-9:a-torn-write]
owns: [crates/state/]
shares: []
---

# T07 · State store: revisions, the tracked set, merged TTL and the state file

## Outcome
`circitor-state` holds every known PR in memory, raises a PR's revision only when something an agent can see changed, keeps merged and closed PRs until their TTL runs out, and saves the tracked set and those PRs to the state file so a restart loses nothing GitHub can't give back.

## Context
The poll scheduler (T09) applies each poll result from its own task; the API (T11) and the wait registry (T12) read snapshots and subscribe to changes. The daemon's wiring (T14) decides the directory and when to save at shutdown.

## Read first
- `plan/conventions.md`; `plan/contracts/model.md` (the PR record and what counts as a change), `plan/contracts/state.md`, `plan/contracts/clock.md`
- `specs/03-state.md`: STA-1 to STA-7; `specs/01-model.md#revisions`: MOD-9
- `specs/07-testing.md#state`: TST-4's state-file part and TST-9's torn write
- `docs/04-platform.md` §3: the state directory on macOS and Linux
- `testdata/state/`: the golden state files T02 created

## Scope
- Applying a result (STA-1, MOD-9): compare the incoming record with the cached one field by field, as the model contract lists them; bump the PR's revision only on a difference. `fetched_at` alone is not a change.
- The tracked set (STA-2, STA-3): PRs added by a client stay tracked until untracked or until their TTL after merge or close; tracking one twice is not an error.
- Merged and closed PRs (STA-4): kept for the configured TTL, measured on wall time from the clock contract, then dropped. A client asking for a dropped PR gets "not known", which the API (T11) turns into its response.
- The state file (STA-5, STA-6): what's written, its `version` field, and the atomic write: a temp file in the same directory, flushed and synced, then renamed over the old one.
- Loading (STA-7): a missing file is an empty state; a file with a newer version, or one that doesn't parse, is set aside under a dated name and the daemon starts empty, with a warning the status object carries.
- Change notification: a `watch` channel per the contract; this crate sends, it doesn't decide who waits.

## Interfaces
- Consumes: `model::PullRequest` and `model::visible_change`, `clock::Clock` (wall time for the TTL), the `StateDir` path type.
- Provides: `state::Store`, `state::Snapshot` and `state::Changes`, as declared in `plan/contracts/state.md`.

## Out of scope
- Deciding when to poll and what a forced refresh does: T09. Evaluating wait conditions: T12.
- HTTP responses and status codes: T11. Where the state directory is, and saving at shutdown: T14.
- The model's derived state: T05.

## Acceptance
- `crates/state/src/**` unit tests and `crates/state/tests/`, on a temp directory and the fake clock:
  - applying the same record twice leaves the revision unchanged; changing the head SHA, a check's conclusion or the review decision each raise it by one (MOD-9)
  - a PR merged at t stays readable until t + TTL and is gone at t + TTL exactly (STA-4), including when the wall clock jumps forward past it
  - save, drop the store, load: the tracked set and the merged PRs come back identical, and revisions never go below the saved ones (TST-4:state-file)
  - a write cut off after each step (temp file half written, synced but not renamed) leaves the previous state file readable (TST-9:a-torn-write)
  - `testdata/state/v1-newer-version.json` and `v1-truncated.json` are set aside and the store starts empty (STA-7)
- `cargo test -p circitor-state` and `make verify` pass.

## Traps
- The TTL runs on wall time, not on a monotonic timer: the laptop sleeps, and a monotonic clock doesn't count that (docs/04-platform.md, sleep and timers).
- `rename` is atomic only inside one file system: create the temp file next to the target, not in the system temp directory.
- Don't hold the store's lock across an `.await`: readers answer API requests from other tasks.
- Derive `Deserialize` for the state file without `deny_unknown_fields` only if STA-6 says older daemons must read newer files; today it says the opposite.

## Open questions
- STA-4's TTL default is marked (proposal). Build it as written, read from the config contract, in one constant.
````

## 2. plan/README.md

The orchestrator's entry point. Keep the generated markers exactly as written: `plan.py render` replaces what's between them, and nothing else.

````markdown
# circitor implementation plan

**Built from:** the specs as of <date> (draft <n>); the snapshot is in `graph.json`. **Scope:** <the whole system | prefixes>. **Tool:** `python3 -I .claude/skills/build-plan/scripts/plan.py <command>` (`plan.py` below).

<!-- plan:status -->
<!-- /plan:status -->

The orchestrator reads this file, `conventions.md` and `contracts/README.md`. An executor reads `conventions.md`, the contract files its task cites, and its task.

## Sources of truth
- `specs/` for behaviour; the facts docs (`docs/02-github-api.md`, `docs/03-clients.md`, `docs/04-platform.md`) for what GitHub, the clients and the platforms do; this plan for order, ownership and interfaces.
- A spec question is the user's. Nobody resolves one by editing a spec, a test or a contract on their own.

## Milestones
| Milestone | Checkpoint: what runs end to end | Integration task |
|---|---|---|
| M1 | … | T… |

## Running the swarm

### Each round
1. `plan.py ready` lists the tasks that can start, most urgent (longest remaining path) first.
2. Start up to <n> executors at once, one per task, each in its own git worktree (Agent tool, `isolation: "worktree"`), with the prompt below. Then `plan.py status Tnn in-progress`.
3. When an executor returns, set `review`, read its report, and check the diff: only its `owns` and `shares` paths changed; the contracts are kept; every requirement in its frontmatter is accounted for; the tests prove the acceptance rather than mirror the code. Run its acceptance commands and `make verify` in its worktree yourself.
4. Accepted: commit on a branch in the repository's style and merge in wave order, rebasing the later ones; resolve the hotspot files listed below; run `make verify` on main. Then `plan.py status Tnn done`, add an **Evidence** section to the task (date, commands and results, branch or PR), and `plan.py render`.
5. Not accepted: send the gaps back to the same executor (SendMessage keeps its context), or set `blocked` and write down why.

### Executor prompt
```text
Implement task <Tnn> of the circitor implementation plan in this git worktree.
1. Read plan/conventions.md, then plan/tasks/<file>, then everything its "Read first" section lists.
2. Build what its Scope says, inside the paths in its frontmatter `owns` (and `shares`, only as the task describes). Don't edit specs/, docs/ or plan/.
3. Meet every Acceptance item: write the tests it names, run the commands it lists and `make verify`, and fix what fails.
4. If a contract, the spec or another task's code stops you, don't work around it, and don't change behaviour to make a test pass: report it as an open item.
5. Don't commit, push or open pull requests.
6. End with the report in plan/conventions.md, "Report", and nothing else.
```

### Contract changes
An executor that needs a contract changed doesn't change it: it reports it. The orchestrator decides, updates `plan/contracts/` and the contract code (as a small follow-up task if the contracts task is done), and tells the running executors whose tasks use that seam.

### Worktree setup and limits
<one `target/` per worktree or a shared `CARGO_TARGET_DIR`, and what each costs in disk and waiting; how many executors the machine takes; anything else learned the hard way>

## Waves
<!-- plan:waves -->
<!-- /plan:waves -->

## Critical path
<!-- plan:critical -->
<!-- /plan:critical -->

## Dependency graph
<!-- plan:graph -->
<!-- /plan:graph -->

## Merge hotspots
<!-- plan:hotspots -->
<!-- /plan:hotspots -->

<How to resolve a conflict in each hotspot file: one line per entry, keep both, in wave order.>

## Plan decisions
Choices the specs leave open, made so that work can start. All are proposals until the user agrees.

| # | Decision | Why | Affects | Status |
|---|---|---|---|---|
| P1 | … | … | T… | proposal |

## Open questions and risks
<!-- plan:open -->
<!-- /plan:open -->

<For each open decision and finding above: what the tasks build meanwhile, and what changes if it's decided the other way.>

### Spec issues found while planning
<Contradictions, gaps and stale labels, with file:line. Not fixed here; `/harden` the file, or decide them.>

### Changes waiting on running tasks
<Spec changes that touch in-progress tasks (update mode only).>

## Outside the swarm
<The manual tasks, and anything only the owner can do: checks on a Mac, recordings and acceptance against the live GitHub API, a release to the Homebrew tap.>

## Change log
- <date>: plan created from the specs as of <date>.
````

## 3. plan/conventions.md

Short and binding. Everything in it applies to every task, so it's the one file every executor reads in full. Derive it from the specs (00-overview's technology and layout, 06-operations' build and platforms, 07-testing's rules, the cross-cutting rules) and the plan decisions. Cover:

- **Authority:** specs for behaviour, the facts docs for what GitHub, the clients and the platforms do, `plan/contracts/` for interfaces, the task for scope. A conflict or a gap is an open item in the report, not a decision for the executor. Never edit `specs/`, `docs/` or `plan/`.
- **Layout:** the repo layout of 00-overview, plus the plan's crates and modules, with what each holds.
- **Toolchain and commands:** the Rust toolchain (pinned in `rust-toolchain.toml`), `make verify` and exactly what it runs (`cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and the compile check for the macOS target if the plan decided on one), and how to run one crate's or one module's tests.
- **Code rules that come from the specs:** the rules the specs make absolute, each with its requirement ID. Expect these, and check each against the current specs: what the daemon may send to GitHub (queries only, if the specs say it never writes); all GitHub access through the client contract; all time from the injected clock, wall and monotonic kept apart; the state and config directories touched only through the one module that owns them; the token never logged, written to disk or put in an error; units and names across seams (specs/README conventions); what is logged, and when.
- **Platform rules:** the code is written on Linux and ships to macOS first, so nothing Linux-only without a `cfg` and a macOS path beside it; paths come from the directories the operations spec names; Unix socket paths are short (the limit is in docs/04-platform); no blocking call on the async runtime (`std::thread::sleep`, `std::process::Command`, file I/O in a hot path); no lock held across an `.await`.
- **Requirement tracing:** a `// STA-6:` comment at the code that implements a rule and above each test that proves it, and the ID in the test's name (`sta6_torn_write_keeps_the_old_file`). Rust test names can't hold a hyphen, so the comment is what `grep STA-6` finds.
- **Tests:** hermetic (in-process or loopback only; never api.github.com, never the machine's `gh` login or any real token: use the fake GitHub server, the fake token source and the recorded fixtures); deterministic (the fake clock, seeded randomness, no sleeps); sockets and state in a temp directory with a short path; golden files and snapshots in the crate's `testdata/` or `tests/`, rewritten by a documented switch, never by hand.
- **Dependencies:** only those the scaffold declared in `Cargo.toml`. A new one is a plan decision: report it, don't add it.
- **Git:** work in the worktree; no commits, pushes or pull requests.
- **Report:** the format in §5, copied in full.

## 4. plan/contracts/

One file per seam, plus `plan/contracts/README.md`: a table of the seams (seam, crate or path, producer task, consumer tasks, file). Each seam file has: what it's for; producer and consumers; the declarations (Rust, in code blocks, complete enough to compile once copied); the semantics (who calls it from which task or thread, what awaits or blocks, errors, invariants, each with its requirement IDs); the fake or fixture that ships with it; and what a consumer may assume about it before its producer is merged. How to find the seams and what to put in them: `contracts.md` in this directory.

## 5. The executor report

The executor ends with this, filled in, and nothing else. The orchestrator reads it before the diff.

```markdown
**Task:** Tnn · done | partly done | blocked
**Changed paths:** one per line
**Commands run:** each command, with its result (pass, or the failure in one line)
**Requirements:** each ID from the frontmatter → where it's implemented (path and function) and the test that proves it, or "not done: why"
**Deviations:** what was built differently from the brief or the contracts, and why; or "none"
**Contract changes needed:** the change, and which tasks it affects; or "none"
**Open items:** checks not run, spec gaps, questions for the orchestrator; or "none"
```
