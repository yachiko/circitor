# circitor

A small local daemon that watches your GitHub pull requests, so coding agents can ask it instead of GitHub. One poller serves every agent session, which keeps them off your API quota.

**Status:** facts gathered, the specs drafted and hardened (draft 0.2), and the implementation plan written. Nothing is built.

| Folder | Holds |
|---|---|
| [`docs/`](docs/01-direction.md) | What's decided, and the measured facts about GitHub, the agent clients and the platform |
| [`specs/`](specs/README.md) | Behaviour, as numbered requirements |
| [`plan/`](plan/README.md) | Contracts and task briefs for building it |
| `.claude/skills/` | `harden`, `build-plan` and `add-feature`: how the specs and the plan are made |

Start with [direction](docs/01-direction.md).

## Building

`make verify` formats, lints, tests, and builds for macOS. **From Linux, the macOS build needs zig and cargo-zigbuild:**

```sh
sudo pacman -S zig    # or your system's package, or ziglang.org
cargo install cargo-zigbuild --locked
```

On a Mac, plain cargo builds for macOS, so neither is needed.
