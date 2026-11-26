---
id: T16
title: "State file: the format, atomic writes through a fakeable file system, loading, the saver"
kind: code
milestone: M2
size: M
status: done
depends_on: [T04]
requirements: [STA-6, STA-7, STA-8:a-binary-reads-every-earlier-version, STA-8:a-file-that-isn-t-json, STA-8:a-file-with-a-newer, STA-8:a-missing-file-is-normal, TST-10]
owns: [src/state/file/, testdata/state/]
shares: []
---

# T16 · State file: the format, atomic writes through a fakeable file system, loading, the saver

## Outcome
`state::file` reads `state.json` by STA-8's rules, and its `Saver` writes it by STA-7's: when the content changes, at most once in 5 s, atomically, through a `StateFs` a test can stop after any step. A write cut anywhere leaves the old or the new file, never half of one (TST-10).

## Context
T02 declared `FileContent`, `FileEntry`, `FileSource`, `Flush` and `StateFs`. Every file in `testdata/state/` is this task's, `v1.json` included. T15's store is the real `FileSource`; this task doesn't need it and tests with a small one of its own. T28 calls `load`, starts the `Saver` and calls `shutdown`; T22 calls `Flush::flush` after a `track` or an `untrack`; the harness (T29) hands `FaultFs` to the daemon.

## Read first
- `plan/conventions.md`
- `plan/contracts/state.md`: the shared declarations, "Declarations (`src/state/file/`)", and the saver's lines under "Semantics"; `plan/contracts/clock.md`: timers, `Activity`; `plan/contracts/log.md`: `Warnings`, and this task's row in "Who logs what"
- `specs/03-state.md#the-state-file`: STA-6, STA-7, STA-8
- `specs/07-testing.md#what-is-tested`: TST-10
- `specs/06-operations.md#the-daemons-life`: OPS-7's final write

## Scope
- **The format (STA-6).** `{"version": 1, "prs": [...]}`. `FileContent` has no `version`: the key and the object around `prs` are a private on-disk struct of this module, wrapping `FileContent`. Writing puts `version` 1 first. Reading takes `version` from the raw JSON before any entry is decoded, then decodes `prs` entry by entry. The entries are written in the order the source gives them. Nothing but STA-6's keys can reach the file: the types hold no title, token, check or comment.
- **Writing (STA-7), through `StateFs`, one call per step:** `write_tmp` (`state.json.tmp`, same directory, mode 0600, replacing one left by a crash), `sync_file`, `rename` over `state.json`, `sync_dir`. `RealFs` does each with the real call.
- **The saver.** One task owns every write. It wakes on `file_changed()`, serialises `file_content()`, and writes when the bytes differ from the last ones written.
  - **The 5 s spacing** is on the monotonic clock (POL-26): a change within 5 s of the last write is written 5 s after that write.
  - **A failed write** raises `state_file_unwritable` and is tried again at the next change or 60 s later, whichever comes first; a good one clears it. The task goes on.
  - **Interpretation, the baseline at `spawn`:** the bytes on disk, read through `StateFs::read`. So content that differs from the file (entries dropped at loading, a first run) is written at the first chance, and an unchanged load writes nothing.
  - **`Flush::flush`** asks the task to write now if the content differs, whatever the spacing, and returns when that write succeeded or failed. A caller that goes away doesn't cancel it.
  - **`shutdown`** (OPS-7's share here) always writes once, without the spacing, returns its result and ends the task. T28 logs a failure.
  - **`abort`** ends the task **without** a write, also with a change pending: `Running::crash` calls it (TST-5).
  - **With `writable: false`** nothing is ever written: `flush` returns at once, `shutdown` is `Ok`.
- **Loading (STA-8), `load`:**
  - The part **"a missing file is normal"**: `Content` with no entries, no warning, no log line.
  - The part **"a file that isn't JSON, or has no integer `version`"**: renamed to `state.json.bad` through `StateFs::rename`, `state_file_set_aside` raised with the time in its message, `SetAside`. **An entry that doesn't decode is skipped and logged** by its index, and the rest load: a key missing, an unknown `source`, a node ID given twice (the second is skipped). Read as not decoding too: `source` `asked`, and `ended.state` `open`.
  - The part **"a binary reads every earlier `version`"**: a `version` at or below 1 is read; unknown keys are ignored at every level.
  - The part **"a file with a newer `version`"**: `Newer`, `state_file_newer` raised, the file untouched.
- **`FaultFs`** (test builds): in-memory files that keep "written" and "synced" content apart, for data and for names. `fail_at(step)` makes a step return an error; `stop_after(step)` makes every later call fail, as a dead process; `power_cut()` drops what wasn't synced: a file's unsynced bytes, and a rename whose directory wasn't synced.
- **`testdata/state/`:** write `v1.json`, by hand from STA-6: its example, with an unknown key added at the top and in an entry, and one entry per `source` the file can hold (`authored`, `manual`) and per `ended` (`null`, `merged`, `closed`, `gone`). And the bad files: not JSON, no `version`, `version` 2, an entry with a key missing, a node ID twice.
- **Logging (OPS-9's share, log.md's T16 row):** three `warn` lines of this task's own, by their words: `state file set aside`, `state file newer`, `state file entry skipped` (with the entry's index). **A write failing and working again logs no line here:** it's the `state_file_unwritable` warning raised and cleared, and `Warnings` writes those two lines itself (T04).

