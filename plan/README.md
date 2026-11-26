# circitor implementation plan

**Built from:** the specs as of 2026-11-25 (draft 0.2: notify and narrower conditions, D7 and D8; 01 to 07 hardened through the planning findings, 08-notify through pass 1); the snapshot is in `graph.json`. **Scope:** the whole system. **Tool:** `python3 -I .claude/skills/build-plan/scripts/plan.py <command>` (`plan.py` below).

<!-- plan:status -->
_Generated 2026-11-26._ **50 tasks:** 17 todo, 3 in-progress, 28 done, 2 blocked. **9 waves**, critical path weight 23. **Coverage:** 145 of 145 requirements.
<!-- /plan:status -->

The orchestrator reads this file, `conventions.md` and `contracts/README.md`. An executor reads `conventions.md`, the contract files its task cites, and its task.

## Sources of truth

- `specs/` for behaviour; the facts docs (`docs/02-github-api.md`, `docs/03-clients.md`, `docs/04-platform.md`, and `docs/07-notify.md` for Claude Code's session socket) for what GitHub, the clients and the platforms do; this plan for order, ownership and interfaces.
- A spec question is the user's. Nobody resolves one by editing a spec, a test or a contract on their own.

## Milestones

| Milestone | Checkpoint: what runs end to end | Integration task |
|---|---|---|
| **M1 Poll** | In one process on the test clock, the engine polls the fake GitHub through the real client. PRs are found and tracked with the right status and revisions; a push and a merge are seen as the specs say; an hour's points stay inside the budget. | T26 |
| **M2 Serve** | A whole daemon starts in STA-11's order and answers on a Unix socket in a temp directory. PRs come in through polls; waits end satisfied, ended and timed out; a client that disconnects leaves no waiter; a stop tells every session with an open subscription, through a recording poster, and leaves no socket and a valid state file. The demand side (T25) and the notify routes (T47) aren't needed yet. | T28 |
| **M3 CLI** | The harness runs every CLI command as a function against the in-process daemon and the fake GitHub, with output and exit codes; a command with no daemon starts one through the fake spawner; a waiter makes its PR hot; an untracked PR is read through; every command has a fake session socket in its environment. | T29 |
| **M4 Operate** | The built binary as real processes: the CLI starts a daemon that outlives it, two CLIs leave one daemon, an older daemon is replaced, the fake `gh`, the lock, the stale socket, permissions. The README and service files exist. | T36 (with T19, T03) |
| **M5 Harden** | Every TST-8 scenario, TST-6's quota figures and TST-7's golden output pass with no ignored test. **Notify runs end to end** (D7): a session registers through the CLI, a poll ends the subscription, and the fake session socket holds the confirmation and one notice. Then the owner's checks. | T30 to T35 and T49; then T37 to T40 |

M5 has no single integration task: it's reached when T30 to T35 and T49 are done **and no test is `#[ignore]`d**.

**Notify (D7) has no milestone of its own.** Its parts land where their kind lands: the conditions (T44), the notifier (T46) and its routes (T47) with M2, the fake session socket (T45) and the command (T48) with M3, its scenarios (T49) with M5. T28 wires the notifier in on the critical path; everything else of it runs beside. Each ignored test names the task whose code is wrong; the orchestrator turns each into a follow-up fix task (the next free ID, depending on the task it fixes).

## Running the swarm

### Each round

1. `plan.py ready` lists the tasks that can start, most urgent (longest remaining path) first.
2. Start up to **four** executors at once, one per task, each in its own git worktree (Agent tool, `isolation: "worktree"`), with the prompt below. Then `plan.py status Tnn in-progress`.
3. When an executor returns, set `review`, read its report, and check the diff: only its `owns` and `shares` paths changed; the contracts are kept; every requirement in its frontmatter is accounted for; the tests prove the acceptance rather than mirror the code. Run its acceptance commands and `make verify` in its worktree yourself.
4. Accepted: commit on a branch in the repository's style and merge in wave order, rebasing the later ones; run `make verify` on main. Then `plan.py status Tnn done`, add an **Evidence** section to the task (date, commands and results, branch or PR), and `plan.py render`.
5. Not accepted: send the gaps back to the same executor (SendMessage keeps its context), or set `blocked` and write down why.

**The first two tasks run alone.** T01, then T02: every other task starts from T02's commit. T43 (the API's shared parts and the fakes), T41 (the golden files) and T44 (the conditions) come straight after and unblock the fourth and fifth waves: start them first. Review T02 against `contracts/` line by line before anything else starts; a wrong declaration there is built on forty times. T02 leaves `unimplemented!("Tnn")` stubs: between T02 and T27 the binary panics on every command, which is expected.

**Commit before T01.** Whatever of `docs/`, `specs/`, `plan/` and `.claude/` is uncommitted goes in first: a worktree starts from a commit, and executors must be able to read the plan they're given.

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

An executor that needs a contract changed doesn't change it: it reports it. The orchestrator decides, updates `plan/contracts/` and the contract code (as a small follow-up task if T02 is done), and tells the running executors whose tasks use that seam. Expect a few after T02 and after T21: the store's and the poller's inside seams were designed on paper.

### Worktree setup and limits

- **One `target/` per worktree**, no shared `CARGO_TARGET_DIR`: a shared one makes builds wait for each other's lock. A debug build with every dependency is a few GB; remove a worktree's `target/` once its task is merged. The machine has 8 cores, 15 GB of memory and about 79 GB of free disk: four executors at once is the limit that leaves room for the linker.
- **`make verify` on main after every merge**, before the next wave starts from it.
- **The real-process tests (T36) take real seconds** and start processes: run that task alone. It depends only on T20, T27 and T28, but it's easier to debug once T29's in-process tests pass: when both are ready, start T29 first.
- **Executors have a real `gh` login in their shell.** Conventions forbid using it; check an executor's commands for `gh` and `api.github.com` when reviewing.
- **Executors run inside a Claude Code session, with its messaging socket in their environment.** A test that posts to `CLAUDE_CODE_MESSAGING_SOCKET` puts text into a live session, the executor's own or yours. Conventions forbid it; when reviewing T45 to T49, T28 and T29, check that no test reads the process's environment for it and that every post goes to a socket the test bound.

## Waves

