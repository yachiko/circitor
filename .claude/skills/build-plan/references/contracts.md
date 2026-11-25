# Designing the contracts

Contents:
1. What a contract is here, and what each seam file holds
2. The seams of circitor (a starting list: re-derive it from the current specs)
3. Formats the specs already define
4. The contracts tasks: what they deliver
5. Suggested checkpoints
6. Decisions you'll face
7. Traps worth passing on

## 1. What a contract is here

Two executors working at the same time can't ask each other anything. Whatever they must agree on, the plan fixes in advance: crate and module paths, type names and fields, trait and function signatures, who calls whom and from which task, what awaits and for how long, what an error means, and the formats of files and messages. That's a contract. Fix it before slicing, because the seams decide the task boundaries: a task is usually the producer of one seam, built against the contracts of the seams it consumes.

A seam file (`plan/contracts/<seam>.md`) holds:
- **Purpose**, in two lines, and the requirement IDs it serves.
- **Producer and consumers**, as task IDs (fill them in once the tasks exist).
- **Declarations:** Rust in code blocks, complete enough to compile once copied: structs and enums with their fields and serde attributes, traits with their methods, constructor signatures, error enums. Doc comments carry the requirement IDs.
- **Semantics:** for each method, who calls it and from which task, whether it awaits and for how long, whether its future may be dropped half-way (cancellation), what each error means, ordering and idempotency, and the invariants ("a revision never goes down, not across a restart either").
- **The fake** that ships with it, and what it's good enough for: consumers test against it before the producer is merged.
- **What not to assume:** behaviour that only the real implementation has.

Specify what crosses the seam; leave the inside of a module to its task. If a declaration only one module uses ends up in a contract, it's in the wrong place.

The precision to aim for (an illustration, not a design to copy; its requirement IDs are invented):

```rust
/// The daemon's only source of time (TST-2).
pub trait Clock: Send + Sync + 'static {
    /// Wall time. It can jump, and it keeps counting while the machine sleeps (STA-4's TTL uses it).
    fn now(&self) -> SystemTime;
    /// Monotonic time since the clock started. It never jumps; whether it counts sleep is the
    /// platform's choice (docs/04-platform), so nothing that must survive a sleep is timed on it.
    fn mono(&self) -> Duration;
    /// Completes after `d` of this clock's monotonic time, so the fake clock drives every wait.
    /// Network timeouts don't use it: they're real time (TST-2).
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()>;
}
```

...followed by semantics such as: "POL-9's 'woke from sleep' isn't a method here: it's a gap between `now()` and `mono()` that the scheduler notices (T09 owns that rule)."

## 2. The seams of circitor

00-overview names the components and the repo layout; specs/README's Code column says which spec each crate or module implements. The testing spec adds the seams that make parallel work testable. A starting list, with the questions each contract must answer (take the requirement IDs from `plan.py extract`; the Spec column says where to look):

| Seam | Spec | The contract must settle |
|---|---|---|
| Clock | 07-testing, 02-polling, 03-state | wall and monotonic time, kept apart; the sleeps and timers the fake fires; the fake's controls (advance, jump the wall clock apart from monotonic time to play a laptop's sleep and wake); the network-timeout exception |
| GitHub client | 02-polling, 06-operations | a trait with the queries polling needs (discovery, refresh by node ID, one PR by repository and number), and no method that writes to GitHub; typed results that carry the rate-limit block (cost, remaining, reset) and partial data with errors; how failures are classified (auth, primary limit, secondary limit with its wait, network, 5xx, not found); a fake scripted from fixtures that can move a PR through its life, lag its search index, leave `mergeable` unknown, fail in each class, and count the points it charged |
| PR model | 01-model | the record every component shares: identity (host, repository, number, node ID), head SHA, state fields, check entries, the derived state, `fetched_at`, the revision; the serde names, matching 04-api exactly; which differences count as a visible change; who computes the derived state |
| State store | 03-state | applying a poll result, with a revision raised only on a visible change; the tracked set; the TTL of merged and closed PRs; snapshots for readers and the change notification; the state file's format, version and atomic write; what it reports to status; who decides when to save |
| Poll scheduler | 02-polling | the cadence tiers and what moves a PR or the daemon between them (client activity, waiters, pending checks); the forced refresh and how concurrent ones are coalesced; backing off on limits and errors; the quota budget; catching up after a sleep; what it publishes to status |
| Wait registry | 04-api | registering a waiter (PR, condition, since-revision, expected head SHA, deadline); evaluation at registration and on every change; the outcomes (satisfied, terminal, timeout); deregistering when the client goes away; how the scheduler learns a waiter exists; limits on waiters |
| API types | 04-api | the request and response JSON of every endpoint, the error body and the status codes, as Rust types the server and the CLI share |
| CLI-to-daemon transport | 04-api, 05-cli, 06-operations | HTTP over the Unix socket: how the socket path is resolved, its permissions, a listener and a connector behind one abstraction so TCP or a named pipe can replace them later; the auto-spawn handshake (who takes the lock, who creates the socket, readiness, the version check); what a stale socket file is and who removes it |
| Config | 06-operations | the struct every component reads, defaults, validation, the file's location, whether it's re-read |
| Token source | 06-operations | a trait that returns the token for a host (from `gh`, or the environment); when it's read again; its errors; that the token is never logged or stored; a fake |
| Status | 04-api, 06-operations | the status object and which component fills each part (cadence tier, quota left, last poll, last error, warnings) |
| Daemon and CLI | 05-cli, 06-operations | a daemon the test harness can start in-process with an injected clock, GitHub client, token source and directories, and restart on the same state directory; start and stop order; the single-instance lock; the commands, their output formats and exit codes |
| Test harness | 07-testing | daemon plus fake GitHub on the fake clock, restarts, temp directories, and the checks every scenario shares |
| Build and release | 06-operations | the version in the binary; the release artifacts; the formula and the service files |

