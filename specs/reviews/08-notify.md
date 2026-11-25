# Hardening log: 08-notify.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 2/9/1 | 12 | 0 | 0 | 0 | 0 |
| 2 | 2026-11-25 | failure modes | 2/5/5 | 12 | 0 | 0 | 0 | 0 |
| 3 | 2026-11-25 | implementability | 0/10/8 | 12 | 0 | 0 | 0 | 6 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3, with 30 findings between them, merged by root cause into 13. Dropped in verification: 1, settled by a measurement ("closing straight after the write was never measured": a line posted to a session and closed at once was delivered, 2026-11-25). A second measurement backs H1-1: the socket files of six test sessions that had ended were gone. Both were added to docs/07 afterwards, on the owner's say.

### H1-1 · high · A subscription whose session has ended keeps the daemon in the active tier for up to 24 hours
- Location: specs/08-notify.md NTF-4, NTF-5, NTF-6; specs/02-polling.md POL-26
- Status: applied: each subscription's socket is checked at every tick, by path, device and inode (NTF-4, a proposal for a killed session); NTF-5 names the active tier's cost; POL-26's lists take the socket check and `for`

### H1-2 · high · A long-lived subscription outlives the event history and can post a false notice
- Location: specs/08-notify.md NTF-4, NTF-8; specs/03-state.md STA-9, STA-10; specs/04-api.md API-10
- Status: applied: a subscription remembers matching events from registration and doesn't search the history; a cursor that goes too old while held neither satisfies it nor drops a part; a notice whose cursor is too old names `show`

### H1-3 · medium · The refresh rate of a subscribed PR contradicted POL-14 and POL-10
- Location: specs/08-notify.md NTF-2, NTF-5; specs/02-polling.md POL-10, POL-14; specs/03-state.md STA-1
- Status: applied: the notify routes name their PR for STA-1's 60 minutes only; POL-14 excepts them; POL-10 gives an open PR with a subscription its tier's rate whatever its source

### H1-4 · medium · `Ask again` lost the cursor
- Location: specs/08-notify.md NTF-8
- Status: applied: `Ask again` carries `--since` with the subscription's cursor, `--for` included; both examples changed

### H1-5 · medium · The rules for a notice that fails didn't fit each other, and the tested failure couldn't happen
- Location: specs/08-notify.md NTF-2, NTF-6, NTF-10; specs/07-testing.md TST-8, TST-16
- Status: applied: once its line is written a notice is never sent again; a peer that doesn't read counts as posted; a second try only before the line is written in full; a failed confirmation isn't retried; `stopped` notices go together with 1 s for all; the test double gains a connection that never completes

### H1-6 · medium · The `stopped` notice made sessions restart the daemon the user had stopped
- Location: specs/08-notify.md NTF-8, NTF-10; specs/05-cli.md CLI-11
- Status: applied: the `stopped` notice's command has `--no-spawn`; the guide says exit code 6 then means a stop on purpose

### H1-7 · medium · A session socket the daemon can't reach was answered as wrong usage
- Location: specs/08-notify.md NTF-2, NTF-11; specs/04-api.md API-3; specs/05-cli.md CLI-5
- Status: applied: a new error `no_session` (409), mapped to exit code 11 with the `wait` named; `bad_request` stays for a path that isn't absolute

### H1-8 · medium · The registration and cancel routes left their contract open
- Location: specs/08-notify.md NTF-2, NTF-3, NTF-4, NTF-11; specs/04-api.md API-15, API-18; specs/05-cli.md CLI-8
- Status: applied: required and typed body keys, `limit`; the answer kept or not by API-11's outcome; `outcome` and `reason` `null` when registered; a loaded PR is read through; cancel never reads through or starts a daemon; NTF-4's `ended` row cites API-11; API-15 and API-18 name subscriptions and the cancel route

### H1-9 · medium · Asking again with other parameters silently replaced the earlier subscription
- Location: specs/08-notify.md NTF-2, NTF-11; specs/04-api.md API-19
- Status: applied: only a request with the same `socket`, PR, `until`, `sha`, `fail_fast` and `kind` replaces; the answer has `replaced: true` and the CLI says so; a replacement takes no new place; the limit is checked after the evaluation

### H1-10 · medium · The agent wasn't told when to conclude that no confirmation arrived, or that unattended and resumed sessions lose notices
- Location: specs/08-notify.md NTF-7, NTF-12; specs/05-cli.md CLI-11
- Status: applied: the confirmation arrives with the command's result; the README and the guide cover unattended runs, a resumed session and a daemon crash

### H1-11 · medium · NTF-12 said more than was measured, and left out `hold`
- Location: specs/08-notify.md NTF-12; specs/07-testing.md TST-13
- Status: applied: each case is listed with what is verified and what is only documented; TST-13's step 7 records what a session that bypasses prompts does instead of expecting an outcome