<!-- plan:waves -->
| Wave | Task | Title | Kind | Size | Status | Depends on |
|---|---|---|---|---|---|---|
| 1 | [T01](tasks/T01-scaffold.md) | Scaffold: the package, every dependency, the toolchain, make verify | infra | S | done | – |
| 2 | [T02](tasks/T02-contracts.md) | Contracts as code: the module tree, every seam's declarations, and the small shared types | infra | L | done | T01 |
| 2 | [T03](tasks/T03-ci.md) | CI: format, lints and every test on Linux and macOS, and the four release targets | infra | S | blocked | T01 |
| 2 | [T50](tasks/T50-check-macos-zig.md) | Follow-up to T01: check-macos from Linux through cargo-zigbuild, last step of make verify | infra | S | done | T01 |
| 3 | [T04](tasks/T04-clock-log.md) | Clock, test clock, work in flight, sleep detection, logger and warnings | code | M | done | T02 |
| 3 | [T05](tasks/T05-model.md) | Model: checks summary, status, settling, kept mergeability, new items, and the merge that says what changed | code | L | done | T02 |
| 3 | [T06](tasks/T06-config-paths.md) | Config file, environment and paths | code | M | done | T02 |
| 3 | [T08](tasks/T08-poller-limits.md) | Poller limits: the quota count, the quota states and the backoff | code | M | done | T02 |
| 3 | [T09](tasks/T09-poller-cadence.md) | Poller cadence: tiers, periods, hot PRs, the forced-refresh windows and staleness | code | M | done | T02 |
| 3 | [T11](tasks/T11-cli-args-naming-guide.md) | CLI arguments, naming a PR, and the guide for agents | code | M | done | T02 |
| 3 | [T38](tasks/T38-first-ci-run.md) | Owner: the first CI run, on Linux and macOS | manual | S | blocked | T03 |
| 3 | [T41](tasks/T41-golden-files.md) | Golden files: the API's and the model's JSON, and GitHub's public schema | infra | M | done | T02 |
| 3 | [T43](tasks/T43-api-shared-fakes.md) | The API's shared parts and the simple fakes | infra | M | done | T02 |
| 3 | [T44](tasks/T44-conditions.md) | Conditions: checking and evaluating a wait's or a subscription's condition, once | code | M | done | T02 |
| 4 | [T07](tasks/T07-github-client.md) | GitHub client: queries, the HTTP request, decoding, error classes, synthetic fixtures | code | L | done | T02, T41 |
| 4 | [T10](tasks/T10-cli-render.md) | CLI text output: every line of list, show, changes, wait, status, refresh and stop, with golden files | code | L | done | T02, T41 |
| 4 | [T12](tasks/T12-token-source.md) | Token source: running gh auth token | code | M | done | T04 |
| 4 | [T13](tasks/T13-poller-auth.md) | Poller auth: when the token is read, and the auth state | code | M | done | T04, T43 |
| 4 | [T14](tasks/T14-fake-github-engine.md) | Fake GitHub: the scripted world and the GraphQL engine that answers from it | test | L | done | T02, T41 |
| 4 | [T15](tasks/T15-state-store.md) | State store: the tracked set, applying refreshes, revisions and events, retention, cursors | code | L | done | T04, T05, T43 |
| 4 | [T16](tasks/T16-state-file.md) | State file: the format, atomic writes through a fakeable file system, loading, the saver | code | M | done | T04 |
| 4 | [T17](tasks/T17-transport.md) | Transport: the HTTP client over the Unix socket, and the socket probe | code | S | done | T04, T43 |
| 4 | [T18](tasks/T18-daemon-instance.md) | Daemon instance: the lock, the lock file, the stale socket, binding | code | M | done | T04 |
| 4 | [T19](tasks/T19-readme-service-files.md) | README, service files and sandbox settings | infra | S | todo | T11 |
| 4 | [T45](tasks/T45-fake-session.md) | Fake session socket: what a Claude Code session's inbox looks like from outside | test | S | todo | T04 |
| 5 | [T20](tasks/T20-fake-github-server.md) | Fake GitHub: the HTTP server, faults, rate-limit controls, the rules that fail a test | test | L | done | T04, T14 |
| 5 | [T21](tasks/T21-engine-poll.md) | Poller engine: the loop, the gate, the poll, errors, the tick | code | L | done | T08, T09, T13, T15 |
| 5 | [T22](tasks/T22-api-routes.md) | API routes: list, show, changes, track, untrack, refresh, status, shutdown, and starting up | code | L | done | T09, T15, T41 |
| 5 | [T23](tasks/T23-api-wait.md) | Waiters: the wait route, its conditions, outcomes, deadline and clean end | code | L | done | T09, T15, T41, T44 |
| 5 | [T24](tasks/T24-cli-conn.md) | CLI and the daemon: connecting, starting, replacing, stopping, request limits | code | L | done | T06, T17, T18, T41 |
| 5 | [T37](tasks/T37-record-fixtures.md) | Owner: record real responses from yachiko/experiment | manual | S | todo | T07 |
| 5 | [T46](tasks/T46-notify-core.md) | Notify: subscriptions, the loop that evaluates them, the poster, and every notice's words | code | L | done | T09, T10, T15, T43, T44 |
| 6 | [T25](tasks/T25-engine-demand.md) | Poller engine: hot PRs, forced refreshes, reading through, track, the mergeable follow-up | code | L | done | T21 |
| 6 | [T26](tasks/T26-m1-poll.md) | M1: the real client against the fake GitHub, polling end to end | integration | M | in-progress | T07, T20, T21 |
| 6 | [T27](tasks/T27-cli-commands.md) | CLI commands: run, each command, the wait's deadline and reconnect, exit codes | code | L | in-progress | T10, T11, T24 |
| 6 | [T28](tasks/T28-daemon-run.md) | M2: the daemon runs: start order, serving the socket, modes, signals, stop, idle exit | integration | L | in-progress | T06, T07, T12, T16, T17, T18, T21, T22, T23, T46 |
| 6 | [T47](tasks/T47-api-notify.md) | Notify routes: registering a subscription and cancelling it | code | M | todo | T09, T41, T46 |
| 7 | [T29](tasks/T29-harness.md) | M3: the harness, and the CLI against a running daemon | integration | L | todo | T07, T20, T25, T26, T27, T28, T45 |
| 7 | [T36](tasks/T36-real-processes.md) | M4: real processes: spawning, two CLIs, upgrades, signals, files and permissions | integration | L | todo | T20, T27, T28 |
| 7 | [T42](tasks/T42-engine-pr-errors.md) | Poller engine: a PR's errors: gone, pr_failing, and its one log line | code | M | todo | T25 |
| 7 | [T48](tasks/T48-cli-notify.md) | CLI: the notify command | code | M | todo | T27, T46 |
| 8 | [T30](tasks/T30-scenarios-checks.md) | Scenarios: checks and settling | test | L | todo | T29 |
| 8 | [T31](tasks/T31-scenarios-waits.md) | Scenarios: a PR's life and waits | test | L | todo | T29, T42 |
| 8 | [T32](tasks/T32-scenarios-tracking.md) | Scenarios: tracking and polling | test | L | todo | T29, T42 |
| 8 | [T33](tasks/T33-scenarios-daemon.md) | Scenarios: the daemon's life and failures | test | L | todo | T29 |
| 8 | [T34](tasks/T34-quota-tests.md) | Quota tests: what an hour costs in each tier and state | test | M | todo | T29 |
| 8 | [T35](tasks/T35-golden-cli.md) | Golden output end to end: every command's text and JSON, every exit code | test | L | todo | T29, T42, T47, T48 |
| 8 | [T39](tasks/T39-release.md) | Owner: set up the release tool, publish to the tap | manual | M | todo | T19, T36 |
| 8 | [T49](tasks/T49-scenarios-notify.md) | Scenarios: notify | test | L | todo | T29, T42, T47, T48 |
| 9 | [T40](tasks/T40-acceptance.md) | Owner: the acceptance checks on a Mac and on Linux | manual | M | todo | T19, T36, T39, T47, T48 |
<!-- /plan:waves -->

## Critical path

<!-- plan:critical -->
T01 (S) → T02 (L) → T05 (L) → T15 (L) → T21 (L) → T28 (L) → T36 (L) → T39 (M) → T40 (M)

Weight 23 of 115 (S = 1, M = 2, L = 3). Start these first, and keep them small: every task on this path delays the end by its full length.
<!-- /plan:critical -->

The path runs through the model, the store and the engine's poll, each an L task that the next needs for real, then the daemon's wiring and the real-process tests. The engine's demand side (T25, T42), the routes and waiters (T22, T23), the CLI (T10, T11, T24, T27) and the fake GitHub (T14, T20) run beside it. **Notify adds no weight to it:** T28 needs the notifier (T46), which is as long as the engine's poll and runs beside it, on T15, T10 and T44; the notify routes, the command and the scenarios (T47, T48, T49) are off the path. That's why the conditions are a task of their own (P16): had the notifier waited for the wait route, the path would be three longer. The harness (T29) and the scenarios end one L task later than T36; the owner's T39 and T40 close the path. Nothing on it waits for an implementation where a contract would do.

## Dependency graph

