---
name: harden
description: Stress-test one spec file of the circitor project (specs/*.md) for problems and improvement opportunities, such as contradictions with the facts in docs/02-github-api, docs/03-clients and docs/04-platform or with other spec files, missing failure handling and edge cases, and requirements nobody could build or test as written. It uses three parallel fresh-eyes reviewer subagents, verifies their findings, then walks the user through them and edits the spec on their OK. Each run is one pass with its own lens (1 consistency, 2 failure modes, 3 implementability), tracked in a findings log, so repeat runs go deeper instead of repeating themselves. Use it whenever the user says harden, stress-test, poke holes in, red-team, sanity-check or critically review a spec file, asks for another, second or third pass over one, or wants gaps, inconsistencies or edge cases found in anything under specs/, even if they don't say "harden".
---

# Harden a spec file

The files in `specs/` are the build contract for `circitor`, a local daemon that polls GitHub for the user's pull requests and serves their state to coding agents. They were written quickly from research notes and a session of live measurements, so they contain mistakes that are cheap to fix now and expensive once code exists. This skill finds them.

How a run works:
- **One run is one pass, with one lens.** Pass 1 checks consistency, pass 2 failure modes, pass 3 implementability.
- **Three independent reviewer subagents find the issues.** They haven't seen this conversation, so they read what the spec *says*, not what its author meant. Often the author is this very session.
- **You verify what they report**, and the user decides each finding.
- **Nothing in the spec changes without the user's OK.**

Paths below are relative to the project root: the directory that holds `specs/` and `docs/`.

**The facts docs** are `docs/02-github-api.md`, `docs/03-clients.md` and `docs/04-platform.md`. They hold what GitHub's API, the agent clients and the platforms do, each fact with a confidence label: `verified` (measured live, dated), `observed` (seen, its meaning or generality unconfirmed), `documented` (from the vendor's docs, not measured) or `assumed` (not checked). `docs/05-measurements.md` is the dated log the verified facts come from.

## Step 1: Work out the pass

1. The target is the spec file the user names. Accept `03`, `state` or `specs/03-state.md`. If it's ambiguous, ask.
2. **Read the findings log** `specs/reviews/<target basename>`, e.g. `specs/reviews/03-state.md`. It doesn't exist before the first run; create it in step 5.
3. **If the log has findings with status `open`** (raised but not yet decided, for example by a report-only run), offer to walk through those before starting a new pass. Undecided findings are cheaper to finish than to rediscover.
4. **The next pass** is one more than the number of completed passes in the log:
   - pass 1: consistency and correctness
   - pass 2: failure modes and edge cases
   - pass 3: implementability, testability and simplification
   - The user can override this, e.g. "harden 03 for failure modes" or "redo pass 1".
5. **If all three passes are done**, don't run a generic fourth pass; returns drop off fast, and the cost is the user's time. Instead:
   - say so, and list anything still open or deferred in the log
   - offer a targeted lens from `references/lenses.md` (security of the socket and the token, quota cost, agent UX and output size, operability, portability) or stopping

Tell the user in one line what's about to happen, e.g. "Pass 2 on 03-state (failure modes): three reviewers, takes a few minutes."

## Step 2: Three reviewers in parallel

1. **Read `references/lenses.md`.** It splits each lens into three sub-focuses (A, B, C).
2. **Spawn one `general-purpose` subagent per sub-focus, all three in a single message**, so they run concurrently.
3. **Fill in `references/reviewer-brief.md`** for each one:
   - the target
   - the pass and lens, and the sub-focus with its checklist
   - the reading list (below)
   - every finding already in the log (ID, title, status, and the reason for rejected ones), so they're not raised again

**Keep your own suspicions out of the briefs.** A reviewer handed a hypothesis tends to come back with it confirmed, which defeats the point of fresh eyes. If you suspect something, note it privately and check it yourself in step 3.

**Reading list for the brief:**
- **Always:**
  - the target
  - `specs/README.md` (conventions)
  - `specs/00-overview.md`
  - `specs/01-model.md` (every other file uses its field and state names)
  - `specs/decisions.md`
  - `docs/02-github-api.md`
- **Plus**, from this table, the files for the target:

| Target | Also read |
|---|---|
| 01-model | `02-polling`, `04-api`, `docs/05-measurements.md` (latest entries), the recorded responses docs/05 points to |
| 02-polling | `03-state`, `06-operations`, `docs/05-measurements.md`, the recorded responses docs/05 points to |
| 03-state | `02-polling`, `04-api`, `06-operations`, `docs/04-platform.md` |
| 04-api | `03-state`, `05-cli`, `docs/03-clients.md`, `docs/04-platform.md` |
| 05-cli | `04-api`, `06-operations`, `docs/03-clients.md` |
| 06-operations | `02-polling`, `03-state`, `05-cli`, `docs/03-clients.md`, `docs/04-platform.md` |
| 07-testing | every spec file it cites, `docs/05-measurements.md`, the recorded responses docs/05 points to |
| 08-notify | `04-api`, `05-cli`, `02-polling`, `06-operations`, `docs/07-notify.md` (the session socket's facts, and the measurements behind them), `docs/03-clients.md` |
| 00-overview, decisions | all spec files, `docs/01-direction.md` |

- **Also:** any file the target links to, or whose requirement IDs it cites.

This table follows the spec files as first laid out. If `specs/README.md` lists different files, re-derive the rows from its file table: a target's row is the files that share requirement IDs, formats or names with it, plus the facts docs it draws on.

**When sources disagree:**
- The facts docs beat other docs, and beat a fact or a confidence label copied into a spec.
- Within the facts docs, `verified` beats `documented`, and both beat `assumed`.
- The latest dated entries in `docs/05-measurements.md` beat older ones.
- `specs/decisions.md` beats `docs/06-decision-log.md`.
- `docs/01-direction.md` is input to the spec and may be stale. It never decides anything, and stale doc text is not a problem in the spec.

**If you can't spawn subagents** (no Agent tool):
- Run the three sub-focuses yourself, one after another. Re-read the target from the top for each, as if you'd never seen it.
- Tell the user it was a single-reviewer pass.

## Step 3: Verify, merge, rank

Reviewers make mistakes in the same confident tone as their real findings, so check every finding before the user sees it.

- **Check the evidence.** Open the cited lines yourself. Drop a finding when:
  - the quotes don't say what the reviewer claims
  - the spec already handles the issue somewhere else (search for it)
  - it's already in the log, under any status
  - it's settled in `specs/decisions.md`
- **Measure where that's cheap.** If a quick measurement settles a question, run it rather than argue, e.g. against the recorded responses with `jq` or `python3 -I`. A question only the live GitHub API can settle needs the user's OK first: it spends their quota and reads their repositories. If they give it, the result goes into `docs/05-measurements.md` only on their say.
- **Merge findings with the same root cause.** Keep the best evidence and the best fix.
- **Assign severity:**
  - **high:** would tell an agent something false about a PR (checks done, merged, unchanged), exhaust or waste the API quota, lose tracked state, or block implementation
  - **medium:** an implementer would likely get it wrong, or a failure path is missing
  - **low:** clarity, naming, or a small simplification
- **Rank** by severity, then confidence.
- **Walk through at most 12.** Log the rest as `parked`, with their titles, so a later pass or the user can pick them up.
- **Out of scope:** stale `docs/` text, or problems in another spec file that don't affect the target. These go in a short list at the end; they aren't walked through.

Keep a count of what you dropped and why. The user should know verification happened, and how noisy the reviewers were.

## Step 4: Walk through with the user

**If the user asked for a report only** ("just report", "don't change anything", "I'll go through them later"):
- Skip the walk-through.
- Log every verified finding as `open`, with its full card.
- Show the findings, and stop.

**Otherwise:**
1. **Open with one short overview:** pass, lens, the counts (high/medium/low), what verification dropped, and what was parked.
2. **Then go in batches of up to 4 findings.** For each batch:
   - write a compact card per finding in chat
   - make one `AskUserQuestion` call with one question per finding

Card format (an illustration: its requirement IDs and line numbers are invented):

```
**H2-3 · high · problem**: A checks wait can be answered from the previous commit's checks
`specs/04-api.md:58` (API-9) vs `specs/02-polling.md:41` (POL-6)
- Spec says: "`until=checks-done` returns when no check is pending" / "a PR is refreshed at the next tick of its tier"
- Why it matters: an agent pushes and waits at once; until the next tick the cache holds the old head's finished checks, so the wait returns "done" for a commit that hasn't been tested.
- Fix: the wait takes the head SHA it expects; a different cached SHA forces a refresh, and "done" needs checks on that SHA.
```

For each question:
- **`header`:** the finding ID.
- **`question`:** "H2-3: <title>. Apply the fix?"
- **Options:**
  - "Apply fix": mark it (Recommended) when you'd apply it yourself
  - "Reject"
  - "Defer as open decision"
- **`preview`:** the proposed edit as old → new text, whenever there is one. It's often what the user decides on.
- The user can pick "Other" and type their own change. Apply that instead.

If `AskUserQuestion` isn't available, number the findings and ask for replies like "1 apply, 2 reject: not needed, 3 defer".

Don't move to the next batch until this one is decided and applied. Each decision can change what the next findings mean.

## Step 5: Apply and record

**Apply accepted fixes** with Edit, keeping the conventions in `specs/README.md`:
- **Requirement IDs:** use the file's prefix. A new requirement takes the next free number. IDs are never reused, and a dropped requirement is struck through, not deleted.
- **(proposal)** stays on anything the user hasn't approved.
- **Style:** short sentences, British spelling, the key phrase in bold. Match the surrounding text.
- **A fix that spans files** (e.g. the API and the CLI) means editing both. The user approved the fix, not just one file's half of it.
- **Never edit the facts docs or `docs/05-measurements.md`** to make a spec fit. They record what GitHub, the clients and the platforms did. If a doc looks wrong, say so and leave it to the user.

**Record each decision:**
- **Deferred:** add an open decision to `specs/decisions.md` with the next free D-number and your recommendation. Log the finding as `deferred: D<n>`.
- **Rejected:** log the user's reason. That's what stops the next pass from raising it again.
- **Status line:** update the target's, e.g. `**Status:** draft 0.1, 2026-11-25. Hardened: pass 1 (2026-11-26).`
- **Log:** write it (format below).

**End with a short summary:**
- what changed (file:line)
- decisions added
- anything parked or out of scope
- the recommended next step

The next step is the next lens. If this pass found little of high or medium severity, say the file looks hardened for now, and don't recommend another pass.

## Findings log format

`specs/reviews/<target basename>`:

```markdown
# Hardening log: 03-state.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-26 | consistency | 1/4/3 | 6 | 1 | 1 | 0 | 0 |

## Pass 1: consistency (2026-11-26)

Reviewers: 3; dropped in verification: 4 (2 already handled, 1 misread, 1 duplicate).

### H1-1 · high · The state file's path differs between 03-state and 06-operations
- Location: specs/03-state.md STA-7; specs/06-operations.md directories
- Status: applied: both now name the one state directory, and 05-cli's `status` output cites it
```

**Status values:**
- `applied: <what changed>`
- `applied with change: <what>`
- `rejected: <reason>`
- `deferred: D<n>`
- `open`: decided later; for open findings, keep the full card so the walk-through can resume
- `parked`: not walked through

IDs are `H<pass>-<n>` and never reused within a log.

Use today's date (`date +%F`).
