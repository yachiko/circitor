# CLI

**Status:** draft 0.1, 2026-11-24. Hardened: pass 1 (2026-11-25), pass 2 (2026-11-25), pass 3 (2026-11-25).

**The CLI is circitor's contract with agents and people** (log D4). Its commands, text output and exit codes stay stable; the API behind it may change.

## Commands

- **CLI-1** The commands:

| Command | Does | API |
|---|---|---|
| `circitor list [--all] [--since CURSOR]` | The tracked PRs | API-6 |
| `circitor show [PR] [--fresh]` | One PR, with its checks | API-7 |
| `circitor wait [PR] --until merged\|checks-done\|changed [--sha SHA] [--rerun-since CURSOR] [--since CURSOR] [--timeout S]` | Blocks until the condition holds | API-9 |
| `circitor changes [PR] [--since CURSOR]` | What happened since a cursor | API-8 |
| `circitor track PR`, `circitor untrack PR` | Add or remove a PR by hand | API-15 |
| `circitor refresh` | Poll now | API-16 |
| `circitor status` | The daemon's health | API-5 |
| `circitor stop` | Stop the daemon | API-17 |
| `circitor daemon` | Run the daemon in the foreground | OPS-6 |
| `circitor guide` | Print the instructions for agents | CLI-11 |
| `circitor version`, `circitor help` | – | – |

  - `--json` (CLI-4) is taken by `list`, `show`, `wait`, `changes`, `track`, `untrack`, `refresh` and `status`.
  - `changes` without `PR` covers every tracked PR (API-8).
- **CLI-2** **Naming a PR.** `PR` is one of:
  - `owner/repo#12`, or the PR's URL
  - `12`: the repository comes from the `origin` remote of the git repository in the working directory. `#12` isn't taken: a shell reads a word starting with `#` as a comment, so `circitor show #12` would run as `circitor show`.
  - nothing, for `show` and `wait`: **the open tracked PR whose head branch is the current branch** of that repository. The current branch is the name `git` pushes it to on `origin`: its upstream's branch, or its own name without one. A detached HEAD is exit code 2. **Only a PR whose `author` is the login in use** (API-5's `github.login`) matches: a contributor's PR from a fork can share the branch name. With no open match, the CLI asks for a poll (API-16) and looks again: a PR just opened isn't tracked before a poll finds it (POL-4). **The poll's answer doesn't end the search:** after `shared` or a problem, the CLI still looks again in memory. Only then does it take an ended match (`merged`, `closed` or `gone`), the one with the latest `closed_at` (for a gone one, its `fetched_at`), and its output says so. Two open matches are exit code 2, listing them.
  
  If the working directory isn't a git repository with a github.com `origin`, or no tracked PR matches the branch, the command fails with exit code 2 and says which form to use. **After the poll, exit code 2 means GitHub was read and no PR matches.** If the poll answered a problem and nothing matched, the command exits with that problem's code (CLI-5). After `shared`, the line adds that a PR opened seconds ago may not be found yet, and to name it by number. The short forms are resolved by the CLI with `git`, not by the daemon. The repository is `origin`'s. In a clone of a fork, where the PR belongs to the parent repository, use the full form (PRs from forks aren't measured, docs/02 §4). (proposal: the short forms)

## Output

