# Direction

This is **input to the software spec**, not the spec itself. It records what has been decided so far, the constraints the measured facts put on the design, and what the spec still has to decide. The reasons behind each decision are in the [decision log](06-decision-log.md).

## What it is

**circitor** is a small daemon on the developer's own machine. It asks GitHub about the user's pull requests on a schedule and keeps the answers in memory. Coding agents (Claude Code first, also OpenCode and pi) then ask circitor instead of GitHub: for the list, for one PR, or to block until something happens.

**The main purpose is to take pressure off the GitHub API quota.** Several agents each polling `gh pr checks` share one 5 000-point hourly budget; one poller serving all of them doesn't multiply with the number of agents.

A *circitor* was the Roman soldier who walked the rounds of the watch posts.

## Decided

| Topic | Decision | Log |
|---|---|---|
| Source of data | **Polling GitHub's GraphQL API. No webhooks.** | D2 |
| Language | **Rust** | D3 |
| What agents use | **The CLI.** The daemon serves an HTTP API; the CLI is its client and the stable contract. | D4 |
| MCP | **Later**, as a thin stdio shim over the same API | D4 |
| Transport | **A Unix socket** by default, with TCP as an opt-in that needs a token. The transport is private to the CLI and the daemon. | D5 |
| State | **In memory, plus a small state file** for what GitHub can't give back. No database. | D6 |
| Authentication | **gh's login** (`gh auth token`). No OAuth app of circitor's own. | D7 |
| `gh` extension | **Not the primary form.** A thin wrapper is possible later. | D7 |
| Menu bar | **A separate client, after v1.** A SwiftBar plugin first. | D8 |
| Distribution | **Homebrew, from the owner's tap**, macOS first. Linux later. Windows undecided. | D9 |
| Polling cadence | **Driven by demand**, not a fixed 30 s | D10 |
| Finding and refreshing | **Two steps:** find the PRs, then refresh the known ones by ID | D11 |
| Process | docs, then specs, then plan, with the `harden`, `build-plan` and `add-feature` skills | D13 |
| Starting the daemon | **The CLI starts it** on first use; it exits when idle. No `brew services` in v1. | D14 |
| What's tracked | **The user's own open PRs**, filtered to configured owners, plus PRs added by hand. No free-form searches in v1. | D15 |
| Comments | **What's new, with IDs, never the text** | D16 |
| Hosts | **github.com, gh's active account** in v1 | D17 |

## The API as first sketched

From the owner's notes. The spec turns these into CLI commands and decides the details.

- `GET /prs`: every tracked PR and its status
- `GET /prs/<repo>/<n>`: one PR
- `GET /prs/<repo>/<n>/wait?until=merged|checks-done|changed&timeout=600`: block until the condition holds
- `POST /track`: follow a PR the user didn't author

A merged PR stays until a TTL expires. The default search is the user's open PRs in one organisation.

## Constraints from the facts

Each cites the fact it rests on. A spec that contradicts one of these needs a new measurement first.

