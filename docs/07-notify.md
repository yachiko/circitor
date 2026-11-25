# Notify: pushing a message into a session

**Status:** an idea, recorded 2026-11-25. **It isn't in the specs or the plan.** If it's taken up, it enters through `/add-feature`, like any feature. This doc holds the owner's idea, what was measured about Claude Code's session socket the same day, and a first sketch.

Labels are those of [GitHub API facts](02-github-api.md): **verified**, **observed**, **documented**, **assumed**. The facts here aren't yet in [client facts](03-clients.md), which the plan takes a snapshot of; they move there when the feature is specified.

## The idea

The owner's, after seeing one Claude Code session send a message to another:

1. A session asks the daemon to watch a PR for a condition, and goes on with its work.
2. The daemon keeps the session's address, the PR and the condition.
3. When the condition holds, **the daemon sends a message to that session.**

Today the specs assume the opposite direction: a `wait` is a CLI process that stays alive until it has an answer (API-9, CLI-6).

## What v1 already gives

In Claude Code, a `circitor wait` run as a background command behaves like a push from the session's side: the session works on or sits idle, and is woken when the command exits ([docs/03 §2](03-clients.md#2-claude-code)). It needs no address, because the process belongs to the session. OpenCode v2 does the same; OpenCode v1 and pi can't (docs/03 §1).

So the idea adds two things: **no process per wait**, and **several events from one request**.

## The session socket

Measured on Claude Code 2.1.332 on Linux, by posting to the measuring session's own socket. The probe script is [`post-to-session.py`](measurements/2026-11-25/post-to-session.py).

| Item | Value | Label |
|---|---|---|
| Where | Every session binds a Unix socket and exports its path to its shell commands and hooks as `CLAUDE_CODE_MESSAGING_SOCKET`. Here: `/run/user/1000/cc-socks/<number>.sock`, mode 0600, in a directory of mode 0700. | verified |
| The token | `CLAUDE_CODE_MESSAGING_TOKEN`, 32 characters, exported the same way | verified |
| What it's for | "for a script or hook to post into a session" | documented |
| **The message format** | One JSON line: `{"type":"user","message":{"role":"user","content":"<text>"}}`. **It isn't on the documentation page.** It comes from a hint in Claude Code's own debug-log text, found in the 2.1.332 binary. | verified (delivered); undocumented |
| The auth line | `{"type":"auth","token":"<token>"}` as the first line. Optional on macOS and Linux, required on Windows. | documented |
| **Without the auth line, or with a wrong token** | **Delivered all the same** on Linux. Only the file permissions protect the socket. | verified |
| Other lines | A plain-text line, and JSON with `type: "message"`, were dropped without a word | verified |
| Replies | **The socket never answers**: not to a message, a wrong token or a malformed line. It doesn't close the connection either. | verified |
| Several lines on one connection | Each is delivered as its own message | verified |
| The same text twice, 1 s apart | Both delivered. (The documentation says identical repeats "within a short window" are dropped.) | verified |
| Multi-line `content` | Arrives intact | verified |
| A sender's name | `from` and `name` keys in the line are ignored. The session is told "Another Claude session sent a message", with no name. | verified |
| **How it reads to the session** | As a message from another Claude session, **not from the user**, followed by a standing caution: it can't grant permissions or approve a prompt. | verified |
| **To a busy session** | Delivered between two tool calls | verified |
| **To an idle session** | **Starts a new turn.** Posted at 22:19:17; the session's first command ran at 22:19:21. | verified |
| A session that bypasses permission prompts | **Holds** each message for the user's approval, for 5 minutes by default, unless the sender identifies itself as bypassing too | documented |
| `crossSessionInbound` | `accept`, `hold` or `refuse`, set by the user; with none set, the rule above | documented |
| Limits | A connection with no complete line within 30 s is closed. About a million characters a message. At most 50 messages queued, 100 held. | documented |
| Sessions without the socket | Those started in bare mode | documented |
| macOS and Windows; a held or refused message seen from the sender; a session inside a permission prompt; a subagent's shell | Not measured | assumed |

Source for the documented rows: https://code.claude.com/docs/en/cross-session-messaging

## Other ways in, and why not