- **CLI-3** **Text is the default and is compact**, because agents pay for every line. One line a PR:

  ```
  yachiko/circitor#12  checks_pending  1a2b3c4  checks 3/5, 0 failed  review none  "Add the poller"
  ```

  - `show` adds one line per check that failed or is still running, with its URL, and lines for `mergeable`, `merge_state` and the counts. Passed checks are summarised as a number. On `mergeable unknown` it adds `mergeable unknown: GitHub is computing it; circitor wait <pr> --until changed`.
  - `changes` and the end of `wait` print one line per event kind, and one per new item (MOD-15, MOD-12). One example per kind:
    ```
    yachiko/circitor#12  status checks_pending -> checks_failing
    yachiko/circitor#12  head 1a2b3c4 -> 9f8e7d6
    yachiko/circitor#12  checks pending (3 of 5, 1 failed)
    yachiko/circitor#12  checks failing (1 of 5): build https://github.com/…
    yachiko/circitor#12  merged
    yachiko/circitor#12  draft false
    yachiko/circitor#12  mergeable no (dirty)
    yachiko/circitor#12  review changes_requested
    yachiko/circitor#12  new review changes_requested by octocat, 3 inline  gh api repos/yachiko/circitor/pulls/12/reviews/604123  gh api repos/yachiko/circitor/pulls/12/reviews/604123/comments
    yachiko/circitor#12  new comment by octocat  gh api repos/yachiko/circitor/issues/comments/918273
    yachiko/circitor#12  edited title, base
    yachiko/circitor#12  tracked (manual)
    ```
    `closed`, `reopened`, `gone` and `untracked` print like `merged`.
  - **Durations** are the largest unit and the next: `45s`, `4m`, `1h3m`, `2d5h`.
  - **A title** is in double quotes, with `"` and `\` escaped by `\` and control characters as `\u{..}`. It's never cut.
  - **After the events, in this order:** the `pending` and `untracked` lines, the outcome (`wait`), `more events:`, `again:`, `problem:`, and `cursor:`.
  - `refresh` prints `refresh: polled, 1 point` or `refresh: shared`; `stop`, `stopped` or `not running`. `status` prints one `key: value` line per key of API-5, in its order, and the CLI's version when it differs.
  - **A new comment or review is printed with the command that fetches it** (MOD-12), and a review with `inline` above 0 also with the one that lists its comments (`fetch_inline`): a thread reply's text is only there.
  - `more: true` prints `more new items than shown` with the paths that list them.
  - **`changes` and `wait` print at most 50 events** (`limit`, API-8), so the output stays under the clients' cut (docs/03 §2: 30 000 characters in Claude Code). With more, they end, before the cursor, with `more events: circitor changes --since <cursor>`. (proposal: 50)
  - **A reset is never silent.** When the answer has `reset: true` (STA-9), `changes` and `wait` first print `reset: cursor too old, current state follows` and one `list` line per record in the answer, then the events.
  - A stale record is marked `(stale, 4m old)`, and `(stale, 4m old, error FORBIDDEN)` when its own refresh fails (MOD-2 `error`).
  - **A daemon problem is never silent.** When `daemon.problem` (API-4) isn't `null`, the output ends, before the cursor, with one line: its code and its message, which says what to do. A time is relative, with the local time after: `problem: rate_limited for 12m (until 14:05 local), …`.
  - `list --since` adds a line `untracked <pr>` for each PR dropped since the cursor and not tracked now.
  - `list` and `changes` add a line `pending <pr>` for each PR in `pending` (API-18).
  - The last line of `list`, `changes` and `wait` is `cursor: <cursor>`.
  - `list` without `--since` prints at most 50 PRs, then says how many more there are. **With `--since` it prints every record in the answer**: the cursor covers them all.
  - The exact text is fixed by golden files (TST-7). (proposal: the layout)
- **CLI-4** `--json` prints the API's response body unchanged, on one line.
- **CLI-5** **Exit codes:**

| Code | Meaning |
|---|---|
| 0 | Done. For `wait`: the condition holds, and for `checks-done` the checks **pass** (or there are none). |
| 1 | An unexpected error, or a route the daemon doesn't know |
| 2 | Wrong usage (`bad_request`), or a PR that couldn't be named (CLI-2) |
| 3 | `wait`: timed out |
| 4 | `wait`: ended without the condition (`closed`, `gone`, `head_moved`, or `merged` for `checks-done`) |
| 5 | The PR doesn't exist or can't be seen (`not_found`; that a PR the token can't see answers like a missing one is assumed, docs/02 §5) |
| 6 | The daemon can't be reached or started |
| 7 | No usable GitHub login (`auth`) |
| 8 | `wait --until checks-done`: the checks are done and **some failed** |
| 9 | GitHub can't be read now: `starting`, `github_unavailable` or `rate_limited`. The line says when to try again where the daemon knows (`until`, API-4), and otherwise to try again in a minute. |
| 10 | `limit`: too many tracked PRs or waiters (STA-3, API-14) |
| 130, 143 | Interrupted by `SIGINT` or `SIGTERM` |

  - **Each API error (API-3) has one code.**
  - **Code 8 exists so `circitor wait --until checks-done && next-step` doesn't go on after a failure.** (proposal)
  - **On code 9, `gh` doesn't help**: it spends the same quota (docs/02 §2). The guide says so (CLI-11).
  - Errors go to stderr as one line, and say what to do when there's something to do.

## Waiting

- **CLI-6** `wait` prints nothing until it ends. Then it prints the events (CLI-3), a line for the outcome, and the cursor.
  - **The outcome line says what the wait found, and on which head:**
    - `checks passed (5/5) on 1a2b3c4`
    - `checks failed (1 of 5) on 1a2b3c4`
    - `no checks reported on 1a2b3c4`: code 0, but nothing ran
    - `merged`; `changed`
    - `ended: closed`, `ended: gone`, `ended: merged`
    - `ended: head moved to 9f8e7d6, expected 1a2b3c4`

    Each takes the `(stale, 4m old)` mark when it applies (CLI-3).
  - **With `--sha`, while the head isn't it, the timeout line names both heads** in place of the status: `timeout after 100s: yachiko/circitor#12 head 9f8e7d6, not 1a2b3c4 (stale, 2m old)`. Another head's status never answers a wait for this one.
  - **A wait answered before its PR has a record** (`record: null`, API-9) prints `<pr> no record yet` in place of the status.
  - A truncated `failing` adds `(truncated: unread checks may still run)` to the outcome line.
  - **On a timeout it prints the command to run next**, with the cursor filled in, so repeating a wait loses nothing:
    ```
    timeout after 100s: yachiko/circitor#12 checks_pending (3/5, 0 failed)
    again: circitor wait yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --since k3x9q2mz-418
    cursor: k3x9q2mz-418
    ```
    It names the PR in full (`owner/repo#number`) and repeats the other parameters given, `--sha` and `--rerun-since` only if they were and as they were, with `--since` set to the cursor.
  - **A wait that ends without an answer** prints one line, then `again:` and `cursor:`. Their cursor is the `--since` given or, without one, the first `Circitor-Cursor` it got. Never a later one: it would skip events that weren't printed. `<n>` is the timeout the command was given.
    - no time left: `timeout after <n>s: <pr> (no answer from the daemon)`, exit code 3
    - no daemon within 8 s, or a second drop (CLI-10): `daemon stopped`, exit code 6
    - the CLI's own limit (CLI-7): `daemon not answering on <path>`, exit code 6
  - **Without `--timeout`, the deadline is 100 s**, API-12's default: the CLI always sends `timeout`.
  - **One deadline for the whole command**, from the CLI's start. Starting or replacing the daemon (CLI-7, CLI-9) counts against it, and each request passes the seconds left, rounded down, as `timeout`. A request with under 1 s left isn't sent, unless the user gave `--timeout 0`: the wait ends as a timeout. `--timeout 0`'s limit on the CLI's side is 60 s.
  - `SIGINT` and `SIGTERM` end it at once (CLI-5).

