---
id: T19
title: "README, service files and sandbox settings"
kind: infra
milestone: M4
size: S
status: todo
depends_on: [T11]
requirements: [OPS-12, OPS-14, OPS-15, OPS-18, OPS-10, NTF-12]
owns: [README.md, packaging/]
shares: []
---

# T19 · README, service files and sandbox settings

## Outcome
The README tells a user how to install circitor from the tap, give it to their agents, run it under Claude Code's sandbox, where `notify`'s notices arrive and where they don't, run it as a service and upgrade it; `packaging/` holds the LaunchAgent plist and the systemd user unit of OPS-18.

## Context
T11 wrote the guide and its two shipped forms, `skills/circitor/SKILL.md` and `AGENTS.section.md`; this task tells users where they go. Nothing here is code. **None of it can be run on the development machine** (no Homebrew, no Mac): the owner checks it in T40, and publishes to the tap in T39.

## Read first
- `plan/conventions.md`: "Authority" and "Git". The code rules don't apply.
- `specs/06-operations.md#shipping`: OPS-10, OPS-12, OPS-14, OPS-15, OPS-18; `specs/06-operations.md#the-daemons-life`: OPS-6, OPS-7 (every stop exits 0); `specs/06-operations.md#files` and `#configuration`: OPS-1 to OPS-4 (the paths and keys the README lists); `specs/06-operations.md#the-token`: OPS-5 (how a service finds `gh`)
- `specs/05-cli.md#the-daemon`: CLI-7 (`--no-spawn`), CLI-8 (`stop`), CLI-9 (which daemons an upgrade replaces)
- `specs/decisions.md`: D5, D1, and D7's notes
- `specs/08-notify.md#for-the-user`: NTF-12, both parts; NTF-7 (why there's a confirmation); NTF-10 (what a stop, a crash and a resumed session do to a notify)
- `docs/07-notify.md`: "The session socket" (the rows on a session that bypasses permission prompts, `crossSessionInbound`, how a notice reads to the session, a resumed session), and "What it costs"
- `docs/03-clients.md` §1, §2 and §6: where each client reads skills and instructions; the sandbox keys; that none of it was run
- `docs/04-platform.md` §2, §3 and §8: tap trust, quarantine, what `dist` writes, the unchecked items
- `README.md` as it is; `skills/circitor/SKILL.md` and `AGENTS.section.md` (T11's)

## Scope
- **Keep** the README's first paragraph, its folder table and its "Building" section (T50: zig and cargo-zigbuild for `make verify`). Update the status line, the `plan/` row, and add rows for `skills/`, `packaging/` and `src/`. Then these sections, in this order. Short sentences, British spelling, bold for the key phrase.
- **Install (OPS-10's share).** `brew install yachiko/tap/circitor`, by its full name, and one line on why (it trusts that one item, docs/04 §2). It needs `gh`, logged in (`gh auth login`). **It's a formula, written by `dist`** (D5, OPS-10): the README needn't say so. No `brew services`: OPS-10 and D1 leave it out of v1. Publishing the release and the tap is T39's share of OPS-10.
- **First use.** `circitor list` starts the daemon; `circitor guide` prints what an agent needs; `circitor status` and `circitor stop`. Three or four lines, no copy of the guide.
- **For agents (OPS-12's README share).** Where the two files go, per client, from docs/03 §1: the skill folder for Claude Code, OpenCode v2 and pi, in the project or under `~`; `AGENTS.section.md` pasted into `AGENTS.md` for OpenCode and pi, or into `CLAUDE.md`. Say that installing isn't automated in v1. Add Claude Code's `permissions.allow` entry `"Bash(circitor *)"` (docs/03 §2).
- **Notices in Claude Code (NTF-12).** A section of its own, after "For agents". What `circitor notify` does, in two lines, and that it's for Claude Code only. Then, **each case with what is verified and what is only documented**, as NTF-12 lists them:
  - a session that asks before acting, with `crossSessionInbound` unset: notices arrive (verified on Linux);
  - a session that bypasses permission prompts: notices are held. An unattended one never gets them (verified); an interactive one asks its user, for 5 minutes (documented, not measured);
  - `crossSessionInbound` set to `accept`: they arrive in both, **and so do messages from every local sender, not only circitor's**. Say that plainly, beside the setting. With `hold` or `refuse`: never.
  - Claude Code shows a notice as a message from another session. Notices rest on an interface Claude Code doesn't document: if a release changes it, confirmations stop arriving, and `wait` still works.
  - **In an unattended run** (`claude -p`, the SDK, CI) a notice arrives only while the session still works; there a `wait` in the background is the way.
  - **After a session is resumed, or the daemon crashes, a notify is lost:** ask again.
  - Mark the section: measured on Linux only (docs/07).
- **Under Claude Code's sandbox (OPS-14).** The two settings, as JSON a user can paste: `sandbox.excludedCommands` with `circitor`, which the README recommends; or `sandbox.network.allowUnixSockets` with the socket's path, which `circitor status` prints under `paths`. With the second, say what OPS-14 says about a daemon the CLI starts, and to run `circitor daemon` outside the sandbox. **Mark the section untested** (docs/03 §6 item 1), and the Linux rule as assumed.
- **As a service (OPS-18).** Two files, shown whole in the README and kept in `packaging/`:
  - `packaging/launchd/dev.circitor.daemon.plist`: `ProgramArguments` `/opt/homebrew/bin/circitor` and `daemon`; `RunAtLoad`; `KeepAlive` as a dictionary with `SuccessfulExit` false; `StandardErrorPath` `~/Library/Logs/circitor.log` (OPS-18), written as an absolute path with a `USERNAME` placeholder to edit (launchd doesn't expand `~`).
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
- `grep -n` finds each of these in `README.md`: `brew install yachiko/tap/circitor`, `excludedCommands`, `allowUnixSockets`, `untested`, `launchctl kickstart -k`, `systemctl --user restart`, `launchctl bootout`, `systemctl --user stop`, `skills/circitor/SKILL.md`, `AGENTS.section.md`; and for NTF-12: `circitor notify`, `crossSessionInbound`, `accept`, `every local sender`, `claude -p`, `resumed`, `circitor wait`.
- `grep -n 'brew services\|Cellar' README.md packaging -r` finds nothing: no `brew services`, no versioned path.
- `make verify` still passes.

## Traps
- **Restart on failure only.** `KeepAlive` as plain `true`, or `Restart=always`, would start the daemon again after `circitor stop`, which exits 0 (OPS-7).
- A versioned path (`…/Cellar/circitor/0.1.0/…`) dies with the next upgrade: only Homebrew's `bin`.
- `StandardErrorPath` must not be `circitor.log` in the state directory: that file belongs to daemons the CLI starts, and the CLI rotates it (OPS-9).
- docs/03 names the sandbox keys, not the form of their values. Write `["circitor"]` and the socket's path, and don't invent a pattern syntax.
- OPS-12 and docs/03 §1 agree: the skill is read by Claude Code, OpenCode v2 and pi, each from its own skills folders. Name all three.
- **Don't promise that a notice arrives** (NTF-12). circitor can't tell a delivered notice from a held or a refused one; only the agent can, by the confirmation. And don't recommend `accept` without its cost in the same sentence.
- Don't promise what isn't checked: the unsigned binary on Apple Silicon, `gh` from a LaunchAgent, the sandbox (docs/04 §8 items 1, 2 and 4; docs/03 §6 item 1). Say "expected", and point at nothing more.
- The README is for users. No requirement IDs in its prose.

## Open questions
- **D5 is resolved:** a formula written by `dist` (`specs/decisions.md`). T39 sets it up.
- **The label `dev.circitor.daemon` and the log path `~/Library/Logs/circitor.log` are OPS-18's** (proposal). Homebrew's own label (`homebrew.mxcl.circitor`) is for `brew services`, which v1 doesn't use.
- **NTF-12 is a (proposal)**, and rests on Linux measurements only (docs/07). What a held notice looks like in an interactive session, and anything on macOS, isn't measured: T40's seventh check does it. Write "expected" where the spec says documented.
- OPS-14 and OPS-18 are (proposal) and unchecked: docs/03 §6 item 1; docs/04 §8 items 2 and 4. T40's steps 5 and 6 check them. Build them as written.
