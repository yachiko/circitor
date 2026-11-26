---
id: T64
title: "Owner: record a subagent's environment, and check where its notify lands"
kind: manual
milestone: M5
size: S
status: todo
depends_on: []
requirements: [TST-17, CLI-7:the-cli-says-who-starts-it]
owns: []
shares: []
---

# T64 · Owner: record a subagent's environment, and check where its notify lands

## Outcome
docs/05 has a dated entry, and docs/07's "A subagent's shell" row says what a subagent's environment holds: the same as its session's. NTF-11's subagent sentence and CLI-7's use of `CLAUDE_CODE_SESSION_ID` can then drop their (proposal). After T63 ships, TST-17's by-hand part has been checked once.

## Context
D18 rests on two observations made on 2026-11-26:
- **The owner's field test** on macOS 27.2.1 (Claude Code 2.1.331, `notify-findings.md`): a subagent's confirmation and notice arrived in the main conversation.
- **A comparison on Linux** (Claude Code 2.1.332), made in the session that drafted D18. A subagent's shell had the same `CLAUDE_CODE_MESSAGING_SOCKET`, token, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_PID`, `CLAUDE_CODE_CHILD_SESSION` and parent process (`claude`) as the main session's. Only `CLAUDE_EFFORT` differed, and only because the subagent ran on another model.

Neither is recorded yet. Steps 1 and 2 touch no GitHub. Step 3 posts a comment, so it's a live measurement: `yachiko/experiment` only, docs/05's rules (CLAUDE.md).

## Read first
- `CLAUDE.md`, "Never break": how a fact changes. `docs/05-measurements.md`: "Rules for a new entry".
- `docs/07-notify.md`: "The session socket", the row "A subagent's shell".
- `specs/08-notify.md`: NTF-11. `specs/07-testing.md`: TST-17. `specs/decisions.md`: D18.

## Scope
1. In a main session and in a subagent it spawns, run `env | grep -E '^(CLAUDE|AI_AGENT)' | sed -E 's/(TOKEN=).*/\1<redacted>/' | sort` and `echo $PPID`, and compare. Do it on Linux and on macOS if you can, on the same model.
2. Write the dated docs/05 entry, and update docs/07's row (verified, with the versions). Also record `CLAUDE_CODE_SESSION_ID` in docs/07's socket table: it's set in a session's shell, and it's the same in a subagent's. CLI-7 uses it for the daemon's `start` line (D19, T65).
3. After T63 is in a release: on a PR in `yachiko/experiment`, have a subagent run `circitor notify <pr> --until changed --kind activity`, and post a comment by hand. The confirmation, with `asked here or by a subagent`, and the notice arrive in the main conversation. The subagent's output says it never sees them and uses `wait`.

## Out of scope
- The text circitor prints: T63's.
- Any GitHub request but step 3's one comment.

## Acceptance
- docs/05 has the entry, and docs/07's row is updated.
- Step 3 is recorded in docs/05, or noted here as not done and why.

## Open questions
- If a later Claude Code release gives a subagent its own socket or a variable of its own, D18 can be revisited: `notify` could then refuse with exit 11 or tell the subagent itself.
