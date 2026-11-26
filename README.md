# circitor

A small local daemon that watches your GitHub pull requests, so coding agents can ask it instead of GitHub. One poller serves every agent session, which keeps them off your API quota.

**Status:** facts gathered, the specs drafted and hardened (draft 0.2), and the implementation plan written. The code is being built to that plan; nothing has run on a Mac yet.

| Folder | Holds |
|---|---|
| [`docs/`](docs/01-direction.md) | What's decided, and the measured facts about GitHub, the agent clients and the platform |
| [`specs/`](specs/README.md) | Behaviour, as numbered requirements |
| [`plan/`](plan/README.md) | Contracts and task briefs for building it |
| `src/`, `tests/` | The daemon and the CLI, and their tests |
| [`skills/`](skills/circitor/SKILL.md) | The guide for agents, as a skill; `AGENTS.section.md` at the root is the same guide as a section to paste |
| [`packaging/`](packaging/) | A LaunchAgent plist and a systemd user unit, to run the daemon as a service |
| `.claude/skills/` | `harden`, `build-plan` and `add-feature`: how the specs and the plan are made |

Start with [direction](docs/01-direction.md).

## Install

```sh
brew install yachiko/tap/circitor
```

Use the full name. It trusts that one item from the tap and nothing else. circitor needs the GitHub CLI, logged in: `gh auth login`.

## First use

- `circitor list` starts the daemon if none runs, and lists your open pull requests.
- `circitor guide` prints what an agent needs to know.
- `circitor status` shows the daemon's health and its paths. `circitor stop` stops it.

## For agents

The guide ships in two forms. **Installing them isn't automated in v1**: copy them by hand.

- **A skill**, [`skills/circitor/SKILL.md`](skills/circitor/SKILL.md). Claude Code, OpenCode v2 and pi each read skills from their own skills folders. Put the `circitor` folder in the project's skills folder, or in the one under `~`, such as `~/.claude/skills/`.
- **A section**, [`AGENTS.section.md`](AGENTS.section.md). Paste it into `AGENTS.md`, which OpenCode and pi read, or into `CLAUDE.md`.

Claude Code asks before running a command it hasn't seen. To allow circitor, add this to `permissions.allow` in its settings:

```json
"Bash(circitor *)"
```

## Notices in Claude Code

`circitor notify` asks the daemon to post a short message into your Claude Code session when something happens to a pull request. It is for Claude Code only. **Measured on Linux only**; nothing here was run on a Mac.

**circitor can't tell a delivered notice from a held or a refused one.** Only the agent can, by the confirmation it gets. Where notices go:

- **A session that asks before acting**, with `crossSessionInbound` unset: notices arrive. Verified on Linux.
- **A session that bypasses permission prompts:** notices are held. An unattended one never gets them (verified). An interactive one asks its user, for 5 minutes (documented, not measured, so expected).
- **`crossSessionInbound` set to `accept`:** notices arrive in both kinds of session, **and so do messages from every local sender, not only circitor's**. Set it only if you accept that. With `hold` or `refuse`, notices never arrive.
- Claude Code shows a notice as a message from another session.
- Notices rest on an interface Claude Code doesn't document. If a release changes it, confirmations stop arriving, and `circitor wait` still works.
- **In an unattended run** (`claude -p`, the SDK, CI) a notice arrives only while the session still works: the session ends with its turn, and its socket with it. There, a `circitor wait` in the background is the way.
- **After a session is resumed, or the daemon crashes, a notify is lost.** Ask again.

## Under Claude Code's sandbox

**This is untested**, and the Linux rule is assumed. Claude Code's sandbox may stop the CLI from reaching the daemon's socket. Two settings are expected to help.

Recommended: run `circitor` outside the sandbox. This also covers a daemon the CLI starts.

```json
{
  "sandbox": {
    "excludedCommands": ["circitor"]
  }
}
```

Or allow the socket. Take its path from `circitor status`, under `paths`.

```json
{
  "sandbox": {
    "network": {
      "allowUnixSockets": ["/run/user/1000/circitor/circitor.sock"]
    }
  }
}
```

With the second setting, a daemon the CLI starts may be confined by the sandbox and unable to reach GitHub. Start it outside the sandbox instead, with `circitor daemon`.

## As a service

