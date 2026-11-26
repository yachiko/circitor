# Notify

**Status:** draft 0.3, 2026-11-26: what a stop means (D19). Draft 0.2, 2026-11-26: a subagent's notify (D18). Draft 0.1, 2026-11-25: notify (D7). Owner's answers to the planning questions (D9, 2026-11-26). Hardened: pass 1 (2026-11-25), pass 2 (2026-11-25), pass 3 (2026-11-25).

A Claude Code session asks to be told once about a PR, and goes on working or sits idle. circitor posts one message into that session when there's something to tell (U14, D7). `wait` stays the way to block, for every client ([04-api](04-api.md)).

The facts about the session socket are in [docs/07](../docs/07-notify.md). **The message format there is known only from Claude Code's debug log, and only Linux is measured, so everything that posts is a (proposal) until that changes.**

## Subscriptions

- **NTF-1** **A subscription** is a request to be told once, held by the daemon without a connection: a session's socket path and token, a PR, a condition with its parameters (API-10), and an expiry. `notify` makes one (NTF-11). **It holds its PR by node ID** (MOD-1), and remembers the name it was registered under; a notice names the PR as its record does when the notice is built. (proposal: a rename isn't measured, docs/02 §6) It's for Claude Code only: the socket is Claude Code's (docs/07). Whether OpenCode or pi has anything like it wasn't checked.
- **NTF-2** `POST /v1/prs/{owner}/{repo}/{number}/notify` registers one. Its body holds `until` and `socket`, and optionally `token`, `for`, `limit` (API-8), and `sha`, `rerun_since`, `since`, `fail_fast` and `kind`, with API-10's meanings: `fail_fast` is a JSON boolean, and `kind` one comma-separated string.
  - `socket` is an absolute path that names a socket owned by the user, checked when registering. There's no rule on its length: the path is Claude Code's, and one that can't be connected to answers `no_session`. `token` is a string or `null`. `for` is the subscription's life in seconds: default **86 400**, from 60 to 86 400. Anything else is `bad_request` (API-1). (proposal: the values)
  - **Registering evaluates once, as a `wait` with `timeout=0` does** (API-9, API-12): the read-through if due, the arrival refresh (API-13), one evaluation. **A PR loaded from the state file with no record is read through**, as `show` reads it (STA-8): a subscription isn't held for it as a wait is. It doesn't make the PR hot (NTF-5). **The notify routes name their PR for STA-1's 60 minutes only** (STA-1).
  - **In the `auth` state (POL-20) registering answers `auth` (API-3), unless the answer is already known:** nothing can be told until a person logs in.
  - **If that evaluation's outcome is `satisfied` or `ended` (API-11), it answers as that wait would** (`outcome`, `reason`, `record`, `events`), with `registered: false` and `expires_at` `null`. Nothing is kept and nothing is posted.
  - **Otherwise the subscription is kept**, the confirmation is posted (NTF-7), and it answers `registered: true`, with `expires_at`, `outcome` and `reason` `null`, the record, the events since `since` (none without it) and the cursor.
  - **The confirmation is posted before the answer.** **A confirmation that can't be posted, for any reason, isn't retried:** nothing is kept, and the answer is `no_session` (API-3). So is a `socket` that isn't a socket of the user's. `bad_request` stays for one that isn't an absolute path.
  - **A `since` that is too old when registering** (STA-9) is answered with `reset: true` and the records, and then let go: the subscription's cursor is the one at registration.
  - **A registration, in order:** the parameters (API-1); the `socket` check, a `stat` (`no_session` if it isn't a socket of the user's); the read-through and the arrival refresh; then, in one step with no refresh applied between, the evaluation, the `auth` state, the limit, and the subscription entered; the confirmation; the answer. **So a bad `socket` never reaches GitHub**, and gets `no_session` even when the answer would be known.
  - **The answer is built from the registration's view**, the one its evaluation read (MOD-13): its record, events and cursor are that view's, so its `cursor` is the subscription's (NTF-8). A refresh applied while the confirmation is posted is in the notice, not the answer.
  - **A refresh applied while the confirmation is posted is evaluated as usual, but its notice waits for the confirmation:** it's posted after it, and not at all if the confirmation fails. The answer is still `registered: true`. A confirmation that fails takes the subscription out again, and the one it replaced with it.
  - **Registering again** for the same `socket`, PR, `until`, `sha`, `fail_fast` and `kind` replaces the earlier subscription, so the route is safe to repeat (CLI-10). The answer then has `replaced: true`, and the confirmation is posted again. It's replaced also when the new request's answer is already known: the earlier one is dropped, nothing is posted, and the answer has `registered: false` and `replaced: true`. **They're compared as the daemon reads them:** the PR by node ID, `sha` lower-cased as given (a short and a full SHA differ), `kind` as a set, and `fail_fast` left out as `false`. **Requests that differ in any of these are held side by side.**
  - At most 64 subscriptions are held, and at most 32 for one socket, under the 50 messages a session queues (docs/07, documented); one more gets `limit` (API-3). They aren't waiters, and don't count against API-14's 64. A replacement takes no new place and never gets `limit`. The limit is checked after the evaluation, so an answer already known is never refused. (proposal)
