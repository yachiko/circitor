# Operations

**Status:** draft 0.2, 2026-11-25: notify (D7). Draft 0.1, 2026-11-24. Hardened: pass 1 (2026-11-25), pass 2 (2026-11-25), pass 3 (2026-11-25), planning findings (2026-11-25).

Configuration, files, the token, the daemon's life, and how circitor is shipped. Platform facts are in [docs/04](../docs/04-platform.md); nothing in it was checked on a Mac yet.

## Configuration

- **OPS-1** **One optional TOML file**, `config.toml`, in `$XDG_CONFIG_HOME/circitor/`, or `~/.config/circitor/` on macOS and Linux alike. It's read at the daemon's start only, before anything else. An unknown key or a value out of range stops the daemon with a message naming it, on stderr; `circitor daemon --check-config` checks the same without starting: exit 0 when the file is missing or valid, otherwise exit 1 with the same one-line message on stderr. **The intervals are checked against each other:** `hot_interval` ≤ `interval` ≤ `idle_interval`. A file that breaks this stops the daemon like a value out of range.
  - **Only a missing file means the defaults.** A file that can't be read, isn't valid TOML, or has a value of the wrong type stops the daemon like a value out of range.
  - **A bad config never costs a working daemon or a long wait:** CLI-9 checks it before replacing one, and CLI-7 ends its wait when a spawned daemon stops on it.
- **OPS-2** The keys (proposal: every default and range):

| Key | Default | Range | Used by |
|---|---|---|---|
| `owners` | `[]` (every owner) | list of logins | POL-5 |
| `interval` | 30 | 10–300 s | POL-13 |
| `hot_interval` | 10 | 5–60 s | POL-14 |
| `idle_interval` | 300 | 60–3 600 s | POL-13 |
| `idle_after` | 600 | 60–86 400 s | POL-13 |
| `exit_after` | 3 600 | 300–604 800 s; 0 never | OPS-8 |
| `keep_done` | 86 400 | 0–604 800 s | STA-2 |
| `quota.max_per_hour` | 600 | 60–4 000 | POL-17 |
| `quota.reserve` | 500 | 0–4 000 | POL-18 |
| `gh_path` | `gh` | an absolute path, or a name without `/` | OPS-5 |

  - **`keep_done` stops at 7 days**, so ended PRs don't fill STA-3's 200 places: nothing is pushed out to make room.

## Files

- **OPS-3** **The state directory** holds `state.json`, `circitor.lock` and `circitor.log`:
  - Linux: `$XDG_STATE_HOME/circitor/`, or `~/.local/state/circitor/`
  - macOS: `~/Library/Application Support/circitor/`
  
  It's created with mode 0700. An existing one owned by the user is set to 0700; one owned by anyone else stops the daemon, naming it. The same holds for `$XDG_RUNTIME_DIR/circitor/`. **A directory named by `CIRCITOR_SOCKET` is used as it is:** it's never created or changed, and the socket's mode 0600 protects it.
