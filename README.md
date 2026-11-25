# circitor

A small local daemon that watches your GitHub pull requests, so coding agents can ask it instead of GitHub. One poller serves every agent session, which keeps them off your API quota.

**Status:** facts gathered and the specs drafted (draft 0.1, not yet hardened). Nothing is built.

| Folder | Holds |
|---|---|
| [`docs/`](docs/01-direction.md) | What's decided, and the measured facts about GitHub, the agent clients and the platform |
| [`specs/`](specs/README.md) | Behaviour, as numbered requirements |
| `plan/` | Contracts and task briefs (not written yet) |
| `.claude/skills/` | `harden`, `build-plan` and `add-feature`: how the specs and the plan are made |

Start with [direction](docs/01-direction.md).