- **NTF-3** `DELETE /v1/prs/{owner}/{repo}/{number}/notify?socket=…` drops that socket's subscriptions on that PR, and answers with their number, `cancelled`. Nothing is posted. It's idempotent. **It never reads through:** on a PR that isn't tracked it answers `cancelled: 0`. It also drops a subscription registered under the name given, whatever its PR is called now. **A subscription whose notice is already being posted has ended:** a cancel doesn't count it and doesn't stop the post, and a registration doesn't replace it (NTF-2, D9). (proposal, as NTF-1's)

## While one is open

- **NTF-4** **A subscription is evaluated as a waiter is:** after every applied refresh of its PR (STA-4), with API-10's conditions and API-11's outcomes. `head_moved`'s 60 s run from the registration. It ends on the first of:

| End | When | Posts |
|---|---|---|
| `satisfied` | The condition holds | The outcome (NTF-8) |
| `ended` | API-11's ends, with its reasons, as for a wait with the same parameters | The outcome |
| `expired` | `for` has passed: on the monotonic clock plus the sleep noticed since registering, as a waiter's timeout runs (POL-22) | `expired` |
| `stopped` | The daemon stops in order (NTF-10) | `stopped` |
| `problem` | Checked at each tick (POL-26): the daemon has been in the `auth` state for 10 minutes (POL-20), or its PR has `pr_failing` raised (POL-21, API-5) | `problem` |
| cancelled | NTF-3, or replaced (NTF-2) | Nothing |
| session gone | Posting fails (NTF-6), or at a tick (POL-26) the socket path is missing or is no longer the socket it was when registering (the same device and inode) | Nothing |

  - **A PR with a subscription stays tracked**, as one with a waiter does (API-9, STA-1). **A subscription's end counts as the end of a request that named its PR** (STA-1): an `asked` PR stays 60 minutes after it, and an ended PR past `keep_done` stays as long, so the notice's command is answered from memory.
  - **`expires_at` is the wall time expected when registering.** A clock step back doesn't move the expiry. A step forward can't be told from a sleep (POL-22), so it counts as one, and the catch-up below applies (D9).
  - **A subscription doesn't wait out a problem only the user can end:** `problem` tells the session, which is the one placed to tell the user. A `wait` in the same spot would have shown the problem within its timeout (CLI-3). (proposal: the 10 minutes)
  - **The 10 minutes run on the monotonic clock from when `daemon.problem` became `auth`** (POL-20). A try with a new token that fails doesn't restart them. A subscription registered while its PR has `pr_failing` raised posts `problem` at the first tick.
  - **Expiry never beats a catch-up.** When `for` has passed at a tick that noticed a sleep (POL-22), or while its PR's record is `stale` (MOD-16), the subscription is evaluated first at the next applied refresh of its PR, and expires only if it didn't end there. With none within 60 s, it expires as it is. (proposal: the 60 s)
  - **A subscription searches the event history once**, in its registration's evaluation, as a wait does: for an event of a named kind (`kind`), and for a `checks` event after `rerun_since`. What that finds is remembered. After it, the subscription looks only at events recorded after that evaluation, those recorded while the confirmation was being posted included, and never at the history again (STA-10), which a day can outlast. A cursor that goes too old while it's held (STA-9) neither satisfies it nor drops a part.
  - **Every subscription's socket is checked at each tick**, with a `stat`, never a connection, so a session that has ended doesn't hold the active tier for the rest of `for`. The device and inode also catch a path taken over by another session. A `stat` that fails for another reason than a missing path changes nothing at that tick, and is logged once. (proposal: a session that ends in order removes its socket (docs/07, verified); one that is killed may leave the file, which isn't measured)
- **NTF-5** **A subscription doesn't make its PR hot** (D7): its PR is refreshed by the ordinary poll, at its tier's rate whatever its `source` (POL-10). A PR that has ended is refreshed every `idle_interval`. **While a subscription on an open PR is held, the poller stays in the active tier** (POL-13); once it ends, the tier follows the last client request. **While any subscription is held, a spawned daemon doesn't exit idle** (OPS-8). One on a PR that has ended keeps its PR tracked (NTF-4) and the daemon alive, and is served by the `idle_interval` refresh. So a notice follows GitHub by about `interval`, 30 s, at no cost beyond the active tier's. **That tier is the cost:** about 120 points an hour, where an idle daemon spends 12 (POL-13). Slowed, capped, rate-limited or in a backoff (POL-23, POL-19, POL-25) it comes later: the subscription waits, and never fails for that. It doesn't wait out a problem only the user can end (NTF-4's `problem`).

## Posting

- **NTF-6** **A notice is posted in one connection:** connect to the socket; if there's a token, send `{"type":"auth","token":"<token>"}` and a newline; send `{"type":"user","message":{"role":"user","content":"<text>"}}` and a newline; close (docs/07). **No answer is read: none comes** (docs/07). Connecting and writing have 2 s together.
  - **Once its line is written, a notice is never sent again.** circitor can't know whether it was delivered, held, refused or read (docs/07).
  - **A peer that doesn't read isn't a failure:** the line fits the socket's buffer, and counts as posted.
  - **A connect or a write that fails, or isn't done in 2 s, before the line is written in full, ends the subscription:** it's dropped, and that's logged (OPS-9). **Nothing is tried again**, as for the confirmation (NTF-2): a notice is a hint (NTF-8). A full listen queue fails at once, with `EAGAIN` on Linux (docs/07, verified; macOS refuses it instead, verified 2026-11-26).
  - **Before connecting, the path is checked as at a tick** (NTF-4): a socket that isn't the one registered means the session is gone.
  - **circitor never states a permission mode or a reply address**, whatever that would unlock (docs/07): it posts as what it is, a script.
  - Notices are sent apart from the requests to GitHub (POL-2). Posting never delays a poll, or any answer but NTF-2's.
- **NTF-7** **The confirmation** tells the agent that notices reach this session:

  ```
  circitor: watching yachiko/circitor#12 until checks-done on 1a2b3c4, for 1d, asked here or by a subagent. If you can read this, notices reach this session.
  ```

  **The condition is written the same way here and in `registered:`** (NTF-11): `<pr> until <until>`, then, only where given and in this order, ` on <sha>` (its first 7 characters, lower case), ` after <rerun_since>`, ` fail-fast`, ` kind <kinds>` and ` since <since>`; then `, for <for>` (CLI-3's durations). For example:

  ```
  circitor: watching yachiko/circitor#12 until changed kind review since k3x9q2mz-418, for 2h, asked here or by a subagent. If you can read this, notices reach this session.
  ```

  **The words after `for` are fixed:** `, asked here or by a subagent`, as a subagent's confirmation arrives in the main conversation (NTF-11, D18). `registered:` doesn't carry them.

  The confirmation is the only way a held or refused notice is noticed: only the session sees what arrived (docs/07). **The confirmation arrives with the command's result:** it's posted before the command returns, and a busy session gets a message between two tool calls (docs/07). **It arrives in the main conversation, also when a subagent asked** (NTF-11), so only the main conversation can judge it. When the main conversation doesn't see it straight after `notify`'s output, it **cancels it (`notify --cancel`) and uses `wait`**: a subscription the session can't hear still holds the active tier (NTF-5). **A subagent never cancels:** a cancel drops every notify of the session's socket on that PR, the main conversation's too (NTF-3). The guide says so (CLI-11).
- **NTF-8** **A notice's text is circitor's own words.** It holds the PR's name, CLI-6's outcome line or the end's name, short SHAs, numbers, a cursor and circitor commands. **It never holds a title, a check's name, a login, a URL, or anything else a GitHub user wrote** beyond the PR's name: a notice enters a session unasked, and such text could steer an agent. One line each:

  ```
  circitor: yachiko/circitor#12 checks failed (1 of 5) on 1a2b3c4. Details: circitor changes yachiko/circitor#12 --since k3x9q2mz-418
  circitor: yachiko/circitor#12 merged. Details: circitor changes yachiko/circitor#12 --since k3x9q2mz-418
  circitor: yachiko/circitor#12 ended: closed. Details: circitor changes yachiko/circitor#12 --since k3x9q2mz-418
  circitor: notify for yachiko/circitor#12 expired after 1d; it's checks_pending (stale, 2d5h old). Ask again: circitor notify yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --since k3x9q2mz-418
  circitor: can't read yachiko/circitor#12 from GitHub (auth); notify is cancelled. Details: circitor status
  circitor: the daemon is stopping; notify for yachiko/circitor#12 is cancelled. Ask again: circitor notify yachiko/circitor#12 --until checks-done --sha 1a2b3c4 --no-spawn
  ```

  - The cursor is the subscription's `since`, or the one at registration, so `changes` shows everything since then while the history holds it (STA-10). **When that cursor is too old as the notice is built** (STA-9), the notice names `show` in place of `changes`: `circitor: yachiko/circitor#12 merged. History too old; details: circitor show yachiko/circitor#12`.
  - `Ask again` repeats the parameters given, `--for` included, with `--since` set to the subscription's cursor (its `since`, or the one at registration), as CLI-6's `again:` does. **The `stopped` notice writes `--since` only for `changed`** (D19). After a stop the cursor is from another epoch. For `merged` and `checks-done` it would only set where the answer's events start (API-10), so it would buy a `reset: true` and nothing else. For `changed` the new daemon answers at once, with `reset: true` (STA-9): what happened during the stop can't be told. `--rerun-since` stays as given: its `reset: true` is the only sign that its part was dropped (API-10). Otherwise the re-ask registers again, and the new subscription takes the new daemon's cursor (NTF-2). With `--fail-fast`, the outcome can be CLI-6's `checks failing (1 failed, 3 of 5 done) on 1a2b3c4`.
  - The `problem` notice names the daemon's problem code (`auth`, API-4), or `pr_failing`, and never GitHub's text. For `pr_failing` it's the same sentence (D9): `circitor: can't read yachiko/circitor#12 from GitHub (pr_failing); notify is cancelled. Details: circitor status`.
  - The `expired` notice gives the record's `status`, with CLI-3's stale mark when the record is stale, as in the example, and without it otherwise.
  - **A notice is a hint, never the record.** Whatever it says is also in `changes` and `show`; one that's lost loses nothing.
  - The exact text is fixed by golden files (TST-7). (proposal: the layout)

## Keeping and stopping

- **NTF-9** **Subscriptions are held in memory only.** They're never written to `state.json` (STA-6), and a daemon that starts has none. **The session token is held only while its subscription lasts**, and is never written to a file, a log, an error message or an API response, like GitHub's (OPS-5). **The daemon's own environment never holds a session's socket or token** (CLI-7). `status` shows their number as `subscriptions` (API-5).
- **NTF-10** **A daemon that stops in order tells each session** (D7): OPS-7's stop posts `stopped` to every open subscription before it closes its connections: **all together, not one at a time, with 1 s for all**, inside its 5 s deadline. Those it hasn't reached by then are dropped, and that's logged. **Asking again starts no daemon, but waits for one:** the `stopped` notice's command has `--no-spawn` (CLI-7), and with it and no daemon running, `notify` waits up to 8 s for one to answer, connecting as CLI-7 does, as CLI-10's wait does. After a replacement (CLI-9) or a takeover (OPS-6) the new daemon answers. So does one another client started meanwhile (CLI-7, CLI-8): the re-ask then registers with it. Only after those 8 s does the command exit 6, and the guide tells the agent that the daemon was stopped on purpose (CLI-11). **A stop the daemon chose itself** (its socket or lock file not its own, STA-11) **posts the command without `--no-spawn`:** nobody asked for that stop. **A crash tells nobody.** A subscription is also lost when its session is resumed: the socket belongs to the session's process (docs/07), so a resumed session asks again.

## The command

- **NTF-11** `circitor notify [PR] --until merged|checks-done|changed [--sha SHA] [--rerun-since CURSOR] [--fail-fast] [--kind KINDS] [--since CURSOR] [--for S]` registers a subscription for the session it runs in, and returns at once. `circitor notify [PR] --cancel` drops this session's subscriptions on that PR. `PR` is named as for `wait` (CLI-2). Both take `--json` (CLI-4). Registering starts the daemon when needed (CLI-7). **`--cancel` never starts one** (CLI-8): with none running it prints `cancelled: 0` and exits 0.
  - **The session is the one in the environment:** `CLAUDE_CODE_MESSAGING_SOCKET`, and `CLAUDE_CODE_MESSAGING_TOKEN` if it's set (docs/07). **Without the first, or when the daemon can't reach that socket (`no_session`, NTF-2), the command exits 11**, with one line naming the `wait` to run in its place: `no session here: use circitor wait <pr> --until <until>`, with `--sha`, `--rerun-since`, `--fail-fast`, `--kind` and `--since` as given, and **never `--timeout`**: `--for` has no counterpart (API-12). **Without the socket it exits before anything else**, so `<pr>` is the PR argument as typed, and left out when none was given: `wait` names it the same way (CLI-2). After `no_session` it's the PR in full. That line is all it prints; the daemon's reason is in its log and in `--json` (D9).
  - **A subagent is told nothing:** its shell has its parent's socket and token (docs/07), so its confirmation and notice go to the main conversation, and its `--cancel` drops the main conversation's notifies on that PR too (NTF-3). circitor doesn't tell a subagent from its session, and registers as for any other. (proposal: nothing else in a subagent's environment was found to differ from its session's; measured on Linux only, 2026-11-26, not yet in docs/07) The output's third line and the guide say so, and send a subagent to `wait` (CLI-11, D18).
  - **When the answer is already known** (`registered: false`), it prints what `wait` prints (CLI-6), with `already:` before the outcome line.
  - **Otherwise** it prints:

    ```
    registered: yachiko/circitor#12 until checks-done on 1a2b3c4, for 1d
    yachiko/circitor#12  checks_pending  1a2b3c4  checks 3/5, 0 failed  review none  "Add the poller"
    a confirmation was posted to this session's main conversation; if none arrives there, notices are held or refused: run circitor notify yachiko/circitor#12 --cancel, then use circitor wait. A subagent never sees it: it uses circitor wait, and never cancels.
    cursor: k3x9q2mz-418
    ```

    The second line is CLI-3's one line for the PR, so one that has already ended shows as `merged` or `closed` at once.

  - **The order of the lines**, in both cases: the reset and the events; `already:` and the outcome line, or `registered:`, the PR's line and the confirmation line; `replaced an earlier notify` with `replaced: true`; `more events:`; `again:`; `problem:` (CLI-3); `cursor:`.
  - **A registered answer with `reset: true`, or with events, prints them first**, as `wait` does (CLI-3): a reset is never silent.
  - **When `already:` comes with `reset: true`**, nothing since the reset is known to match, and the output ends, before the cursor, with `again: circitor notify <pr> --until … --since <new cursor>`, the other parameters as given.
  - `--cancel` prints `cancelled: <n>`. Outside a session it exits 11, with a line that names no `wait`.
  - **Its exit code says only that the command worked:** 0 for registered, for already known and for cancelled, whatever the checks did. It doesn't follow `wait`'s codes 3, 4 and 8, so `circitor notify … && next-step` proves nothing; the guide says so (CLI-11). API errors keep their codes (CLI-5). (proposal)

## For the user

- **NTF-12** **The README says where notices work** (docs/07). They arrive in a session that asks before acting, with `crossSessionInbound` unset (verified on Linux). In one that bypasses permission prompts they're held: an unattended one never gets them (verified); an interactive one asks its user, for 5 minutes (documented, not measured). With `accept` they arrive in both, **and so do messages from every local sender, not only circitor's**. With `hold` or `refuse`, never. It says that Claude Code shows a notice as a message from another session, and that notices rest on an interface Claude Code doesn't document: if a release changes it, confirmations stop arriving, and `wait` still works. (proposal)
  - **In an unattended run** (`claude -p`, the SDK, CI) **a notice arrives only while the session still works:** the session ends with its turn, and its socket with it (docs/07). There a `wait` in the background is the way (docs/03 §2). The README and the guide (CLI-11) say so.
  - **After a session is resumed, or the daemon crashes, a notify is lost** (NTF-10): the README and the guide say to ask again.
  - **A subagent's notify reaches the main conversation**, not the subagent (docs/07): the README says so, and that a subagent uses `wait` (D18).