- **OPS-4** **The socket** is `circitor.sock`:
  - Linux: in `$XDG_RUNTIME_DIR/circitor/` (mode 0700), or the state directory if that variable isn't set
  - macOS: in the state directory. macOS has no runtime directory, and its temp directory is cleaned after three idle days (docs/04 §4).
  - `CIRCITOR_SOCKET` overrides the path, for the CLI and the daemon alike. **It must be absolute:** a relative one is refused, naming it, by both. An empty one counts as unset, and so does an empty or relative `XDG_CONFIG_HOME`, `XDG_STATE_HOME` or `XDG_RUNTIME_DIR`.
  - **A socket answers** when `GET /v1/status` gets a complete response within 2 s. A missing file or a refused connection is no daemon; anything else, a timeout included, is a daemon that doesn't answer.
  - **The socket's mode is 0600 before it's at its path:** it's bound at a temporary name in the same directory, as long as its own name, set to 0600, then renamed over the path (D10). The process's umask is never changed, so nothing else the process creates meanwhile is affected. macOS isn't checked. **At start, once the lock is held (STA-11), a file at the socket path is removed only if it's a socket that refuses a connection.** One that answers is another daemon's: the daemon exits as STA-11 says. Anything else stops it, naming the path.
  - **A path over 103 bytes is refused** with a message naming `CIRCITOR_SOCKET` (docs/04 §4: the limit is 104 on macOS).
  - **The CLI also tries the socket path in `circitor.lock`** (STA-11, CLI-7).
  
  (proposal: the macOS location; whether `gh` and sockets behave there needs the owner's Mac, docs/04 §8)

## The token

- **OPS-5** **The token source** is `gh auth token --hostname github.com`, run with a 10 s limit. gh itself lets a non-empty `GH_TOKEN`, then `GITHUB_TOKEN`, win over its login (docs/02 §1). **The token source in use** is the variable that is set, or gh's login.
  - **The token is held in memory only.** It's never written to a file, a log, an error message or an API response.
  - **A spawned daemon uses gh's login** (D4): CLI-7 starts it without `GH_TOKEN` and `GITHUB_TOKEN`, so a token exported in one agent's shell isn't used for all. gh's own settings (`GH_CONFIG_DIR`, `XDG_CONFIG_HOME`) still come from the shell that started it. `status` shows the login and the token source in use, never the value.
  - **The token is read after the socket is bound** and never delays it. **Until that first read ends, the daemon isn't in `auth`:** `daemon.problem` stays `null`, and every request that needs GitHub (the first poll, read-throughs, `track`, `fresh`, a wait's arrival refresh) waits for it. `status` answers at once.
  - A read that ends without a token puts the daemon in the `auth` state (POL-20); it still runs.
  - **A `gh_path` with a `/` is the only one tried:** one that can't be run is a failure naming it. A name is looked for in the daemon's `PATH`, and then in Homebrew's `bin` by its fixed paths (`/opt/homebrew/bin`, `/usr/local/bin`, `/home/linuxbrew/.linuxbrew/bin`), never the directory circitor's binary resolves to: launchd gives a service a minimal `PATH` (docs/04 §2). **`gh` is looked up again at each read**, so a daemon that runs for weeks survives `brew upgrade gh`. `status` shows the path that was run. (proposal; Homebrew's layout isn't in docs/04)
  - **A read has three outcomes, told apart:**
    - a token: exit 0, and stdout holds one non-empty line once its newline is cut. Stderr is ignored: gh may warn there (docs/02 §1).
    - **no login:** a non-zero exit whose stderr contains `no oauth token found` (docs/02 §1). Stderr is read for this only, never logged. The token in hand is dropped, the daemon is in `auth` (POL-20), and the fix is `gh auth login`.
    - **a failure:** anything else: `gh` can't be run, fails otherwise, or doesn't answer in 10 s and is killed. The token in hand is kept (POL-3). Without one, the `auth` message names `gh auth token`, its exit status or the timeout, and `gh_path`, never a login.

    One read runs at a time; a request that needs one shares it. `gh`'s output is never logged, only its exit status. The login (`viewer`, POL-4) is logged at the first poll and when it changes, and `status` shows it. (proposal; whether `gh auth token` works from a service on macOS, and what it answers while the Keychain asks for access, isn't checked, docs/04 §8)

## The daemon's life

- **OPS-6** **Two ways to run it**, the same code otherwise:
  - `circitor daemon`: in the foreground, logging to stderr, for launchd, systemd or a terminal. It never exits for idleness.
  - `circitor daemon --spawned`: what the CLI starts (CLI-7). Its stderr is the log file (CLI-7), and it exits when idle (OPS-8).
  - **A foreground daemon takes over from a spawned one.** If the lock's holder answers with `mode: spawned` (API-5), it asks it to shut down (API-17) and waits for the lock as STA-11 says. **It repeats the takeover** each time the holder answers with `mode: spawned`, until it holds the lock or 15 s have passed. **A foreground daemon never exits 0 for want of the lock:** without it by then, it exits 1 naming the holder's socket, so a service manager starts it again. The spawned daemon's waiters reconnect to it (CLI-10). A foreground holder is left alone: a second foreground daemon exits with an error naming the holder's socket.
- **OPS-7** **Stopping.** On `SIGTERM`, `SIGINT`, `SIGHUP` or a shutdown request: stop accepting connections and remove the socket if it's still its own (STA-11), stop polling, post `stopped` to every open subscription (NTF-10), close every open connection without an answer, waiters' and others' alike (CLI-10 repeats them), write the state file (STA-7) unless STA-8 left it alone, release the lock, exit 0. **A waiter that reconnects starts no daemon** (CLI-10): a stop stays a stop.
  - **The stop has a 5 s deadline.** The final write doesn't wait for STA-7's 5 s spacing, a running `gh` is killed, and a request to GitHub in flight is dropped. A stop not done by then is logged, and the daemon exits at once: the lock goes with the process, and `state.json` stays as last renamed (STA-7).
  - A second `SIGTERM` or `SIGINT` during the stop does the same at once. A final write that failed is logged.
  - **Every stop exits 0**, so a service manager doesn't restart it (OPS-18). Status 1 is for a crash.
