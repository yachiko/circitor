---
id: T02
title: "Contracts as code: the module tree, every seam's declarations, and the small shared types"
kind: infra
milestone: M1
size: L
status: done
depends_on: [T01]
requirements: [API-19, CLI-13, NTF-9]
owns: [src/]
shares: []
---

# T02 · Contracts as code: the module tree, every seam's declarations, and the small shared types

## Outcome
Every declaration in `plan/contracts/` compiles as Rust in its final module, with `unimplemented!()` where a later task builds the behaviour; the small shared types are real (`Time`, `Mono`, `Activity`, `PrName`, `Cursor`, `NoFlush`, `NoNotify`, `api::types` and the rest of the list below); and `testkit::golden` is there for every task that keeps golden files (the API's and the model's own are T41's). After this, tasks build against the seams, not against each other.

## Context
T01 left a package whose library holds only `version`. This task turns `plan/contracts/` into code: the whole module tree, every declaration, and the small real parts everything shares. The API's shared parts and the simple fakes are declared here and built straight after by T43. **From here on up to thirty tasks build at the same time, each inside one leaf directory, against what you write.** A signature you get wrong is built wrong thirty times; a parent `mod.rs` you leave unfinished can't be fixed by anyone, because nobody else owns it. The golden JSON files and GitHub's schema are T41's, which checks your types against the specs' JSON.

## Read first
- `plan/conventions.md`, all of it: "Layout" is the tree you create.
- `plan/contracts/README.md`, then **every file in `plan/contracts/`, as they stand when you start. They are the declarations**: this brief names none of them one by one, and where it seems to differ from a contract file, the contract wins. Each file's "Semantics" and "Don't assume" say what the real parts must do; its "The fake" section names what you declare and T43 builds.
- `specs/04-api.md#basics` (API-2 to API-5, API-19: the envelope, the error body, each route's keys); `specs/01-model.md` (MOD-1, MOD-2, MOD-12, MOD-15) for the record, items and events as JSON; `specs/03-state.md` (STA-6's layout, STA-9's cursor); `specs/06-operations.md#configuration` (OPS-2's defaults); `specs/05-cli.md` (CLI-5's exit codes, CLI-13); `specs/08-notify.md` (NTF-2's body and answer, NTF-9's token).
- `specs/README.md`, "Conventions": times, durations and names in JSON.

## Scope
- **The tree.** `src/lib.rs` declares every module of conventions' Layout; `testkit` only under `cfg(any(test, feature = "test-endpoint"))`. `src/main.rs` in its final form: `std::process::exit(circitor::cli::main())`.
- **Where a declaration lives** (`plan/contracts/README.md`, "Rules for every seam": this is the rule that makes the plan work):
  - A **parent** module (`github`, `token`, `state`, `poller`, `api`, `cli`, `daemon`, `testkit`) has a `mod.rs` that is **yours and final**: it holds the declarations the contract puts there whose insides are complete, declares its submodules, and re-exports.
  - A **leaf** directory (`clock`, `log`, `status`, `model`, `config`, `transport`, `notify`, and every subdirectory a later task owns: `api/notify`, `cli/notify` and `testkit/session` among them) gets one `mod.rs` from you with that leaf's declarations, which its task then owns and fills. `src/api/condition.rs` is a leaf of one file (T44's).
  - **Anything whose inside is a later task's goes in that task's leaf**, whatever path the contract shows it at, and the parent re-exports it: `daemon::Running` and `daemon::StartError` in `src/daemon/run/mod.rs`; `CliEnv::real` as an `impl CliEnv` block in `src/cli/commands/`; `cli::run` and `cli::main` calling `commands::run` and `commands::main`. Check every `/* … */` and every "(Tnn)" in the contracts against this rule: the owner can't touch your `mod.rs`.
- **Copy every declaration**: names, derives, serde attributes, signatures, constants with their values, field order. Where a contract shows `/* … */` for a struct's inside, write a private placeholder field.
- **Stubs.** A function another task builds is `unimplemented!("Tnn")` with that task's ID, taken from the contract (its "Producer" line, or the task named beside the declaration). The one exception is `poller::cadence::stale`, which returns `false` until T09 (`plan/contracts/api.md`). `server::routes()`, `wait::routes()` and `notify::routes()` return an empty `Router<AppState>`. `cli::guide::GUIDE` is an empty string. Start each stubbed leaf file with `#![allow(unused_variables, dead_code)] // stub: Tnn removes this`.
  - **T43's files hold only stubs:** `src/api/request.rs` (the `impl Request` block of `method`, `path_and_query` and `body`), `params.rs`, `respond.rs`, `router.rs` (with `router`, `InFlight`'s methods and `test_state`), and `src/github/fake.rs`, `src/token/fake.rs`, `src/poller/fake.rs` and `src/cli/fake.rs`, the fakes under `cfg(any(test, feature = "test-endpoint"))`. `src/api/mod.rs` re-exports from them what api.md shows under `api::`.
