---
id: T06
title: "Config file, environment and paths"
kind: code
milestone: M2
size: M
status: todo
depends_on: [T02]
requirements: [OPS-1, OPS-2, OPS-3, OPS-4:linux, OPS-4:macos, OPS-4:circitor-socket-overrides-the-path, OPS-4:a-path-over-103-bytes]
owns: [src/config/, testdata/config/]
shares: []
---

# T06 · Config file, environment and paths

## Outcome
`config::load` reads and checks `config.toml` with OPS-1's one-line errors, `config::resolve` computes every path of OPS-1, OPS-3 and OPS-4 for Linux and macOS from an `Env`, and `ensure_private_dir` makes a 0700 directory. The daemon and the CLI get their settings and paths from here and nowhere else.

## Context
T02 left `src/config/` with `Config` and its defaults, `Env`, `Platform`, `Paths`, a working `check_socket_path`, and stubs for `load`, `resolve` and `ensure_private_dir`. This task builds the three and takes over the rest. The daemon (T28) loads the config first of all and makes its directories with your function; the CLI (T24, T27) resolves the same paths, checks the config before it replaces a daemon, and runs `daemon --check-config` on `load`.

## Read first
- `plan/conventions.md`: "Platform rules" (platform is a value; the 103 bytes), "Unknown input" (the config file's row).
- `plan/contracts/config.md`: the declarations and what callers assume.
- `specs/06-operations.md#configuration`: OPS-1 and OPS-2's table. `specs/06-operations.md#files`: OPS-3, and OPS-4's first three bullets and its 103-byte bullet.
- `specs/07-testing.md#rules`: TST-1, for `exit_after` in a test build.
- `docs/04-platform.md` §4: socket path lengths and the directories each system has.
- `src/config/` as T02 left it.

## Scope
- **`load` (OPS-1, with both its parts; OPS-2).**
  - **Only a missing file is the defaults.** A file that can't be read (a denied permission, a directory), isn't valid TOML, has a value of the wrong type, an unknown key (also inside `[quota]`, and an unknown table), or a value outside OPS-2's range is a `ConfigError`.
  - **Every range and default of OPS-2 sits in one table** (they're all (proposal)), which the check takes as a value. `exit_after`: 0 is `None`; its lowest other value is 300, and 1 in a test build (TST-1), as a `cfg(feature = "test-endpoint")` constant that feeds the table and appears nowhere else.
  - Numbers are whole seconds: a fraction or a negative number is a wrong type or out of range, naming the key.
  - `gh_path`: an absolute path, or a non-empty name without `/`. `owners`: a list of strings; nothing more is checked.
  - **Then the intervals against each other**, on the values in force once defaults are filled in: `hot_interval ≤ interval ≤ idle_interval`.
  - **The message is one line** naming the file and the key: `<path>: <key>: <what's wrong>`, the key written as in OPS-2 (`quota.reserve`); for the file as a whole, `<path>: <what's wrong>` with the line number for a syntax error. The exact words are yours, fixed by your tests. That `load` exists as a function both sides call is this task's share of "a bad config never costs a working daemon"; the calls are T24's (CLI-9) and T27's (`--check-config`).
- **`resolve` (OPS-1's location; OPS-3 and its two parts; OPS-4's parts `linux`, `macos`, `circitor-socket-overrides-the-path`, `a-path-over-103-bytes`).** It computes and touches no file.
  - An XDG variable that is empty or relative counts as unset (`Env::get` already drops the empty ones).
  - `config_file`: under `XDG_CONFIG_HOME`, else `~/.config`, **on both systems**.
  - `state_dir`: Linux under `XDG_STATE_HOME`, else `~/.local/state`; macOS `~/Library/Application Support/circitor`, whatever the XDG variables say. `state_file`, `lock_file` and `log_file` are in it.
  - `socket`: `CIRCITOR_SOCKET` when set, which must be absolute (else an error naming the variable), with `socket_dir: None`: its directory is used as it is. Otherwise on Linux `$XDG_RUNTIME_DIR/circitor/circitor.sock` with `socket_dir` set, or the state directory when that variable is unset; on macOS always the state directory.
  - **Over 103 bytes is refused**, wherever the path came from, with a message that names `CIRCITOR_SOCKET` as the way out. `check_socket_path` is the same check alone (T17 and T18 call it); T02 built it, and its message is yours to finish.
  - **No home directory is an error only when a path needs it.** (interpretation: the spec doesn't say)
- **`ensure_private_dir` (OPS-3).** Creates the directory, parents included, and sets it to exactly 0700 whatever the umask; sets an existing one owned by the user to 0700; refuses one owned by anyone else, or a path that isn't a directory, naming it and changing nothing. Callers pass `state_dir`, and `socket_dir` when it's `Some` (T28, T24).
- **`testdata/config/`, all of it yours:** `full.toml` (all ten keys of OPS-2 with values that aren't the defaults, inside the ranges, intervals in order), `defaults.toml` (a comment and no key), and one file per kind of error (unknown key, unknown key under `[quota]`, out of range, wrong type, not TOML, intervals out of order).
- `Config::default()`, `Env::from_process`, `Env::get`, `Platform::current` and `check_socket_path` are T02's work and now yours: prove them against OPS-2 and OPS-4, and fix what's off.

## Interfaces
- Consumes: T02's types in `src/config/`. The variants of `ConfigError`, `PathError` and `DirError` are yours; `PathError` already has what `check_socket_path` needs.
- Provides: `load`, `resolve`, `ensure_private_dir`, `check_socket_path`, and the variants of `ConfigError`, `PathError` and `DirError`. Their `Display` is what T27 and T28 print on stderr, and what the log's tail shows after a spawned daemon stops on a bad config (CLI-7).

## Out of scope
- Calling `load` at the daemon's start and stopping on its error: T28. `daemon --check-config`: T27. Checking the config before replacing a daemon: T24.
- The socket itself: whether it answers (T17), its mode, a stale file, binding (T18), the path in `circitor.lock` (T24).
- Reading `CIRCITOR_LOG`, `CIRCITOR_NO_SPAWN` or the test-build variables: T28, T11, T27.
- What each setting does: the tasks OPS-2's last column points at.
- Modes of real files under a running daemon: T36.

## Acceptance
Unit tests in `src/config/`, with files in a `ShortDir` and `Env` values built by hand:
- `ops1_missing_file_is_the_defaults`, and `defaults.toml` loads to `Config::default()`; `full.toml` loads with every one of its ten values, none equal to its default.
- `ops2_defaults`: 30, 10, 300, 600, `Some(3 600)`, 86 400, 600, 500, `gh`, no owners.
- `ops2_ranges`: for every numeric key, the lowest and highest values load and one below and one above are refused, the message naming the key: `interval` 10 and 300 (9, 301); `hot_interval` 5 and 60; `idle_interval` 60 and 3 600; `idle_after` 60 and 86 400; `keep_done` 0 and 604 800; `quota.max_per_hour` 60 and 4 000; `quota.reserve` 0 and 4 000.
- `ops2_exit_after`: 0 is `None`; with the release table 300 loads and 299 is refused; with the test-build table 1 loads; 604 800 loads and 604 801 is refused.
- `ops2_gh_path`: `gh` and `/opt/homebrew/bin/gh` load; `bin/gh`, `./gh` and an empty string are refused.
- `ops1_intervals_are_checked_against_each_other`: `hot_interval = 60` alone is refused (the default `interval` is 30), naming both keys; `interval = 120` with `idle_interval = 60` is refused; all three equal at 60 load.
- `ops1_every_error_is_one_line_naming_it`: each bad file of `testdata/config/` gives a message with no newline that holds the file's path and, where a key is at fault, the key (`quota.reserve` for the nested one; the wrong-type case too). A directory in the file's place, and a file with mode 000 (skipped when running as root), are errors naming the path.
- `ops3_ops4_linux_paths`: with only a home of `/home/u`: `/home/u/.config/circitor/config.toml`, `/home/u/.local/state/circitor/` with `state.json`, `circitor.lock`, `circitor.log` and `circitor.sock` in it, `socket_dir` `None`. With `XDG_RUNTIME_DIR=/run/user/1000`: the socket is `/run/user/1000/circitor/circitor.sock` and `socket_dir` that directory. With the three XDG variables set, each moves its own path.
- `ops3_ops4_macos_paths`: `Platform::MacOs` with home `/Users/u`: the state directory is `/Users/u/Library/Application Support/circitor`, the socket is in it, also with `XDG_RUNTIME_DIR` and `XDG_STATE_HOME` set; the config file still follows `XDG_CONFIG_HOME`.
- `ops4_empty_or_relative_variables_count_as_unset`: `XDG_STATE_HOME=""`, `XDG_STATE_HOME=state`, and the same for the other two.
- `ops4_circitor_socket`: an absolute one is the socket, `socket_dir` `None`, on both platforms; `sock` and `./x.sock` are refused naming `CIRCITOR_SOCKET`; an empty one is unset.
- `ops4_103_bytes`: a `CIRCITOR_SOCKET` of exactly 103 bytes resolves, of 104 is refused; on `Platform::MacOs` a home long enough to push the default socket to 104 bytes is refused, and the message names `CIRCITOR_SOCKET`; a path with a multi-byte character is counted in bytes.
- `ops3_no_home`: an error when a path needs it; none when `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_RUNTIME_DIR` give every path on Linux.
- `ops3_private_dir`: a new directory, with a missing parent, ends at mode 0700; an existing 0755 one becomes 0700; a regular file in its place is an error naming it; `/usr` (owned by root; skipped when running as root) is an error naming it and keeps its mode.
- `cargo test --features test-endpoint --lib config::` and `make verify` pass.

## Traps
- **`toml`'s own error prints several lines** (a snippet with the line and a caret), and a serde type error doesn't always name the key. OPS-1 wants one line naming it. The sure way: parse to a `toml::Table`, then walk the known keys yourself, so every message is yours. `deny_unknown_fields` on a derived struct is fine for unknown keys only if the type errors name their key too: your tests decide.
- **The 300 s floor can't be tested through the feature:** `make verify` runs tests in a test build, where it's 1. That's why the table is a value: test the release table directly.
- **Count bytes, not characters**, and count the whole path: `Application Support` already costs 19.
- **Don't read the process's environment or `HOME` inside `resolve`:** everything comes from `Env`, or `Platform::MacOs` can't be tested on Linux.
- **Never create or chmod the directory of a `CIRCITOR_SOCKET` path** (OPS-3): `socket_dir` is `None` for it, and callers skip it.
- **Check the owner before changing the mode**, and compare with the effective user (`nix`), not with a name.
- `std::fs` is fine here: these run once at start, not on a hot path. No `cfg(target_os)`: both platforms' branches compile and are tested on Linux.
- Tests make their directories with `testkit::ShortDir`, never under `std::env::temp_dir()`.

## Open questions
- **OPS-2's defaults and ranges, and OPS-4's macOS location, are (proposal).** Build them as written, in the one table and in one function per platform. Nothing here was run on a Mac (docs/04 §8); TST-13 checks it there.
- **Whether a missing home is an error when no path needs it** is this brief's reading; list it under Open items.
- D5 and D6 don't touch this task.