**Quota**
- **One poll costs 1 point for up to about 49 PRs**, when it lists IDs and refreshes by ID in one request ([docs/02 §2](02-github-api.md#measured-cost)). At a fixed 30 s that's 120 points an hour, 2.4 % of the budget. At 10 s it's 7.2 %.
- So quota alone doesn't force a slow cadence. The cadence tiers are about **not polling when nobody is asking**, and about answering a `wait` quickly.
- **Cost follows the page size asked for**, so page sizes are chosen, not left at 100.
- The quota counters lag, so circitor budgets from its own count of what it spent, and treats `remaining` as a hint.

**Finding PRs**
- **`viewer.pullRequests` has no lag and search has several seconds of it** (docs/02 §3). The user's own PRs come from `viewer.pullRequests`; search is for queries it can't express.
- **A PR an agent just created may not be in a search yet.** Asking for an unknown PR must fetch it directly, not answer "not found".
- **A PR leaves the open list when it merges or closes, and the list can't say which.** Every PR that disappears is read once more by ID.

**Checks**
- **`updatedAt` doesn't move when a check starts or finishes** (docs/02 §4). Check state is re-read on every refresh; `updatedAt` can only gate the fields it does cover.
- **For 3–9 s after a push or a PR's creation, the PR looks finished:** first the old head with its old result, then the new head with no checks at all. A repository with no CI looks the same for ever. "Checks done" therefore needs the head SHA the caller expects, and a rule for how long "no checks yet" must last before it counts.
- **The rollup says `FAILURE` as soon as one check fails, while others still run.** "Checks done" is decided from each check's own status, never from the rollup's state.
- Check runs and commit statuses come in one list with different fields; a skipped job counts as completed.
- Reads can arrive out of order (a check went from running back to queued), so a single read is never a transition.

**Comments and reviews**
- **The three counts are right at once; `updatedAt` trails a review by up to 13 s** (docs/02 §4). New comments are noticed from the counts, which are already in the record.
- **Asking for comment IDs on every poll doubles its cost, and with threads it's six times.** Asking only for a PR whose count moved costs one point when it happens.
- A reply in a thread arrives as a new review, so the review list covers replies. The timeline doesn't.
- GraphQL's `databaseId` is the ID the REST endpoints take, so an agent can fetch one comment directly.

**Required checks and reviews**
- **The owner's private repositories can't have them** (no paid plan), so they matter only on public repositories, the owner's or other people's (docs/02 §4, observed only).
- **One commit can carry more than 100 checks** (187 seen), which is more than one page.
- Which checks are required can be asked per PR, but not in a fragment shared across PRs.
- `mergeStateStatus` never read `DRAFT`; `isDraft` is the draft signal.

**Mergeability**
- **`mergeable` is `UNKNOWN` until asked for, and again after every push.** It's "not known yet", never a state to act on. On a merged PR it's `UNKNOWN` for good.
- A refused token comes back as HTTP 401 with a REST-style body, not as a GraphQL error (docs/02 §5).

**Clients**
- **Three of the four clients stop waiting for a command after 120 s by default, and one of them kills it** ([docs/03 §5](03-clients.md#5-what-this-means-for-circitor)). A `wait` must end by itself in time, and be cheap to repeat with a cursor.
- Background commands exist in two clients only. The design can use them and can't need them.
- With Claude Code's sandbox on, the socket needs an allow entry.

**Platform**
- **A socket path is at most 104 bytes on macOS**, and macOS has no runtime directory ([docs/04 §4](04-platform.md#4-sockets-and-directories)).
- **`brew services` needs a formula; the owner's tap ships casks** (docs/04 §2).
- gh's token can expire and be rotated (docs/02 §1). circitor reads it again when GitHub refuses it, and never stores it.
- A laptop sleeps. Timers, the cadence and a `wait`'s deadline have to behave across a suspend.

**Safety**
- **Read-only towards GitHub.** circitor sends queries, never mutations.
- **The token is never logged, stored or sent to a client.**
- PR titles from private repositories are served only to the local user.

## For the spec to decide

- The PR record: its fields, the derived state an agent reads first, and what counts as a change.
- The exact queries, page sizes, and what happens past one page.
- The cadence tiers, what moves between them, and the quota budget.
- `wait`: the conditions, the cursor, the head SHA rule, the default and longest timeout, and the outcomes (satisfied, ended, timed out).
- How long "no checks yet" lasts before `checks-done` holds.
- Tracking: adding and removing a PR by hand, the TTL for merged and closed PRs, and the limits.
- The state file: location, format, and what survives a restart.
- The CLI: commands, the compact default output, JSON output, exit codes.
- How the CLI starts the daemon so it outlives the calling command, when it exits by itself, and whether a foreground mode exists for a service manager.
- The socket's location on macOS and on Linux.
- Configuration: the file, its keys, and the list of owners.
- Errors: no login, a refused token, the network down, the limit reached, GitHub failing.
- Formula or cask, and which release tool.
- Comments: which kinds are reported, what goes with each ID, how many are kept, and what happens to edits and deletions.
- How each client is taught the CLI: a skill, `AGENTS.md`, or both.
- The test strategy: a fake GitHub server, a fake clock, recorded responses.
- What is left out of v1: the MCP shim, the menu bar, Linux packages, Windows, GitHub Enterprise Server.
