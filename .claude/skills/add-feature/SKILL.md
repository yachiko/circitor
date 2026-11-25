---
name: add-feature
description: Add a new feature to the circitor project (the local daemon that watches GitHub pull requests for coding agents) the way its layers expect, spec first and then plan. It reads the specs the feature touches, asks the owner only what the specs don't answer, drafts the change as new requirement IDs in specs/ with a D-number in specs/decisions.md for the owner's OK, then updates plan/ through build-plan's update rules (new task files, follow-ups and notes on done tasks, changes waiting on running ones) and stops at a ready plan. Use it whenever the user asks for a new feature, capability, PR field, wait condition, endpoint, CLI command or flag, config option, client integration, platform or behaviour change in circitor ("I'd like the daemon to also track…", "can the CLI show…", "add a … to the API", "it should wait until…", "add an MCP server", "a menubar widget"), even if they don't say "feature" or "spec". Not for bugs in built code, which are follow-up fixes, and not for reviewing a spec, which is harden.
---

# Add a feature

circitor is built in layers, and each one cites the layer above it:
- `docs/` holds the facts (`docs/02-github-api.md`, `docs/03-clients.md`, `docs/04-platform.md`: what GitHub's API, the agent clients and the platforms do, each labelled `verified`, `observed`, `documented` or `assumed`), the dated measurements behind them (`docs/05-measurements.md`) and the project's history. It's read-only here.
- `specs/` holds behaviour, with requirement IDs (MOD, POL, STA, API, CLI, OPS, TST, and U for user requirements) and decisions D1, D2, ….
- `plan/` holds tasks and contracts. Every task cites requirement IDs, and `plan.py check` measures coverage against them.
- The code cites the IDs in comments and test names.

A feature therefore enters at `specs/`. If it went straight into a task, no requirement would back it: `plan.py` couldn't see it, the next spec change could contradict it silently, and executors would have nothing authoritative to build against. So this skill writes the spec change first, gets the owner's OK, and only then updates the plan, using build-plan's update rules, which already handle new tasks, follow-ups of done tasks and running work.

Each run handles one feature request. It ends at a ready plan: nothing is built, and no executor is started.

The spec files were first laid out as 00-overview, 01-model, 02-polling, 03-state, 04-api, 05-cli, 06-operations, 07-testing and decisions. If `specs/README.md` lists different files or prefixes, it wins: re-derive from its file table wherever this skill and `references/ripple.md` name a file.

## Step 1: Understand where it lands

1. Restate the feature in one sentence, and check it against the non-goals in `specs/00-overview.md`. Read them there rather than trusting a list here. Two kinds of request need care:
   - **A request that crosses a non-goal** (webhooks were ruled out from the start; so is anything the overview says the daemon never does, such as changing something on GitHub) is a decision for the owner, not a draft: say so and ask before going on.
   - **A request the direction put after v1** (the MCP shim, the menubar widget, Linux packaging, a Windows transport, further search queries) is what this skill is for. It still starts with a decision row, and it usually adds a client or a platform, so check the facts docs for it first.
2. Read `specs/README.md`, `specs/00-overview.md`, `specs/decisions.md`, and then every spec file the feature touches. [references/ripple.md](references/ripple.md) maps which files a change to one usually drags in: read it now. If the feature needs a fact about GitHub, a client or a platform, read the relevant part of the facts docs.
3. Check whether the specs already cover it, wholly or in part. If they do, tell the user the IDs. It may be a plan gap or a bug instead (see "Not a feature" below).
4. Read the plan's state: `python3 -I .claude/skills/build-plan/scripts/plan.py status` and `plan.py ready`, and the README sections "Spec issues found while planning" and "Changes waiting on running tasks". Which tasks own the code the feature changes, and whether they're todo, running or done, decides how the plan absorbs it (step 4). If there's no `plan/` yet, the feature goes into the specs only, and build-plan picks it up when the plan is first made.

## Step 2: Ask only what the specs don't answer

Keep it short: usually 2–5 questions, asked with `AskUserQuestion` in one or two rounds, each with a recommended option first and a line on why. Ask about what the owner has to decide: what an agent or the user sees and where, which values and limits, what happens on failure or with missing data, and what counts as done. Don't ask what a spec, a decision or a facts doc already settles; cite it instead.

Typical gaps by layer (ripple.md lists more):
- **A PR field or state:** its source (a GraphQL field in docs/02-github-api, or derived from other fields), its type and when it's null, whether a change to it raises the revision, and whether the fact behind it is `verified`.
- **Polling:** which query carries it, what it adds in points per poll, and which cadence tier it belongs to.
- **State:** whether it's kept across restarts, and for how long.
- **API:** the endpoint or field, and when it's null; for a wait condition, when it's satisfied and which states end it early.
- **CLI:** the command or flag, its compact and JSON output, and its exit codes.
- **Config:** the key, its default and range.
- **A client or a platform:** what it needs from the daemon, and which facts in docs/03-clients or docs/04-platform it rests on.

**Facts that aren't verified** (a field never queried, a client behaviour only read about, anything on a Mac nobody has tried) aren't guessed, and `docs/` isn't edited. The requirement is marked `(proposal)`, and step 4 adds a manual owner task to measure the fact and record it in docs/05-measurements and its facts doc. The code tasks that depend on it wait on that task.

## Step 3: Draft the spec change and get the OK

Write the draft as the exact text you'll put in each file, then show it, grouped by file, before editing anything:
- **New requirement IDs** take the next free number of their file's prefix (`grep -o 'API-[0-9]*' specs/04-api.md | sort -t- -k2 -n | tail -1`). IDs are never reused: a struck-through one still counts. A new user-level need gets the next U-number in 00-overview's table.
- **Changed requirements** keep their ID. The text changes in place, and a line under the decision says what changed.
- **The decision:** the next free D-number in `specs/decisions.md`, under Resolved, phrased as the existing rows are: "**<outcome>** …, decided <date>", linking to the requirements. If the feature reverses an earlier decision, say so in both rows.
- **The status line** of each changed file: bump the draft's minor number and name the feature with its D-number ("draft 0.5, <date>: <feature> (Dnn)"). Keep its "Hardened: …" history.
- **Follow every ripple** in ripple.md that applies, in the same draft: for example the model's field table and its JSON example, the CLI's output and exit-code tables, the config table in 06-operations, the status object, and a TST requirement saying how the feature is tested (07-testing). A feature with no test requirement can't be accepted.
- Mark `(proposal)` anything resting on an unverified fact or on a value the owner chose without data. Keep each file under about 250 lines (specs/README).

Then ask for the OK with `AskUserQuestion`: apply, adjust (with a note), or drop. Apply the edits only after an OK. Don't edit `docs/`. `plan/` waits for step 4.

## Step 4: Update the plan

Load the `build-plan` skill and follow its section "Updating a plan". It runs `plan.py diff` on the spec change and handles each touched task by status:
- **todo:** edited in place;
- **done:** a new follow-up task, with a "Follow-ups: Tnn" line in the done task;
- **in-progress or review:** listed under README's "Changes waiting on running tasks";
- **contract changes:** updated in `plan/contracts/`, or made a follow-up task if the contracts task is done.

Two things to add for features:
- **New task files** follow build-plan's task format (`references/formats.md` in that skill): the next free task IDs, `owns` paths that don't overlap other todo or running tasks, dependencies on the tasks whose code they extend, and acceptance commands naming the new tests. Once v1 is built, most features are follow-ups of done tasks, with an integration or scenario task when the feature crosses crates.
- **A manual owner task** (`kind: manual`) for each fact to verify against the live GitHub API or on a Mac, and each value to choose. Name it "Owner: …", and make the dependent code tasks depend on it.

Finish with build-plan's checks: `plan.py check` must pass, then run `plan.py render --accept-spec-changes`, and add a dated line to README's change log naming the feature and its D-number.

## Step 5: Report

End with:
- **Feature:** one line, its D-number, and the spec files and IDs added or changed.
- **Plan:** tasks added (ID, title, wave, depends on), follow-up lines added to done tasks, and changes waiting on running tasks.
- **Owner tasks:** the manual tasks, and what each unblocks.
- **Open:** anything left `(proposal)` or undecided.
- **Next:** `plan.py ready` for the new tasks, and a suggestion to run `/harden <file>` on each spec file the feature changed: this skill drafts quickly, and harden is the review.
- **Commit:** suggest two commits in the repository's style, `docs(spec): <feature> (Dnn)` and `docs(plan): <feature>: Tnn–Tmm`. Make them only if the user asks.

## Not a feature

- **A bug in built code** (it behaves against the spec): no spec change. It's a follow-up fix on its own branch. Say so and stop.
- **The spec is wrong or contradictory:** that's harden's job, or a spec issue for the owner. Say so and suggest `/harden <file>`.
- **A plan-only change** ("split T14", "add a test for X" where X is specified): build-plan handles it directly.
- **A fact changed** (GitHub now behaves differently, a client gained a capability): the facts docs are the owner's to update, with a dated entry in docs/05-measurements. Say what you saw and stop; the feature, if one follows, starts from the updated fact.

## Rules

- The rule that executors and build sessions never edit `specs/`, `docs/` or `plan/` (CLAUDE.md, `plan/conventions.md`) doesn't bind this skill: it's the owner's way to change the specs, so it edits `specs/` and `plan/`, and only after the owner's OK. It never edits `docs/`.
- Never renumber or reuse a requirement, decision or task ID.
- Don't invent product behaviour the owner didn't choose: ask, or mark it `(proposal)` with your recommendation.
- Don't start executors or write code. The plan is ready when `plan.py check` passes and `plan.py ready` lists the new tasks, or their owner tasks.