## The daemon

- **CLI-7** **A command that needs the daemon starts it if it isn't running.** If connecting fails, the CLI starts `circitor daemon --spawned` as a detached process: its own session, without `GH_TOKEN` and `GITHUB_TOKEN` (OPS-5), standard streams to the log file, so it never holds the agent's output open, and the root directory as working directory. It then waits up to 8 s for the socket to answer (STA-11's 6 s and start-up), and goes on. If the daemon doesn't come up, exit code 6, with the log's last lines: up to 20 lines the new daemon wrote, from the log's size when the CLI opened it.
  - **Connecting** tries the CLI's socket path (OPS-4), then the one in `circitor.lock` (STA-11), with a warning on stderr naming both when the second answers. Every command connects this way.
  - **Only a missing socket file or a refused connection on both means no daemon.** Any other error starts nothing: exit code 6. A denied permission names `sandbox.excludedCommands` and `sandbox.network.allowUnixSockets` (docs/03 §2); any other error names the path and the system's error. The CLI checks the path's length (OPS-4) before connecting. (proposal; the error each sandbox gives isn't measured, docs/03 §6)
  - **Before starting it, the CLI creates the state directory** (OPS-3, mode 0700), rotates the log if it's over 5 MB (OPS-9), and opens it for appending. If it can't, it exits 6 at once, naming the path and the error.
  - **The CLI watches the process it started.** If it exits with an error before the socket answers, the CLI stops waiting and exits 6 with the log's last lines; a bad config shows as its OPS-1 message. If it exits 0, another daemon holds the lock (STA-11), and the CLI goes on waiting for the socket named in the lock file. So two CLIs starting it at once is harmless.
  - **Every request has a limit on the CLI's side**, so a daemon that accepts and never answers can't hang an agent. (proposal: the values)
    - a `wait`: its deadline (CLI-6) plus 10 s; `--timeout 0`, 60 s
    - any other request: 90 s, under the 120 s at which clients stop waiting (docs/03 §5). A read-through can queue behind a request in flight (POL-2, POL-15) and read several pages (POL-8).

    A `wait` past its limit ends as one without an answer (CLI-6). Any other command exits 6: `daemon not answering on <path>`.
  - `--no-spawn`, or `CIRCITOR_NO_SPAWN=1`, turns this off: the command then fails with code 6.
- **CLI-8** `status`, `stop`, `version`, `help` and `guide` never start a daemon. With none running, `status` says so and exits 6; `stop` exits 0. They connect as CLI-7 does: only a missing socket file or a refused connection on both paths is "not running". They never replace a daemon (CLI-9): `status` shows both versions, and `stop` stops whatever answers.
  - **`stop` returns once the daemon has exited:** after the answer it waits up to 8 s for the lock in `circitor.lock` to be released (STA-11, OPS-7), then exits 0. Still held, it goes on as for a daemon that doesn't answer.
  - **`stop` ends a daemon that doesn't answer.** With no answer within its limit (CLI-7), it sends `SIGTERM` to the process in the lock file (STA-11), and `SIGKILL` 5 s later, only while that lock is still held.
- **CLI-9** **A daemon older than the CLI is replaced.** **A command that needs the daemon reads `GET /v1/status` first.** If its `version` is older than the CLI's, the CLI asks that daemon to shut down, waits up to 8 s for the lock in `circitor.lock` to be released (OPS-7), starts its own, and only then sends the request. Still held, it uses the old daemon and warns on stderr. This is what makes `brew upgrade` take effect (OPS-15). (proposal)
  - **Versions are compared as semantic versions** (OPS-11), ignoring build metadata. A `version` that doesn't parse is never replaced, with a warning.
  - Checking first means no answer is thrown away: an answer with events can't be asked for twice, as the old daemon saves their items as seen and the new one won't report them (STA-5).
  - A newer daemon, or one whose `mode` is `foreground` (API-5), is never replaced: the CLI warns on stderr and uses it.
  - **Before replacing, the CLI checks the config in its own process**, as `circitor daemon --check-config` does (OPS-1). If that fails, it keeps the old daemon and warns on stderr, naming the key.
  - At most one replacement per command.
  - **A `bad_request` or `unknown_route` from an older daemon that isn't replaced** (a foreground one) says the daemon is older than the CLI and how to restart it, and exits 1, not 2.
- **CLI-10** **A wait whose connection drops without an answer runs its `again:` command once** (CLI-6), with the time left. If its `Circitor-Cursor` had arrived, it starts no daemon: it waits up to 8 s, within the time left, for one to answer, connecting as CLI-7 does. A replacement (CLI-9) or a service manager starts the new daemon; a `stop` or a signal leaves none, and stays a stop. A cursor from the old daemon gets `reset: true` from a new one (STA-9), so nothing is lost silently, whatever the condition. The endings are CLI-6's.
  - **Any other command whose connection drops before a full answer is tried once more**, as a new command (CLI-7, which may start a daemon), within its limit. So is a wait whose connection drops before its `Circitor-Cursor` arrived. Every route is safe to repeat (API-15, API-16). A second drop is exit code 6.

## For agents

- **CLI-11** `circitor guide` prints a short text an agent can follow: when to use circitor in place of `gh`, the commands with one example each, the exit codes, the rule to pass `--sha` after a push, the rule to pass `--rerun-since` with the cursor from before starting CI again on the same head (API-10), and **never to loop `show --fresh`**: each one can cost a point, where a `wait` shares one refresh among every agent (POL-15), and that a cursor from one PR's `wait` or `changes` is for that PR only (API-8). The same text ships as a skill and as an `AGENTS.md` section (OPS-12). It's under 60 lines. (proposal)
- **CLI-12** **No command asks a question or opens a pager.** Output is never coloured unless stdout is a terminal.

## For tests

- **CLI-13** **The CLI's inputs come through one interface each**, so the harness (TST-5) can drive them: its clocks (TST-4), its working directory and environment, its version, and the way it starts a daemon. Its durations (CLI-6 to CLI-10) go by the monotonic clock; ages and local times by the wall clock.
