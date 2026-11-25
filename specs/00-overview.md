# Overview

**Status:** draft 0.2, 2026-11-25: notify (D7). Draft 0.1, 2026-11-24.

## Summary

`circitor` is one Rust binary. Run as a daemon on the developer's machine, it:
- asks GitHub's GraphQL API about the user's pull requests, on a cadence that follows demand
- keeps each PR's state in memory: checks, mergeability, review state, and what's new in comments
- answers the CLI over a Unix socket: the list, one PR, what changed, or a `wait` that blocks until something happens
- posts one message into a Claude Code session that asked to be told about a PR (D7)

Run as a CLI, it's what coding agents call instead of `gh`. One poller serves every agent session, so watching PRs stops multiplying against the GitHub quota.

## User requirements

| ID | Requirement |
|---|---|
| U1 | **Agents ask circitor instead of GitHub** about the user's PRs. The main goal is lower pressure on the GitHub API quota. |
| U2 | Data comes from **polling GitHub**. No webhooks. (log D2) |
| U3 | It tracks **the user's own open PRs**, filtered to configured owners, plus **PRs added by hand**. (D2) |
| U4 | For each PR it knows the state, the checks, the review state and whether it can merge. |
| U5 | For comments and reviews it reports **what's new, with IDs, never the text**, so an agent fetches one item directly. (D3) |
| U6 | An agent can **block until** a PR is merged, its checks are done, or it changed. |
| U7 | A merged or closed PR **stays for a while**, then is forgotten. |
| U8 | **Agents use a CLI.** Claude Code first; OpenCode and pi must work too. (log D4) |
| U9 | **The CLI starts the daemon** when needed, and the daemon exits when idle. It can also run in the foreground under a service manager. (D1) |
| U10 | **A CLI controls the daemon:** its status, stopping it, forcing a refresh. |
| U11 | It uses **gh's login**. It has no authentication of its own. (log D7) |
| U12 | **Rust**, shipped through **Homebrew for macOS first**; Linux later. (log D3, D9) |
| U13 | **github.com and one account** in v1. (D4) |
| U14 | **A Claude Code session can ask to be told once** when a PR is merged, its checks are done, or it changed, and work on or sit idle meanwhile. (D7) |

## Non-goals (v1)

- **Writing to GitHub.** circitor sends queries and never a mutation, in any version (POL-1).
- **Comment and review text.** It serves IDs, authors and times (U5).
- Webhooks (U2).
- An MCP server, a menu bar widget, a `gh` extension. Each is a later client of the same API (log D4, D7, D8).
- Windows, GitHub Enterprise Server, several accounts or hosts.
- Free-form search queries, PRs awaiting the user's review, issues, notifications.
- A TCP listener. The Unix socket is the only way in to the daemon in v1. (proposal: log D5 allowed TCP as an opt-in; nothing in v1 needs it) The one connection the daemon makes besides GitHub's is to a session's own socket, to post a notice (D7).
- Notices that repeat, one per event, and notices for clients other than Claude Code (D7).
- `brew services`. The daemon's foreground mode is enough for a user who wants a service (D1).
- Edited and deleted comments, resolved threads, auto-merge and the merge queue: reported only as far as [01-model](01-model.md) says.

## Components

```
 agent ──► circitor (CLI) ──HTTP over a Unix socket──► api ──► waiters
                                                        │
                    config ──► poller ──► state ◄───────┘
                                 │          │
              token source ──► github     state file
                                 │
                                 ▼
                         api.github.com/graphql
```

| Component | Responsibility | Spec |
|---|---|---|
| CLI | Parses commands, starts the daemon if it isn't running, prints compact text or JSON, maps outcomes to exit codes | [05-cli](05-cli.md) |
| api | The HTTP routes on the socket; registers and answers waiters | [04-api](04-api.md) |
| notifier | Holds subscriptions, and posts a notice into a Claude Code session when one ends | [08-notify](08-notify.md) |
| state | The tracked PRs, their revisions and events; retention; the state file | [03-state](03-state.md), [01-model](01-model.md) |
| poller | Decides when to ask GitHub and for what; keeps to the quota budget; backs off on errors | [02-polling](02-polling.md) |
| github | Builds the queries, sends them one at a time, turns responses into records | [02-polling](02-polling.md) |
| token source | Gets the token from the environment or from `gh` | [06-operations](06-operations.md) |
| config | One optional TOML file | [06-operations](06-operations.md) |

## Technology

| | |
|---|---|
| Language | **Rust**, one binary that is both the CLI and the daemon |
| Libraries (proposal) | `tokio`, `axum` (the server, on a `UnixListener`), `hyper` (the CLI's client over the socket), `reqwest` with rustls (GitHub), `serde`/`serde_json`, `toml`, `clap`. The plan confirms the list. |
| Targets | macOS arm64 and x86-64; Linux x86-64 and arm64 (OPS-13) |
| Developed on | Linux. Anything macOS-specific is checked by the owner on a Mac ([07-testing](07-testing.md)). |

## Repo layout (proposal)

```
src/main.rs        argument parsing; the CLI or the daemon
src/model/         the PR record, checks, status, events
src/github/        queries, the GraphQL client, response decoding
src/poller/        cadence, quota, backoff
src/state/         the tracked set, retention, the state file
src/api/           routes, JSON types, waiters
src/notify/        subscriptions, posting to a session's socket
src/cli/           commands, output, exit codes, starting the daemon
src/daemon/        start, stop, the lock, the socket, signals, logging
src/config/        the config file and defaults
tests/             scenarios against the fake GitHub
testdata/github/   recorded responses, scrubbed
specs/  docs/  plan/
```

## Glossary

| Term | Meaning |
|---|---|
| PR | A GitHub pull request, named `owner/repo#number` |
| Head | The PR's latest commit; its SHA is `head_sha` |
| Check | One entry of a commit's CI results: a **check run** (GitHub Actions and apps) or a **commit status** (the older API) |
| Rollup | GitHub's own summary of a commit's checks. circitor doesn't trust its state (MOD-5). |
| Record | Everything circitor knows about one PR ([01-model](01-model.md)) |
| Revision | A counter that grows whenever a record visibly changes (MOD-13) |
| Cursor | An opaque string a client gets with every answer and hands back to ask "what changed since?" (STA-9) |
| Poll | One round of asking GitHub: finding PRs and refreshing the known ones |
| Tier | The cadence the poller is in: idle, active, or hot for single PRs (POL-13) |
| Waiter | A `wait` request the daemon is holding open |
| Subscription | A request to be told once, held without a connection (NTF-1) |
| Notice | The one message circitor posts into a session (NTF-6) |
| Tracked | In the daemon's set: authored, added by hand, or asked about (STA-1) |
| Owner | The user or organisation a repository belongs to |
| Point | The unit of GitHub's GraphQL rate limit: 5 000 an hour (docs/02 §2) |
