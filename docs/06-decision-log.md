# Decision log

In the order the decisions were made. All of these are from 2026-11-24, the day the project started.

> Once the specs exist, software decisions are recorded in `specs/decisions.md`. The numbers in this log are separate from the spec's D-numbers: cite these as "log D5".

## D1. One local poller that agents ask, instead of each agent polling

Coding agents watch their PRs by calling `gh`: `gh pr view`, `gh pr checks --watch`. Each call spends the user's GitHub quota, and the cost multiplies with the number of sessions. One daemon polls once and answers all of them. Lowering quota pressure is the main use case.

## D2. Polling, no webhooks

A webhook needs an endpoint GitHub can reach, or a relay, and organisation-level setup. The tool must work on a laptop with nothing but the user's login.

## D3. Rust

The owner's choice.

## D4. The CLI is what agents use; MCP comes later

| Option | Outcome |
|---|---|
| MCP server only | **Rejected.** A `wait` of several minutes is one tool call, and each client treats a long call differently. Tool definitions cost context in clients that load them eagerly. |
| Raw HTTP, called with `curl` | **Rejected.** Agents would need to know the socket and the routes, and the transport could never change. |
| **A CLI over an HTTP API** | **Chosen.** Every client can run a command with no setup. A blocking `wait` is an ordinary command, which Claude Code and OpenCode v2 can run in the background and be told about. |
| MCP as well | **Later**, as a stdio shim (`circitor mcp`) over the same API. |

One reason given at the time was wrong: pi was said to have no MCP support. It has had it since 0.99.0 ([docs/03 §4](03-clients.md#4-pi)). The other reasons stand.

## D5. Unix socket by default; TCP only as an opt-in

- **Unix socket:** file permissions are the authentication, and there's no port to pick or collide on.
- **TCP on localhost:** any local process can reach it, and so can a web page (cross-site requests to `POST /track`, DNS rebinding to read private PR titles). It stays possible as an opt-in that requires a token.
- **Windows** isn't ruled out by this: because clients go through the CLI, the transport is private, and Windows can use a named pipe or TCP later. tokio has no Unix sockets on Windows ([docs/04 §4](04-platform.md#4-sockets-and-directories)).

## D6. State in memory, plus a small state file

Everything can be fetched again from GitHub except two things: which PRs the user asked to track by hand, and merged PRs waiting out their TTL. A small file holds those. **Rejected:** SQLite, as more than the data needs.

## D7. Not a `gh` extension; reuse gh's login

A `gh` extension can be a precompiled Rust binary, but it can't install a service or ship a menu bar app. circitor is its own binary. It does take the token from `gh auth token`, which avoids an OAuth app of its own and brings GitHub Enterprise hosts and SSO authorisation along. A thin `gh circitor` wrapper stays possible.

## D8. The menu bar is a separate client, after v1

A native tray icon needs the main thread's event loop, and in practice an app bundle, a cask and notarisation. A SwiftBar plugin that calls the CLI gets most of the value in a few lines. Either way it's a client of the same API, never part of the daemon.

## D9. Homebrew from the owner's tap, macOS first

Linux later; Windows to be decided. Whether it's a formula or a cask, and which release tool builds it, is open ([direction](01-direction.md#for-the-spec-to-decide)): `brew services` needs a formula, and the tap's other tools are casks.

## D10. Polling driven by demand

The first idea was a fixed 30 s. **Chosen instead:** slow or paused when no client has asked for a while, the normal rate when clients are active, and faster for a PR someone is waiting on.

The reason given at the time was quota: an estimated 2–3 points a poll. **Measured the same day, a poll costs 1 point** ([docs/02 §2](02-github-api.md#measured-cost)), so a fixed 30 s would use 2.4 % of the budget. The decision stands for other reasons: not polling all night for nobody, and answering a `wait` faster than 30 s.

## D11. Find, then refresh by ID

A search for open PRs can't see a merge: the PR just leaves the list. So finding PRs and refreshing them are separate steps, and a PR that disappears is read once more by ID. Measurement added two things: the two steps fit in one request for 1 point, and `viewer.pullRequests` has none of the search index's lag (docs/02 §3).

## D12. The name: circitor

The owner's projects carry Latin role names. `vigil` was liked but has about 26 000 repositories on GitHub. `circitor`, the soldier who walked the rounds of the watch posts, had none, and was free on crates.io and Homebrew.

## D13. Docs, then specs, then plan

The process of the owner's `lg-therma-v-monitoring` project, with its three skills copied in and retargeted:
- `docs/` holds facts, each labelled by how it's known.
- `specs/` holds behaviour as numbered requirements, hardened one file at a time with `/harden`.
- `plan/` holds contracts and one brief per task, built with `/build-plan`; executors work in git worktrees.
- A later feature enters at the specs, with `/add-feature`.

The thermav skills' eval cases weren't copied: they test thermav's content.

## D14. The CLI starts the daemon

The daemon starts on the first CLI call and exits by itself after a time with no clients. This is how tools whose daemon exists only to serve their own CLI usually work (sccache, the Bazel and Gradle daemons, watchman). A service manager is the usual choice where the daemon has work to do with nobody asking, which circitor doesn't.

- **No `brew services` in v1**, so circitor needn't be a formula and can ship as a cask like the tap's other tools.
- **The daemon can also be run in the foreground** (`circitor daemon`), so launchd or systemd can own it (agreed the same day). It's the fallback where starting it from the CLI doesn't work, which is expected under Claude Code's sandbox: a daemon started by a sandboxed command inherits the sandbox.

## D15. v1 tracks the user's own open PRs, by owner

The user's open PRs on github.com, filtered to a configured list of owners, plus PRs added by hand. The owner has no organisation, so the list starts as their own account. **No free-form search queries in v1:** search is the path with lag ([docs/02 §3](02-github-api.md#3-finding-the-users-prs)), and `viewer.pullRequests` covers the main case without it.

## D16. Comments: report what's new, with IDs, never the text

The owner's choice: circitor reports **changes**, and for a new comment or review it gives the **ID**, so the agent fetches that one item instead of the whole PR. circitor doesn't hold or serve comment text.

Measured the same day ([docs/02 §4](02-github-api.md#comments-and-reviews)): the IDs GraphQL gives are the ones the REST endpoints take; the counts in the record show a new comment at once; and asking for IDs only when a count moves keeps a poll at 1 point. The details are for the spec.

## D17. github.com and one account in v1

gh's active account on github.com. GitHub Enterprise Server and several accounts are left out of v1.