### H1-12 · low · Three slips: the path limit, `24h`, a citation
- Location: specs/08-notify.md NTF-1, NTF-2, NTF-7, NTF-8, NTF-11
- Status: applied: no length rule on the session's socket path; the examples print `1d`, as CLI-3's durations do; NTF-1 cites docs/07 and says the other clients weren't checked

### H1-13 · low · The `expired` notice has no stale mark and no form for a PR without a record
- Location: specs/08-notify.md NTF-8
- Status: applied with change: taken up by H2-1 in pass 2, which gives the notice CLI-3's stale mark; a subscription always has a record, since registering reads through (NTF-2)

## Pass 2: failure modes (2026-11-25)

Reviewers: 3, with 22 findings between them, merged by root cause into 12. Dropped in verification: 0. H1-13, parked in pass 1, went into H2-1.

### H2-1 · high · After a sleep or a clock step, a subscription expired on a stale record, one poll before the truth
- Location: specs/08-notify.md NTF-4, NTF-8; specs/02-polling.md POL-22, POL-26; specs/07-testing.md TST-8
- Status: applied: `for` runs as a waiter's timeout does, and a clock step doesn't move the expiry; an expiry due at a wake or on a stale record waits for the next applied refresh, 60 s at most; the `expired` notice takes the stale mark; POL-22's wake poll counts subscriptions; `sleep` and `clock-back` gain a subscription

### H2-2 · high · With no confirmation the agent was told to use `wait`, never to cancel
- Location: specs/08-notify.md NTF-7, NTF-11; specs/05-cli.md CLI-11
- Status: applied: the output line and the guide say to cancel the notify first, then use `wait`

### H2-3 · medium · The `stopped` notice's `--no-spawn` command could lose the race with a replacement, and misread a stop nobody asked for
- Location: specs/08-notify.md NTF-10; specs/05-cli.md CLI-7; specs/07-testing.md TST-8
- Status: applied: `notify --no-spawn` waits up to 8 s for a daemon before exiting 6; a stop the daemon chose itself posts the command without `--no-spawn`

### H2-4 · medium · A problem only the user can fix left an idle session waiting in silence
- Location: specs/08-notify.md NTF-2, NTF-4, NTF-5, NTF-8, NTF-11
- Status: applied: a new end `problem` with its notice, after 10 minutes in `auth` or on `pr_failing` (proposal: the 10 minutes); registering in `auth` answers `auth` unless the answer is already known; the registered output carries the `problem:` line

### H2-5 · medium · The order of evaluating, keeping and confirming a registration wasn't fixed
- Location: specs/08-notify.md NTF-2, NTF-4
- Status: applied: the order is stated, with the evaluation, the limit and the entry in one step; the history is searched once, when registering; an outcome that lands while the confirmation is posted follows it; a failed confirmation takes out the subscription and the one it replaced

### H2-6 · medium · A cursor from before a restart stayed the subscription's cursor
- Location: specs/08-notify.md NTF-2, NTF-8, NTF-11
- Status: applied: a `since` too old when registering is answered with the reset and let go; the registered output prints a reset and events first; `already:` with a reset ends with the command to register afresh

### H2-7 · medium · A plain `changed` on a PR that had ended held the active tier for a day, unseen
- Location: specs/08-notify.md NTF-5, NTF-11; specs/02-polling.md POL-13
- Status: applied: the registered output shows the PR's one line; only a subscription on an open PR holds the active tier

### H2-8 · low · A PR kept only by its subscription was dropped just before the notice's command arrived
- Location: specs/08-notify.md NTF-4; specs/03-state.md STA-1
- Status: applied: a subscription's end counts as the end of a request that named its PR, for 60 minutes

### H2-9 · low · A spawned daemon inherited the session's socket path and token
- Location: specs/05-cli.md CLI-7; specs/08-notify.md NTF-9; specs/07-testing.md TST-9
- Status: applied: the daemon is started without both variables, and TST-9 checks it

### H2-10 · low · Three edges around posting were undefined
- Location: specs/08-notify.md NTF-4, NTF-6
- Status: applied: a subscription is open until its second try, which a cancel calls off and a stop makes at once; a `stat` that fails otherwise changes nothing; the path is checked again before connecting

### H2-11 · low · One session could be sent more notices at once than Claude Code queues
- Location: specs/08-notify.md NTF-2; specs/07-testing.md TST-8
- Status: applied: at most 32 subscriptions for one socket (proposal)

