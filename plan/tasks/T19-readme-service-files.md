---
id: T19
title: "README, service files and sandbox settings"
kind: infra
milestone: M4
size: S
status: todo
depends_on: [T11]
requirements: [OPS-12, OPS-14, OPS-15, OPS-18, OPS-10]
owns: [README.md, packaging/]
shares: []
---

# T19 · README, service files and sandbox settings

## Outcome
The README tells a user how to install circitor from the tap, give it to their agents, run it under Claude Code's sandbox, run it as a service and upgrade it; `packaging/` holds the LaunchAgent plist and the systemd user unit of OPS-18.

## Context
T11 wrote the guide and its two shipped forms, `skills/circitor/SKILL.md` and `AGENTS.section.md`; this task tells users where they go. Nothing here is code. **None of it can be run on the development machine** (no Homebrew, no Mac): the owner checks it in T40, and publishes to the tap in T39.

## Read first
- `plan/conventions.md`: "Authority" and "Git". The code rules don't apply.
- `specs/06-operations.md#shipping`: OPS-10, OPS-12, OPS-14, OPS-15, OPS-18; `specs/06-operations.md#the-daemons-life`: OPS-6, OPS-7 (every stop exits 0); `specs/06-operations.md#files` and `#configuration`: OPS-1 to OPS-4 (the paths and keys the README lists); `specs/06-operations.md#the-token`: OPS-5 (how a service finds `gh`)
- `specs/05-cli.md#the-daemon`: CLI-7 (`--no-spawn`), CLI-8 (`stop`), CLI-9 (which daemons an upgrade replaces)
- `specs/decisions.md`: D5 (open) and D1
- `docs/03-clients.md` §1, §2 and §6: where each client reads skills and instructions; the sandbox keys; that none of it was run
- `docs/04-platform.md` §2, §3 and §8: tap trust, quarantine, what `dist` writes, the unchecked items
- `README.md` as it is; `skills/circitor/SKILL.md` and `AGENTS.section.md` (T11's)

## Scope
- **Keep** the README's first paragraph and its folder table. Update the status line, the `plan/` row, and add rows for `skills/`, `packaging/` and `src/`. Then these sections, in this order. Short sentences, British spelling, bold for the key phrase.
- **Install (OPS-10's share).** `brew install yachiko/tap/circitor`, by its full name, and one line on why (it trusts that one item, docs/04 §2). It needs `gh`, logged in (`gh auth login`). **Written for a formula; a note under the line says the form waits on D5.** No `brew services`: OPS-10 and D1 leave it out of v1. Publishing the release and the tap is T39's share of OPS-10.
- **First use.** `circitor list` starts the daemon; `circitor guide` prints what an agent needs; `circitor status` and `circitor stop`. Three or four lines, no copy of the guide.
- **For agents (OPS-12's README share).** Where the two files go, per client, from docs/03 §1: the skill folder for Claude Code, OpenCode v2 and pi, in the project or under `~`; `AGENTS.section.md` pasted into `AGENTS.md` for OpenCode and pi, or into `CLAUDE.md`. Say that installing isn't automated in v1. Add Claude Code's `permissions.allow` entry `"Bash(circitor *)"` (docs/03 §2).
- **Under Claude Code's sandbox (OPS-14).** The two settings, as JSON a user can paste: `sandbox.excludedCommands` with `circitor`, which the README recommends; or `sandbox.network.allowUnixSockets` with the socket's path, which `circitor status` prints under `paths`. With the second, say what OPS-14 says about a daemon the CLI starts, and to run `circitor daemon` outside the sandbox. **Mark the section untested** (docs/03 §6 item 1), and the Linux rule as assumed.
- **As a service (OPS-18).** Two files, shown whole in the README and kept in `packaging/`:
  - `packaging/launchd/dev.circitor.daemon.plist`: `ProgramArguments` `/opt/homebrew/bin/circitor` and `daemon`; `RunAtLoad`; `KeepAlive` as a dictionary with `SuccessfulExit` false; `StandardErrorPath` in `~/Library/Logs/`, written as an absolute path with a `USERNAME` placeholder (launchd doesn't expand `~`).
  - `packaging/systemd/circitor.service`: `ExecStart=/home/linuxbrew/.linuxbrew/bin/circitor daemon`, `Restart=on-failure`, `WantedBy=default.target`. Nothing redirects stderr: the journal keeps it.
  - The README names the other Homebrew prefix (`/usr/local/bin` on an Intel Mac), the commands that load and start each (`launchctl bootstrap gui/$(id -u) …`, `systemctl --user enable --now circitor`), the ones that stop each (`launchctl bootout`, `systemctl --user stop`), and OPS-18's two sentences: `circitor stop` stops the service's daemon and it stays stopped, but one that `stop` had to kill is started again.
  - No `PATH` is set in either file: OPS-5 finds `gh` in Homebrew's `bin` by itself.
- **Upgrades (OPS-15).** `brew upgrade` replaces the binary. A daemon the CLI started is replaced by the next command (CLI-9). A service runs the old version until restarted: `launchctl kickstart -k gui/$(id -u)/dev.circitor.daemon`, `systemctl --user restart circitor`. The same two commands after a change to `config.toml` (OPS-18).
- **Files and environment.** The config file's path and OPS-2's keys with defaults and ranges; the state directory and the socket per platform (OPS-3, OPS-4); `CIRCITOR_SOCKET`, `CIRCITOR_LOG`, `CIRCITOR_NO_SPAWN`. One line on privacy (OPS-16) and on the token (OPS-5: gh's login, never stored).

## Interfaces
- **Consumes:** T11's two files, by path. The paths and keys of 06-operations.
- **Provides:** `packaging/launchd/dev.circitor.daemon.plist` and `packaging/systemd/circitor.service`, which T40 installs by hand, and the README's section names, which T39 and T40 refer to.

## Out of scope
- The guide's text and its two files: T11. Don't edit them; a mistake in them is an open item.
- The release workflow, the formula or cask, the tap: T39. CI: T03.
- Running any of this on a Mac, in a sandbox or under systemd: T40.
- `CLAUDE.md`, `docs/`, `specs/`, `plan/`.

## Acceptance
No Rust changes, so no new tests. Run these and paste the results in the report:
- `python3 -I -c 'import plistlib,sys; d=plistlib.load(open(sys.argv[1],"rb")); assert d["ProgramArguments"]==["/opt/homebrew/bin/circitor","daemon"]; assert d["KeepAlive"]=={"SuccessfulExit":False}; assert d["StandardErrorPath"].startswith("/"); assert d["Label"]=="dev.circitor.daemon"' packaging/launchd/dev.circitor.daemon.plist` passes: the plist parses, runs Homebrew's unversioned path, restarts on failure only and keeps stderr.
- `python3 -I -c 'import configparser,sys; c=configparser.ConfigParser(strict=False); c.optionxform=str; c.read(sys.argv[1]); assert c["Service"]["ExecStart"].endswith("/bin/circitor daemon"); assert c["Service"]["Restart"]=="on-failure"; assert "StandardError" not in c["Service"]' packaging/systemd/circitor.service` passes.
- `systemd-analyze verify --user packaging/systemd/circitor.service`, if the tool is there: its only complaint may be that the binary doesn't exist.
- Each of the two files appears whole in the README: check with a `diff` of the fenced block against the file.
- `grep -n` finds each of these in `README.md`: `brew install yachiko/tap/circitor`, `D5`, `excludedCommands`, `allowUnixSockets`, `untested`, `launchctl kickstart -k`, `systemctl --user restart`, `launchctl bootout`, `systemctl --user stop`, `skills/circitor/SKILL.md`, `AGENTS.section.md`.
- `grep -n 'brew services\|Cellar' README.md packaging -r` finds nothing: no `brew services`, no versioned path.
- `make verify` still passes.

## Traps
- **Restart on failure only.** `KeepAlive` as plain `true`, or `Restart=always`, would start the daemon again after `circitor stop`, which exits 0 (OPS-7).
- A versioned path (`…/Cellar/circitor/0.1.0/…`) dies with the next upgrade: only Homebrew's `bin`.
- `StandardErrorPath` must not be `circitor.log` in the state directory: that file belongs to daemons the CLI starts, and the CLI rotates it (OPS-9).
- docs/03 names the sandbox keys, not the form of their values. Write `["circitor"]` and the socket's path, and don't invent a pattern syntax.
- OPS-12 says the skill is read by Claude Code and OpenCode v2; docs/03 §1 adds pi. The facts doc wins: name all three.
- Don't promise what isn't checked: the unsigned binary on Apple Silicon, `gh` from a LaunchAgent, the sandbox (docs/04 §8 items 1, 2 and 4; docs/03 §6 item 1). Say "expected", and point at nothing more.
- The README is for users. No requirement IDs in its prose except the D5 note.

## Open questions
- **D5 is open** (formula or cask, and the release tool). Build the formula form, as `specs/decisions.md` recommends. A cask would keep the same install line; T39 corrects the note.
- **The label `dev.circitor.daemon` and the log path under `~/Library/Logs/` are this plan's choices**, not the spec's: say so in the report. Homebrew's own label (`homebrew.mxcl.circitor`) is for `brew services`, which v1 doesn't use.
- OPS-14 and OPS-18 are (proposal) and unchecked: docs/03 §6 item 1; docs/04 §8 items 2 and 4. T40's steps 5 and 6 check them. Build them as written.
