# State

**Status:** draft 0.2, 2026-11-25: notify (D7). Draft 0.1, 2026-11-24. Hardened: pass 1 (2026-11-25), pass 2 (2026-11-25), pass 3 (2026-11-25).

Which PRs the daemon holds, for how long, and what little it writes to disk (log D6).

## The tracked set

- **STA-1** A PR is tracked for one of three reasons, its `source`:

| `source` | How it got in | How it leaves |
|---|---|---|
| `authored` | Found among the user's open PRs, with its owner in `owners` (POL-4, POL-5) | Becomes `asked` when no longer the user's own or outside `owners` (POL-5); STA-2 |
| `manual` | `track` (API-15) | `untrack`, which may leave it `authored` or `asked` (API-15); STA-2 |
| `asked` | A client asked for a PR that wasn't tracked (POL-16) | 60 minutes after the last request that named it ended, or STA-2 (proposal) |

  - A PR tracked for two reasons keeps the stronger: `manual`, then `authored`, then `asked`.
  - `untrack` changes only a `manual` PR (API-15).
  - **A request names a PR** when the PR is in its path or body (API-7 to API-9, API-15, NTF-2, NTF-3). It ends when its answer is sent or its connection closes. A subscription's end counts as one too, and keeps an ended PR past `keep_done` for the same 60 minutes (NTF-4). `list` and `changes` over every PR name none.
  - **`notify`'s two routes (NTF-2, NTF-3) name their PR for this 60 minutes only**, never for POL-11's 60 s or POL-14's 2 minutes.
  - **No PR is dropped while it has a waiter or a subscription**, for any reason (API-9, NTF-4). A drop that falls due meanwhile happens when the last of them ends.
- **STA-2** **Retention.** A PR that becomes `merged`, `closed` or `gone` stays for **`keep_done`, 24 hours**, counted from the send time of the refresh that first showed it ended, by the wall clock, and is then dropped. (proposal: the value, and one value for all three)
  - **The clock is cleared when `state` becomes `open`, and starts again whenever it changes to another of `merged`, `closed` or `gone`**: a reopening and a gone PR's return (MOD-11) follow from it. (proposal; reopening isn't measured, docs/02)
  - The refresh that ends a PR is applied and its waiters evaluated before any drop, also with `keep_done` 0 (STA-1).
  - **`keep_done` and STA-1's 60 minutes go by the wall clock** (POL-26). A PR that becomes `asked` without a request (POL-5) starts its 60 minutes then.
  - Dropping a PR makes an `untracked` event (MOD-15). PRs dropped at one tick take revisions in the order of `pr`.
- **STA-3** **Limits.** At most 200 tracked PRs, of every source and state. At the limit **any request that would read through an untracked PR answers `limit` at once** (API-3), and nothing is sent: a `show`, a `track` or a `wait`. A `track` of a tracked PR takes no new place and is never refused. **A place is taken when a PR is found, loaded or read through**, and given back if its first refresh fails, so the 200 include PRs whose first refresh is in flight. A found PR is sent for its first refresh only while a place is free, newest `updated_at` first; the rest are left out with a warning (POL-7), and aren't `pending` (API-18). Finding stops paging once every place is taken. A tracked PR is never pushed out to make room. Near the limit a poll costs 5 to 6 points, so the active tier runs `slowed` or `capped` (POL-23). (proposal)
- **STA-4** **Applying a poll.** Each refresh is **merged into the stored record**: GitHub's values replace the stored ones, except where MOD-2 (a newer read wins), MOD-6, MOD-8, MOD-9, POL-12 and POL-21 keep an earlier value. **A refresh is applied only to a PR that holds a place (STA-3) when its answer arrives.** One dropped while it was in flight (`untrack`, `keep_done`, an `asked` PR's hour) is discarded, and makes no event. The daemon's own fields (`source`, the seen times and IDs, the settle and `keep_done` clocks) carry over. The revision counter moves only if the change is visible (MOD-14), and then one event is recorded (MOD-15). Waiters and subscriptions on that PR are evaluated after every applied refresh, changed or not (API-9, NTF-4).
- **STA-5** **First sight is a baseline.** When a PR is first tracked, its last comments and reviews are fetched once, with their times (POL-4), and remembered as seen, without events. Only what arrives afterwards is new (MOD-12).
  - **The daemon remembers, per PR and kind, the newest `at` it has seen and the IDs at that time**, in memory as in the file (STA-6). An item is new if its `at` is later than that time, or the same and its ID isn't remembered. **With no item of a kind seen, the time is `null`, and every item of that kind is new.**
  - **A PR loaded from the state file isn't seen for the first time.** Its baseline is compared with the saved times and IDs by the rule above. Its first event carries the new items as `activity`; `more` as POL-12 says.

## The state file

- **STA-6** **One file, `state.json`**, in the state directory (OPS-3), holds only what GitHub can't give back:
  - every tracked PR but the `asked` ones: name, node ID and `source`
  - for a `merged`, `closed` or `gone` one still inside `keep_done`, also which of the three, and when its `keep_done` clock started (STA-2). A loaded gone PR stays gone until GitHub returns it (MOD-11).
  - per PR and kind (comment, review), the time of the newest item it has seen, and the IDs seen at that time
  - per PR, the login it was last read with, for MOD-11's `gone` rule
  - a `version` number, 1. It rises only for a change an older binary would misread.

  **Its layout**, version 1:

  ```json
  {"version": 1, "prs": [{
    "pr": "o/r#12", "id": "PR_kwDOAbc123", "source": "manual", "login": "octocat",
    "ended": {"state": "merged", "seen_at": "2026-11-24T22:09:23Z"},
    "seen": {"comment": {"at": "2026-11-24T21:00:00Z", "ids": [918273]},
             "review": {"at": null, "ids": []}}}]}
  ```

  `ended` is `null` for an open PR.

  **No title, no token, no check or comment data and no subscription** (NTF-9) is written. Everything else is fetched again.