- **Real, built and tested here** (small, shared, with no product timing). Each contract's "Producer" line and comments say what's yours; in short:
  - `clock`: `Time` and `Mono` with their arithmetic, `Time`'s serde and `Display`, `clock::sleep`, `clock::timeout`, and `Activity` whole: `Busy`, and `excuse` with `Excused` in test builds (`in_flight()` leaves out what's excused). T43's `ScriptedGitHub::hold` needs it; T04 takes it over. `SystemClock`, `TestClock` and `SleepWatch` are stubs for T04. `Logger`, the sinks and `Warnings` too, unless a minimal real one is less work for your own tests: T04 owns them afterwards either way.
  - `model`: every type with its serde; `PrName` (parsing, `from_url`, `slug`, `Display`, serde, and `Eq`, `Hash`, `Ord` as the contract says: plan decision P10 for the order); `Mergeable` and `Review` with `from_github`; `NodeId`; **`short_sha`** (T10 calls it before T05 is merged).
  - `state`: the limits' constants (`MAX_SUBSCRIPTIONS` among them), `Limit`, `Cursor` with `Display` and `FromStr` (STA-9's form exactly), the file types, `FileSource`, `Flush`, `NoFlush`, and **`View`'s methods** (`cursor`, `too_old`, `find`, `get`, `events_after`: pure over its public fields, as state.md says), with a test of `too_old`'s three cases (another epoch; a `rev` above the counter; history that no longer starts at revision 1 while the cursor is below its first event).
  - `token`: `Token`, `TokenSourceKind` with `as_str`, `TokenRead`, `GhFailure`, the trait. `github`: every type and the trait.
  - `config`: `Config` and `QuotaConfig` with `Default` (OPS-2's defaults), `Env` with `from_process` and `get`, `Platform::current`, `Paths`, and **`check_socket_path` for real** (T17 and T18 call it before T06 is merged).
  - `poller`: every type of "Outside", `QuotaState`, `Policy`. `daemon`: `Mode`, `StopCause`, `StopHandle`, `Deps`, and `instance`'s constants. `cli`: `exit` (with `NO_SESSION`), the constants, `Output`, `CliEnv`, the traits, `Branch`, `Interrupt` and its trigger; `cli::notify`'s two variable names. `testkit`: the markers (the session's, the check's and the login's among them) and `TEST_VERSION`, `ShortDir`, and `golden`; `testkit::session`'s `Line` and `Notice`.
  - `model`: also `EventKind::word`, the `kind` tag of each variant.
  - `notify` (**NTF-9's share: the token's type**): the constants, `SocketId`, `Looked`, `Posted`, `ProblemWhy` and `End` with `as_str`, `NewSubscription`, `Registered`, `RegisterError`, `NotifyDeps`, the two traits, `text::Told`, and **`SessionToken` whole** (no `Display`, no `Serialize`, a `Debug` that prints `SessionToken(…)`, `Deserialize` from a string) and **`NoNotify` whole** (`look` `Looked::Failed`, `register` `Err(Stopping)`, `drop_same` `false`, `cancel` and `count` 0). `Notifier`, `RealPoster`, `text`'s functions and `fake::RecordingPoster` are stubs for T46.
  - `api`: `types.rs` and `mod.rs` whole (below). The rest of `src/api/` is T43's stubs, `condition.rs` (T44's: its types real, `check` and `evaluate` stubs) and the three route stubs.
- **The API's types.** `types.rs` (**API-19: this task's share is the body types, `NotifyBody` and `CancelBody` among them; the golden files that prove them against the spec are T41's**; also API-2 to API-5's envelope and error body), with `ErrorCode::http()`, `Until::as_str` and `parse`, `Ask::new` and `NotifyBody::as_wait`. `mod.rs`: `AppState`, `DaemonInfo`, the submodules and the re-exports. API-1's strict parser, the `unknown_route` fallback and `Request`'s spelling of the routes are **T43's**.
- **The fakes are declared, not built:** their types and signatures as each contract's fake section gives them, bodies `unimplemented!("T43")`. `state::NoFlush` is real. **CLI-13's share is `CliEnv` and the traits** (`Spawner`, `Spawned`, `Git`, `Interrupt`): one interface per input, so a command can be driven without the process's clock, directory, environment or `exec`. The fakes behind them are T43's; that commands read nothing else is T11's, T24's and T27's to keep.
- **`testkit::golden::check` and `compare`** as testkit.md declares them (`compare` never writes). Build them over an inner function that takes the path and the update switch as arguments, so it can be tested without changing the process's environment.
- **Where a contract is still silent**, write the smallest thing that compiles and fits its Semantics, and list it under "Contract changes needed". Don't settle a question of behaviour that way.

