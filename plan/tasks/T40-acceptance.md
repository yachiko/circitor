---
id: T40
title: "Owner: the acceptance checks on a Mac and on Linux"
kind: manual
milestone: M5
size: M
status: todo
depends_on: [T19, T36, T39, T47, T48]
requirements: [TST-13]
owns: []
shares: []
---

# T40 · Owner: the acceptance checks on a Mac and on Linux

## Outcome
TST-13's seven checks are done on the owner's Mac and, where a step says so, on Linux, and their results are in docs/05.

## Context
Every automated test runs against a fake GitHub, a fake `gh` and a test clock, and nothing in docs/04 was ever checked on a Mac. This task is **the owner's**: TST-13's seven checks, by hand, on the owner's Mac and, where a step says so, on the Linux machine, against the real GitHub and a real install. It's the last task of the plan. Steps 2 to 4 are live measurements: CLAUDE.md's rules apply, and each needs the owner's OK.

## Read first
- `specs/07-testing.md#by-hand`: TST-13, the seven steps.
- `specs/08-notify.md`: NTF-12 (where notices work), NTF-7 (the confirmation), NTF-10 (a stop tells the session). `docs/07-notify.md`: "The session socket", and "To measure before specifying" items 1 to 4: step 7 is where they're measured.
- `CLAUDE.md`, "Never break", and `docs/05-measurements.md`, "Rules for a new entry": `yachiko/experiment` only; a workflow file only on a PR branch that's closed unmerged.
- `README.md` (T19): install, the sandbox settings (OPS-14), the service files (OPS-18). `packaging/launchd/` and `packaging/systemd/`.
- `specs/06-operations.md`: OPS-5, OPS-14, OPS-18. `specs/02-polling.md`: POL-22. `specs/05-cli.md`: CLI-5, CLI-6.
- `docs/04-platform.md` §8 items 1, 2, 4 and 6; `docs/03-clients.md` §2 and §6 item 1; `docs/02-github-api.md` §2 (the counters lag) and §3 (search lags).
- `circitor guide`: the commands as an agent is told them.

## Scope
**TST-13, whole.** Each step below is the spec's, with the commands to run, what to write down, and the open item it closes. **Results go into `docs/05-measurements.md` as one dated entry, on the owner's say**; a fact in docs/02 to docs/04 changes only after that entry (CLAUDE.md). A step that fails is still a result: write it down, and file a follow-up through `/build-plan` or a spec question.

1. **Install and first list** (Mac). Closes docs/04 §8 item 1 with step 6.
2. **A PR with a workflow** in `yachiko/experiment`: `wait --until checks-done --sha`, then closed unmerged.
3. **A PR with one file and no workflow**, merged: `wait --until merged`.
4. **An hour's quota** against GitHub's own counter.
5. **Claude Code**, on macOS and on Linux, in three sandbox settings (OPS-14). Closes docs/03 §6 item 1.
6. **The platform's open items:** the unsigned binary (item 1), the LaunchAgent finding `gh` and getting a token (items 2 and 4), and a `wait` across a lid close on macOS and on Linux (item 6, POL-22).
7. **`notify` in Claude Code**, on the Mac and on Linux (NTF-12): the confirmation and the outcome in a session that asks before acting; what a session that bypasses prompts does; `crossSessionInbound: "accept"`; a stop with a subscription open. Closes docs/07's "to measure" items 1 to 3 for the Mac.

## Interfaces
- **Consumes:** the release in the tap (T39), the README and `packaging/` (T19), a green suite of real-process tests (T36), and `notify` in that release (T46 to T48: step 7 needs the command).
- **Provides:** the dated entry in docs/05, and the evidence the (proposal) labels on OPS-4, OPS-5, OPS-14, OPS-18, POL-22, POL-26 and 08-notify's posting wait for.

