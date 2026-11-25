# circitor

A local Rust daemon that polls GitHub for the user's pull requests and serves their state to coding agents through a CLI. Direction and decisions are in `docs/01-direction.md`; facts about GitHub, the clients and the platform are in `docs/02` to `docs/04`.

## Never break

- Read-only towards GitHub: queries, never mutations.
- The token is never logged, stored or printed.
- Live GitHub measurements use `yachiko/experiment` only, follow the rules in `docs/05-measurements.md`, and need the owner's OK.
- A fact in `docs/02` to `docs/04` changes only with a new dated entry in `docs/05-measurements.md` or a source in `docs/references.md`. Never edit a fact to fit a spec.

## Working

- The order is docs, then specs, then plan. A feature enters at the specs (`/add-feature`), a spec is reviewed with `/harden`, and the plan is built with `/build-plan`.
- Docs and specs use short sentences, British spelling, and bold for the key phrase.

## Building

- Every task follows `plan/conventions.md`.
- `make verify` must pass before a change is done.