## Interfaces
- Consumes: `version::version` and the dependency set (T01). Nothing else.
- Provides: every seam in `plan/contracts/README.md`'s table, as code. After this task the code is the declaration, and a change to it goes through the orchestrator.

## Out of scope
- Any behaviour behind a stub: the task named in it. Don't build "just a little" of a neighbour: its owner will overwrite it.
- **T44:** `Condition::check` and `condition::evaluate`. **T46:** the notifier, the poster, the notices' words and `RecordingPoster`. **T47, T48, T45:** the notify routes, the `notify` command, the fake session socket.
- **Everything under `testdata/`**: the API's and the model's golden files and `schema.graphql` are T41's, `testdata/config/` T06's, `testdata/state/` T16's, GitHub's responses T07's and T37's. Your tests build their values in code.
- `src/version.rs`, `Cargo.toml`, the Makefile: T01. If you need a crate or a feature, report it.
- **T43:** `Request`'s three methods, `Params`, `respond`, `router` with its layer and fallbacks, `InFlight`, `api::test_state`, and the five fakes (`ScriptedGitHub` with `answers`, `FakeTokenSource`, `FakePoller`, `FakeSpawner`, `FakeGit`). Testing them through `router()`: T22.
- `FaultFs` (T16), `FakeGitHub` (T20), `Harness` (T29): their leaf files hold only what the contracts declare in code blocks.