## 3. Formats the specs already define

Cite these; don't redefine them. The contracts task adds golden fixtures for each one that a writer and a reader on different tasks share, so neither waits for the other:
- the PR record as JSON: 01-model and 04-api
- the HTTP routes, parameters, status codes and error body: 04-api
- the wait conditions and outcomes: 04-api
- the CLI's commands, compact and JSON output, and exit codes: 05-cli
- the state file: 03-state
- the config file: 06-operations
- the GraphQL queries, and the recorded responses the fixtures come from: 02-polling, 07-testing, docs/05-measurements
- the fake GitHub's scenario script: 07-testing
- the socket path, the lock and the service files (a launchd plist, a systemd unit): 06-operations

## 4. The contracts tasks

The scaffold and the contracts are on everyone's path, so they must be quick to finish (S or M each) and contain no product behaviour.
- **Scaffold** (`infra`): `Cargo.toml` (a workspace, if the plan decided on one) with every dependency 00-overview names and the plan's test dependencies, and the committed `Cargo.lock`, so no later task edits them; `rust-toolchain.toml`; the directory layout with every crate and its `mod` declarations; a Makefile with `make verify` and the build targets; the rustfmt and clippy settings; `.gitignore`; and a short root `CLAUDE.md` (Claude Code loads it into every executor) with the rules that must never be broken and a pointer to `plan/conventions.md`. If the repository already has a `CLAUDE.md`, the task extends it.
- **Rust contracts** (`infra`): the declarations of `plan/contracts/` as compilable code, with the simple fakes and the golden fixtures, and a test that the fakes satisfy the traits. Constructors of real implementations don't exist yet; that's their tasks' job.
- Fakes with real behaviour of their own (the fake clock with timers and a sleep-and-wake control, the fake GitHub server with its scenario script) are their own early tasks, depending only on the contracts.

## 5. Suggested checkpoints

