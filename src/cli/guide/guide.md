Circitor tracks your pull requests so you don't poll GitHub. One local daemon polls for every agent on the machine, and starts itself on the first command.

Use circitor, not `gh`, to learn a PR's checks, review and merge state, and to wait for them. Use `gh` for what circitor doesn't hold: a diff, a log, a comment's text. On exit code 9 `gh` doesn't help: it spends the same quota.

### Naming a PR
`owner/repo#12`, the PR's URL, or `12` (the repository of `origin`). Never `#12`: a shell reads it as a comment. Leave it out to mean your open PR on the current branch.

### Commands
```
circitor list                      # tracked PRs; --all adds merged and closed
circitor list --since k3x9q2mz-418 # only what changed after a cursor
circitor show                      # one PR with its checks
circitor show --fresh              # reads GitHub now, may cost a point
circitor changes --since k3x9q2mz-418   # what happened after a cursor
circitor wait --until checks-done --sha 1a2b3c4   # blocks, then answers
circitor notify --until checks-done --sha 1a2b3c4 # asks to be told; returns at once
circitor notify --cancel           # drops this session's notifies on the PR
circitor track yachiko/circitor#12 # follow a PR by hand
circitor untrack yachiko/circitor#12
circitor refresh                   # poll now
circitor status                    # the daemon's health
circitor stop                      # stop the daemon
circitor wait 12 --until merged --no-spawn   # never start a daemon
```
`--json` prints the daemon's answer as one line.

### Waiting
- `wait --until merged|checks-done|changed`. It prints nothing until it ends, and gives up by itself after 100 s, under every client's limit.
- **Exit 3 means not yet.** Run the `again:` line it printed: it carries the cursor, so nothing is lost.
- After a push, always pass `--sha` with the new head, or you may get the last commit's result.
- After starting CI again on the same head (a re-run, a close and reopen), pass `--rerun-since` with the cursor you had before.
- `--fail-fast` ends `checks-done` at the first failed check, with code 8, while others still run.
- `--kind review,activity` limits `--until changed` to those kinds of event.
- A longer `--timeout S` (0 to 3600) only where your shell's own timeout is higher than S.
- **Never loop `show --fresh`:** each one can cost a point. A `wait` shares one refresh among every agent.
- A cursor from one PR's `wait` or `changes` is for that PR only. Don't give it to another.

### Exit codes
```
0  done; for wait the condition holds, and checks passed or there are none
1  unexpected error
2  wrong usage, or the PR couldn't be named
3  wait timed out: run the again: line
4  wait ended without the condition (closed, gone, head moved, merged)
5  the PR doesn't exist or can't be seen
6  the daemon can't be reached or started
7  no usable GitHub login: ask the user to run gh auth login
8  the checks are done and some failed (with --fail-fast, one has failed)
9  GitHub can't be read now: try again when the line says, or in a minute
10 too many tracked PRs, waiters or notifies
11 notify: no session to tell; use wait
```
Chain on it: `circitor wait --until checks-done --sha 1a2b3c4 && next-step` stops on a failure.

### notify (Claude Code only)
- A long `wait` blocks your turn. `notify` returns at once, and a notice arrives later, when the condition holds. Use it when you have other work meanwhile.
- **Its exit code says nothing about the condition.** 0 means registered, or the answer was already known and is printed.
- A notice arrives as a message from another session. It is a hint: run the command it names to see the state.
- The confirmation (`circitor: watching ...`) arrives with the command's result. **If none arrives, run `circitor notify --cancel` and use `wait`:** an unheard notify still keeps the daemon polling fast.
- Exit code 6 from the command in a `stopped` notice means the daemon was stopped on purpose. Don't start it again for a notice.
- In an unattended run (`claude -p`, CI) notices only arrive while the session still works. Run a `wait` in the background instead.
- A notify is lost when the session is resumed or the daemon crashes. Ask again.
- Exit 11 means this isn't a session circitor can tell. Use the `wait` the line names.