| Mechanism | Verdict | Label |
|---|---|---|
| **The Monitor tool** | A session runs a command and gets each line it prints as a message. It fits a stream of events and needs nothing new from Claude Code, but a watch expires after 30 minutes at most and must be started again. | verified (the tool's description) |
| **Channels** | An MCP server pushes events into a session. A research preview: a server of one's own needs `--dangerously-load-development-channels` when the session starts. | documented |
| **`claude -p --resume <id>`** | Starts another process on the same transcript. It doesn't reach a session open in a terminal, and two writers can lose messages. | documented |
| **A hook with `asyncRewake`** | A hook process that exits with code 2 wakes an idle session. It would have to stay alive waiting on the daemon, which is a `wait` by another name. | documented |

## A first sketch

Not a spec. Each line would become a requirement, or be dropped, in `/add-feature`.

- **A new command:** `circitor notify [PR] --until merged|checks-done|changed [--sha SHA] [--every]`. It reads the two variables from its own environment, hands them to the daemon with the PR and the condition, prints what it registered, and exits at once. Outside a session that has the socket, it fails and says to use `wait`.
- **The address is the socket path**, not a session ID: it's what the daemon delivers to, and it stops existing when the session does.
- **A subscription** is the socket path, the token, the PR, the condition and a cursor. It's a waiter without a connection: it makes its PR hot (POL-14), counts against the waiter limit (API-14), and keeps a spawned daemon from exiting idle (OPS-8).
- **Delivery:** when the condition holds, or the PR ends, the daemon connects, sends the auth line and one message line, and closes. Without `--every` the subscription is then dropped. With it, every later event of that PR is delivered until the PR ends.
- **The message** is one line of circitor's own words, then the command for the details:
  ```
  circitor: yachiko/circitor#12 checks failing (1 of 5). Details: circitor show yachiko/circitor#12
  ```
- **It ends** when delivered (without `--every`), when the PR is dropped, when connecting to the socket fails (the session is gone), after 24 hours, or with `circitor notify --cancel`.
- **Kept in memory only.** A daemon restart loses subscriptions. The token is never written to disk or logged, like GitHub's.
- **`wait` stays** as it is, for every client.

## What it costs

- **Claude Code only.** OpenCode and pi keep using `wait`.
- **The message format is undocumented.** A Claude Code release can change it, and since the socket never answers, **circitor couldn't tell**: a dropped line and a delivered one look the same from outside.
- **No confirmation at all.** Held, refused, expired and delivered are indistinguishable to the sender. The only failure circitor can see is a socket that's gone.
- **A session that bypasses permission prompts holds the message** until its user approves it. That is the mode an unattended agent is most likely to run in, and the one where a push would matter most.
- **It arrives as "another Claude session"**, with a caution attached. The agent has to be told, in the guide, that circitor's notices come this way.
- **Text from GitHub would enter a session unasked.** A PR's title and a check's name are written by other people. The sketch keeps the message to circitor's own words, a PR's name and a URL for that reason.
- **On Linux and macOS any process of the user can post to any of their sessions.** circitor adds no new exposure there, but it shouldn't be the reason a token is kept longer than needed.

## To measure before specifying

1. The same probes on macOS.
2. A session started with permission prompts bypassed: is the message held, and what does its user see?
3. `crossSessionInbound` set to `hold` and to `refuse`.
4. A message arriving while the session shows a permission prompt or a question.
5. Whether a subagent's shell sees its parent session's socket.
6. Whether the path changes when a session is resumed or cleared.
7. Whether Claude Code prints the format hint with `--debug`, and whether the documentation gains the format: until it does, the feature rests on an undocumented interface.

## Where it would touch the specs

[04-api](../specs/04-api.md) (a route to register and cancel, the subscription's rules), [05-cli](../specs/05-cli.md) (the command and its exit codes, the guide), [03-state](../specs/03-state.md) (subscriptions beside waiters), [02-polling](../specs/02-polling.md) (hot PRs), [06-operations](../specs/06-operations.md) (idle exit, the token rule) and [07-testing](../specs/07-testing.md) (a fake session socket). It also needs a decision on the non-goal "the socket is the only transport": this is the daemon connecting out to another socket, not a new listener.
