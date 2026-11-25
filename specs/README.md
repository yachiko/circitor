# Software spec

**Draft 0.1, 2026-11-24.** This is the specification for `circitor`, a local daemon that watches the user's GitHub pull requests for coding agents. It's built on [direction](../docs/01-direction.md) and the facts in `docs/02` to `docs/04`.

## Files

| File | Contents | Code (proposal) |
|---|---|---|
| [00-overview](00-overview.md) | Goals, user requirements, non-goals, components, repo layout, glossary | – |
| [01-model](01-model.md) | The PR record: fields, checks, the derived status, what's new, revisions and events | `src/model` |
| [02-polling](02-polling.md) | The GitHub queries, finding and refreshing PRs, cadence, quota, errors | `src/github`, `src/poller` |
| [03-state](03-state.md) | The tracked set, retention, the state file, cursors, the single instance | `src/state` |
| [04-api](04-api.md) | The HTTP API over the socket, and what `wait` means | `src/api` |
| [05-cli](05-cli.md) | Commands, output, exit codes, starting the daemon | `src/cli` |
| [06-operations](06-operations.md) | Config, directories, the token, the daemon's life, logging, distribution | `src/daemon`, `src/config` |
| [07-testing](07-testing.md) | The fake GitHub, fixtures, the fake clock, scenarios, acceptance | tests |
| [decisions](decisions.md) | Resolved and open decisions | – |

**Reading order:** first 00, then 01 (every other file uses its field and status names), then the file for the part you're working on.

## Conventions

- **Requirement IDs.** Each file numbers its requirements with its own prefix: MOD, POL, STA, API, CLI, OPS and TST. User requirements are U1, U2, …. Tests and implementation tasks cite these IDs.
  - IDs are never reused. A dropped requirement stays in place, struck through.
- **(proposal)** marks a suggestion waiting for the owner's approval: usually a value or a rule the owner hasn't chosen. **(decision needed)** marks an item listed under open decisions in [decisions](decisions.md).
- **Facts vs behaviour.** What GitHub, the agent clients and the platform do lives in [docs/02](../docs/02-github-api.md), [docs/03](../docs/03-clients.md) and [docs/04](../docs/04-platform.md), each fact with a confidence label. Those docs are the source of truth for facts.
  - These files say what the software does with those facts, and cite them as "docs/02 §4" instead of copying their tables.
  - A requirement resting on a fact labelled `observed`, `documented` or `assumed` says so, and is a (proposal) until the fact is measured.
- **A PR's name** is `owner/repo#number`, as GitHub writes it.
- **Time** in JSON is an RFC 3339 string in UTC (`2026-11-24T22:09:23Z`). Durations and timeouts are whole seconds.
- **Names** in JSON and in CLI output are `snake_case`, lower case; GitHub's upper-case values are lowered (`MERGEABLE` becomes `mergeable`).
- **Size.** Keep each file under about 250 lines, and split it before it grows past that.
