# How a change ripples

How the project's layers depend on each other, and what a change in one spec file usually drags into the others and into `plan/`. Use it in step 1 to find the files to read, and in step 3 to make sure the draft covers each ripple. Not every row applies to every feature: check the ones that do.

The rows follow the spec files as first laid out. If `specs/README.md` lists different files, re-derive them from its file table, and check every requirement ID a row leads you to against the current specs: this file names none, because they change.

## The layers

```
docs/02-github-api.md    GitHub facts: fields, cost, limits, timing; verified or not
docs/03-clients.md       client facts: timeouts, background commands, sandboxes
docs/04-platform.md      platform facts: sockets, directories, service managers, Homebrew
        │  cited by
specs/01-model.md        the one place that maps PR fields to GitHub's (MOD-*)
        │  field and state names used by every other spec file
specs/02 … 07            behaviour, by requirement ID; decisions.md holds D1…; reviews/ holds harden's logs
        │  cited by
plan/tasks/*.md          requirements: [IDs]; plan/contracts/ fixes the seams between crates
        │  built into
the crates               // ID comments; the ID in each test's name
```

- **docs/** is read-only in this skill. A fact the feature needs that the facts docs don't verify becomes a manual owner task, and the spec says `(proposal)` until it's recorded in docs/05-measurements and its facts doc.
- **specs/README.md** sets the conventions: ID prefixes, `(proposal)` and `(decision needed)`, the units and names used across files, and files under about 250 lines.
- **plan/** follows the specs through `plan.py`: `diff` finds the tasks a spec change touches, `check` validates coverage, ownership and cycles, and `render` refreshes README's tables.

## Ripples by spec file

### 01-model (MOD): a new or changed PR field or state
- **Source:** a GraphQL field in docs/02-github-api (its type, nullability, and what it holds before GitHub has computed it) or a rule over other fields. Unverified means `(proposal)` plus an owner task.
- **02-polling:** the query that fetches it grows: more nodes, perhaps another connection, so more points per poll. Recompute the quota budget, and say which cadence tier fetches it.
- **03-state:** whether it's written to the state file. A new persisted field is a state-file version question: what an older daemon does with the newer file, and the reverse.
- **Revisions:** whether a change to it counts as a visible change. If it does, every `changed` waiter wakes on it.
- **04-api:** the PR JSON carries it; say when it's null. A new derived state may need a wait condition, or a terminal case in an existing one.
- **05-cli:** the compact output (does it earn its tokens?), the JSON output, and any exit code that depends on it.
- **07-testing:** the fake GitHub must serve it, from a recorded response or a scripted one, plus a TST row.

### 02-polling (POL): queries, cadence, quota, errors
- **Queries only:** if the specs say the daemon never changes anything on GitHub, a feature that writes crosses a non-goal.
- **Quota:** any new query or shorter cadence changes points per hour in every tier. The budget figures in 02-polling, the status object and the operations spec's targets move together.
- **Cadence changes** ripple into wait latency (04-api: how stale an answer can be) and into what the CLI tells an agent about freshness.
- **A new failure class** (a new limit, a new error from GitHub): the scheduler's backoff, the status object, the CLI's message and exit code.
- **07-testing:** a fake-GitHub action for any new failure mode, and a scenario row.

### 03-state (STA): cache, tracked set, TTL, state file
- **A state-file change** is a version bump: what loading an older and a newer file does, and a stored-version fixture with the upgrade tests extended.
- **TTL and tracking rules** change what the API answers for a PR that has left memory (04-api) and what the CLI prints for it (05-cli).
- **Anything persisted** must be reconstructible or deliberately not: say which, and what a deleted state file loses.

### 04-api (API): HTTP over the socket, waits
- **A new endpoint or field** goes in the API types (the contract api.md) that the server and the CLI share; a state-changing endpoint needs its idempotency said.
- **A new wait condition:** when it's satisfied, which states end it early (closed without merging, a new head commit), what a timeout returns, and what it asks of the scheduler (does a waiter move its PR to a faster tier?).
- **05-cli:** every endpoint an agent should reach has a command; the CLI is the agents' interface, not the socket.
- **Another transport or protocol** (TCP with a token, a named pipe, an MCP shim): the transport contract, the security rules in 06-operations, and docs/03-clients for the client that wants it.

### 05-cli (CLI): commands, output, exit codes, auto-spawn
- **Output is an interface:** agents parse the compact form and branch on exit codes. A changed field or code is a breaking change: say so, and list it in the decision.
- **Size:** compact output is paid for in tokens on every call; a new field needs a reason to be there by default.
- **Auto-spawn and version checks:** a command an older daemon can't answer needs its failure said (the upgrade case in 06-operations).
- **Client guidance:** if the skill or AGENTS.md snippet that teaches agents the CLI is part of the specs, a new command changes it.
- **Timeouts:** a blocking command's default must fit the clients' tool timeouts (docs/03-clients).

### 06-operations (OPS): config, token, service, logging, distribution, platforms
- **A config key:** the config table (default, range, required or not), validation, good and bad example files in `testdata/`, and whether a running daemon picks it up.
- **The token:** any new host, scope or token source; it's never logged, stored or put in an error.
- **Logging:** what's logged when a condition starts and ends, never per poll.
- **A new platform or package** (Linux packaging, Windows): directories, the service manager, the socket or its replacement, the release artifacts, and facts in docs/04-platform that someone has to verify on that platform: owner tasks.
- **A GUI client** (the menubar widget): a second binary or an app bundle, how it's shipped (a cask rather than a formula), signing, and that it stays a client of the API rather than part of the daemon.

### 07-testing (TST): how it's proven
- Every feature needs at least one TST requirement or a row in an existing TST table that says how it's tested and with which values.
- Checks that need a Mac, the live GitHub API or a real agent client are owner tasks; everything else runs against the fake GitHub on the fake clock.
- A new recorded response is the owner's to record and scrub; until then the task builds against a synthetic one written from docs/02-github-api.

## Ripples in plan/

- **Contracts** (`plan/contracts/*.md` and the Rust files headed `// Contract: …`): a new or changed seam updates the contract file. When the contracts task is done, the change is a follow-up task, and every task using that seam is checked (contracts/README's table lists the consumers).
- **Ownership:** a new task's `owns` can't overlap another todo or running task's. Shared files (`Cargo.toml`, `Cargo.lock`, the Makefile, a crate root's `mod` lines) are `shares`, listed in README's merge hotspots. A new dependency is a plan decision first.
- **Status of the owner:**
  - todo: the task is edited in place;
  - done: a follow-up task plus a "Follow-ups: Tnn" line;
  - running: listed under "Changes waiting on running tasks".
- **Milestones:** new feature work after v1 usually forms a new milestone row in README, with its own integration or scenario task when it crosses crates.
- **Spec issues and decisions:** a question the plan has to settle without the owner goes in README's "Spec issues found while planning" as `(proposal)`. The owner's choices go in `specs/decisions.md`.