<!-- plan:graph -->
```mermaid
flowchart LR
  T01["T01 Scaffold: the package, every depe…"]:::done
  T02["T02 Contracts as code: the module tre…"]:::done
  T03["T03 CI: format, lints and every test…"]:::blocked
  T04["T04 Clock, test clock, work in flight…"]:::done
  T05["T05 Model: checks summary, status, se…"]:::done
  T06["T06 Config file, environment and paths"]:::done
  T07["T07 GitHub client: queries, the HTTP…"]:::done
  T08["T08 Poller limits: the quota count, t…"]:::done
  T09["T09 Poller cadence: tiers, periods, h…"]:::done
  T10["T10 CLI text output: every line of li…"]:::done
  T11["T11 CLI arguments, naming a PR, and t…"]:::done
  T12["T12 Token source: running gh auth tok…"]:::done
  T13["T13 Poller auth: when the token is re…"]:::done
  T14["T14 Fake GitHub: the scripted world a…"]:::done
  T15["T15 State store: the tracked set, app…"]:::done
  T16["T16 State file: the format, atomic wr…"]:::done
  T17["T17 Transport: the HTTP client over t…"]:::done
  T18["T18 Daemon instance: the lock, the lo…"]:::done
  T19["T19 README, service files and sandbox…"]
  T20["T20 Fake GitHub: the HTTP server, fau…"]:::done
  T21["T21 Poller engine: the loop, the gate…"]:::done
  T22["T22 API routes: list, show, changes,…"]:::done
  T23["T23 Waiters: the wait route, its cond…"]:::done
  T24["T24 CLI and the daemon: connecting, s…"]:::done
  T25["T25 Poller engine: hot PRs, forced re…"]:::done
  T26["T26 M1: the real client against the f…"]:::active
  T27["T27 CLI commands: run, each command,…"]:::active
  T28["T28 M2: the daemon runs: start order,…"]:::active
  T29["T29 M3: the harness, and the CLI agai…"]
  T30["T30 Scenarios: checks and settling"]
  T31["T31 Scenarios: a PR's life and waits"]
  T32["T32 Scenarios: tracking and polling"]
  T33["T33 Scenarios: the daemon's life and…"]
  T34["T34 Quota tests: what an hour costs i…"]
  T35["T35 Golden output end to end: every c…"]
  T36["T36 M4: real processes: spawning, two…"]
  T37["T37 Owner: record real responses from…"]:::manual
  T38["T38 Owner: the first CI run, on Linux…"]:::blocked
  T39["T39 Owner: set up the release tool, p…"]:::manual
  T40["T40 Owner: the acceptance checks on a…"]:::manual
  T41["T41 Golden files: the API's and the m…"]:::done
  T42["T42 Poller engine: a PR's errors: gon…"]
  T43["T43 The API's shared parts and the si…"]:::done
  T44["T44 Conditions: checking and evaluati…"]:::done
  T45["T45 Fake session socket: what a Claud…"]
  T46["T46 Notify: subscriptions, the loop t…"]:::done
  T47["T47 Notify routes: registering a subs…"]
  T48["T48 CLI: the notify command"]
  T49["T49 Scenarios: notify"]
  T50["T50 Follow-up to T01: check-macos fro…"]:::done
  T01 --> T02
  T01 --> T03
  T02 --> T04
  T02 --> T05
  T02 --> T06
  T02 --> T41
  T41 --> T07
  T02 --> T08
  T02 --> T09
  T41 --> T10
  T02 --> T11
  T04 --> T12
  T02 --> T43
  T04 --> T13
  T43 --> T13
  T41 --> T14
  T04 --> T15
  T05 --> T15
  T43 --> T15
  T04 --> T16
  T04 --> T17
  T43 --> T17
  T04 --> T18
  T11 --> T19
  T04 --> T20
  T14 --> T20
  T08 --> T21
  T09 --> T21
  T13 --> T21
  T15 --> T21
  T09 --> T22
  T15 --> T22
  T41 --> T22
  T02 --> T44
  T09 --> T23
  T15 --> T23
  T41 --> T23
  T44 --> T23
  T06 --> T24
  T17 --> T24
  T18 --> T24
  T41 --> T24
  T21 --> T25
  T07 --> T26
  T20 --> T26
  T21 --> T26
  T10 --> T27
  T11 --> T27
  T24 --> T27
  T09 --> T46
  T10 --> T46
  T15 --> T46
  T44 --> T46
  T06 --> T28
  T07 --> T28
  T12 --> T28
  T16 --> T28
  T17 --> T28
  T18 --> T28
  T21 --> T28
  T22 --> T28
  T23 --> T28
  T46 --> T28
  T04 --> T45
  T25 --> T29
  T26 --> T29
  T27 --> T29
  T28 --> T29
  T45 --> T29
  T29 --> T30
  T25 --> T42
  T29 --> T31
  T42 --> T31
  T29 --> T32
  T42 --> T32
  T29 --> T33
  T29 --> T34
  T46 --> T47
  T27 --> T48
  T46 --> T48
  T29 --> T35
  T42 --> T35
  T47 --> T35
  T48 --> T35
  T20 --> T36
  T27 --> T36
  T28 --> T36
  T07 --> T37
  T03 --> T38
  T19 --> T39
  T36 --> T39
  T39 --> T40
  T47 --> T40
  T48 --> T40
  T29 --> T49
  T42 --> T49
  T47 --> T49
  T48 --> T49
  T01 --> T50
  classDef done fill:#d8f0dc,stroke:#2f7d3b
  classDef active fill:#fff1c2,stroke:#a07800
  classDef blocked fill:#f8d7da,stroke:#a12a35
  classDef manual stroke-dasharray: 4 3
```

Edges implied by others are left out. Green: done; yellow: in progress or review; red: blocked; dashed: manual.
<!-- /plan:graph -->

## Merge hotspots

<!-- plan:hotspots -->
No task shares a path with another.

Paths that pass from one task to a later one (`owns` of ordered tasks):

| Path | Tasks, in order |
|---|---|
| `Makefile` | T01 → T50 |
| `rust-toolchain.toml` | T01 → T50 |
| `src/` | T01 → T02 → T04 → T05 → T06 → T07 → T08 → T09 → T10 → T11 → T12 → T43 → T13 → T14 → T15 → T16 → T17 → T18 → T20 → T21 → T22 → T44 → T23 → T24 → T25 → T27 → T46 → T28 → T45 → T29 → T42 → T47 → T48 |
| `src/api/condition.rs` | T02 → T44 |
| `src/api/notify/` | T02 → T47 |
| `src/api/params.rs` | T02 → T43 |
| `src/api/request.rs` | T02 → T43 |
| `src/api/respond.rs` | T02 → T43 |
| `src/api/router.rs` | T02 → T43 |
| `src/api/server/` | T02 → T22 |
| `src/api/wait/` | T02 → T23 |
| `src/cli/args/` | T02 → T11 |
| `src/cli/commands/` | T02 → T27 |
| `src/cli/conn/` | T02 → T24 |
| `src/cli/fake.rs` | T02 → T43 |
| `src/cli/guide/` | T02 → T11 |
| `src/cli/naming/` | T02 → T11 |
| `src/cli/notify/` | T02 → T48 |
| `src/cli/render/` | T02 → T10 |
| `src/clock/` | T02 → T04 |
| `src/config/` | T02 → T06 |
| `src/daemon/instance/` | T02 → T18 |
| `src/daemon/run/` | T02 → T28 |
| `src/github/client/` | T02 → T07 |
| `src/github/fake.rs` | T02 → T43 |
| `src/lib.rs` | T01 → T02 |
| `src/log/` | T02 → T04 |
| `src/main.rs` | T01 → T02 |
| `src/model/` | T02 → T05 |
| `src/notify/` | T02 → T46 |
| `src/poller/auth/` | T02 → T13 |
| `src/poller/cadence/` | T02 → T09 |
| `src/poller/engine/` | T02 → T21 → T25 → T42 |
| `src/poller/fake.rs` | T02 → T43 |
| `src/poller/limits/` | T02 → T08 |
| `src/state/file/` | T02 → T16 |
| `src/state/store/` | T02 → T15 |
| `src/status/` | T02 → T04 |
| `src/testkit/github/` | T02 → T14 |
| `src/testkit/harness/` | T02 → T29 |
| `src/testkit/server/` | T02 → T20 |
| `src/testkit/session/` | T02 → T45 |
| `src/token/fake.rs` | T02 → T43 |
| `src/token/gh/` | T02 → T12 |
| `src/transport/` | T02 → T17 |
| `src/version.rs` | T01 → T02 |
| `tests/real/` | T01 → T36 |
<!-- /plan:hotspots -->

