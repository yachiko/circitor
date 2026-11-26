---
id: T38
title: "The first CI run: Linux on a pull request, Linux and macOS at the first tag"
kind: manual
milestone: M4
size: S
status: in-progress
depends_on: [T03]
requirements: [TST-14]
owns: []
shares: []
---

# T38 · The first CI run: Linux on a pull request, Linux and macOS at the first tag


> **Rescoped (2026-11-26, the owner):** CI runs Linux only on a pull request; macOS runs at a release tag, in `release.yml`'s `verify` job (T03's Evidence). So **the first run** is `ci.yml` on the pull request that adds the workflows, and **the run that counts** for macOS is `release.yml` at `v0.0.1` (or its dry run by hand, once the workflow is on `main`). The repository is public already. Where the brief below says `verify` runs on macOS on every push, read "at a tag".
## Outcome
The repository is on GitHub, the workflow of T03 has run green on a Linux and a macOS runner, and whatever macOS showed that Linux couldn't is fixed or filed.

## Context
T03 wrote the CI workflow and checked its syntax; it has never run, because the repository has no remote. The development machine is Linux, with no Mac at hand. `make check-macos` builds for macOS from there through cargo-zigbuild (T50), but nothing has run on macOS. So **the first run on a macOS runner is the first time the code runs on macOS at all.** This task is **the owner's**: put the repository on GitHub, run the workflow, and turn what macOS shows into fixes or follow-up tasks.

## Read first
- `.github/workflows/` (T03): the jobs, and which commands each runs.
- `plan/conventions.md`, "Toolchain and commands" and "Platform rules": what `make verify` runs, and what's expected to differ on macOS.
- `specs/07-testing.md`: TST-14, TST-9 (the real-process tests run "on the CI's Linux and macOS runners"), TST-1.
- `specs/06-operations.md`: OPS-13 (the four targets).
- `docs/04-platform.md` §1, §4 and §8: nothing there was checked on a Mac.

## Scope
**TST-14 is shared with T03.** T03's share: the workflow as a file. **This task's share: its first real runs**, green on a Linux and a macOS runner, and the list of what macOS showed. The CI service is a (proposal) in TST-14; the plan built GitHub Actions (plan decision P9).

- **Create the remote** and push `main`. Whether the repository is public or private is the owner's choice; it changes what macOS minutes cost and what T39 can release (see Traps).
- **The first run, as soon as T03 is merged:** it proves the workflow starts, the toolchain installs, and whatever exists by then compiles and passes on macOS. Early is the point: a macOS compile error found at T05 is cheap.
- **The run that counts: on a commit that holds T36.** TST-9's tests must pass on the macOS runner. This task isn't done before that run.
- **For every red job:** read the log, and sort the cause: the workflow itself (fix it in `.github/workflows/`, a follow-up to T03), a test that assumes Linux (a follow-up to the task that owns the test), or production code that's wrong on macOS (a follow-up to the task that owns the code). File each through `/build-plan`'s update; don't patch on `main` by hand.
- **Record** the run's URL, commit, each job's result and duration, and the minutes used.

## Interfaces
- **Consumes:** T03's workflow; `make verify`; for the run that counts, T36's `tests/real/`.
- **Provides:** a remote (`origin`) and a CI that T39's release and every later merge rely on.

## Out of scope
- Writing or restructuring the workflow: T03, or a follow-up to it.
- Releases, tags, the tap, any secret: T39. This workflow needs no secret and no token.
- Fixing macOS failures in `src/` or `tests/`: follow-up tasks, each in the owner task's paths.
- The checks on the owner's Mac (Homebrew, launchd, the Keychain, a lid close): T40. A hosted runner proves none of them.

## Acceptance
The owner ticks each line.

- [ ] **Decided: public or private.** Written down, with the reason.
- [ ] **The work tree is committed and clean:** `git status --short` prints nothing, and `git log --oneline | head` shows the merged tasks.
- [ ] **No secret is in the history:** `git grep -nE 'gh[ops]_[A-Za-z0-9]{20,}' $(git rev-list --all) -- . | grep -vE 'CIRCITOR_TEST_MARKER|gho_0{20,}'` prints nothing (the two exclusions are the tests' marker and T37's made-up token).
- [ ] **Remote created and pushed:** `gh repo create yachiko/circitor --private --source . --remote origin` (or `--public`), then `git push -u origin main`.
- [ ] **The run started by itself:** `gh run list --limit 5` shows it; `gh run watch` follows it.
- [ ] **First run:** every job is green, or each red job has a follow-up task with the failing step's last lines (`gh run view <id> --log-failed`).
- [ ] **`permissions` is `contents: read` and no secret is used:** `grep -n 'secrets\.' .github/workflows/*.yml` finds nothing.
- [ ] **The run that counts**, on a commit with T36 merged: `verify` is green on `ubuntu-latest` and on `macos-latest`, and its log shows `tests/real` ran there (`gh run view <id> --log | grep -c 'test tst9_'` is above 0 for the macOS job).
- [ ] **The four release builds are green** (OPS-13): `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.
- [ ] **What macOS showed that Linux couldn't** is listed: each item fixed (with its commit) or filed (with its task ID). "Nothing" is a valid list.
- [ ] **A second run of the same commit is green too** (`gh run rerun <id>`): one pass of real-process tests on a shared runner proves little.
- [ ] **Recorded:** the run URLs, the commit, each job's duration and the billed minutes (the run's Usage page), given to the orchestrator for this task's Evidence.

## Traps
- **macOS minutes on a private repository are billed at ten times Linux's.** The free allowance goes quickly with a macOS job on every push: consider running the macOS job on `main` and on demand only, as a follow-up to T03.
- **Don't re-run a red job until it's green.** A real-process test that passes one time in three is a bug in the test or the code: file it.
- **Hosted macOS runners aren't the owner's Mac:** no Homebrew `gh` login, no Keychain prompt, no launchd service, another `$TMPDIR`. The tests don't use any of these; T40 checks them by hand.
- **The tests never reach GitHub's API** (TST-1): they need no token, and the workflow must not pass `GITHUB_TOKEN` or `GH_TOKEN` into `make verify`. A test that fails without one is a finding.
- **CLAUDE.md's "experiment only" is about live measurements.** Creating circitor's own repository and running its CI isn't one, but nothing here may open PRs or run queries anywhere else.
- `build.rs` reads the commit with `git`: a shallow checkout still has `HEAD`, so `circitor version` works in CI.
- The first push publishes `docs/` and `specs/` as they are. If the repository is public, read `docs/05-measurements.md` and `docs/measurements/` once more for anything private first.

## Open questions
- **This task depends on T03 only, on purpose:** the early run is worth having. Its Outcome is met only by the second run, on a commit that holds T36; T39 waits for this task.
- **TST-14's CI service is a (proposal).** GitHub Actions is the plan's choice (P9); the owner can still pick another, which makes T03 a different task.
- **`make check-macos` builds for macOS from Linux** (T50, through cargo-zigbuild), so compile errors show before this run. It proves nothing about running there.
- D5 belongs to T39. D6 doesn't touch this task.