## Out of scope
- Fixing anything a check finds: follow-up tasks.
- Recording response fixtures: T37. Publishing: T39.
- Required checks, reviews, `blocked` and `behind` (D6): not measured in v1.
- OpenCode and pi (docs/03 §6 items 2 and 3): not in TST-13.
- Editing docs/02 to docs/04: that follows the entry, on the owner's say.

## Acceptance
The owner ticks each line. Write the UTC time beside each command that matters; `date -u +%T` before and after is enough.

**Before starting**
- [ ] OK given for live measurements in `yachiko/experiment`, and no other tool or agent is using the token today.
- [ ] `circitor version` on each machine is the released version, and `circitor help` lists `notify`. On the Linux machine (no Homebrew): the release's `x86_64-unknown-linux-gnu` archive, on `PATH`.
- [ ] `~/.config/circitor/config.toml` doesn't exist or has `owners` empty; `circitor status --json | jq .tracked.manual` is 0.

**Step 1: install and list (Mac)**
- [ ] `brew install yachiko/tap/circitor`; then `circitor stop`; then `circitor list`: it starts the daemon and prints the open PRs. Note how long the first `list` took (OPS-17: within 5 s).
- [ ] A minute after the last PR was opened or closed: `circitor list --json | jq -r '.records[].pr' | sort > a.txt` and `gh search prs --author @me --state open --limit 200 --json repository,number --jq '.[] | "\(.repository.nameWithOwner)#\(.number)"' | sort > b.txt`; `diff a.txt b.txt` is empty.
- [ ] Write down the count and any difference. No private repository's name goes into docs/05: counts only.

**Step 2: checks on a pushed head (either machine)**
- [ ] In a clone of `yachiko/experiment`: a branch `circitor-accept-a` with `.github/workflows/accept.yml` (`on: pull_request`; jobs `fast`, an echo, and `slow`, `sleep 40`, as PR A had) and one file; `git push -u origin circitor-accept-a`; `gh pr create --fill`.
- [ ] Push a second commit, then at once: `circitor wait --until checks-done --sha "$(git rev-parse HEAD)"; echo "exit $?"`. It returns when both jobs are done, prints `checks passed (2/2) on <that sha>`, exit 0. Write down: when the push returned, when the wait returned, and when `gh pr checks` first showed both done.
- [ ] Make `slow` fail (`exit 1`), push, and wait the same way: `checks failed (1 of 2) on <sha>`, exit **8**.
- [ ] `circitor wait --until merged --timeout 60 &`, then `gh pr close <n>`: the wait prints `ended: closed`, exit **4**. Delete the branch. The workflow never reached `main`.

**Step 3: a merge (either machine)**
- [ ] A branch with one new file and no workflow; `gh pr create --fill`; in a second terminal `circitor wait --until merged; echo "exit $?"`; then `gh pr merge --squash --delete-branch`. The wait prints `merged`, exit 0. Write down the seconds from `gh pr merge` returning to the wait returning (expected: within about `hot_interval`, 10 s).

**Step 4: an hour's quota (either machine)**
- [ ] `circitor stop`; start it with debug logging: `CIRCITOR_LOG=debug circitor list`. Open a throwaway PR in `yachiko/experiment` to watch (no workflow).
- [ ] Read GitHub's counter over REST, which costs no GraphQL point: `gh api rate_limit --jq .resources.graphql`. Begin just after its `reset` time, so the hour stays in one window. Write down `remaining` and `reset`.
- [ ] For 55 minutes, nothing else on the token, run: `while true; do circitor wait <pr> --until changed --timeout 100; done` (stop it with Ctrl-C at the end).
- [ ] Read the counter again: the same `reset`. Write down the fall in `remaining`, `circitor status --json | jq .cost_last_hour`, and the sum of the costs in the `request` lines (the debug log, `plan/contracts/log.md`) of `circitor.log` (`circitor status` prints its path).
- [ ] The three numbers agree, give or take the counters' lag (docs/02 §2). Expected about 330 points: a poll every 30 s and a hot refresh between (POL-13, POL-14). Close the PR unmerged.