- **STA-7** **Writing.** The file is written when its content changes, at most once in 5 s: **a change within 5 s of the last write is written 5 s after that write**. It's also written at shutdown. **A `track` or an `untrack` that changes the file is written before it's answered**, whatever the 5 s spacing. If that write fails, it's answered anyway, and retried as below. It's written to **`state.json.tmp`** in the same directory, replacing any left by a crash, synced, and renamed over `state.json`; then the directory is synced. Its mode is 0600. A failed write raises `state_file_unwritable` (API-5) and is retried at the next change or 60 s later, whichever comes first; one that succeeds clears it. The daemon keeps running.
- **STA-8** **Loading.** At start the file is read and its PRs are tracked and refreshed by ID in the first poll. Until its first refresh is applied, a loaded PR has no record: `list` leaves it out (it waits for the first poll, API-18), a `wait` naming it is held as a waiter until that refresh is applied, and any other request naming it reads it through (POL-16), keeping its saved `source`. So a PR that merged or closed while no daemon ran is seen ending at the first poll.
  - **A loaded PR keeps its saved entry until its first refresh is applied.** Every write carries that entry unchanged. Its `keep_done` clock runs from its saved time, and its `NOT_FOUND`s count towards `gone` (POL-21) as for its saved state, under its saved login. **That first refresh is the same whether a poll or a read-through sends it:** it keeps the saved `source`, compares items with the saved ones (STA-5), and an ended state it confirms keeps its saved time.
  - **A binary reads every earlier `version`**, and ignores keys it doesn't know.
  - **A file that isn't JSON, or has no integer `version`,** is renamed to `state.json.bad`, replacing any earlier one (the warning and the log give the time), and the daemon starts empty with a warning. **An entry that doesn't decode** (a key missing, an unknown `source`, a node ID given twice) **is skipped and logged**, and the rest load.
  - **A file with a newer `version` is left alone:** the daemon starts empty and doesn't write it, with a warning (API-5).
  - A missing file is normal.
  - Entries past their `keep_done` are dropped on loading, without an event: the new epoch resets every cursor (STA-9).

## Cursors

- **STA-9** **A cursor is `<epoch>-<rev>`.** The epoch is 8 random lower-case letters and digits, made at each daemon start; `rev` is the revision counter (MOD-13): each start's first revision is 1, and a cursor taken before any is `<epoch>-0`. A cursor that isn't `<epoch>-<rev>` is `bad_request` (API-1).
  - Clients treat a cursor as opaque.
  - **A cursor is "too old" when it's from another epoch, its `rev` is above the counter, or an event after it is no longer held (STA-10).** **It's judged when the answer is built**, not only on arrival: a wait whose `since` or arrival cursor fell out of the history while it was held answers with `reset: true`. Revisions are compared only within one epoch. Every route that takes a cursor (API-6, API-8, API-9) then answers with the full current state and `reset: true`. Nothing is lost silently across a daemon restart, a wait without `since` included (CLI-10).
- **STA-10** **Event history.** The daemon keeps the last **500 events** in memory, across all PRs. It isn't written to disk. (proposal)

## One daemon

- **STA-11** **One daemon per user.** At start the daemon takes an exclusive lock on `circitor.lock` in the state directory and holds it for life. **The lock is `flock` with `LOCK_EX`**, on a descriptor kept open for life, never a POSIX record lock (`fcntl`). **To test it** (CLI-8, CLI-9), a process takes `LOCK_EX | LOCK_NB` on its own descriptor and lets go at once: taken means released. A daemon that can't take it tries again every 100 ms for up to 6 s: OPS-7's stop time and a margin (15 s for a foreground daemon taking over, OPS-6). **A lock file that can't be opened or created** (a denied permission, a read-only disk) stops the daemon at once, naming the path and the system's error; it isn't retried. If it gets it, it starts. Otherwise **a spawned daemon** exits with status 0 if the holder answers on its socket (OPS-4), and with an error if it doesn't. **A foreground daemon follows OPS-6:** it takes over from a spawned holder, and never exits 0 for want of the lock. The lock file holds the holder's socket path, so the check doesn't depend on the second daemon's environment.
  - **The order at start:** the lock file is opened without truncating it. Only once the lock is held does the daemon truncate it and write one line: its process ID, one space, and its socket path, **which is the rest of the line and may hold spaces** (macOS's `Application Support`, OPS-3). A reader takes only a complete line; anything else counts as no path. Then it removes a stale socket (OPS-4) and reads `state.json` (STA-8). Then it binds the socket, and only then reads the token (OPS-5).
  - **The daemon checks every minute that its socket and its lock file are the ones it made** (the same device and inode: the path's `stat` against the open descriptor's `fstat`). If either was removed or replaced, it stops (OPS-7). At exit it removes the socket only if it's still its own.
- **STA-12** **Nothing else is written** at run time but `state.json` and `state.json.tmp` (STA-7), a set-aside `state.json.bad` (STA-8), the lock file, the socket, and the log with its one old copy (OPS-9). All are in circitor's own directories, except a socket placed by `CIRCITOR_SOCKET` (OPS-4).