- **OPS-8** **Idle exit.** A spawned daemon exits when it has had no client request, no waiter and no open subscription (NTF-5) for `exit_after`, on the monotonic clock (POL-26). A `status` request doesn't count (API-5), so watching a daemon doesn't keep it alive. The next CLI call starts a new one; it loses the event history, the cursors' epoch (STA-9) and the `asked` PRs. It stops as OPS-7 does.
- **OPS-9** **Logging.** One line per event, `<time> <level> <message>`, to stderr: the time RFC 3339, UTC; the level `error`, `warn`, `info` or `debug`. `info` is the default; `CIRCITOR_LOG` names another, and an unknown value is `info`, with a warning line.
  - **The log file is rotated only before a daemon starts:** the CLI (CLI-7) renames a `circitor.log` over 5 000 000 bytes to `circitor.log.1`, replacing any earlier one, and opens a new one for appending. A foreground daemon's stderr is the service manager's (OPS-18).
  - **Logging never stops the daemon.** A line that can't be written is dropped, with the `log_unwritable` warning (API-5) while it lasts.
  - Logged: start and stop with the version and paths, and the stop's cause (a signal, a shutdown request (API-17: from `stop`, or a CLI replacing it, CLI-9), idle, its socket or lock file not its own, the 5 s deadline); a sleep noticed, with its length (POL-22); the tier changing; a problem beginning and ending (never each retry); quota warnings; a PR tracked, untracked or gone; the state file set aside; every warning of API-5, when it's raised and cleared; a state-file write failing and working again (STA-7); a failed token read (POL-3); a PR's error, once while it lasts (POL-24). A subscription registered and ended, with its end's name and never a notice's text; a notice that couldn't be posted (NTF-6, NTF-10).
  - **Never logged:** the token, a session's token (NTF-9), PR titles, or free text from a response (error messages, comment or review content). From a response only names, numbers, states, costs and error `type`s are logged.
  - `CIRCITOR_LOG=debug` adds one line per request to GitHub: its kind, cost and duration.

## Shipping

- **OPS-10** **Homebrew, from the owner's tap** (`yachiko/homebrew-tap`), with prebuilt binaries from GitHub releases. **A formula written by `dist`** (D5). It doesn't need `brew services` in v1. A cask is quarantined, so its binary must be notarised; a formula isn't (docs/04 §2). The README installs with the full name, `brew install yachiko/tap/circitor`, which also trusts that item (docs/04 §2).
- **OPS-11** The binary carries its version and commit, printed by `circitor version` and sent as `version` (API-2). **`version` is `MAJOR.MINOR.PATCH`, with `+<commit>` as build metadata** (CLI-9).
- **OPS-12** **For agents,** the repository ships the guide (CLI-11) in two forms: `skills/circitor/SKILL.md`, read by Claude Code, OpenCode v2 and pi, each from its own skills folders, and a section to paste into `AGENTS.md`, read by OpenCode and pi (docs/03 §1). The README says where each goes. Installing them isn't automated in v1. (proposal)
- **OPS-13** **Targets:** macOS arm64 and x86-64, Linux x86-64 and arm64. Windows isn't built.
- **OPS-14** **Sandboxed agents.** The README gives two Claude Code settings: `sandbox.excludedCommands` for `circitor`, which runs the CLI, and a daemon it starts, outside the sandbox on macOS and Linux; or `sandbox.network.allowUnixSockets` with the socket's path, whose Linux rule is assumed (docs/03 §2). With the second, a daemon the CLI starts may be confined by the sandbox and unable to reach GitHub; the README then recommends starting it outside, with `circitor daemon`. (proposal; untested, docs/03 §6)
- **OPS-15** **Upgrades.** `brew upgrade` replaces the binary. CLI-9 says which daemons are then replaced; a foreground one runs the old version until it's restarted (OPS-18).
- **OPS-18** **Running as a service.** The README gives a LaunchAgent plist and a systemd user unit for `circitor daemon`. Each:
  - runs Homebrew's `bin/circitor`, never a versioned path
  - restarts it on failure only, so `circitor stop` stays a stop
  - keeps its stderr: `StandardErrorPath` on macOS, the journal on Linux
  - on macOS, has the label `dev.circitor.daemon` and writes stderr to `~/Library/Logs/circitor.log`; launchd doesn't expand `~`, so the README's plist shows the absolute path to edit (proposal)
  - says that a daemon `circitor stop` had to kill (CLI-8) is restarted, and that a service is stopped with `systemctl --user stop` or `launchctl bootout`

  It names the command that restarts the service after `brew upgrade` or a config change (`launchctl kickstart -k`, `systemctl --user restart`). (proposal)
- **OPS-16** **Privacy.** circitor talks to `api.github.com`, and to the socket of a Claude Code session on the same machine that asked for a notice (NTF-6). It talks to nothing else. There's no telemetry and no update check.

## Targets for behaviour

- **OPS-17** (proposal)
  - A CLI call answered from memory returns within 50 ms.
  - From a cold start to the first `list` answer: within 5 s on a working network.
  - The daemon uses under 30 MB of memory with 200 tracked PRs, and no CPU between polls but POL-26's tick.