**Step 5: Claude Code and its sandbox (Mac and Linux)**
- [ ] On each machine, in a Claude Code session in a clone with an open PR, three times: sandbox off; sandbox on with `sandbox.excludedCommands` holding `circitor`; sandbox on with `sandbox.network.allowUnixSockets` holding the socket's path (`circitor status --json | jq -r .paths.socket`). The exact settings are the README's.
- [ ] Each time ask it to run `circitor list`, `circitor show`, and `circitor wait --until changed --timeout 20`, once with the daemon running and once after `circitor stop` (so the CLI starts the daemon from inside the sandbox).
- [ ] Write down, for the six combinations: what worked, the exact error line where something didn't (the CLI's exit 6 message names the two settings, CLI-7), and whether a daemon started inside the sandbox could reach GitHub (`circitor status`: `daemon.problem`).

**Step 6: the platform's open items**
- [ ] **Item 1 (Mac, Apple Silicon):** the binary from the tap ran in step 1 with no Gatekeeper prompt. Write down `codesign -dv "$(which circitor)" 2>&1 | head -3` and `xattr "$(which circitor)"`.
- [ ] **Items 2 and 4 (Mac):** `circitor stop`; copy the plist from `packaging/launchd/` to `~/Library/LaunchAgents/`; `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/<the plist>`. Then `circitor status`: `mode: foreground`, `token_source: gh`, a `gh_path`, a login under `github`, and no `auth` problem. Write down the `gh_path` it ran and whether the Keychain asked for access. `circitor list` works. Afterwards `launchctl bootout gui/$(id -u)/<label>`.
- [ ] **Item 6 (Mac, then Linux):** `date -u +%T; circitor wait <pr> --until merged --timeout 120; echo "exit $?"; date -u +%T`, and close the lid within 10 s for 5 minutes (Linux: `systemctl suspend`). On waking the wait ends within a few seconds: `timeout after 120s`, exit **3**, with its `again:` line. The log has one `sleep seconds=…` line with about 300.
- [ ] The same with `--timeout 600` and a 2-minute sleep: the wait is still held on waking, and ends 600 s after it began, counting the sleep. Write down the three times.