### H2-12 · low · A repository renamed while a subscription is held
- Location: specs/08-notify.md NTF-1, NTF-3
- Status: applied: a subscription holds its PR by node ID and remembers the name it was registered under; cancel matches that name too (proposal: a rename isn't measured)

## Pass 3: implementability (2026-11-25)

Reviewers: 3, with 23 findings between them, merged by root cause into 18. Dropped in verification: 0. One measurement, on Linux, in the scratchpad: a Unix socket whose listen queue is full fails a connect at once with `EAGAIN`, also with a 1 s timeout; it never hangs. That macOS answers `ECONNREFUSED` is from source, not measured. Neither is in docs/05 yet.

### H3-1 · medium · The `problem` end had no point where it was checked, so it never fired
- Location: specs/08-notify.md NTF-4; specs/02-polling.md POL-26; specs/07-testing.md TST-8
- Status: applied: checked at each tick; the 10 minutes run on the monotonic clock from when `auth` began, and a failed try doesn't restart them; a subscription registered while `pr_failing` is raised posts `problem` at the first tick; POL-26's tick lists it; new row `notify-problem`

### H3-2 · medium · "Notify doesn't make its PR hot" was written in four places, and the copies disagreed
- Location: specs/08-notify.md NTF-2, NTF-5; specs/02-polling.md POL-11, POL-13, POL-14; specs/03-state.md STA-1
- Status: applied: the rule is stated once, in STA-1, and POL-11, POL-14 and NTF-2 cite it; the tier is active while a subscription on an open PR is held, and then follows the last client request

### H3-3 · medium · `notify-keeps-active` asked one subscription to show what needs two
- Location: specs/07-testing.md TST-8
- Status: applied: split into `notify-keeps-active` (open PR: the tier and the idle exit) and `notify-keeps-ended` (an ended PR kept past `keep_done`, dropped 60 minutes after the end)

### H3-4 · medium · A replacement whose answer was already known was silent, and `notify`'s line order was only partly fixed
- Location: specs/08-notify.md NTF-2, NTF-11
- Status: applied: that answer has `registered: false` and `replaced: true`, and nothing is posted; `replaced an earlier notify` is printed in both cases; one order for the lines

### H3-5 · medium · The test clock's settle didn't wait for notices
- Location: specs/07-testing.md TST-4
- Status: applied: notices being posted are work in flight, and the fake session must have read every connection to its end

### H3-6 · medium · The socket check had no place in the registration order
- Location: specs/08-notify.md NTF-2
- Status: applied: the `stat` comes straight after the parameters, so a bad socket never reaches GitHub and gets `no_session` even with a known answer; `auth` before the limit

### H3-7 · medium · The confirmation and the `registered:` line had a format for one condition only
- Location: specs/08-notify.md NTF-7, NTF-11
- Status: applied: one rule for the condition's words, with an example for `changed` with `kind`

### H3-8 · medium · The second try covered a failure the platforms don't produce
- Location: specs/08-notify.md NTF-6, NTF-10; specs/07-testing.md TST-16, TST-8
- Status: applied with change: the second try is dropped, and a failed or unfinished post ends the subscription, logged; TST-16 keeps its full-queue mode, now "so a connect fails at once", and `notify-session-gone` expects `EAGAIN` to drop it. The `EAGAIN` measurement is cited as not yet in docs/05.

### H3-9 · medium · Which view the registered answer was built from wasn't said
- Location: specs/08-notify.md NTF-2
- Status: applied: the registration's view (MOD-13); its cursor is the subscription's, and a refresh during the confirmation goes into the notice

### H3-10 · medium · The registration rules had no scenarios
- Location: specs/07-testing.md TST-8
- Status: applied: `notify-no-session` covers a refused confirmation; new row `notify-register`

### H3-11 · low · The `wait` named on exit 11 wasn't defined
- Location: specs/08-notify.md NTF-11
- Status: applied: the condition's parameters as given, never `--timeout`

### H3-12 · low · "The same" request for a replacement wasn't defined
- Location: specs/08-notify.md NTF-2
- Status: applied: the PR by node ID, `sha` lower-cased, `kind` as a set, a missing `fail_fast` as `false`

### H3-13 · low · `notify --cancel` without a PR can force a poll
- Location: specs/08-notify.md NTF-11; specs/05-cli.md CLI-2
- Status: parked

### H3-14 · low · The `sleep` row's failing wake poll doesn't keep the catch-up from happening, and doesn't name the condition
- Location: specs/07-testing.md TST-8 `sleep`
- Status: parked

### H3-15 · low · The fake session records lines, not connections, and can't be replaced at the same path
- Location: specs/07-testing.md TST-16
- Status: parked

### H3-16 · low · The sleep branch of the expiry catch-up is covered by the stale branch (MOD-16)
- Location: specs/08-notify.md NTF-4
- Status: parked

### H3-17 · low · The "History too old; show" form of a notice repeats what `changes` does with a reset
- Location: specs/08-notify.md NTF-8
- Status: parked

### H3-18 · low · Two subscription limits (64, and 32 per socket) where one would do
- Location: specs/08-notify.md NTF-2; specs/07-testing.md TST-8
- Status: parked