There are no shared files by design: T01 declares every dependency and test target, T02 writes every parent `mod.rs`, T43 the router and the fakes, and each later task owns leaf directories or files. Notify follows it: `src/notify/` is one leaf (T46), `src/api/condition.rs` one file (T44), and `src/api/notify/`, `src/cli/notify/` and `src/testkit/session/` a leaf each (T47, T48, T45). Places where tasks follow each other in one directory:
- `src/poller/engine/`: T21, then T25, then T42. Each starts from the one before's merge.
- `src/`: T02 creates the tree; later tasks own leaves. `testdata/` has one owner per directory: `api/`, `model/` and `github/schema.graphql` T41, `config/` T06, `state/` T16, `github/responses/` T07, `github/recorded/` T37.
- `tests/real/`: T01 writes a placeholder `main.rs` (its `[[test]]` entry needs the file); T36 replaces it.

If two tasks ever need one file, it's a contract change: stop and decide.

## TST-8's scenarios

`plan.py` doesn't split TST-8's table into parts (see spec issues), so T30 to T33 and T49 each list TST-8 whole and this table is the coverage map. 58 scenarios, each in one task, but for the subscription sentences of `sleep` and `clock-back`, which T49 plays beside T33's.

| Task | File | Scenarios |
|---|---|---|
| T30 | `tests/scn_checks.rs` | `push-old-head`, `no-ci`, `first-check-early`, `late-ci`, `one-fails-others-run`, `fail-fast`, `skipped-and-cancelled`, `commit-status`, `check-out-of-order`, `settled-at-once`, `many-checks`, `unlisted-values`, `head-back` |
| T31 | `tests/scn_waits.rs` | `merged`, `closed`, `reopened`, `conflict`, `mergeable-unknown`, `base-changed`, `head-moved`, `rerun`, `waiter-limit`, `new-review`, `changed-kind`, `baseline`, `gone-returns` |
| T32 | `tests/scn_tracking.rs` | `read-through`, `owners`, `renamed`, `repo-gone`, `pr-error`, `many-prs`, `track-untrack`, `forced`, `tiers`, `bare-pr`, `retention` |
| T33 | `tests/scn_daemon.rs` | `cursor-reset`, `github-down`, `token-rotated`, `sleep`, `restart`, `starting`, `first-run`, `restart-offline`, `crash`, `files-removed`, `clock-back` |
| T49 | `tests/scn_notify.rs` | `notify-outcome`, `notify-already`, `notify-ends`, `notify-session-gone`, `notify-stop`, `notify-keeps-active`, `notify-keeps-ended`, `notify-problem`, `notify-no-session`, `notify-register`; the subscription parts of `sleep` and `clock-back` |

## Plan decisions

Choices the specs leave open, made so that work can start. All are proposals until the user agrees. The costliest to change come first.