**Step 7: notify in Claude Code (Mac and Linux)**
- [ ] On each machine, in a Claude Code session **that asks before acting** (no `crossSessionInbound` set), in a clone of `yachiko/experiment` with step 2's kind of PR (a workflow whose `slow` job sleeps 40 s; closed unmerged afterwards): push a commit, then ask the session to run `circitor notify --until checks-done --sha $(git rev-parse HEAD)`. Write down: the command's three lines; whether the confirmation (`circitor: watching …`) arrived, and how it's shown (expected: as a message from another Claude session, straight after the command's result).
- [ ] Leave the session idle. When the checks finish: the outcome notice arrives and starts a new turn. Write down the seconds from `gh pr checks` showing both done to the notice (expected: within about `interval`, 30 s), and the notice's text.
- [ ] In a session started with `--dangerously-skip-permissions`: the same `notify`. Write down **whether the confirmation is held, what the user sees, and what approving it does** (docs/07: held, in a headless run; an interactive one isn't measured). Then with `crossSessionInbound` set to `accept` in that session's settings: the confirmation arrives.
- [ ] With a subscription open (`circitor status` reads `subscriptions: 1`), run `circitor stop` in a terminal: the session is told (`circitor: the daemon is stopping; …`), and the command in that notice, run by the agent, exits 6 after waiting 8 s, without starting a daemon (`--no-spawn`, NTF-10).
- [ ] `circitor notify --until merged` in a plain terminal, outside any session: exit **11**, and a line naming the `wait`.
- [ ] On the Mac, write down `echo $CLAUDE_CODE_MESSAGING_SOCKET` from inside a session (the directory only, and the path's length) and `stat -f '%Sp %Su' "$CLAUDE_CODE_MESSAGING_SOCKET"`: where the socket is and its mode there aren't measured (docs/07).

**Afterwards**
- [ ] `yachiko/experiment`: every PR of today is closed or merged, every branch deleted, no workflow on `main`. List what was left behind, as docs/05 does.
- [ ] **The entry is in `docs/05-measurements.md`**, dated, with the setup (machine, OS version, `circitor version`, `gh --version`, Claude Code's version), each step's result, the points spent, and which open items it closes: docs/04 §8 items 1, 2, 4, 6; docs/03 §6 item 1; docs/07's "to measure" items 1 to 3.
- [ ] Each failed check has a follow-up task or a spec question, named in the entry.
- [ ] Told the orchestrator, so the task gets its Evidence.

## Traps
- **`gh` spends the same quota** (docs/02 §2). During step 4 every `gh` call but `gh api rate_limit` spoils the comparison, and so does another agent session on the machine.
- **GitHub's counters lag, and can read higher after reading lower** (docs/02 §2). A difference of a few points in step 4 isn't a finding; tens are.
- **The search lags the list** by up to 8 s on appearing and 7 s on disappearing (docs/02 §3): hence step 1's minute. `circitor list` as text stops at 50 PRs: compare the `--json`.
- **After a push GitHub serves the old head for up to 3 s** (docs/02 §4). Step 2 is the check that `--sha` holds against it: run the wait at once, not after a pause.
- **A spawned daemon doesn't see `GH_TOKEN`** (OPS-5): it uses gh's login. A LaunchAgent has launchd's small `PATH`: `gh_path` in `status` is how to see which `gh` it found.
- **`circitor stop` under a service manager stays a stop** only because the plist restarts on failure alone (OPS-18). If the daemon comes back after `stop`, that's a finding for T19.
- **`CIRCITOR_LOG` reaches a spawned daemon from the command that starts it**, so stop the old one first. A debug log holds costs and names, never the token or a title (OPS-9): read ten lines before pasting any into docs/05.
- **A lid close on a Mac on power may not sleep at once.** Check with `pmset -g log | grep -e ' Sleep ' -e ' Wake ' | tail -4` that it did.
- **Nothing private goes into docs/05:** no title, no body, no other repository's name, no token, and no session token (`CLAUDE_CODE_MESSAGING_TOKEN`).
- **Step 7 runs inside a real session, on purpose:** it's the one place a notice is posted to a real Claude Code socket. Use a throwaway session, not one doing other work: a notice starts a turn in it.
- **A session that bypasses prompts and gets no confirmation isn't a failure of circitor** (NTF-12): it's the result to write down. `accept` opens that session to every local sender, not to circitor alone: set it for the check, and take it out afterwards.
- A workflow file on a branch that gets merged breaks docs/05's rule. Step 3's PR has none; step 2's is closed unmerged.

## Open questions
- **TST-13's checks run "on a release published to the tap and before it's announced".** T39 publishes `v0.1.0`, and this task accepts it. The owner may want a pre-release instead.
- **Everything checked here is unverified today:** docs/04 §8 items 1, 2, 4 and 6 and docs/03 §6 item 1 are open, and OPS-4's macOS location, OPS-5's lookup of `gh`, OPS-14, OPS-18 and POL-22 are (proposal) on them. A result against the spec becomes a spec change, through the owner.
- **Step 5 on Linux** rests on an assumed rule (docs/03 §2: the seccomp filter and Unix sockets). `allowUnixSockets` may not work there at all; that's the answer to write down.
- **Everything about notify on macOS is unmeasured** (docs/07): the socket's place and mode, the format, what a held message looks like. Step 7's results decide whether 08-notify's (proposal) labels can go, through the owner.
- **D6:** nothing here measures required checks (`blocked` and `behind` are built as proposals).
