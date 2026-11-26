---
id: T11
title: "CLI arguments, naming a PR, and the guide for agents"
kind: code
milestone: M3
size: M
status: done
depends_on: [T02]
requirements: [CLI-1, CLI-2, CLI-11, CLI-12, OPS-12, U8, NTF-11]
owns: [src/cli/args/, src/cli/naming/, src/cli/guide/, skills/, AGENTS.section.md]
shares: []
---

# T11 · CLI arguments, naming a PR, and the guide for agents

## Outcome
`cli::args::parse` turns arguments into a `Command` (CLI-1), `cli::naming` parses the forms of a PR's name and picks the PR of the current branch from a list (CLI-2), and the guide exists once as text and ships as `circitor guide`, a skill and an `AGENTS.md` section (CLI-11, OPS-12).

## Context
T02 gives `Command`'s declaration, `Env`, `PrName` and `Record`. Nothing here talks to a daemon, `git` or a terminal: three pure modules and two text files. T27 calls `parse`, runs `git` and the requests, and feeds `naming` with what it got. T19 tells users where the two guide files go.

## Read first
- `plan/conventions.md`
- `plan/contracts/cli.md`: the blocks "`src/cli/args/`", "`src/cli/naming/`" and "`src/cli/guide/`" (the signatures you build), `Git` and `Branch` (what T27 will hand you), "Semantics"; `plan/contracts/config.md`: `Env::get`; `plan/contracts/model.md`: `PrName` (`parse`, `from_url`, its order), `Record`; `plan/contracts/api.md`: `Until`
- `specs/05-cli.md#commands`: CLI-1, CLI-2; `specs/05-cli.md#for-agents`: CLI-11, CLI-12; `specs/05-cli.md#output`: CLI-5 (the guide lists the codes); `specs/05-cli.md#waiting`: CLI-6 (what the guide says about a timeout); `specs/05-cli.md#the-daemon`: CLI-7's last bullet (`--no-spawn`)
- `specs/04-api.md#waiting`: API-10 (`sha`, `rerun_since`, `fail_fast`, `kind`), API-12 (the timeout's range); `specs/04-api.md#reading`: API-8's last bullet (a cursor's scope)
- `specs/08-notify.md`: NTF-11 (the command's flags), NTF-2 (`for`'s range), and NTF-7, NTF-10 and NTF-12 for what the guide tells an agent about `notify`; `docs/07-notify.md`, "How it reads to the session"
- `specs/06-operations.md#shipping`: OPS-12; `specs/00-overview.md`: U8
- `docs/03-clients.md` §1, §4 and §5: which client reads which file, what a skill's `description` is for, and how long a `wait` may block

## Scope
- **Arguments (CLI-1).** clap's derive, through `try_parse_from` with `circitor` put in front of `args`. The derive types stay private; `parse` maps them to `Command`. `--json` exists only on the nine commands CLI-1 names. `--no-spawn` is global; `CIRCITOR_NO_SPAWN` set to exactly `1` (through `env.get`) means the same (the flag is CLI-7's; acting on it is T24's).
- **Checked here, as usage errors:** `--until` is required and one of `merged`, `checks-done`, `changed`; `--timeout` is 0 to 3 600 (API-12); `--sha` is 7 to 40 hex characters, kept as given; `--sha`, `--rerun-since` and `--fail-fast` only with `checks-done`, `--kind` only with `changed` (API-10); `track` and `untrack` need a PR; `daemon` takes `--spawned` or `--check-config`, not both. **Cursors and `--kind`'s words are passed on unread**: the daemon checks them. `wait`'s six condition words go into one `Ask` (`plan/contracts/api.md`).
- **`notify` (NTF-11's share here: the arguments; the command is T48's).** `notify [PR] --until … [--sha] [--rerun-since] [--fail-fast] [--kind] [--since] [--for S]` is `NotifyWhat::Register`, with the same `Ask` and the same checks as `wait`, and `--for` 60 to 86 400 (NTF-2); it takes no `--timeout`. `notify [PR] --cancel` is `NotifyWhat::Cancel`: `--cancel` with `--until` or any other of those flags is a usage error, and so is neither.
- **Help and version.** `help`, `--help` and `-h` give `Ok(Command::Help(text))`; `version`, `--version` and `-V` give `Ok(Command::Version)`. No command at all is a usage error.
- **A usage error is one line** (CLI-5): what's wrong, then `; see: circitor help`. clap's own multi-line text isn't returned.
- **CLI-12:** `parse` never prints, never exits, never asks. Help and errors carry no colour: `ColorChoice::Never`.
- **Naming, the forms (CLI-2; shared with T27).** `pr_arg`: `owner/repo#12` and the URL are `Full` (`PrName::parse`, `PrName::from_url`), digits are `Number`, nothing is `Bare`. `#12`, `0` and anything else are errors that say which forms exist; for `#12`, to write `12`. `repo_of_origin`: github.com only, without case, in the forms `https://github.com/o/r[.git]`, `git@github.com:o/r[.git]` and `ssh://git@github.com/o/r[.git]`; a trailing `/` is allowed. Anything else is `None`.
- **Naming, the choice (CLI-2).** `match_branch` keeps the records whose repository is `owner/repo` (without case), whose `head` is `branch` (exactly) and whose `author` is `login` (without case). One open one is `Open`; several are `Several`, in `PrName`'s order. With none open and `ended_too`: the merged, closed or gone one with the latest `closed_at` (a gone one's `fetched_at`; a missing `closed_at` counts as `merged_at`, then `fetched_at`) is `Ended`. **`login` `None` matches nothing** (`plan/contracts/cli.md`, "Semantics"): CLI-2 lets only the login's PRs match.
- **T27's share of CLI-2:** running `git`, the requests (`list`, `refresh`, `list` again), the exit codes and every line printed.
- **The guide (CLI-11).** One source, `src/cli/guide/guide.md`, under 120 lines (CLI-11, D9), read with `include_str!` into `GUIDE`. It says everything CLI-11 lists, and that `gh` doesn't help on code 9 (CLI-5). Every command of CLI-1 an agent uses has one example line. It names `--fail-fast` and `--kind`.
- **The guide on `notify` (CLI-11's second half), for Claude Code:** when `notify` fits better than a long `wait`; that its exit code doesn't say the condition holds; that a notice arrives as a message from another session and is a hint to run the command it names; that the confirmation comes with the command's result, and **when none arrives, to cancel the notify (`notify --cancel`) and then use `wait`** (NTF-7: an unheard subscription still holds the active tier); that exit code 6 from the command in a `stopped` notice means the daemon was stopped on purpose, and isn't to be started again for a notice; that in an unattended run a `wait` in the background is the way; and that a notify is lost when the session is resumed or the daemon crashes, so ask again. Exit code 11 is in the list of codes.
- **U8's share:** the wording works in Claude Code, OpenCode and pi. A `wait` ends by itself after 100 s, under every client's 120 s; exit 3 means "not yet, run the `again:` line"; a longer `--timeout` only where the shell tool's own timeout was raised above it. No client's tool or parameter is named.
- **OPS-12's share:** `skills/circitor/SKILL.md` is frontmatter (`name: circitor`, and a `description` that says when to reach for it: it's all a client puts in its prompt) and then the guide; `AGENTS.section.md`, at the repository root, is a `## circitor` heading and then the guide. Telling users where they go is T19's.

## Interfaces
- **Consumes:** `config::Env`, `model::{PrName, Record, PrState}`, `api::types::Until`, clap.
- **Provides:** `args::{Command, Parsed, parse}`, `naming::{PrArg, pr_arg, repo_of_origin, BranchMatch, match_branch}` and `guide::GUIDE`, exactly as `plan/contracts/cli.md` declares them.

## Out of scope
- Running `git`, CLI-2's flow and its messages, printing help, the version's text, `daemon --check-config`: T27.
- Acting on `--no-spawn`: T24. Text and exit codes of answers: T10.
- The README's section on where the skill and the section go: T19.
- `.claude/skills/` is the project's own tooling. Don't touch it; the shipped skill is `skills/circitor/`.

## Acceptance
Unit tests beside the code.
- `cli1_every_command_parses`: a table with each line of CLI-1's table, each flag once, giving the expected `Command`. `wait 12 --until checks-done --sha 1A2b3c4 --rerun-since k3x9q2mz-400 --fail-fast --since k3x9q2mz-418 --timeout 300 --json` fills every field, the SHA unchanged; `wait --until changed --kind activity,merged` keeps `kind` as given.
- `ntf11_notify_parses`: `notify 12 --until checks-done --sha 1a2b3c4 --for 3600 --json` is `Register` with `life` `Some(3600)`; without `--for`, `None`; `notify --cancel` and `notify 12 --cancel --json` are `Cancel`. Usage errors, one line each: `notify 12` with neither; `--cancel --until merged`; `--cancel --for 60`; `--for 59`; `--for 86401`; `--timeout 5`; `--fail-fast` with `--until merged`.
- `cli1_usage_errors`: each is `Err` with one line and no escape character: an unknown flag; `stop --json`; `guide --json`; `wait 12` without `--until`; `--until done`; `--timeout 3601`; `--timeout -1`; `--sha 12345`; `--sha zzzzzzz`; `--sha 1a2b3c4` with `--until merged`; `--fail-fast` with `--until changed`; `--kind activity` with `--until checks-done`; `track`; `daemon --spawned --check-config`; no command.
- `cli1_no_spawn`: `--no-spawn` before and after the command, and `CIRCITOR_NO_SPAWN=1`, set `no_spawn`; `CIRCITOR_NO_SPAWN=0`, an empty one and none don't.
- `cli12_help_is_plain`: `help` and `--help` are `Ok(Help)`, name all fourteen commands and hold no `\x1b`.
- `cli2_pr_arg`: `yachiko/circitor#12`, `https://github.com/yachiko/circitor/pull/12` and the same with `/files` are `Full`; `12` is `Number(12)`; `None` is `Bare`; `#12` (error naming `12`), `0`, `-3`, `abc` and `yachiko/circitor` are errors.
- `cli2_repo_of_origin`: the three forms, with and without `.git`, and `GitHub.com`, give `("yachiko", "circitor")`; `https://gitlab.com/a/b.git`, `https://github.example.com/a/b` and `/srv/git/r.git` give `None`.
- `cli2_match_branch`: one open match; a fork contributor's PR on the same branch isn't matched; the login and the repository compare without case, the branch doesn't; two open are `Several` with both names; of a merged and a closed one, the later `closed_at` wins; a gone one counts by `fetched_at`; an ended one is `None` unless `ended_too`; `login` `None` is `None`.
- `cli11_guide_is_one_text`: the body of `SKILL.md` after its frontmatter, the body of `AGENTS.section.md` after its heading, and `GUIDE` are equal; `GUIDE` has under 120 lines.
- `cli11_guide_says_it_all`: it names `list`, `show`, `wait`, `notify`, `changes`, `track`, `untrack`, `refresh`, `status` and `stop`, `--sha`, `--rerun-since`, `--fail-fast`, `--kind`, `show --fresh`, `--no-spawn`, and each exit code 0 to 11.
- `cli11_examples_parse`: every line of the guide that starts with `circitor ` parses with `args::parse`.
- `cargo test --features test-endpoint --lib -- cli::args:: cli::naming:: cli::guide::` and `make verify` pass.

## Traps
- **clap prints and exits by default.** Only `try_parse_from`; a `DisplayHelp` or `DisplayVersion` error becomes `Ok`, not `Err`.
- `Command` and `Parsed` are the contract's. Don't add a field or a variant: report what's missing.
- `args` has no program name in it.
- The three guide files must not drift: the test reads the two shipped files with `include_str!`, so a change to one fails the build's tests.
- The guide is paid for line by line (CLI-3's reason): one example per command, no prose an agent can't act on.
- `repo_of_origin` is not a URL parser for every host: an Enterprise host is `None` on purpose (U13 is github.com only).
- A branch name is case-sensitive; a login and a repository name aren't.

## Open questions
- CLI-2's short forms, CLI-11, OPS-12 and NTF-11 are (proposal): build them as written.
- **The guide must say all of CLI-11 in under 120 lines.** If it can't, keep every rule and cut prose, and report the count.
- docs/03 §1: whether Claude Code reads `AGENTS.md` wasn't checked, so the skill is its way in. docs/03 §6 items 2 and 3 (a killed `wait`, a command silent for minutes) are open: the guide says nothing that rests on them.

## Evidence
- **2026-11-26**, branch `t11-cli-args-naming-guide`, executor on **Sonnet, medium**; reviewed and merged by the orchestrator.
- Changed: `src/cli/args/`, `src/cli/naming/`, `src/cli/guide/` (with `guide.md`, **63 lines**), `skills/circitor/SKILL.md`, `AGENTS.section.md`. Only `GUIDE`'s value changed among public items. `cargo test --features test-endpoint --lib -- cli::args:: cli::naming:: cli::guide::`: 11 passed. `make verify` passes on main after the merge (353 lib tests), the macOS check included.
- Choices: a clap usage error is cut to its first line (so its wording is clap's); `--version` exists only to map `DisplayVersion` to `Command::Version`; the example test strips a trailing ` # comment`; exit codes in the guide are lines starting `N `; `untrack` without a PR is a usage error too. `Command::Help(text)` is clap's rendered help (T27 prints it).

Follow-ups: T63 (D18), T65 (D19)
