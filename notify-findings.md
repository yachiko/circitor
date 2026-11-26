# circitor notify: two issues from a field test

Field test of `circitor notify` on 2026-11-26, run from a Claude Code session in another workspace. Please investigate both issues, reproduce them, and propose fixes to the spec, the guide and the code. Don't change behaviour that the spec already settles without saying which rule you're changing.

Environment: circitor 0.0.1+29aac3d (Homebrew, `/opt/homebrew/bin/circitor`), macOS 27.2.1, Claude Code 2.1.331, interactive session in auto mode, `crossSessionInbound` unset, daemon `mode: spawned`. Test PR: `mTABNPD/workflows-sandbox#40`, a private repo, draft, now closed.

## What worked (data for the README, which says only Linux was measured)

- Confirmations arrived each time, as a message from another session, before the tool call returned.
- `--until changed --kind activity`: a comment was posted at 10:58:28Z. The notice woke the **idle** session without user input and was read by 10:59:26Z.
- `--until checks-done --sha 2e2c9bd` was registered while the record still showed the old head 5988da7, whose checks had passed. It did not fire on the old head. The last check on 2e2c9bd completed at 11:00:33Z and the notice had been read by 11:00:56Z.
- `notify` on a condition that already held printed `already:`, exited 0 and posted nothing.
- `circitor stop` posted the `stopped` notice with a `--no-spawn` re-ask command.

## Issue 1: a subagent's notify reports to the parent, and the guide tells the subagent to cancel it

**Observed.** A Claude Code subagent (Agent tool) ran:

```
circitor notify mTABNPD/workflows-sandbox#40 --until changed --kind activity --since x0xic2mb-72
```

- The subagent's shell had the parent's socket: `CLAUDE_CODE_MESSAGING_SOCKET=/tmp/cc-socks/66665.sock`, the same path as the main session.
- `notify` printed `registered: ...` and `a confirmation was posted to this session; if none arrives, notices are held or refused here: run circitor notify ... --cancel, then use circitor wait`.
- The subagent reported that it got no `circitor: watching` message. The confirmation arrived in the **main** conversation instead.
- A comment was posted at 11:01:29Z. The notice went to the main conversation too.
- The subagent was told not to cancel. A subagent that follows the CLI's own line, or the guide's "If none arrives, run `circitor notify --cancel` and use `wait`", would cancel a subscription that works. The main session would then get a confirmation for something it never asked for.

NTF-11 already says "A subagent's shell has its parent's socket (docs/07): its notice goes to the main conversation". Nothing the agent sees says it: not the CLI output, not the guide.

**To investigate.**
1. Can circitor tell that it runs inside a subagent? Check what Claude Code puts in a subagent's environment that differs from the main session's (env vars, process tree). If there is a reliable signal, should `notify` refuse with exit 11 and name the `wait`, or register and say where the notices will go?
2. If it can't tell, change the CLI line and the guide so that an agent that sees no confirmation doesn't cancel blindly. At minimum, the guide should say "subagents: use `wait`, not `notify`".
3. Is a confirmation for an unknown subscription in the main session a risk under NTF-8's rules? It carries only circitor's words, but the main agent may act on it.

## Issue 2: `circitor stop` doesn't stick when another client polls

**Observed timeline (UTC, from `circitor.log` and `ps`):**

```
11:02:3xZ  notify ... --until merged                       registered, confirmation arrived
11:02:34Z  circitor stop                                    log: stop cause=request
11:02:34Z                                                   log: subscription ended pr=... end=stopped
           "stopped" notice arrived: Ask again: circitor notify mTABNPD/workflows-sandbox#40 --until merged --since x0xic2mb-78 --no-spawn
11:02:38Z                                                   log: start mode=spawned  (pid 8127, ppid 1)
~11:02:4x  the re-ask command, run verbatim                 took 8.9 s, printed "reset: cursor too old, current state follows", then registered (exit 0)
```

Per NTF-10, the re-ask should exit 6 after 8 s when nobody restarts the daemon. It registered because another client had already started one 4 s after the stop.

**Probable spawner, not proven.** A Claude Code mod in other sessions polled `circitor show <pr> --json` every 20 s without `--no-spawn`. I've since added `--no-spawn` to it. Several sessions were running, and one started about 10 minutes before the stop. The daemon is detached (ppid 1), and its `start` log line doesn't say who spawned it. That's why I couldn't prove the cause.

**To investigate.**
1. **Log the spawner.** The spawning CLI could pass its parent pid, its argv[0..2] (`show`, `wait` ...), and `CLAUDE_CODE_SESSION_ID` or the cwd. The `start` line would then answer "who restarted it".
2. **Make a deliberate stop stick.** Any command without `--no-spawn` undoes `circitor stop` within one poll period. Options: a "stopped by request" marker in the state dir that spawning commands honour for a while, or until `circitor start` or an interactive command; a `stop --hold` flag; or the guide telling pollers and mods to always pass `--no-spawn`. Pick one and update CLI-7, CLI-8 and the README.
3. **The 8.9 s re-ask.** It took 8.9 s even though a daemon had been running since 11:02:38Z. Was that NTF-10's 8 s wait for a daemon, or the new daemon's cold read-through? Check whether `--no-spawn` waits the full 8 s when a daemon is already up but still starting.
4. **The re-ask carries a dead cursor.** The `stopped` notice's command includes `--since x0xic2mb-78`. Any new daemon has a new cursor epoch (`yom6292t-...`), so the re-ask always prints `reset: cursor too old`. That's harmless for `until merged`, but noisy. Should the notice leave `--since` out, or should a new daemon accept a cursor from a previous daemon's epoch when `state.json` carried the history?

## Repro sketch

- Issue 1: in Claude Code, have a subagent run `circitor notify <pr> --until changed --kind activity`, then post a comment on the PR. Watch which conversation gets the confirmation and the notice.
- Issue 2: run `while :; do circitor show <pr> --json >/dev/null; sleep 20; done` in one shell. In another, `circitor notify <pr> --until merged`, then `circitor stop`, then run the `stopped` notice's command and time it.