## Interfaces
- **Consumes:** `state::{FileContent, FileEntry, FileSource, Flush}`; `clock::{SharedClock, Activity}`; `log::Logger`; `status::{Warnings, WarningCode}`.
- **Provides:** `state::file::{StateFs, RealFs, Loaded, load, SaverDeps, Saver}`, `impl Flush for Saver`, and `FaultFs` with the three controls above. `FaultFs`'s method names are this task's: document them in the module.

## Out of scope
- What the content is, the loaded entry kept unchanged, entries past `keep_done`: T15 (STA-8's other two parts).
- The order `load`, `Store::new`, `Saver::spawn`, and logging a failed final write: T28.
- When a route flushes: T22. The marker sweep of a real daemon's file: T29 (TST-11).

## Acceptance
Unit tests in `src/state/file/`, on `TestClock` and `FaultFs`, with a hand-made `FileSource`; `RealFs` on a `testkit::ShortDir`:
- `tst10_cut_after_each_step`: for a stop after `write_tmp`, `sync_file`, `rename` and `sync_dir`, each followed by `power_cut()`, `load` gives the old content or the new, never anything else. The same without the power cut.
- `tst10_tmp_left_by_a_crash`: a `state.json.tmp` holding rubbish doesn't stop `load` or the next write.
- `sta7_spacing`: a write at t; a change at t + 2 s is in the file at t + 5 s, not before, with one write. A change at t + 9 s is written at once.
- `sta7_failed_write`: `fail_at` each step: `state_file_unwritable` is raised, the old file still loads, and the write is tried again at 60 s or at the next change; when it works the warning is cleared. The task is still running.
- `sta7_flush_ignores_spacing`: a `flush` 1 s after a write writes, and returns after it; with no change it writes nothing. `sta7_mode_0600`: `RealFs` leaves `state.json` with mode 0600.
- `ops7_shutdown_writes`: with no change and no file, `shutdown` leaves a valid file; with `fail_at` it returns `Err`. `abort_writes_nothing`: `abort` with a change pending leaves the file as it was, and no later change is written.
- `sta8_missing`, `sta8_not_json`, `sta8_no_version`: the last two leave `state.json.bad` (replacing an earlier one), no `state.json`, and the warning. `sta8_newer`: `Newer`, the file's bytes unchanged, and a saver with `writable: false` never calls `write_tmp`, also at `shutdown`.
- `sta8_bad_entries`: the file with a missing key and the one with a duplicate ID load every other entry, with one `state file entry skipped` line per skipped entry. `ops9_lines`: `sta8_not_json` logs `state file set aside` and `sta8_newer` logs `state file newer`; a failed write and the next good one log only `warning raised code=state_file_unwritable` and `warning cleared code=state_file_unwritable`.
- `tst10_every_version_loads`: each `testdata/state/v*.json` loads with every entry, its unknown keys ignored; `v1.json` gives both sources and all four `ended` values, and STA-6's own example entry field for field. Written back and loaded again, the content is the same.
- `sta6_no_title_no_token`: the bytes written for a content of every shape hold no `title` key and no `testkit::MARKER_TOKEN`; a written file decodes into the same `FileContent`.
- `cargo test --features test-endpoint --lib state::file::` and `make verify` pass.

## Traps
- **`rename` is atomic only inside one directory:** the temporary file is `state.json.tmp` beside the target, never in a temp directory.
- **No subscription is ever written** (STA-6, NTF-9): `FileContent` has no field for one, and none is to be added. A session's socket path or token in the file is a bug in whoever put it there.
- **Sync before the rename.** Without it a power cut leaves an empty `state.json`. `FaultFs` must be faithful enough to fail that test.
- `StateFs` is blocking: the saver calls it inside `spawn_blocking`, and takes `activity.begin()` when it wakes (conventions).
- All of the saver's waiting is on the clock: `clock.sleep_until` with absolute times, never `tokio::time`.
- A log line names an entry by its index: a bad entry's content can be anything.
- Don't set `deny_unknown_fields` on anything read from the file (STA-8).

## Open questions
- **A `state.json` that exists and can't be read** (a denied permission): STA-8 doesn't say. Build: one `error` line, `Content` with no entries. Report it.
- **A version-1 file whose `prs` is missing or isn't a list:** STA-8 sets aside only the two cases above. Build: no entries, one `warn` line. Report it.
- STA-7's 5 s and 60 s aren't marked (proposal); keep each in one constant all the same.

## Evidence
- **2026-11-26**, branch `t16-state-file`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- Changed: `src/state/file/` (`mod.rs`, new `load.rs`, `saver.rs`, `fault.rs`, `tests.rs`) and six files in `testdata/state/`. `load`, `Loaded`, `Saver`, `SaverDeps` moved to their own files and are re-exported from `state::file` with the contract's signatures; added `state::file::{VERSION, Step}` and `FaultFs`'s controls (`put`, `file`, `fail_at`, `heal`, `stop_after`, `revive`, `power_cut`, `calls`). `cargo test --features test-endpoint --lib state::file::`: 17 passed. `make verify` passes on main after the merge (370 lib tests), the macOS check included.
- Built as the brief's open questions say: an unreadable `state.json` is one `error` line and no entries; a version-1 file without a list of `prs` is one `warn` line and no entries. Also: an entry needs all six keys (`login` and `ended` may be `null`); a negative `version` reads as an earlier one; `shutdown` raises `state_file_unwritable` when its write fails; the saver ignores `SaverDeps.logger` (log.md gives it no line).
- **Open:** the `state_file_set_aside` warning's message holds no time (`load` has no clock); the time is in the log line and in the warning's `since`.

Follow-ups: T54 (found by T36: the saver spins on a moving clock and never writes while running).
