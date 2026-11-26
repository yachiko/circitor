# Decisions

**Status:** draft 0.2, 2026-11-25: notify and narrower conditions (D7, D8); the owner's answers to the planning questions (D9, 2026-11-26). Draft 0.1, 2026-11-24.

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
| D7 | Telling a session (`notify`) | **In v1, a Claude Code session can ask to be told once**, through its own inbox socket ([08-notify](08-notify.md), U14). The subscription is refreshed at the normal rate, not hot (NTF-5); a daemon that stops in order tells each session, and subscriptions live in memory only (NTF-9, NTF-10). It narrows the non-goal "the Unix socket is the only transport" to "the only way in". Everything that posts is a (proposal): the message format is known only from Claude Code's debug log (docs/07). Decided 2026-11-25. See below. |
| D8 | Narrower conditions | **`--kind` on `changed` and `--fail-fast` on `checks-done`**, for `wait` and `notify` alike (API-10, API-11, CLI-5, CLI-6). Decided 2026-11-25. See below. |
| D9 | Gaps found while planning | **The owner's answers to the plan's open spec issues**, written into the specs they touch: `fail_fast` doesn't wait for `settled` (API-10); `again:` repeats `--timeout` and `--json` when given, and `more events:` names the PR after a per-PR answer (CLI-3, CLI-6); durations leave out a zero second unit (CLI-3); the guide is under 120 lines (CLI-11); the fake GitHub marks check names and logins by default (TST-11); a forward clock step is a sleep (NTF-4, TST-8 `clock-back`); `pr_failing`'s notice, the exit-11 line, a cancel during a post (NTF-3, NTF-8, NTF-11). Decided 2026-11-26. See below. |
| D10 | Setting the socket's mode | **Bind at a temporary name, set 0600, rename into place** (OPS-4, STA-12), in place of binding under a umask of 0177: the umask is the whole process's, so a directory another thread made meanwhile came out 0600 (found by T18, 2026-11-26). Decided 2026-11-26. |
| D11 | What `refresh` refreshes | **Every tracked PR**, whatever its period, besides finding (POL-15, API-16): an ordinary poll made now would refresh only what's due, so in the idle tier a `refresh` a minute after a poll would refresh no record (found by T21, 2026-11-26). Decided 2026-11-26. |
| D12 | Refreshing after a failure | **A PR whose last refresh failed is due at the next poll**, whatever its period (POL-19, MOD-16): by POL-13's send times alone the try that recovers left those PRs stale for one more period (found by T26, 2026-11-26). Decided by the orchestrator under the owner's rule of 2026-11-26. |
| D14 | Refreshing after a sleep | **A record marked slept is due at the next poll**, whatever its period (POL-22): the poll that starts on waking otherwise sent only the finding, the monotonic clock having hardly moved, and left every record stale for one more period (found by T31, 2026-11-26). Decided by the orchestrator under the owner's rule of 2026-11-26. |
| D15 | The limit's warning when paging stops | **A finding page left unread because every place is taken counts as PRs left out**, so `pr_limit` is raised (STA-3, POL-7, API-5); it clears at a poll whose finding is read to its end with none left out. Stopping the paging stays, so a full set costs no extra page a poll (found by T32, 2026-11-26). Decided by the orchestrator under the owner's rule of 2026-11-26. |

## Open

None.

## Notes on resolved decisions

**D9: gaps found while planning** (resolved 2026-11-26)
- Raised as spec issues in `plan/README.md` (numbers 23, 24, 30, 32, 33, 38, 39, 41, and two readings of 08-notify's third pass). The owner took the recommended answer each time.
- **`--fail-fast`:** a failed check answers at the first refresh that shows it, `settled` included; TST-8's `fail-fast` row already read that way.
- **`again:`** repeats every parameter given, so it runs as printed. **`more events:`** names the PR when the cursor covers one PR, so running it doesn't show every PR's events.
- **A forward step of the wall clock** can't be told from a sleep (POL-22), so a subscription's `for` counts it, as it counts a sleep. Not counting wall-only sleep would let a 1-day notify outlive a night on a platform whose monotonic clock stops in suspend.
- **`pr_failing`'s notice** is the `auth` sentence with its own code.
- **The exit-11 line** names the PR as typed: `notify` without a session exits before it starts a daemon or asks GitHub. After `no_session` the one line is all it prints.
- **A notice already being posted** isn't counted by a cancel and isn't replaced: nothing waits on anything, as NTF-6 has it.
- **The guide's cap** goes to 120 lines: everything CLI-11 asks for doesn't fit in 70.
- **Check names and logins carry markers by default** in the fake GitHub, so every scenario proves NTF-8 and OPS-9, not only the notify ones.
- Left to the executors, fixed by golden files and reviewed in their reports: the text of `track`, `untrack`, a `refresh` that met a problem, an `activity` with no items, and the stream of CLI-6's endings without an answer (issue 23).
- Left as it is: which HTTP status an exhausted limit has (docs/02 §6); the client handles a 403 and a 200 alike.
- Recorded the same day: the full listen queue's `EAGAIN` (docs/05, docs/07), which NTF-6 and TST-16 cite.

**D7: telling a session** (resolved 2026-11-25)
- The owner's idea, after seeing one Claude Code session message another: the daemon keeps a session's address, a PR and a condition, and posts to the session when the condition holds. Measured the same day (docs/07): a script can post into a session, and an idle session wakes on it.
- **Cadence:** the owner chose the normal rate. A hot PR costs about 240 points an hour, and a subscription can stay open for a day.
- **On an orderly stop** the owner chose to tell each session and drop the subscription, over saving subscriptions in the state file. A crash tells nobody.
- **One notice per subscription.** A session that wants more asks again; the notice carries a cursor, so nothing between two is lost.
- **A confirmation is posted when registering**, because the socket never answers: a held or refused notice looks like a delivered one from outside, and only the agent can tell (NTF-7).
- **A session that bypasses permission prompts holds notices** unless its user sets `crossSessionInbound` to `accept` (docs/07, verified). circitor never claims a permission mode to get past that (NTF-6).
- **`notify`'s exit code doesn't carry the condition** (NTF-11): 0 would otherwise mean both "registered" and "passed".
- `wait` is unchanged, and stays the way for OpenCode and pi.

**D8: narrower conditions** (resolved 2026-11-25)
- Raised while drafting D7: `changed` fires on any change, so an agent waiting for a review was woken by a check finishing; and `checks-done` reports a failure only when the last check has finished.
- `--kind` filters `changed` by MOD-15's event kinds. With it, a merge or a close ends the wait unless its kind is named.
- `--fail-fast` is the flag `gh pr checks --watch` has. Exit code 8 then also means "one failed, others may still run".

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
