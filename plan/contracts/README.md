# Contracts

What parallel tasks agree on before any of them is built. T02 turns every declaration here into code; from then on the code is the declaration and these files are its semantics. **A change to a seam goes through the orchestrator**, never through an executor's diff.

| Seam | Path | Declared by | Built by | Used by | File |
|---|---|---|---|---|---|
| Clock, work in flight, sleep detection | `src/clock/` | T02 | T04 | every task | [clock](clock.md) |
| Logging and warnings | `src/log/`, `src/status/` | T02 | T04 | every daemon-side task | [log](log.md) |
| The PR model and the merge | `src/model/` | T02 | T05 | T07, T09, T10, T15, T22, T23, T27 | [model](model.md) |
| The GitHub client; the token source | `src/github/`, `src/token/` | T02 | T07; T12 | T13, T21, T25, T26, T28, T29 | [github](github.md) |
| Config, environment, paths | `src/config/` | T02 | T06 | T08, T09, T15, T18, T21, T24, T27, T28 | [config](config.md) |
| The state store; the state file | `src/state/` | T02 | T15; T16 | T21, T22, T23, T25, T28, T46, T47 | [state](state.md) |
| The poller, outside and inside | `src/poller/` | T02 | T08, T09, T13, T21, T25 | T22, T23, T26, T28 | [poller](poller.md) |
| API types, the server's shared parts, and a condition's one evaluation | `src/api/` | T02 | T43 (shared parts), T44 (conditions), T22, T23, T47 | T10, T17, T24, T27, T28, T46, T48 | [api](api.md) |
| Transport | `src/transport/` | T02 | T17 | T18, T24, T27, T28, T29 | [transport](transport.md) |
| The CLI | `src/cli/` | T02 | T10, T11, T24, T27, T48 | T29, T36, T46 | [cli](cli.md) |
| The daemon | `src/daemon/` | T02 | T18, T28 | T22, T24, T27, T29, T36 | [daemon](daemon.md) |
| Notify: subscriptions, the poster, the notices' words | `src/notify/` | T02 | T46 | T28, T47, T48, T29, T49 | [notify](notify.md) |
| The test kit | `src/testkit/` | T02 | T14, T20, T45, T29 | T26, T29, T30 to T36, T49 | [testkit](testkit.md) |

## Formats the specs define

Cited, not redefined.

| Format | Defined in | Golden files |
|---|---|---|
| The record, events and items as JSON | 01-model MOD-2, MOD-12, MOD-15; 04-api API-19 | `testdata/api/`, `testdata/model/` (T41) |
| Routes, parameters, status codes, the error body | 04-api API-1 to API-19; 08-notify NTF-2, NTF-3 | `testdata/api/` (T41); `api::types::Request` |
| CLI text, outcome lines, exit codes | 05-cli CLI-3, CLI-5, CLI-6; 08-notify NTF-11 | `tests/golden/render/` (T10), `tests/golden/notify_cmd/` (T48), `tests/golden/cli/` (T35) |
| A notice's line on the session socket, and its words | 08-notify NTF-6 to NTF-8; docs/07 | `tests/golden/notify/` (T46), `tests/golden/cli/` (T35, as posted) |
| `state.json`, version 1 | 03-state STA-6 | `testdata/state/` (T16) |
| `config.toml` | 06-operations OPS-1, OPS-2 | `testdata/config/` (T06) |
| The GraphQL queries | 02-polling POL-4, POL-6, POL-8, POL-12, POL-16 | `testdata/github/schema.graphql` (T41), `tests/cost/` (T14) |
| GitHub's responses | docs/02 §4, §5; `docs/measurements/2026-11-24/` | `testdata/github/responses/` (T07, synthetic), `testdata/github/recorded/` (T37, real) |
| The lock file's line; the socket path | 03-state STA-11; 06-operations OPS-4 | – (`daemon::instance`, `config::resolve`) |
| The LaunchAgent plist and the systemd unit | 06-operations OPS-18 | `packaging/` (T19) |

## Rules for every seam

- **Declarations are complete enough to copy.** Where a contract shows `/* … */`, the inside is the building task's; the name and the signature aren't.
- **A parent `mod.rs` is T02's and final.** It declares its submodules and re-exports what the contract names. A building task adds files inside its leaf directory.
- **Where an inside lives.** A type or function a contract shows under a parent module but whose inside is a later task's (`daemon::Running`, `daemon::StartError`, `cli::run`, `cli::main`, `CliEnv::real`) is defined in that task's leaf directory and re-exported by the parent, or its inherent `impl` block sits in the leaf.
- **Stubs.** Where a function's body is another task's, T02 writes `unimplemented!("T15")` with the task's ID. A task that finds such a stub in a path it doesn't own has a missing dependency: report it.
- **Fakes live beside their trait** (`github::fake`, `token::fake`, `poller::fake`, `cli::fake`, `notify::fake`), compiled under `cfg(any(test, feature = "test-endpoint"))`. T02 declares them; **T43 builds them**, so a task whose tests use a fake depends on T43. The one exception is `notify::fake::RecordingPoster`: it sits in T46's leaf, T46 builds it, and only tasks after T46 use it.
