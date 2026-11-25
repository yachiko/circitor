# Decisions

**Status:** draft 0.1, 2026-11-24.

The spec's decisions, numbered D1, D2, … in the order they were raised. [docs/06 decision log](../docs/06-decision-log.md) is the project's history up to the first spec draft. It uses its own numbers, so cite those as "log D5". New decisions go here.

## Carried from the log

Decided before the spec and binding on it: one local poller (log D1), polling without webhooks (log D2), Rust (log D3), the CLI as the contract with MCP later (log D4), a Unix socket (log D5), memory plus a small state file (log D6), gh's login and no `gh` extension (log D7), the menu bar later (log D8), Homebrew from the owner's tap (log D9), cadence by demand (log D10), finding and refreshing as separate steps (log D11).

## Resolved

| # | Decision | Outcome |
|---|---|---|
| D1 | How the daemon starts | **The CLI starts it on first use, and it exits when idle** (CLI-7, OPS-8). It can also run in the foreground for a service manager (OPS-6). No `brew services` in v1. Decided 2026-11-24 (log D14). |
| D2 | What is tracked | **The user's own open PRs, filtered to configured owners, plus PRs added by hand** (POL-4, POL-5, STA-1). No free-form searches. The owner has no organisation, so `owners` is empty by default. Decided 2026-11-24 (log D15). |
| D3 | Comments and reviews | **What's new, with IDs and the path to fetch each, never the text** (MOD-12, POL-12). Decided 2026-11-24 (log D16). See below. |
| D4 | Hosts and accounts | **github.com and gh's active account** in v1 (OPS-5). Decided 2026-11-24 (log D17). |
| D5 | Formula or cask, and the release tool | **A formula written by `dist`** (OPS-10). Decided 2026-11-25. |
| D6 | Measuring required checks and reviews before v1 | **Not before v1:** `blocked` and `behind` are built as (proposal) (MOD-9, MOD-11); measured later on a small public repository, if the owner creates one. Decided 2026-11-25. |

## Open

None.

## Notes on resolved decisions

**D5: the release tool** (resolved 2026-11-25)
- The tap's other tools are casks written by GoReleaser. `dist` builds Rust releases and writes a formula, not a cask (docs/04 §3).
- A formula isn't quarantined, so the binary needn't be notarised (docs/04 §2). Whether GoReleaser builds Rust wasn't checked (docs/04 §8).

**D6: required checks and reviews** (resolved 2026-11-25)
- They can't be set up on a private repository without a paid plan, so `blocked` rests on observation of other people's repositories, and `behind` on GitHub's documentation alone (docs/02 §4, §6). A check's `required` isn't read in v1 (MOD-3).

**D3: comments and reviews** (resolved 2026-11-24)
- The owner's idea: report changes only, and give the ID of each new comment so an agent fetches that one item instead of the whole PR.
- Measured the same day (docs/02 §4): GraphQL's `databaseId` is the ID the REST endpoints take; the three counts already in the record show a new item at once; asking for IDs only when a count moves keeps a poll at 1 point, where asking every time costs 2 to 6.
- **A submitted review is one item**, with the number of inline comments it carries and the path that lists them. A reply in a thread arrives as a review of its own.
- **Not reported in v1:** edits, deletions, resolved threads. How GitHub shows them isn't measured.
- The full record stays available (`show`), for a first call and for a cursor that's too old.

**D1: starting the daemon** (resolved 2026-11-24)
- Tools whose daemon only serves their own CLI usually start it from the CLI and let it exit when idle (sccache, the Bazel and Gradle daemons, watchman). A service manager is the usual choice where the daemon works with nobody asking.
- The foreground mode is the fallback where starting from the CLI doesn't work. The expected case is an agent's sandbox, which a daemon started inside it would inherit (OPS-14, untested).
