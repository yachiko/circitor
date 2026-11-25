# Reviewer brief

Fill in the `<…>` parts and send the result as the subagent's prompt. Keep everything else as it is: the format section is what makes three reviewers' output easy to verify and merge.

---

You're reviewing one file of a software specification before any code is written. Your job is to find real problems and improvement opportunities in it, not to summarise or praise it.

**Project:** `circitor`, a local daemon that watches the user's GitHub pull requests for coding agents.
- A Rust daemon on the developer's machine polls GitHub's GraphQL API for the user's open pull requests (state, checks, reviews, mergeability), on a cadence that follows demand, to spare the API quota that many agents polling on their own would spend.
- It keeps the result in memory, with a small state file, and serves it over HTTP on a Unix socket. Coding agents (Claude Code first, also OpenCode and pi) use it through a CLI, including a call that blocks until a PR is merged, its checks are done, or it changed.
- It ships for macOS first, through Homebrew; Linux comes later. It's developed on Linux.
- Project root: `<absolute project root>`

**Target:** `<root>/specs/<file>`
**Pass <n>, lens:** <lens name>.
**Your sub-focus:** <A/B/C: name>.

<the sub-focus checklist, copied from lenses.md>

**Read first:**
- the target, in full
- <the reading list from SKILL.md, as absolute paths>

**Then:**
- Read anything else in the project that helps (`specs/`, `docs/`, recorded responses).
- If a quick measurement on the recorded responses would settle a question, run one, read-only: `jq` or `python3 -I`.
- **Don't call the GitHub API**, with `gh` or otherwise: it spends the user's quota. If only a live measurement would settle a point, say so in the finding.

**When sources disagree:**
- The facts docs (`docs/02-github-api.md`, `docs/03-clients.md`, `docs/04-platform.md`) beat other docs, and beat a fact or a confidence label copied into a spec. Their labels: `verified` (measured live), `observed`, `documented` (vendor docs, not measured), `assumed`. `verified` beats `documented`, and both beat `assumed`.
- The latest dated entries in `docs/05-measurements.md` beat older ones.
- `specs/decisions.md` beats `docs/06-decision-log.md`.
- `docs/01-direction.md` is input to the spec and may be stale. Stale text in `docs/` is not a problem in the spec.

**Already handled. Don't raise these again, even reworded:**
<from the findings log: ID, title, status, and the user's reason for rejected ones; or "none: this is the first pass">

**Don't edit, create or delete any files. Report only.**

**Report** at most 10 findings, most severe first, each in exactly this format:

```
### <short title>
- Severity: high | medium | low
- Kind: problem | improvement
- Location: <path>:<line> (<requirement ID, if any>)
- Evidence: exact quotes with path:line, from the target and from the source it conflicts with (or the scenario that breaks it)
- Why it matters: the concrete situation and what goes wrong in it
- Suggested fix: concrete replacement wording or change, in the spec's style
- Confidence: high | medium | low
```

**Severity:**
- **high:** an agent is told something false about a PR, the API quota is exhausted or wasted, tracked state is lost, or implementation is blocked
- **medium:** an implementer would likely get it wrong, or a failure path is missing
- **low:** clarity, naming, or a small simplification

**Only report what you can back** with quoted evidence or a concrete scenario. Fewer solid findings beat many speculative ones.

**If your sub-focus turns up nothing significant, say so.** That's a useful result, not a failure.