Each checkpoint has an integration task that wires the parts so far (in the daemon's wiring module or the binary) and proves the checkpoint end to end. Adjust these to the current specs:

| Milestone | Runs end to end |
|---|---|
| M1 Poll | in one process on the fake clock, the scheduler polls the fake GitHub replaying the recorded fixtures; the store holds the PRs with the right derived state and revisions; a merge, a push and a lagging search index are each seen as the specs say; the points charged stay inside the budget |
| M2 Serve | the API answers over a Unix socket in a temp directory; waits return satisfied, terminal and timeout as specified, and a waiter moves its PR to the fast tier; a client that disconnects leaves no waiter behind |
| M3 CLI | the CLI's commands against a running daemon, with their output and exit codes; a CLI with no daemon starts one and gets its answer; two at once start one daemon |
| M4 Operate | the config file, the token from a fake `gh`, the state file across restarts, the lock and the stale socket, the service files and the release build |
| M5 Harden | the testing spec's failure scenarios: quota exhausted, token revoked, network down, sleep and wake, a crash mid-write, upgrades; the manual acceptance checklist goes to the owner |

Owner tasks sit outside the swarm: the Homebrew install and the service on a Mac, the acceptance checks against the live GitHub API, the release to the tap.

## 6. Decisions you'll face

Each one you make goes in README's Plan decisions as a proposal:
- the crate layout: one crate with modules, or a workspace with a crate per component (a crate per component keeps `owns` disjoint and takes `mod` lines out of the hotspots); the crate and binary names; the edition and the pinned toolchain (check `cargo --version`)
- crates beyond 00-overview's layout: the clock, the model, the API types, the transport, the config, the status object, the daemon as a library, the fake GitHub as a library, the test harness
- the async runtime, the HTTP server and client, the TLS backend, and whether the GraphQL queries are hand-written with serde types or generated (if 00-overview names a library, changing it is the user's call)
- the fake GitHub: an in-process fake behind the client trait, a loopback HTTP server that speaks GraphQL, or both (the second also tests the real client's parsing)
- time in tests: the plan's own clock trait, tokio's paused time, or both, and which tests may use real time
- test dependencies (the standard test harness only, or snapshot and property-test crates), and the clippy lint level
- whether the macOS target is compile-checked from Linux (`cargo check --target aarch64-apple-darwin` needs the target installed and no dependency that needs the macOS SDK), left to CI on macOS runners, or left to the owner's Mac
- whether there's CI, and where
- the release tooling, and where the formula lives
- how the CLI detaches the daemon it auto-spawns, and what readiness means

## 7. Traps worth passing on

Put each in the Traps section of the tasks it affects, after checking it against the current specs and the facts docs:
- **Unix socket paths are short:** about 104 bytes on macOS and 108 on Linux (take the figures from docs/04-platform). macOS temp directories are long, so tests bind in a short directory they create, and the spec's default path must fit a long user name.
- **A stale socket file makes `bind` fail.** Remove it only once the lock is held or a connect has failed; removing a live daemon's socket orphans the daemon.
- **Two CLIs can auto-spawn at once.** The lock decides, not the socket's existence; the loser waits for readiness and connects.
- **A dropped future is a cancelled request.** The server drops a handler when its client disconnects, so a waiter deregisters in `Drop`, not at the end of the function.
- **No lock held across an `.await`,** and no blocking call on the runtime: running `gh` is `tokio::process` or `spawn_blocking`.
- **Paused tokio time and real I/O don't mix.** Auto-advance fires timeouts while a real socket is still working; tests on fake time talk in-process (a duplex stream, the client trait's fake), and tests over a real socket run on real time with short waits.
- **Monotonic time may not count sleep.** Whatever must survive a laptop's sleep (the TTL, "due since") is measured on wall time; check docs/04-platform for what each platform does.
- **Written on Linux, shipped to macOS.** `cfg(target_os = "macos")` code isn't compiled in a worktree unless the plan decided on a target check. Keep platform code in one small module, and say in the task what couldn't be compiled.
- **GitHub facts move.** The facts docs can be newer than a spec's copy: give the model and the client tasks docs/02-github-api, not the spec's summary. Check there before relying on any of: `mergeable` being known on the first read, a new PR appearing in search at once, checks existing right after a push, `updatedAt` moving when a check finishes.
- **The fake GitHub mustn't share the client's response types.** It serves the recorded JSON, or builds JSON itself from docs/02-github-api, so a wrong field name in the client fails a test instead of agreeing with itself.
- **Don't refuse unknown fields from GitHub.** GitHub adds fields and enum values; a strict deserialiser turns that into an outage. The state file is the opposite case: its version field decides.
- **Tests never touch GitHub, or the machine's `gh` login.** An executor's shell has a real login and a real token: the token source is faked, and no test runs `gh`.
- **Recordings nobody can make in a worktree.** Executors can't call the live API, so fixtures are the scrubbed responses recorded for docs/05-measurements, and where none exists, a synthetic one written from docs/02-github-api's examples: list the stand-in as a plan decision, and a manual task records the real one and replaces it. No code task waits for it.
- **Recorded responses hold private data.** Repository names, titles, logins and URLs are scrubbed before a fixture is committed, and a token never is.
- **Exit codes and compact output are an interface.** Agents branch on them: a changed code or field is a contract change, not a refactor.
- **Worktrees:** each one builds its own `target/` (slow, and large) unless `CARGO_TARGET_DIR` is shared, which makes builds wait for each other's lock. Decide which, and say so in conventions.
- **`Cargo.lock` and `mod` lines are merge hotspots.** Only the scaffold adds dependencies, and the contracts task declares the modules.
