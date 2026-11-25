---
name: build-plan
description: Turn the circitor specs (specs/*.md, with GitHub, client and platform facts from docs/02-github-api, docs/03-clients and docs/04-platform) into an implementation plan in plan/ that an orchestrator session and a swarm of executor subagents in git worktrees build the project from. It designs the contracts between components first, then writes one self-contained brief per task (owned paths, dependencies, requirement IDs, acceptance tests and commands, return format); a bundled script checks requirement coverage, file ownership and cycles, and computes waves and the critical path. Re-running it updates an existing plan after the specs change, without renumbering tasks or rewriting finished ones. Use it whenever the user wants to plan the implementation, break the specs into tasks or slices, prepare work for agents, subagents or a swarm, work out the build order or what depends on what, see what can be built in parallel, or refresh, replan or re-slice plan/ after spec changes, even if they don't say "plan".
---

# Build the implementation plan

`plan/` is the build contract for the swarm, the way `specs/` is the product contract. An orchestrator session reads `plan/README.md`, starts one executor subagent per ready task in its own git worktree, reviews what comes back and merges it. An executor knows only what its brief points it to, so the plan decides whether twenty agents build one system or twenty.

What makes a plan work for a swarm:
- **Contracts before code.** Parallel tasks agree on module APIs, types and formats because `plan/contracts/` fixes them first, and an early task turns them into compilable code with fakes. Tasks then depend on the contract, not on each other's implementations.
- **Self-contained briefs.** A task file plus what it cites is everything an executor needs.
- **Disjoint write sets.** Tasks that can run at the same time never own the same path; the few files several tasks must touch are named as hotspots.
- **Executable acceptance.** Every task names the tests to write, with the spec's concrete values, and the commands that must pass.
- **A short critical path and wide waves.** Enablers (scaffold, contracts, fake clock, fake GitHub server) come first; integration tasks join the parts at checkpoints.

The specs stay the source of truth. The plan cites requirement IDs and spec sections instead of restating rules, which would drift. It adds only what the specs leave open: contracts, order, ownership, acceptance and plan decisions.

Paths are relative to the project root, the directory holding `specs/` and `docs/`. The tool is `python3 -I .claude/skills/build-plan/scripts/plan.py <command>` (`plan.py` below; `--help` lists the commands). It reads the specs fresh each time; run it rather than counting by hand.

**The facts docs** are `docs/02-github-api.md`, `docs/03-clients.md` and `docs/04-platform.md`: what GitHub's API, the agent clients and the platforms do, each fact labelled `verified`, `observed`, `documented` or `assumed`. `docs/05-measurements.md` is the dated log of live measurements behind the verified ones.

## What you produce

```
plan/
  README.md          the orchestrator's guide: protocol, milestones, generated waves, graph,
                     critical path and hotspots, plan decisions, open questions, change log
  conventions.md     rules every executor follows: layout, commands, coding and test rules, report format
  contracts/         README.md (the seam index) and one file per seam: declarations, semantics, fakes
  tasks/Tnn-slug.md  one executor brief per task: frontmatter plus fixed sections
  graph.json         generated: tasks, waves, critical path, and the spec snapshot `diff` compares against
  coverage.md        generated: requirement → tasks
  notes/             temporary: the writer subagents' notes, deleted once folded in (Step 8)
```

Formats, templates and a worked task: `references/formats.md`. How to find and write the seams, with circitor's seams, checkpoints, open choices and traps: `references/contracts.md`. Briefs for writer and reviewer subagents: `references/briefs.md`.

## Step 1: Mode and scope

- **No `plan/`:** a fresh plan. **`plan/` exists:** update mode ("Updating a plan" below), unless it has no `graph.json`: then an earlier run stopped before Step 8, and you resume it. Run `plan.py check` (its "aren't written yet" line lists the bodies still to write), read `plan/notes/` (what the writers reported), and carry on from the first unfinished step. If the user wants a fresh plan while tasks are in progress or done, ask first: those tasks have code behind them.
- **Scope** is the whole system unless the user narrows it ("only the daemon", "M1 first"). A narrow scope still includes what its tasks depend on. Pass the requirement prefixes to `check` and `render` as `--scope`.
- Tell the user in one line what happens, e.g. "Fresh plan for the whole system: contracts, then about 30 task briefs, then two reviewers. It takes a while."

## Step 2: Inventory and reading

1. Run `plan.py extract`. It lists the requirements per prefix, the dropped (struck-through) ones, how many carry (proposal), the umbrella requirements with many parts, IDs mentioned but never defined, the open decisions and the open review findings. You'll come back to this output.
2. Read in full: `specs/README.md`, `00-overview`, `01-model`, `decisions`, the three facts docs, then every other spec file. All of them: contracts cut across files, and a requirement you haven't read can't be placed. Look at the recorded responses `docs/05-measurements.md` points to (the testing spec's fixtures come from them) and at its latest dated entries. Other docs are background: read one when a spec cites it.
3. **When sources disagree:** the facts docs beat every other doc and any fact or confidence label copied into a spec; within them `verified` beats `documented`, and both beat `assumed`. The latest dated entries in docs/05 beat older ones. `specs/decisions.md` beats `docs/06-decision-log.md`. `docs/01-direction.md` is input to the spec, may be stale, and never decides anything.
4. Keep a list of **spec issues found while planning**: contradictions, gaps, requirements you can't place or test, facts in the facts docs newer than a spec. Don't fix them: specs and docs are the user's, and the `harden` skill exists for that. They go in README's open questions.
5. `plan.py show ID` (or `ID:part`) prints one requirement, its references, its parts and its tasks. Use it to check one rule instead of re-reading a file.

The spec files were first laid out as 00-overview, 01-model (MOD), 02-polling (POL), 03-state (STA), 04-api (API), 05-cli (CLI), 06-operations (OPS), 07-testing (TST) and decisions. If `specs/README.md` lists different files or prefixes, it wins: re-derive from its file table wherever this skill and its references name a file.

## Step 3: Design the seams

Read `references/contracts.md`, then:
- List the components and the seams between them, starting from 00-overview's components and repo layout and the testing spec's harness. For each seam, write `plan/contracts/<seam>.md`: producer, consumers, declarations, semantics with requirement IDs, and the fake that ships with it. Formats the specs already define are cited, not redefined; golden fixtures for them let a writer and a reader be built apart.
- Write `plan/contracts/README.md`, the seam index, and `plan/conventions.md` from the outline in `references/formats.md` §3.
- Every choice the specs leave open goes in README's **Plan decisions** table, marked proposal: the crate layout, crate and module names, a library's role, test tools, whether the macOS target is compile-checked from Linux. The user can overrule them before executors build on them, so make the expensive-to-change ones visible.

## Step 4: Slice the work into tasks

- **One task is one executor session, one worktree and one PR.** It must fit a fresh context: conventions, its contract files, its task and about three spec files. S is under ~300 lines including tests, M ~300–800, L ~800–1,500. Split anything bigger by capability. Merge a tiny task into a neighbour, unless it unblocks others.
- **Own a module or crate end to end:** its code, unit tests and testdata. Don't split one module into "types", "logic" and "tests" tasks.
- **Enablers first:** whatever many tasks test against goes in the earliest waves: the scaffold, the contracts with fakes, the fake clock and the harness, the fake GitHub server with its recorded fixtures, and any fault injection the testing spec asks for (the state file's writes, the socket).
- **Tests go with the behaviour they test.** A TST bullet about one component belongs to the task that builds it: split an umbrella TST requirement with `ID:part`. Test infrastructure (the fake GitHub server, the fake clock, the harness, the fixtures) gets its own early tasks. Whole-system scenarios (a PR's life from creation to merge, sleep and wake, quota exhaustion, restarts, two daemons, upgrades, the CLI end to end) go to test or integration tasks after the parts they exercise.
- **Integration tasks at checkpoints.** Pick four to six runnable checkpoints (`references/contracts.md` §5 has a starting set). Each gets an integration task that wires the parts so far and proves the checkpoint end to end.
- **Manual work is a task too.** Acceptance on a Mac (the Homebrew install, the service under launchd, a menubar widget), anything needing the live GitHub API (recording real responses, the live acceptance checks) and anything only the owner can do (a release, a choice of value) is `kind: manual`. The swarm skips them, and coverage stays honest.
- **No executor task waits on a manual one** (`check` makes it an error). Where code needs what only the owner can produce, the code task builds against a stand-in in the documented format (a synthetic fixture written from the facts docs' examples, a placeholder config), and the manual task replaces it later and reruns the tests. Look at the recorded responses first: a real recording may already exist.
- **Every active requirement lands in a task.** Dropped ones land nowhere. (proposal) items are built as written. An open review finding doesn't change the spec until the user decides it.
- **Write the index first:** create every task file with its frontmatter and the section headings only. Run `plan.py check` and fix coverage, ownership and cycles while they're cheap to fix. Its "aren't written yet" line is expected until the bodies exist.

## Step 5: Work out the dependencies

A dependency means "B can't be built or accepted until A is merged". Give each task the smallest true set:
1. **List what it consumes:** types, functions, files, fixtures, a running component. They go in its Interfaces section.
2. **Contract or implementation?** If the contracts task provides it, with a fake, B depends on the contracts task only. B waits for A's implementation only when B's acceptance needs A's real behaviour: an integration test, a fixture A generates, a format A writes and B reads.
3. **Look for hidden inputs:** shared fixtures and testdata, generated files (a snapshot or schema a test writes), Makefile targets, `Cargo.toml` and `Cargo.lock` (the scaffold declares every dependency, so tasks don't fight over them), `mod` declarations in a crate root, and registration points (main wiring, the route table, the CLI's subcommand list).
4. **Run `plan.py hints`.** It lists spec cross-references between tasks that may run at the same time. For each pair, add an edge, put what they share in a contract, or decide it's incidental. Most hints become contract items, not edges.
5. **Shape the graph.** `plan.py check` prints the waves and the critical path. Shorten the path: split its L tasks, move work that doesn't block anything off it, and make sure nothing waits for an implementation when the interface would do.
6. **Ownership.** `owns` lists what a task creates or changes (a directory ends with `/`); tasks that may run at the same time must not overlap, and `check` fails if they do. A file several tasks must touch is owned by the task that creates it and listed under `shares` by the others, with the change spelled out (one line in an index, one call in main). Avoid sharing where a design can: a crate per component, module declarations written by the contracts task, one wiring task per checkpoint.

## Step 6: Write the briefs

Fill in each task from the template in `references/formats.md` §1. The sections are fixed, because executors and reviewers look for them.
- **Cite, don't copy.** Name requirement IDs and spec sections. Quote a rule only where the task interprets it, and say so. Every requirement in the frontmatter appears in the body, and a requirement shared by two tasks says which share is whose.
- **Read first** lists only what the executor needs, each with why.
- **Acceptance** is checkable in the worktree: the tests to write and what each proves, with the spec's concrete values (a fixture's head SHAs and revisions, the points a query costs, exit codes, timeouts) and the failure cases, plus the commands. An executor must be able to tell done from not done without asking.
- **Out of scope** names the neighbouring tasks by ID, so two executors don't build the same edge.
- **Traps** carry what you learned from the specs, reviews and platform that an executor would trip on (`references/contracts.md` §7).
- **Open questions:** the open findings, decisions and unverified facts that touch the task, and what to build meanwhile: the spec as written today, with the rule kept in one place so a decision can change it later.

With more than about 15 tasks and the Agent tool available, have writer subagents draft the bodies in parallel, using the writer brief in `references/briefs.md`. Writers spend most of their effort reading specs, so give each one the tasks that cite the same spec files (one area: model and polling, state, API and waits, the CLI, tests and operations), 10 to 14 tasks each, and at most six writers, in one message. You own the frontmatter and the contracts; writers don't change them.

Each writer saves its notes to `plan/notes/writer-<first task>.md` as well as replying, so nothing is lost if your session stops before it returns. When they're done:
1. Read every body, and fix it or send it back (SendMessage keeps the writer's context). `plan.py check` lists bodies that still aren't written.
2. **Fold the notes in before the review.** For each point, verify it, then act: a wrong or missing dependency goes into the frontmatter; a contract gap goes into the contract file, and every task using that seam gets checked; a note for another task goes into that task's brief; a spec question goes into README's spec issues. Writers see their neighbours' seams closely, so their notes are often the best dependency audit you get; reviewers should find the plan with these already fixed.
3. Run `plan.py check` again.

## Step 7: Fresh-eyes review

Executors fail on what the planner knew but didn't write down, so let agents who know only what an executor knows read the plan. Spawn both reviewers from `references/briefs.md` in one message:
- **Executor dry-run** on 5 to 8 tasks: the first two waves, the critical path's L tasks and one integration task.
- **Dependency audit** of the whole graph and the contracts, with the `index` and `hints` output pasted in.

Verify every point before acting on it: reviewers are wrong in the same confident tone as when they're right. Without the Agent tool, do both passes yourself, re-reading the files cold, and tell the user it was a single-reviewer pass.

## Step 8: Render and report

1. `plan.py check` reports no errors. Each warning gets a fix or a reason.
2. `plan.py render` writes `graph.json` (with the spec snapshot) and `coverage.md`, and fills README's generated blocks. Once every writer note has been folded in, delete `plan/notes/`: the plan holds what mattered, and executors shouldn't read half-applied drafts.
3. Tell the user, briefly: the task count by kind; the waves and the critical path; the first wave to launch; the plan decisions they should confirm, costliest to change first; the spec issues found (and that `harden` handles them); and the manual tasks that block the swarm, if any. Point them at `plan/README.md` to start.

## Updating a plan

Re-running on an existing plan keeps what has been built.
1. Run `plan.py diff` (the spec changes since the snapshot, and the tasks they touch) and `plan.py check`. Read README's Plan decisions and Open questions: the user may have answered some.
2. Handle each change by the status of the tasks it touches:
   - **todo:** edit the task in place (scope, acceptance, requirements), then re-check its dependencies and contracts.
   - **done:** add a follow-up task (the next free ID) that depends on it and covers the change, and add a "Follow-ups: Tnn" line to the done task. Leave the rest of its file alone: it records what the code was built against.
   - **in-progress or review:** don't edit a brief an executor is working from. List the change under README's "Changes waiting on running tasks", and tell the user, so the orchestrator folds it in before accepting or adds a follow-up.
   - **An added requirement** goes into a todo task that fits, or a new task. **A dropped one** comes out of todo tasks; for done tasks, a follow-up removes the code if any.
   - **A changed section** without IDs (a table, a schema, a section of a facts doc): check the tasks that cite it.
   - **A resolved decision or finding:** update the tasks' Open questions and README. It may change scope.
3. A change to a seam updates its contract file. If the contracts task is done, the change is a follow-up task, and every task using that seam is checked.
4. Never renumber or reuse a task ID. A todo task that's split or merged away gets `status: dropped` and a line naming its replacement.
5. Run `plan.py check`, then `plan.py render --accept-spec-changes`, and add a dated line to README's change log. Report the tasks added, changed and dropped, the follow-ups, and the changes waiting on running tasks.

Requests that aren't spec changes ("split T14", "move the CLI earlier", "add a task for X") follow the same rules.

## Rules

- **Never edit `specs/` or `docs/`.** Spec issues go to README and to the user.
- **Don't invent product behaviour.** Where the specs are silent, make the smallest plan decision that lets work go on, mark it proposal, and list it.
- **Executors never edit `plan/`, `specs/` or `docs/`**; the orchestrator does. `conventions.md` says so.
- **Task IDs** are T01, T02, … in roughly the order tasks can start, and are never reused.
- **The plan is for building, not a design document.** A task body over ~150 lines is restating the spec, or it's two tasks.