## Acceptance
Unit tests beside the code. Where one needs a clock, a ten-line `Clock` inside the test module is enough.
- `api19_every_key_is_present_when_empty`: each body type built with empty lists, `false` flags and `None`, inside an `Envelope`, serialises with `version`, `cursor`, `daemon` and then every key of API-19's row, in that order; `ShutdownBody` adds none; `checks.items` and an item's review-only keys are the only keys ever left out.
- `api19_bodies_survive_a_round_trip`: one value of each body and of `ErrorBody`, built in code with every field filled but `Event.id` (an event of each of MOD-15's fourteen kinds among them) and every time in whole seconds, serialises and decodes back equal; and decodes with an unknown key added. `Event.id` is `#[serde(skip, default)]`: a filled one comes back empty.
- `api3_http_status_per_code`: 400, 404, 404, 503, 503, 503, 503, 409, and 409 for `no_session`.
- `ntf2_notify_body`: `NotifyBody` serialises its keys in API-19's order, `registered`, `replaced`, `expires_at` first, with `expires_at`, `outcome` and `reason` written as `null`; `as_wait` is `None` when `registered` and the same outcome, record and events otherwise. `CancelBody` is `{"cancelled":2}`.
- `ntf9_session_token_debug_holds_no_value`: `{:?}` of a `SessionToken` made from `MARKER_SESSION_TOKEN`, and of a `NewSubscription` holding one, doesn't hold it; it deserialises from a JSON string. `no_notify_does_nothing`: `NoNotify`'s five answers.
- `mod15_kind_words`: `EventKind::word` gives the fourteen `kind` tags, each equal to the tag its JSON carries.
- `until_words`: `merged`, `checks-done` and `changed` through `as_str` and back through `parse`; `checks_done` and `Merged` give `None`. `Ask::new(Until::Merged)` has everything else unset.
- `sta9_cursor_form`: `k3x9a2bq-42` and `k3x9a2bq-0` parse and print back; refused: an epoch of 7 or 9 characters, an upper-case letter, no `-`, no digits, a sign, a `rev` beyond `u64`.
- `mod1_pr_name`: `owner/repo#12` and `https://github.com/owner/repo/pull/12`, also with `/files`, a query and a fragment; refused: `owner/repo`, `#12`, `owner/repo#x`, another host, `/issues/12`. `Yachiko/Circitor#12` equals and hashes as `yachiko/circitor#12` and prints as written. Order: `a/b#9`, `a/b#10`, `a/c#1`, `b/a#1`.
- `short_sha_is_seven_characters`: `short_sha("0123456789abcdef")` is `0123456`; a shorter string comes back whole.
- `mod2_mergeable_and_review`: `MERGEABLE`, `CONFLICTING`, `UNKNOWN` and `FOO` give `yes`, `no`, `unknown` and `foo`; a `null` review decision gives `none`; any string decodes.
- `time_json_is_whole_seconds_utc`: `2026-11-24T22:09:23Z` out; an offset and fractions accepted in; `since` gives zero for a later time.
- `clock_timeout_and_sleep`: on the test module's clock, `timeout` gives `Err(Elapsed)` when its timer fires first and the value otherwise. `activity_counts_and_excuses`: two `Busy` and one `Excused` read 1; dropping each moves the count back.
- `ops2_defaults` (30, 10, 300, 600, `Some(3 600)`, 86 400, 600, 500, `gh`, no owners), `ops4_an_empty_variable_is_unset`, `ops4_check_socket_path_103_bytes` (103 passes, 104 is refused).
- `ops5_token_debug_holds_no_value`: `{:?}` of a `Token` made from `MARKER_TOKEN` is `Token(…)`.
- `stop_handle_first_cause_wins_and_counts`; `interrupt_never_pends_and_a_pair_fires` (130, 143); `short_dir_is_short_and_removed` (under `/tmp`, gone after drop).
- `golden_check`: equal text passes; different text panics with both texts in the message; with the switch on, `check` writes the file and nothing panics, and `compare` still panics and writes nothing.
- `make verify` passes with every stub in place. `cargo build --release` passes: nothing of the test kit or the fakes is needed without the feature.
- Your report lists `grep -rn 'unimplemented!("T' src | wc -l` and the count per task ID (T43's among them), and every place you had to choose because a contract was silent.

## Traps
- **`make verify` lints twice, with and without the feature.** A helper used only by a fake or the test kit needs the same `cfg`, or the second run fails on dead code. An unused parameter in a stub is a warning, hence an error: that's what the `#![allow]` line is for.
- **Clippy will object to some declarations as written.** `large_enum_variant` on `model::Merged`: keep its shape, add an `#[allow]` with a one-line reason. `new_without_default` on a `new()` without arguments (`StopHandle`, `Backoff`, `ShortDir`): add a `Default` beside it. Don't box or reshape a contract type on your own; report each allowance.
- **The shared response types must not `deny_unknown_fields`**, and must not skip a key: a CLI reads a newer daemon (CLI-9), and API-19 forbids a missing list or flag. Only the server's own request bodies are strict.
- **Don't glob-import `EventKind`:** four of its variants share names with model types.
- **`Token` has no `Display` and no `Serialize`.** Don't derive `Debug` on it; a type that holds one (`TokenRead`) may derive it.
- **`unsafe` is denied**, and edition 2024 makes `std::env::set_var` unsafe: no test changes the process's environment. That's what `Env` is for, and why `golden::check` has an inner function.
- **`ShortDir` goes under `/tmp`**, never `std::env::temp_dir()`: macOS's is long (OPS-4).
- **A fake's declaration is its signature, closures included:** a stored closure is `Send + 'static` (and `Sync` where cli.md says so), or T43 can't put the fake behind its trait.
- **Don't write a file under `testdata/`, and don't read one in a test:** T41's and T06's files don't exist yet when you build.
- After this task the binary panics on every command until T27: `cli::main` ends in a stub. That's expected.

## Open questions
- **API-19's names, MOD-15's `kinds` as tagged objects, `gh_path` in `status`, `log_unwritable`, and the order of `pr`** are (proposal). Build them as the contracts write them. If a type can't carry the JSON a spec writes, that's a contract change to report, here or when T41's test finds it.
- **The contracts may still move while you build.** Take them as they stand at your start; a later change reaches you through the orchestrator, not by re-reading.
- D5 doesn't touch this task. **D6:** `Status::Blocked` exists as written; nothing here reads a check's `required`.

## Evidence
- **2026-11-26**, branch `t02-contracts` (commit ab3e947, rebased onto T50's 5aa9128), executor on **Opus, medium**; reviewed by the orchestrator, with four read-only Opus reviewers comparing every code block of `plan/contracts/` against the code (one slice each: api/transport/testkit; model/github/log/clock; state/poller/config; cli/daemon/notify). **No declaration was found wrong.**
- In the worktree: `make verify` passes (T01's Makefile, without check-macos), 36 lib tests; `cargo test --lib` without the feature passes; `cargo build --release` passes. Every test the brief names exists, and `sta9_too_old` besides.
- On main after the merge: fmt, both clippy runs and `cargo test --features test-endpoint` (36) pass; **`check-macos` stops with `check-macos needs zig`**: zig isn't installed system-wide yet (T50). To rerun once it is.
- Stubs: `grep -rn 'unimplemented!("T' src | wc -l` is 325: T04 35, T05 4, T06 3, T07 4, T08 22, T09 11, T10 20, T11 4, T12 4, T13 9, T14 7, T15 30, T16 10, T17 7, T18 9, T21 11, T24 9, T27 7, T28 9, T43 69, T44 2, T45 10, T46 28, T48 1. `StartError::code` is `match *self {}` over an empty enum, not a stub.
- **Choices where a contract is silent** (accepted; each sits in its builder's leaf): `EndpointError`, `ConfigError`, `DirError`, `StartError` are empty enums (every user depends on the filler); `PathError` has `RelativeSocket`, `TooLong`, `NoHome`, and `config::MAX_SOCKET_PATH` is 103; `TimeError`, `NameError`, `CursorError` are unit structs; `ErrorCode::http()` returns `http::StatusCode`; `NotifyBody::as_wait` is also `None` without an outcome; `NoNotify::look` is `Looked::Failed("no notifier")`; `InterruptTrigger` is `Clone`, first code wins; `PrName` takes `[A-Za-z0-9._-]+` and a number ≥ 1, `from_url` only `https://github.com/` (host case-blind); `Cursor` accepts leading zeros in `rev`; `Time` clamps at chrono's range; the fakes' constructors return `Arc`, `ScriptedGitHub::hold(&Activity)`, `FakeTokenSource::set_kind`/`set_gh_path` (T13 writes its own source regardless), `FakePoller`'s counters; `testkit::github::schema()` returns `&'static apollo_compiler::Schema`; `WarningCode` has no `Ord` (T04 sorts `list`); `NewSubscription` derives `Debug`.
- Clippy allowances: `large_enum_variant` on `model::Merged`; `ptr_arg` on `conn::ensure`, `conn::existing`, `conn::stop`, `commands::find_bare`. `Default` beside `new()` for `StopHandle`, `Backoff`, `ShortDir`.