A service keeps the daemon running without a first command to start it. Both files below sit in [`packaging/`](packaging/). Neither sets a `PATH`: circitor finds `gh` in Homebrew's `bin` by itself. Both run Homebrew's `bin/circitor`, never a versioned path, so `brew upgrade` doesn't break them. Each restarts the daemon on failure only. These are expected to work; nobody has run them yet.

### macOS

Save as `~/Library/LaunchAgents/dev.circitor.daemon.plist`, and replace `USERNAME` with yours: launchd doesn't expand `~`. On an Intel Mac, Homebrew's prefix is `/usr/local`, so use `/usr/local/bin/circitor`.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.circitor.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>/opt/homebrew/bin/circitor</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardErrorPath</key>
    <string>/Users/USERNAME/Library/Logs/circitor.log</string>
</dict>
</plist>
```

Load and start it, and stop it:

```sh
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.circitor.daemon.plist
launchctl bootout gui/$(id -u)/dev.circitor.daemon
```

### Linux

Save as `~/.config/systemd/user/circitor.service`. For another Homebrew prefix, change `ExecStart`. Stderr goes to the journal: `journalctl --user -u circitor`.

```ini
[Unit]
Description=circitor, a local daemon that watches your GitHub pull requests

[Service]
ExecStart=/home/linuxbrew/.linuxbrew/bin/circitor daemon
Restart=on-failure

[Install]
WantedBy=default.target
```

Enable and start it, and stop it:

```sh
systemctl --user enable --now circitor
systemctl --user stop circitor
```

### Stopping

`circitor stop` stops the service's daemon, and it stays stopped. A daemon that `stop` had to kill is started again by the service. To stop the service itself, use `launchctl bootout` or `systemctl --user stop`.

## Upgrades

`brew upgrade circitor` replaces the binary. A daemon the CLI started is replaced by the next command you run. A service keeps running the old version until you restart it:

```sh
launchctl kickstart -k gui/$(id -u)/dev.circitor.daemon
systemctl --user restart circitor
```

Use the same command after you change `config.toml`: it is read at the daemon's start only.

## Files and environment

**The config file is optional.** It is `config.toml` in `$XDG_CONFIG_HOME/circitor/`, or `~/.config/circitor/` on macOS and Linux alike. An unknown key or a value out of range stops the daemon, naming it. `circitor daemon --check-config` checks the file without starting anything.

| Key | Default | Range |
|---|---|---|
| `owners` | `[]` (every owner) | list of logins |
| `interval` | 30 | 10 to 300 s |
| `hot_interval` | 10 | 5 to 60 s |
| `idle_interval` | 300 | 60 to 3 600 s |
| `idle_after` | 600 | 60 to 86 400 s |
| `exit_after` | 3 600 | 300 to 604 800 s; 0 never |
| `keep_done` | 86 400 | 0 to 604 800 s |
| `quota.max_per_hour` | 600 | 60 to 4 000 |
| `quota.reserve` | 500 | 0 to 4 000 |
| `gh_path` | `gh` | an absolute path, or a name without `/` |

`hot_interval` must not exceed `interval`, and `interval` must not exceed `idle_interval`.

| | Linux | macOS |
|---|---|---|
| State directory (`state.json`, `circitor.lock`, `circitor.log`) | `$XDG_STATE_HOME/circitor/`, or `~/.local/state/circitor/` | `~/Library/Application Support/circitor/` |
| Socket (`circitor.sock`) | `$XDG_RUNTIME_DIR/circitor/`, or the state directory | the state directory |

- `CIRCITOR_SOCKET` sets the socket's path, for the CLI and the daemon. It must be absolute, and at most 103 bytes.
- `CIRCITOR_LOG` sets the log level: `error`, `warn`, `info` (the default) or `debug`.
- `CIRCITOR_NO_SPAWN=1` stops the CLI from starting a daemon, the same as `--no-spawn`. The command then fails with code 6.

**Privacy:** circitor talks to `api.github.com`, and to the socket of a Claude Code session on your machine that asked for a notice. Nothing else. There's no telemetry and no update check. **The token** is the one `gh auth token` gives: circitor never stores it. A non-empty `GH_TOKEN`, then `GITHUB_TOKEN`, takes the place of gh's login, as it does for gh.

## Building

`make verify` formats, lints, tests, and checks that the code compiles for macOS. **From Linux, the macOS build needs zig and cargo-zigbuild:**

```sh
sudo pacman -S zig    # or your system's package, or ziglang.org
cargo install cargo-zigbuild --locked
```

On a Mac, plain cargo builds for macOS, so neither is needed.