| # | Decision | Why | Affects | Status |
|---|---|---|---|---|
| P1 | **One package, `circitor`: a library and a binary, with 00-overview's `src/` layout.** Added modules: `clock`, `log`, `status`, `token`, `transport`, `testkit`, `version`. Parent `mod.rs` files are T02's and final; tasks own leaf directories. | It's the layout the specs propose and cite. Leaf directories keep `owns` disjoint without a workspace, and TST-1's `required-features` and `cargo test --features test-endpoint` assume one package. | every task | **agreed** 2026-11-25 |
| P2 | **The seams of `plan/contracts/`**: one `GitHub::send` for every request; a pure `model::apply_read`; the store as the one shared state, with waiters and naming requests as guards; the engine behind `PollerHandle`; `stale` computed when an answer is built. | They decide the task boundaries. Changing one after T02 is a follow-up for every task on that seam. | every task | **agreed** 2026-11-25 |
| P3 | **The poller is six tasks:** limits (T08) and cadence (T09) as pure functions, the token's life (T13), the loop and the poll (T21), demand (T25), and a PR's error runs with `gone` (T42). | One task would be over 3 000 lines. The pure parts are built beside the model and the store, and T42 beside the daemon's wiring, so only T21 and T25 are on the critical path. | T08, T09, T13, T21, T25, T42 | proposal |
| P4 | **Time is circitor's own `Clock` trait, never tokio's paused time.** The test clock fires timers in order and settles on a count of work in flight (`Activity`), which is always compiled. Wall time is `chrono`. Tests on the test clock use the current-thread runtime. | TST-4 describes this clock. Paused tokio time doesn't mix with the fake GitHub's real loopback I/O. | every task | **agreed** 2026-11-25 |
| P5 | **Two fakes for GitHub:** `ScriptedGitHub` behind the trait for unit tests, and the loopback HTTP server TST-3 describes (T14, T20), which shares no type with the client. | The server tests the real client's queries and decoding; the scripted fake keeps unit tests small. | T07, T14, T20, T21, T26 | **agreed** 2026-11-25 |
| P6 | **GitHub's public schema is downloaded once**, without a token, from `docs.github.com/public/fpt/schema.docs.graphql` into `testdata/github/schema.graphql` (about 1.5 MB), by T41. `apollo-compiler` (stable 1.x) parses and validates against it, in test builds only. | TST-3 needs the real schema; a hand-written one proves nothing. The fallback is `graphql-parser` with a hand-written field check. | T41, T07, T14 | **agreed** 2026-11-25 |
| P7 | **Synthetic GitHub responses stand in for recorded ones.** T07 writes them from docs/02 §4, §5 and the logs into `testdata/github/responses/`; the owner's recording (T37) adds `testdata/github/recorded/`. | docs/05 says no recorded fixtures exist, and executors can't call the API. No code task waits for T37. | T07, T26, T37 | proposal |
| P8 | **Dependencies:** `tokio`, `axum` 0.8, `hyper` 1 with `hyper-util`, `reqwest` with rustls, `serde`, `serde_json`, `toml`, `clap` (00-overview's list), plus `chrono`, `async-trait`, `futures`, `thiserror`, `nix`, `semver`, `fastrand`, `tower`, `http`, `http-body-util`, `bytes`; test builds add `apollo-compiler` and `tempfile`. Hand-written GraphQL strings with serde types; no code generation. No `anyhow`, no logging crate. | 00-overview asks the plan to confirm the list. Only T01 edits `Cargo.toml`. | T01, every task | proposal |
| P9 | **No CI for now** (the owner's choice, 2026-11-25). T03 and T38 are `blocked` until CI is wanted; `make verify` on main after every merge is the only gate. **macOS is compile-checked from Linux by `make verify`'s last step** (not linked: that needs Apple's SDK), through zig and cargo-zigbuild (T50, 2026-11-26); nothing runs on macOS before CI or the owner's Mac. TST-14 and TST-9's "CI's Linux and macOS runners" are unmet meanwhile: TST-9 runs on Linux only (T36). When CI is wanted, T03's brief is ready: a GitHub Actions workflow on Linux and macOS. | The owner deferred CI. The cost: macOS breakage shows only at T40. | T01, T03, T36, T38, T39 | **decided** 2026-11-25 |
| P10 | **Small gaps in the specs, filled so tasks agree** (each is also a spec issue below): the order "of `pr`" is owner, repository (lower-cased), then number as a number; an event's `kinds` is a list of objects tagged `kind`; a `log_unwritable` warning code; `gh_path` as a key of `status`; `paths.state` is `state.json`'s path; `last_poll.outcome` uses API-16's words; an unlisted PR `state` from GitHub makes that node an error `UNLISTED_STATE` (and a node that can't be decoded `UNDECODABLE`, an error without a type `UNTYPED`); a review without `submittedAt` is ignored; `status` doesn't count as a client request for the idle exit; a "replaced" stop is logged like any shutdown request; the socket is bound under umask 0177; every request of a poll marks its PRs as sent at the poll's own send time (else each poll takes two requests for good); the fake GitHub's per-daemon rules hold per connection for real processes; log lines start with a fixed event word (`contracts/log.md`). | Each is the smallest reading that lets two tasks build one thing. | T02, T05, T07, T09, T15, T18, T20, T21, T22, T28 | **in the specs** since the hardening of commit 042dae6 (MOD-1, MOD-2, MOD-12, MOD-15, API-5, OPS-4, OPS-8, OPS-9, POL-13), all but two: the fake GitHub now tells real processes apart by TST-1's `x-circitor-test-pid` header, not per connection; and the log's event words stay this plan's (proposal) |
| P11 | **Release tooling: D5 is decided, a formula written by `dist`** (owner, 2026-11-25; recorded in `specs/decisions.md` and OPS-10). Executors build the version stamp (T01), the release build of the four targets in CI (T03) and the README's install line (T19). The release workflow, the formula and the tap are the owner's (T39). | A release can't be tried from a worktree. | T01, T03, T19, T39 | **decided** 2026-11-25 |
| P12 | **Platform differences are values, not `cfg`:** code takes a `config::Platform`, so macOS paths compile and are tested on Linux. The one `unsafe` is the spawner's `setsid` in `pre_exec`. | Written on Linux, shipped to macOS first. | T06, T24 | proposal |
| P13 | **Logging is circitor's own `Logger` handle**, passed to each component; there's no global logger. | Several in-process daemons in one test each need their own log buffer (TST-5), and OPS-9's "never logged" rules need one narrow door. | T04, every daemon-side task | proposal |
| P14 | **Four executors at once, one `target/` per worktree.** | See "Worktree setup and limits". | the orchestrator | **agreed** 2026-11-25 |
| P15 | **Small gaps in notify and the narrower conditions, filled so tasks agree** (each is also a spec issue below, or a reading too small for one): the notifier looks at each socket, each `for` and each `problem` on a 10 s timer of its own, POL-26's tick period, not inside the engine's tick; **the engine publishes `PollerStatus::problem_since`, the monotonic time the problem's code began**, so NTF-4's 10 minutes run from when `auth` began and a failed try doesn't move them; `Ask again` puts `--for` before `--since`; a client that goes away after its subscription was kept leaves it in place; NTF-2's "one step" is kept by the notifier entering a subscription evaluated on an earlier view and reading every event after that view, so a refresh in between counts as one after the entry; a `stat` of the session's path that finds something other than a socket still ends a subscription only when its device and inode differ; at a stop, a subscription whose confirmation is under way is told `stopped` once it's written, and one whose notice is being posted keeps that post, inside the same 1 s. The owner's answers of 2026-11-26 moved the rest into the specs (D9). | Each is the smallest reading that lets two tasks build one thing. The (proposal) on everything that posts is the spec's own (docs/07). | T02, T09, T21, T44 to T49, T29, T30 | proposal |
| P16 | **Notify is six tasks, and the condition's evaluation is one of them.** `api::condition` (T44, straight after T02) checks and evaluates a condition for the waiters and for subscriptions alike: one function, with the memory NTF-4 asks for. The notifier with its poster and its words (T46), the two routes (T47), the fake session socket (T45), the command (T48) and the scenarios (T49) are a task each. The notifier's seam (`contracts/notify.md`) has a `Poster` with two methods, `look` (NTF-4's `stat`) and `post`, and the API sees it as `NotifyHandle`, with `NoNotify` behind the routes' shared test state. | One evaluation can't drift between a wait and a notice. With T44 early, the notifier runs beside the engine and the critical path stays at 23; behind the wait route it would be 26. `NoNotify` keeps T22 and T23 from waiting on T46. | T02, T23, T28, T43 to T49 | proposal |

## Open questions and risks

<!-- plan:open -->

**Requirements marked (proposal)** (63): built as written unless the user decides otherwise: API-3, API-5, API-10, API-11, API-12, API-14, API-19, CLI-2, CLI-3, CLI-5, CLI-7, CLI-9, CLI-11, MOD-1, MOD-2, MOD-4, MOD-6, MOD-7, MOD-8, MOD-11, MOD-12, MOD-15, NTF-1, NTF-2, NTF-3, NTF-4, NTF-8, NTF-11, NTF-12, OPS-2, OPS-4, OPS-5, OPS-12, OPS-14, OPS-17, OPS-18, POL-2, POL-4, POL-5, POL-7, POL-8, POL-10, POL-11, POL-12, POL-13, POL-14, POL-15, POL-16, POL-17, POL-18, POL-20, POL-21, POL-22, POL-23, POL-24, POL-25, POL-26, STA-1, STA-2, STA-3, STA-10, TST-12, TST-14.
<!-- /plan:open -->

- **D5 is resolved: a formula written by `dist`** (`specs/decisions.md`, OPS-10). T19's install line and T39 follow it. Without CI (P9), `dist`'s release workflow is T39's first CI.
- **D6 is resolved: built as proposal for v1**, measured later on a small public repository. `blocked` and `behind` are built as MOD-9 and MOD-11 write them.
- **D7 and D8 are resolved** (notify; `--kind` and `--fail-fast`). **Everything that posts is a (proposal)**: the session socket's message format is known from Claude Code's debug log only, and only Linux is measured (docs/07). The plan keeps the two lines in one function (`notify::RealPoster`), the words in one module (`notify::text`), and the fake session socket free of the poster's code, so a format change is one edit and shows in a test.
- **55 requirements are proposals.** They're built as written, each value in one named constant, so a decision changes one line and a test.
- **Risks.** The settle mechanism of the test clock (T04) carries every scenario: if it's flaky, everything after M2 is. T04's acceptance has a stress test for it; treat a flaky scenario later as a T04 bug first. The fake GitHub (T14) executes GraphQL against GitHub's whole schema: its size is the least certain estimate in the plan. The real-process tests (T36) and everything `cfg(target_os = "macos")` only run on macOS in CI (T38). **Notify rests on an interface Claude Code doesn't document, measured on Linux only** (docs/07), while circitor ships to macOS first: nothing automated can see what a real session does with a notice, so its first proof on a Mac is the owner's seventh acceptance check (T40). Until then every test of it runs against the fake session socket, which plays Linux.

### Spec issues found while planning

Not fixed here: specs and docs are the owner's, and `/harden` handles them.

**Hardened on 2026-11-25 (commit 042dae6).** Issues 1 to 11, 13 to 22, 27 and 29 below are answered in the specs, and so are the answered halves of 25, 26 and 28. Three were answered otherwise than the plan had built, and the briefs were changed to follow: **21** (real processes are told apart by an `x-circitor-test-pid` header, not per connection: T07, T20, T36), **25** (a reset's `records` leave `checks.items` out, and a too-old `since` of `list` returns the records `state` asks for: T22, T23) and **27** (an unlisted `mergeable` or `review` is read on the safe side for `status`: T05). **Still open:** 12, 23, 24, the exhausted answer's HTTP status in 28, and 30 to 41. The list is kept as it was written, so each number still means what the briefs and the change log say.

**08-notify's third hardening pass (2026-11-25)** answered 31 (TST-16's fourth mode now fails a connect at once, and NTF-6 has no second try), 34 (NTF-7 has one rule for the condition's words), 35 (the socket's `stat` comes straight after the parameters), 36 (once the last subscription ends, the tier follows the last client request: **otherwise than the plan had built**, so T09, T15 and T21 changed) and 40 (`notify-keeps-active` is split, with `notify-keeps-ended`), and half of 33 (the exit-11 line's words). **Still open after it:** 12, 23, 24, 28's HTTP status, 30, 32, 33's other half, 37's status line, 38, 39, 41, and 42 below.

**The owner's answers (2026-11-26, D9 in `specs/decisions.md`)** settled every one of those but 28's HTTP status, which stays unmeasured: the client already reads a 403 and a 200 alike (docs/02 §6). Written into the specs: API-10 (`fail_fast` answers without `settled`, 30), CLI-3 (`more events:` names the PR, durations leave out a zero second unit, 23), CLI-6 (`again:` repeats `--timeout` and `--json` when given, 24), CLI-11 (under 120 lines, 38), TST-11 (the fake marks check names and logins, 32), TST-8 `clock-back` and NTF-4 (a forward step is a sleep, 39), NTF-8 (`pr_failing`'s sentence, 41), NTF-11 (the exit-11 line, 33), NTF-3 (a notice already being posted). The rest of 23 is the executors' to word, fixed by golden files and listed in their reports. 42's fact is in docs/05 and docs/07; 12's and 37's status lines are fixed. **No spec issue is open now** but 28's HTTP status.

1. **OPS-9 has a warning without a code.** `specs/06-operations.md:75` (OPS-9, "Logging never stops the daemon"): "dropped, with a warning (API-5) while it lasts". `specs/04-api.md:34-42` (API-5) lists seven codes, none for it, and says a new warning needs a row.
2. **`status` has no key for `gh`'s path.** `specs/06-operations.md:54` (OPS-5): "`status` shows the path that was run". API-5's keys (`specs/04-api.md:31`) don't include one.
3. **The order "of `pr`" isn't defined.** MOD-15 (`specs/01-model.md:147`), STA-2 (`specs/03-state.md:25`) and API-6 (`specs/04-api.md:59`) sort by `pr`. As a string, `o/r#10` sorts before `o/r#9`.
4. **An event's JSON shape is half-defined.** API-19 (`specs/04-api.md:44`) says "the event's own shape is MOD-15's"; MOD-15 names what each kind carries but not how `kinds` holds it (a list of objects, a map), nor the key of `draft`'s "new value".
5. **`last_poll.outcome`'s values aren't listed** (API-5, `specs/04-api.md:31`).
6. **An unlisted PR `state`.** MOD-2 (`specs/01-model.md:35`) passes unlisted values through, lowered, and "never fails decoding". Retention, `status` and the open list all need `state` to be one of three.
7. **A review that isn't submitted** has no `submittedAt`, which MOD-12 uses as `at` (`specs/01-model.md:114`). Whether `reviews.totalCount` counts the viewer's own pending review isn't in docs/02.
8. **OPS-9 logs a stop's cause as "replaced"** (`specs/06-operations.md:76`), but a replacing CLI and `circitor stop` send the same `POST /v1/shutdown`, which takes no parameter (API-1, API-17).
9. **Whether `status` keeps a spawned daemon alive.** API-5 (`specs/04-api.md:31`) says it "counts no client activity for POL-13"; OPS-8's idle exit (`specs/06-operations.md:72`) says "no client request".
10. **`paths.state`** (API-5): the state directory or `state.json`.
11. **Where GitHub's public schema comes from** (TST-3, `specs/07-testing.md:18`) isn't in `docs/references.md`.
12. *(Answered by the owner, D9.)* **The root `README.md` (line 5) says the specs are "not yet hardened"**; the spec files record three passes each. T19 rewrites its status line.

Found by the brief writers; the first is a fact that's wrong, the next six are contradictions between two places:

13. **OPS-4's umask gives the wrong mode.** `specs/06-operations.md:41`: "The socket's mode is 0600, set by binding it under a umask of 077". On Linux a socket bound under 077 has mode **0700**; 0177 gives 0600 (checked on the development machine on 2026-11-25 with a local bind; no entry in docs/05 yet). The plan builds 0600 by umask 0177 (`contracts/daemon.md`). macOS isn't checked.
14. **TST-8 `many-prs` against STA-3.** The row (`specs/07-testing.md:81`) says that at 200 PRs "a `show` of an untracked PR answers without tracking it"; STA-3 (`specs/03-state.md:26`) says such a `show` answers `limit` and nothing is sent. The plan builds STA-3, as TST-8's own rule says.
15. **TST-8 `restart-offline` against API-18.** The row (`specs/07-testing.md:85`) says the loaded PRs "stay in `pending`"; with GitHub down from the start no request of the first poll is answered, so `list` answers `github_unavailable` (API-18) and `pending` can't be read. T33 asserts the requirement.
16. **`more` (POL-12 against STA-5).** `specs/02-polling.md:43` says "more than 20 new items"; STA-5 (`specs/03-state.md:28`) says "with all 20 of a kind new". Only the last 20 are read. T05 builds: all 20 new, and for a known PR the count rose by more than 20.
17. **TST-6's idle hot PR can't be staged.** `specs/07-testing.md:32`: "one hot PR adds at most … 350 in the idle" tier for an hour. Only a waiter keeps a PR hot that long, a wait lasts at most 3 600 s (API-12), and its arrival keeps the tier active for `idle_after`. T34 measures the idle 3 000 s of one wait against the rate.
18. **TST-8 `sleep`, "run both ways".** When the monotonic clock counts the suspend there's no gap between the clocks, so POL-22 notices no sleep; the outcomes the row lists (`specs/07-testing.md:67`) still hold by other means. T33 asserts the outcomes both ways and the sleep's log line one way.
19. **TST-13 says "before a release"** (`specs/07-testing.md:100`), and its step 1 installs from the tap. T39 publishes `v0.1.0`; T40 accepts it before it's announced.

Gaps the briefs filled; each brief's Open questions says what's built meanwhile:

20. **TST-12 doesn't give the fixtures' format** (`specs/07-testing.md:95`; docs/05 leaves it to the testing spec). Built: `contracts/github.md`'s one object per file.
21. **TST-3: how the fake tells processes apart** ("in TST-9 hold per process", `specs/07-testing.md:20`). Built: per connection.
22. **TST-8 `first-run`'s "within 5 s"** (`specs/07-testing.md:84`) has no requirement behind it; `track-untrack` cites CLI-10 for what API-9 and API-15 say; `conflict` cites nothing.
23. *(Answered by the owner, D9.)* **CLI-3:** `more events: circitor changes --since <cursor>` (`specs/05-cli.md:65`) names no PR though a per-PR cursor covers one PR only (API-8); durations are "the largest unit and the next" but the example is `4m` (`specs/05-cli.md:59`); no text is given for `track`, `untrack`, a `refresh` that met a problem, or an `activity` with no items; nor the stream for CLI-6's endings without an answer.
24. *(Answered by the owner, D9.)* **CLI-6's `again:`** "repeats the other parameters given" (`specs/05-cli.md:118`): whether that includes `--timeout` and `--json`. Built: each only if it was given.
25. **API:** which routes' records carry `checks.items` (built: every route's but `list`'s, which carries them only with `checks=1`, as API-6 says; a reset of `changes` can then be large); what a too-old `since` returns under `state=open` (built: every record); a known path with another method (built: `unknown_route`); whether a 200 sent before the waiter is registered carries `Circitor-Cursor` (built: yes).
26. **A loaded PR with no record** (STA-8): one that keeps answering `NOT_FOUND` has no record to turn `gone` and no event to carry it (built: its saved entry says `gone`); `untrack` of one can't know whose it is (built: dropped, or `asked` with a waiter).
27. **Model:** the order of items inside an `activity` (built: by `at`, comments before reviews, then `id`); an unlisted `mergeable` or `review` value matches no row of MOD-11 and can read `ready`; a rename that changes only case makes no `edited`.
28. **Polling:** "slowed" in the idle tier (built: `idle_interval` doubled); whether a secondary limit with `Retry-After` counts towards the doubling (built: yes); **a PR refreshed in a poll's second request (every newly found one) is due a second after the next poll's first request is built, so by POL-13's letter (`specs/02-polling.md:49`, "at least its period ago") every poll takes two requests for good, twice TST-6's budget** (built: a poll's requests all count as sent at the poll's send time, P10); the cost of an answer without `rateLimit` (built: 1); the exhausted answer's HTTP status (not in docs/02).
29. **OPS-12 against docs/03 §1:** the spec says the skill is read by Claude Code and OpenCode v2; the facts doc adds pi. **OPS-18** gives no label and no stderr path for the plist (built: `dev.circitor.daemon`, `~/Library/Logs/`).

Found while planning notify and the narrower conditions (2026-11-25, after 08-notify's first hardening pass):

30. *(Answered by the owner, D9.)* **Whether `--fail-fast` still waits for `checks.settled`.** API-10 (`specs/04-api.md`, the `checks-done` row and the `fail_fast` bullet) says a failure "stands in for the first part" and that `sha` and `rerun_since` "still have to hold"; it doesn't name `settled`. TST-8's `fail-fast` row ("answers at the first refresh that shows the failure") reads as not waiting. Built: it doesn't (P15, T44). T30's world is settled, so the row holds under either reading.
31. *(Answered by 08-notify's third pass.)* **TST-16's fourth mode doesn't do on Linux what it says.** `specs/07-testing.md` TST-16: "listen without accepting until its backlog is full, so a connection never completes"; TST-8 `notify-session-gone`: "one whose connection never completes is tried once more after 5 s". A non-blocking connect to a full backlog fails at once with `EAGAIN` on Linux (checked on the development machine on 2026-11-25 with a local socket; no entry in docs/05). NTF-6 treats that as "fails otherwise", so the outcome is the row's; only NTF-6's 2 s can't be staged with a real socket (T46 tests them on a recording poster). **On macOS a full backlog may refuse instead**, which NTF-6 reads as "session gone", with no second try: not checked.
32. *(Answered by the owner, D9.)* **TST-11 asks for markers that don't exist.** `specs/07-testing.md` TST-11: "no marker from a title, a check's name or a login is ever in a posted notice"; the same requirement gives the fake GitHub markers in titles, error messages and the 401's body only. Built: two more constants, which the tests that post notices script into their worlds (P15).
33. *(Answered by the owner, D9.)* **NTF-11's exit 11 without a session socket:** whether the PR is named first (which may need a daemon), and what the line says for the `wait` "to run in its place" when the PR was given short or not at all (`specs/08-notify.md` NTF-11). Built: exit 11 before anything else, the PR repeated as typed (P15).
34. *(Answered by 08-notify's third pass.)* **The words of `registered:` and of the confirmation for other parameters** (`specs/08-notify.md` NTF-7, NTF-11): the examples show `until checks-done on 1a2b3c4` only; nothing says whether `--fail-fast`, `--kind` or `--rerun-since` appear, nor where `--for` sits in `Ask again` (NTF-8). Built: the condition's word and `--sha` only; `--for` before `--since` (P15).
35. *(Answered by 08-notify's third pass.)* **When NTF-2 looks at `socket`:** before the evaluation, or only when a subscription would be kept. A request whose answer is already known, with a socket that isn't the user's, gets `no_session` or `registered: false` accordingly (`specs/08-notify.md` NTF-2). Built: before (P15).
36. *(Answered by 08-notify's third pass.)* **When the tier turns idle after the last subscription ends** (`specs/02-polling.md` POL-13, `specs/08-notify.md` NTF-5: "as if a client had just asked"; TST-8 `notify-keeps-active`: "once it ends, all three happen"). Built: `idle_after` after it ended (P15).
37. *(Answered by the owner, D9.)* **Two measurements 08-notify rests on aren't in the facts docs.** `specs/reviews/08-notify.md` (pass 1) names them: a line posted and closed at once was delivered (NTF-6), and the socket files of six sessions that had ended were gone (NTF-4's check at each tick). **Recorded since** (2026-11-25): both are in `docs/07-notify.md`'s socket table, and NTF-4's note cites it. `docs/07-notify.md`'s status line and its "first sketch" also still describe the feature as an idea (hot PRs, a waiter-limit count, `--every`), which the spec decided otherwise.
38. *(Answered by the owner, D9.)* **CLI-11's guide has much more to say in 70 lines** (`specs/05-cli.md` CLI-11): every command with an example, the exit codes, `--fail-fast`, `--kind`, and seven points on `notify`. T11 builds it as written and reports the count if it can't be met without dropping a rule.

Found while folding 08-notify's second hardening pass into the plan (2026-11-25):

39. *(Answered by the owner, D9.)* **A forward step of the wall clock is a sleep to POL-22, so it moves a subscription's expiry.** `specs/08-notify.md` NTF-4: `for` runs "on the monotonic clock plus the sleep noticed", and "a clock step doesn't move the expiry, forwards or back"; TST-8 `clock-back` (`specs/07-testing.md`): "doesn't move when the wall clock steps … a day forward". POL-22 (`specs/02-polling.md`) takes wall time ahead of monotonic time by more than 30 s for a sleep, and a step forward can't be told from one. Built: NTF-4's rule, as a waiter's deadline (T46); a step back moves nothing; T49 plays the day forward as a sleep and reports the row.
40. *(Answered by 08-notify's third pass.)* **TST-8 `notify-keeps-active` and STA-1 disagree on when the PR goes.** The row (`specs/07-testing.md`): "once it ends, all three happen", the PR dropped at `keep_done` among them; STA-1 and NTF-4 (`specs/03-state.md`, `specs/08-notify.md`) keep an ended PR 60 minutes after a subscription's end. And the row's one world can't show both the tier (only a subscription on an open PR keeps it, POL-13) and `keep_done` (an ended PR). Built: two worlds, the PR dropped 60 minutes after the cancel (T49).
41. *(Answered by the owner, D9.)* **NTF-8 gives the `problem` notice for `auth` only.** `specs/08-notify.md` NTF-8's example reads `can't read … from GitHub (auth)`; for `pr_failing` the words aren't given. Built: the same sentence with `pr_failing` (P15, T46).

Found while folding 08-notify's third hardening pass into the plan (2026-11-25):

42. *(Answered by the owner, D9.)* **A measurement two requirements rest on isn't in the facts docs.** NTF-6 (`specs/08-notify.md`) and TST-16 (`specs/07-testing.md`) cite "a full listen queue fails at once, with `EAGAIN` on Linux (measured 2026-11-25, not yet in docs/05)", and the review log says that macOS answers `ECONNREFUSED` is from source only. By the project's rule a fact needs a dated entry in docs/05 and a row in `docs/07-notify.md`. Built as cited: both end the subscription the same way, so the plan holds whichever is right.

One issue with the tool, not the specs: **`plan.py` never yields parts from a table inside a requirement's bullet** (`_bullet_end` takes the table into the bullet, so `_trailing_parts` starts after it). TST-8's 55 scenarios therefore can't be cited as `TST-8:merged`, though the tool's help says they can. The plan works around it with the table above.

### Changes waiting on running tasks

None.

## Outside the swarm

Four tasks are the owner's (`kind: manual`). No executor task waits for any of them.

| Task | What | When |
|---|---|---|
| T37 | Record real, scrubbed responses from `yachiko/experiment` into `testdata/github/recorded/` (TST-12). **The owner runs it by hand** (2026-11-25), under docs/05's rules. | Any time after T07 |
| T38 | **Blocked: no CI for now (P9).** Put the repository on GitHub and get CI green on Linux and macOS (TST-14). | When CI is wanted, after T03 |
| T39 | Set up `dist` (D5: a formula), publish to `yachiko/homebrew-tap` (OPS-10). `dist init` edits `Cargo.toml` and adds its own release workflow: the owner's own edits. | After T36 and T19 |
| T40 | TST-13's seven acceptance checks on a Mac and on Linux, the seventh `notify` in a real Claude Code session; results into docs/05. | After T39, with T47 and T48 in the release |

Also the owner's: confirming the plan decisions above before T01 starts (P1 to P4 are the expensive ones), and every spec issue.

## Change log

- 2026-11-26: **D11, what `refresh` refreshes (owner's answer to T21's question).** A forced poll refreshes every tracked PR, whatever its period. Written into POL-15 and `specs/decisions.md`; T25 (todo) builds it, behind one flag in T21's poll, with `d11_forced_poll_refreshes_every_pr`. **From now on the orchestrator decides open questions by its own recommendation and records each** (owner, 2026-11-26): in `specs/decisions.md` when a spec changes, here otherwise.
- 2026-11-26: **D10, the socket's mode (contract change, daemon seam).** T18 found that binding under a umask of 0177 changes the whole process's umask, so a directory another thread made meanwhile came out 0600 (a config test failed with `PermissionDenied`). The owner chose: bind at a temporary name as long as the socket's own, `chmod` 0600, rename into place, never touching the umask. Written into OPS-4, STA-12, `specs/decisions.md` (D10) and `contracts/daemon.md`; T18, then in review, was sent the change. No task added.
- 2026-11-26: **`check-macos` checks and no longer links.** After T02, the macOS binary links Apple's `Security` and `CoreFoundation` frameworks (through `rustls-platform-verifier`); zig doesn't ship them, so `cargo zigbuild` failed at the link. It's now `cargo check` with zig as the C compiler (T50's Evidence). P9 says compile-checked again.
- 2026-11-26: **`check-macos` works from Linux, through zig and cargo-zigbuild, and is `make verify`'s last step** (the owner's choice, over swapping to `ring`, which fails the same way, or to the pure-Rust `rustls-graviola`). New follow-up T50, done; T01 points to it. P8 unchanged; P9, the conventions' "Toolchain and commands", T03 (the Linux job installs both tools), T19 (keeps the README's "Building") and T38 updated. **T02 is running:** its worktree's Makefile is T01's, so its `make verify` skips the macOS build; run `make verify` on main after merging it.
- 2026-11-26: **the owner's answers to the open spec issues (D9).** Specs changed: API-10, CLI-3, CLI-6, CLI-11, TST-8 `clock-back`, TST-11, NTF-3, NTF-4, NTF-6, NTF-8, NTF-11; `docs/05` and `docs/07` record the full listen queue's `EAGAIN`; the root README's and docs/07's status lines are current. Plan: `render::changes` takes the PR, for `more events:` (`contracts/cli.md`, T10, T27); the guide's cap is 120 lines (T11); the fake GitHub marks every check name and login it makes up, and the sweep looks for `MARKER_CHECK` in the log and the state file (`contracts/testkit.md`, T14, T29, T35, T49); `fail_fast` cites API-10 (T30, T44); T46, T48 and T49 cite the spec where they cited P15 or a spec issue, and T49 no longer plays `clock-back`'s forward step. P15 shrinks to what the specs still leave to the plan. No task added, dropped or renumbered.
- 2026-11-26: **updated for 08-notify's third hardening pass** (`specs/reviews/08-notify.md`, "Pass 3"). **No second try:** a post that fails or isn't done in 2 s ends its subscription, logged, at a stop too; `RETRY_AFTER` and the telling state's wait are gone (`contracts/notify.md`, T46, T49). **`problem`** is checked at each tick, its 10 minutes from `PollerStatus::problem_since`, a new field the engine publishes (`contracts/poller.md`, T21, T46). **The tier** follows the last client request once the last subscription on an open PR ends: `cadence::last_ask` loses its `last_ended`, `Store::open_subscriptions` returns a count (`contracts/poller.md`, `contracts/state.md`, T09, T15, T21). **Registration:** the socket's `stat` straight after the parameters, `auth` before the limit, every answer from the registration's view, `replaced: true` with `registered: false`, "the same request" compared as the daemon reads it (`contracts/notify.md`, `contracts/api.md`, T46, T47). **Words:** NTF-7's one rule in `text::watching`; NTF-11's line order (`replaced an earlier notify` in both cases, `again:` before `problem:`) and its exit-11 line (`contracts/cli.md`, T10, T35, T48). **Tests:** TST-4's settle counts notices being posted and the fake session's reads (`contracts/clock.md`, `contracts/testkit.md`, T04, T45); TST-16's fourth mode is `queue_full`, failing a connect at once (T45); T49 plays `notify-keeps-active` and the new `notify-keeps-ended`, `notify-problem` and `notify-register`, and the extended `notify-session-gone` and `notify-no-session` (now L). STA-1 is cited as the one place the notify routes' naming rule lives (T09, T15, T47). T28's notes name the new rows. No task added or dropped; all were `todo`, so every change is in place. P15 rewritten; spec issues 31, 34, 35, 36 and 40 answered, 42 added.
- 2026-11-25: **updated for the specs' draft 0.2.** **Notify (D7):** a new seam (`contracts/notify.md`), six new tasks (T44 conditions, T45 the fake session socket, T46 the notifier, T47 its routes, T48 the command, T49 its scenarios), and edits in place to T02, T04, T07, T09, T10, T11, T15, T16, T19, T21, T22, T23, T24, T27, T28, T29, T33, T35, T36, T40, T41 and T43, and to the api, cli, daemon, log, model, poller, state and testkit contracts and the conventions. **Narrower conditions (D8):** `--kind` and `--fail-fast` in T44, T10, T11, T23, T27, with `fail-fast` in T30 and `changed-kind` in T31. **Commit 042dae6's hardening changes were taken into the snapshot**: they came from this plan's own findings, and the briefs that had built another reading were changed (T05, T07, T20, T22, T23, T36; T19 and T39 for D5; T32 and T33 for rows that now agree). **Both hardening passes on 08-notify (2026-11-25) are in**, with their edits to 02-polling, 03-state, 04-api, 05-cli and 07-testing: the socket checked by device and inode at each tick and before each post; `for` on a waiter's clock, an expiry held for a catch-up, the stale mark in `expired`; the end `problem`; NTF-2's order, a confirmation that fails taking out what it replaced, a too-old `since` let go, 32 per socket, `no_session` (exit 11); cancel by the registered name, never reading through; only a subscription on an open PR keeps the active tier, and its end keeps the PR 60 minutes; the second try's state; `stop(asked)` and `--no-spawn` waiting 8 s; the spawned daemon without the session's variables; the registered output's extra lines and `again:`. T46 now depends on T09 and T43. 49 tasks, the critical path unchanged at 23. Plan decisions P15 (extended) and P16 added; spec issues 30 to 41.
- 2026-11-25: owner's Q&A. Agreed P1, P2, P4, P5, P6, P14. P9 changed: no CI for now (T03, T38 `blocked`; T39 no longer waits for T38). D5: a formula by `dist`. D6: build as proposal. T37 is the owner's, by hand. Spec issues to be hardened before T01.
- 2026-11-25: plan created from the specs as of 2026-11-25 (draft 0.1, three hardening passes per file). 43 tasks: T41 (golden files) and T43 (the API's shared parts and the fakes) were split from T02, and T42 (a PR's error runs) from T21, while the briefs were written and reviewed. Two fresh-eyes reviews: an executor dry-run of eight briefs and a dependency audit.
