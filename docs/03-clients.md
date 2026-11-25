# Client facts

**Status:** read from each tool's documentation and source on 2026-11-24; a few lines were seen in a live Claude Code session the same day. **Purpose:** what the coding agents that call circitor can and can't do. It decides how long a `wait` may block, how an agent learns the CLI, and what the sandbox lets through.

Labels are those of [GitHub API facts](02-github-api.md): **verified**, **observed**, **documented**, **assumed**. Sources are in [references](references.md). Cited from the specs as "docs/03 §2".

These tools change monthly. A line here is true for the version named, and is re-checked before a spec leans on it.

## 1. Summary

| | Claude Code | OpenCode v2 (2.4.1) | OpenCode v1 (1.19.6) | pi (1.6.0) |
|---|---|---|---|---|
| Shell tool's default timeout | 120 s | 120 s | 120 s | none |
| Longest timeout the agent can ask for | 600 s | no maximum | no maximum | about 24 days |
| At the timeout | Moved to the background | Moved to the background | **Killed** | **Killed** |
| Background command | Yes | Yes | No | No |
| Told when a background command exits | Yes | Yes | – | – |
| MCP | Yes | Yes | Yes | Yes, since 0.99.0 |
| Reads `AGENTS.md` | Not checked; it reads `CLAUDE.md` | Yes, and `CLAUDE.md` if there's none | Yes | Yes, and `CLAUDE.md` |
| Skills folders | `.claude/skills`, `~/.claude/skills` | `.opencode/skills`, `.claude/skills`, `.agents/skills` and the same under `~` | – | `.pi/skills`, `.agents/skills` and the same under `~` |

All documented, except the Claude Code rows marked verified in §2.

## 2. Claude Code

| Item | Value | Label |
|---|---|---|
| Bash timeout | 120 000 ms by default, 600 000 ms at most, per call. `BASH_DEFAULT_TIMEOUT_MS` and `BASH_MAX_TIMEOUT_MS` change them. | verified (the tool's own description), documented (the variables) |
| **A foreground command that hits its timeout** | **Is moved to the background, not killed**, unless it starts with `sleep` | verified (seen in a session on 2026-11-24) |
| `run_in_background` | Returns at once; the agent is invoked again when the command exits | verified (seen in a session on 2026-11-24); the documentation doesn't state it |
| Limit on a background command | None in an interactive session. In unattended runs (`-p`, the SDK, CI): 30 minutes by default, 2 hours at most. | documented |
| Output | Capped at 30 000 characters by default (`BASH_MAX_OUTPUT_LENGTH`) | documented |
| Sandbox | **Off by default.** `/sandbox` or `sandbox.enabled` turns it on (macOS Seatbelt; bubblewrap on Linux). | documented |
| Sandbox and Unix sockets | Blocked unless listed in `sandbox.network.allowUnixSockets`, or `allowAllUnixSockets` is set. On Linux a seccomp filter blocks sockets; the exact rule there wasn't confirmed. | documented; the Linux rule assumed |
| Sandbox and localhost | Blocked by default. macOS: `sandbox.network.allowLocalBinding` opens it. **Linux: each command has a private localhost**, so a host daemon is reachable only by running the command outside the sandbox (`sandbox.excludedCommands`). | documented |
| Allowing a command | `permissions.allow` with `"Bash(circitor *)"`. Each part of a compound command (`&&`, `|`, `;`) must be allowed on its own. | documented |
| Teaching it a CLI | A skill (`SKILL.md` with a `description`) in the project or under `~/.claude/skills`, or `CLAUDE.md`. A plugin can bundle skills; nothing says a plugin can install a Homebrew binary. | documented |
| MCP tool call | Still running after 2 minutes: moved to a background task in an interactive session. In `-p` and in subagents it blocks. Idle limit: 30 minutes for stdio servers, 5 for HTTP. | documented |
| MCP tool definitions | Deferred by default and found through tool search, so an idle server costs little context | documented |

## 3. OpenCode

Two lines are current: v2 (the documentation at opencode.ai/docs) and v1.

| Item | Value | Label |
|---|---|---|
| v2 shell tool | Named `shell`. 2 minutes by default (10 in Code Mode). `timeout` in ms, no maximum. At the timeout a foreground command moves to the background. | documented |
| v2 background | `background: true` returns at once, and the session is notified when the command finishes | documented |
| v1 shell tool | 2 minutes by default (`OPENCODE_EXPERIMENTAL_BASH_DEFAULT_TIMEOUT_MS`). **The command is terminated at the timeout.** No background option. | documented |
| A config key for the v2 default | Not found | assumed |
| MCP | Local (stdio) and remote servers, in `opencode.json` under `mcp.servers` (v2) or `mcp` (v1). Execution timeout 12 h in v2. | documented |
| Instructions | `~/.config/opencode/AGENTS.md`, then every `AGENTS.md` from the working directory upwards; `CLAUDE.md` where there's no `AGENTS.md` | documented |

## 4. pi

The repository moved to `earendil-works/pi`, and the package to `@earendil-works/pi-coding-agent`.

| Item | Value | Label |
|---|---|---|
| Bash tool | Takes `command` and an optional `timeout` in **seconds**, with no default. At the timeout the process group is killed. **Runs synchronously; no background option.** | documented |
| MCP | The author wrote in 2025 that pi "does not and will not support MCP". **That changed:** since 0.99.0 (2026-09-29) MCP is built in, over stdio and streamable HTTP, configured in `~/.pi/agent/mcp.json` or `.pi/mcp.json`. Each request times out after 60 s by default. | documented |
| Instructions | `AGENTS.md` or `CLAUDE.md` from `~/.pi/agent/`, the working directory and its parents | documented |
| Skills | `SKILL.md` with a name and a description; only those two go into the prompt | documented |

## 5. What this means for circitor

These follow from the facts above. They're input to the specs, not requirements.

- **A blocking `wait` is safe for 120 s everywhere, and for longer only where the agent sets a timeout.** Three clients default to 120 s. At that point OpenCode v1 kills the command and the agent gets nothing; Claude Code and OpenCode v2 move it to the background. pi has no default, and kills at whatever timeout the agent set.
- **So a `wait` must be cheap to repeat:** it ends by itself before the common 120 s default unless told otherwise, says "not yet" clearly, and takes a cursor so the next call loses nothing.
- **Only two of the four clients can run a command in the background**, so the design can't depend on it. Where it exists it's the better way to wait.
- **Every client can run a CLI; every client now supports MCP too.** The CLI needs no setup, and an MCP call of several minutes is handled differently by each client (60 s per request in pi).
- **One instruction file doesn't reach every client.** `.claude/skills` is read by Claude Code and OpenCode v2; `AGENTS.md` by OpenCode and pi; `CLAUDE.md` by Claude Code, pi, and by OpenCode as a fallback.
- **With Claude Code's sandbox on, the socket needs an allow entry**, and on Linux a TCP port wouldn't help.

## 6. Open items

1. Run circitor's CLI inside Claude Code's sandbox on macOS and on Linux, and record what's needed.
2. Confirm on a real OpenCode v1 and pi that a killed `wait` leaves nothing behind in the daemon.
3. How each client shows a command that prints nothing for minutes.
