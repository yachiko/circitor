# Briefs for helper subagents

Fill in the parts in angle brackets. Each helper has seen nothing of this conversation, so the brief is everything it knows. Paste the outputs it needs (`plan.py index`, `plan.py hints`) instead of describing them, and keep your own suspicions out: a reviewer handed a hypothesis tends to come back with it confirmed.

Contents:
1. Writer: drafts task bodies in parallel
2. Reviewer A: executor dry-run
3. Reviewer B: dependency and contract audit

## 1. Writer

```text
You're drafting task briefs for the implementation plan of circitor, a local Rust daemon that polls GitHub's GraphQL API for the user's pull requests and serves their state to coding agents through a CLI talking HTTP over a Unix socket (macOS first, through Homebrew; developed on Linux). The project root is <root>. Executor subagents will implement each task in its own git worktree, knowing only what its task file and the files it cites tell them. A brief that leaves them guessing gets built wrong.

Your tasks: <T05, T06, …>. Each file in <root>/plan/tasks/ already has its frontmatter and section headings. Write the body under each heading. Don't change the frontmatter. If it looks wrong (a requirement belongs elsewhere, a dependency is missing, a path is wrong), say so in your reply instead.

Read first:
- <skill dir>/references/formats.md, §1: the sections, what goes in each, and a worked example. Follow it.
- <root>/plan/conventions.md and <root>/plan/contracts/README.md, then the contract files your tasks touch.
- The spec files your tasks' requirements live in (run `python3 -I <skill dir>/scripts/plan.py --root <root> show <ID>` to see where one is, its text and its parts), and the facts docs for what GitHub, the clients and the platforms do: <root>/docs/02-github-api.md, <root>/docs/03-clients.md, <root>/docs/04-platform.md.

The whole plan, for knowing your tasks' neighbours (Out of scope names them by ID):
<output of plan.py index>

Rules:
- Cite requirement IDs and spec sections; don't copy the spec's rules into the brief. Where the task interprets a rule, say how.
- Every requirement in a task's frontmatter is mentioned in its body, and where a requirement is shared with another task, the Scope says which share is this one's.
- Acceptance is checkable in the worktree: name the tests and what each proves, with the spec's concrete values (revisions, head SHAs, points, timeouts, exit codes) and the failure cases, and the commands that must pass.
- Traps: what an executor would get wrong without being told. Open questions: the open findings and decisions touching the task (listed below), and what to build meanwhile: the spec as written today.
- Never edit specs/ or docs/. Write only the task files you were given, and your notes file.
- Read the spec files your tasks cite, not the whole spec set; your tasks were grouped so they share them. Check a fact against the data (a fixture, a recorded response) only when an acceptance value depends on it. Don't call the GitHub API.

Open decisions and review findings: <paste the plan:open block, or `plan.py extract`'s tail>

Before you reply, save your notes to <root>/plan/notes/writer-<first task ID>.md, so they survive if the planner's session stops: the files you wrote; for each task, anything in its frontmatter you think is wrong (a missing or unneeded dependency, a requirement that belongs elsewhere, a path); contract gaps, and notes for other tasks' briefs; and spec questions you hit. Then reply with the same notes.
```

## 2. Reviewer A: executor dry-run

```text
You're about to implement tasks of the circitor implementation plan, each in its own git worktree, knowing nothing but what the plan gives you. Before anyone starts, dry-run them: read each brief as the executor would, and find what would stop you or make you build the wrong thing. Don't implement anything, and don't edit any file.

The project root is <root>. Your tasks: <5–8 task files: the first two waves, the critical path's L tasks, and one integration task>.

For each task, read <root>/plan/conventions.md, the task file, and everything its "Read first" section lists, and nothing else, as an executor would. Then report:
1. Questions you'd have to ask before starting, or decisions you'd have to make that the brief, the contracts or the spec should have made.
2. Things you need that no task in depends_on provides and plan/contracts/ doesn't define: a type, a function, a fixture, a file, a running component.
3. Work that another task seems to own as well (check the other tasks' frontmatter with `python3 -I <skill dir>/scripts/plan.py --root <root> index`).
4. Acceptance items you couldn't check in the worktree, or that a wrong implementation would still pass.
5. Requirement IDs in the frontmatter whose spec text the brief doesn't really cover, or spec rules in the cited sections that no task seems to build.

Cite file:line for every point. Report only what you'd actually trip on, highest impact first; "nothing found" is a fine answer for a task. Format per point: task, severity (blocks / misleads / minor), what, where, the fix you'd want.
```

## 3. Reviewer B: dependency and contract audit

```text
You're auditing the task graph of the circitor implementation plan before a swarm of executor subagents builds from it. Tasks that may run at the same time are built in separate git worktrees from the same starting commit, so a dependency missing from the graph means an executor builds against something that doesn't exist yet, and an unneeded one means it waits for nothing. Don't edit any file.

The project root is <root>. Read <root>/plan/README.md, <root>/plan/contracts/ (all of it) and <root>/plan/conventions.md. Task files are in <root>/plan/tasks/; open any you need.

The task index:
<output of plan.py index>

Spec cross-references between tasks that may run at the same time:
<output of plan.py hints>

Find:
1. Hidden dependencies: a task that uses another task's code, output, fixture or generated file with no path between them in depends_on (directly or through others).
2. Dependencies that could be contract-only: the task needs only an interface that plan/contracts/ already declares, with a fake, but it waits for the implementation.
3. Seams with no contract: two or more tasks that must agree on a type, function, format or message that plan/contracts/ doesn't fix.
4. Fixtures, testdata, generated files and shared files (Cargo.toml, Cargo.lock, the Makefile, `mod` declarations, the daemon's wiring, the route table, the CLI's command list) with no owner, or with two owners that may run at the same time.
5. Ordering mistakes: an integration or scenario task before the parts it exercises, or test infrastructure later than the tasks whose acceptance needs it.
6. The critical path (README's "Critical path"): which tasks to split, shrink or take off it, and what that would save.

Cite task IDs and file:line. For each point: severity (wrong build / wasted wait / minor), what, evidence, the fix.
```

## After the reviewers return

Verify each point before changing anything: open the cited lines, check that the dependency or contract really is missing (search the other tasks and `plan/contracts/`), and drop what's already handled or wrong. Merge points with one cause. Then fix the plan, run `plan.py check`, and tell the user in the report how many points came back, how many you acted on, and why you dropped the rest.
