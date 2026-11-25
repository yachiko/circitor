# Lenses

Each pass has one lens, split into three sub-focuses, one per reviewer. The checklists are prompts for thinking, not boxes to tick. A reviewer who finds something important outside its checklist should report it.

**For every item, ask what the spec says happens.** If it says nothing, that may be the finding. If it says something, check that it's right.

## Pass 1: Consistency and correctness

Goal: the spec says true things, and agrees with everything around it.

**A: Facts.** Check every factual claim against the facts docs (`docs/02-github-api.md`, `docs/03-clients.md`, `docs/04-platform.md`), the latest entries of `docs/05-measurements.md` and the recorded responses it points to:
- GraphQL objects, fields, arguments, enum values and nullability; what a field holds before GitHub has computed it
- query cost: points per query, node limits, page sizes, the primary and the secondary rate limits and which bucket a request counts against
- timing: search index lag, how long a lazily computed field stays unknown, when checks first appear after a push, what does and doesn't move `updatedAt`
- client facts: tool timeouts, how a client runs a long or background command, sandbox rules for sockets and localhost, what each client supports
- platform facts: socket path limits, directories, service managers, what Homebrew installs and where
- claims stated as settled that the facts docs label `observed`, `documented` or `assumed`, or list under open items
- behaviour GitHub was measured *not* to have
- unit and name slips (points vs requests, per hour vs per minute, seconds vs milliseconds, PR number vs node ID, check runs vs status contexts, closed vs merged)

**B: Contracts.** Everything the target shares with other spec files:
- **Model fields and state names** exist in `01-model.md`, with the same type, nullability and meaning.
- **Requirement-ID references** (`POL-8`, `STA-2`, …) exist and say what the citing text claims.
- **API fields** match what the model and polling actually produce, and the **CLI's output and exit codes** match the API's responses and outcomes.
- **Paths and names** (the socket, the state file, the config file, the log) agree between state, operations and the CLI.
- **Config keys** in `06-operations` agree with the files that use them.
- **Resolved decisions** in `decisions.md` are respected, and the user requirements (the U numbers in 00-overview) aren't contradicted.

**C: Internal coherence.**
- **Contradictions** within the file.
- **Arithmetic:** recompute every quota budget, points-per-hour figure, cadence, timeout and worst case yourself.
- **Terms** used with one meaning throughout.
- **Examples** consistent with the rules they illustrate: JSON examples carry the fields the tables list, with the types they claim, and command examples use flags that exist.
- **Loose ends:** every `(proposal)` and open item has a home (a decision, or an owner file).

## Pass 2: Failure modes and edge cases

Goal: the spec says what happens when things go wrong, or reach a boundary.

**A: Time and PR-state boundaries.**
- **Clock:** the laptop sleeps and wakes (timers that didn't run, a poll due hours ago), the wall clock jumps backwards or forwards, monotonic and wall time drift apart.
- **Expiry:** a TTL or timeout reached exactly, and during a sleep.
- **Revisions:** a `since` from before a restart, from the future, or from another PR; a change that changes nothing visible.
- **Head commit:** a push or force-push between a poll and a wait; the head SHA the client expects isn't the one cached.
- **Checks:** none registered yet right after a push; a PR with no checks at all; required checks expected but never reported; a rerun that turns a finished check pending again.
- **Lazily computed values:** `mergeable` unknown, and how long it may stay so.
- **PR transitions:** closed without merging, reopened, converted to draft and back, merged through a queue or auto-merge, transferred, its repository renamed, archived, deleted or no longer accessible.
- **Search index lag:** a PR just created that discovery doesn't list yet; a PR just merged that it still lists.
- **Limits:** more PRs than one page, more checks than one page, more waiters than expected.
- **Empty:** no open PRs at all; an unknown PR; values exactly at a threshold.

**B: Infrastructure failure.**
- **Quota:** the primary rate limit exhausted (by circitor, or by other tools sharing the token), a secondary limit with or without `Retry-After`.
- **Token:** missing, expired, revoked, lacking a scope, not authorised for an organisation's SSO; `gh` not installed or logged out.
- **Network:** down, DNS failing, a captive portal, a request that hangs.
- **GitHub:** 5xx, a timeout, a GraphQL response with `data` and `errors` together, a malformed or truncated body.
- **Disk:** a crash or power cut in the middle of writing the state file; disk full; the state or config directory unwritable; a corrupt or truncated state file.
- **Processes:** two daemons started at once; a stale socket file from a crash; a socket path that's too long or has the wrong permissions; a CLI that auto-spawns while another does.
- **Clients:** a client that disconnects in the middle of a wait; many waiters at once; a client that never reads its response.
- **Restarts:** a crash or restart in the middle of a poll, or with waiters connected.
- **Concurrency:** a poll result arriving while a request is being answered; a track request for a PR being refreshed.

**C: Lifecycle.**
- **First run:** no `gh` login, no config file, empty state, no network.
- **Upgrades:** `brew upgrade` with a daemon running (an old daemon and a new CLI, or the reverse); a state file written by an older or a newer version.
- **Config changes** while the daemon runs.
- **TTL expiry:** a merged PR leaving memory while a client still asks for it.
- **Recovery:** a long outage, or a week asleep, and the catch-up afterwards.
- **Growth:** hundreds of PRs, a daemon that runs for months.
- **Idempotency:** tracking a PR twice, untracking one that isn't tracked, starting a daemon that's already running.
- **Manual intervention:** the state file deleted or edited by hand, the socket removed, the daemon killed.
- **Uninstall:** what's left behind.

## Pass 3: Implementability, testability, simplification

Goal: someone could build and test this from the spec alone, and nothing in it is dead weight.

**A: Implementer.**
- Read the file as if you had to implement it tomorrow, with only the specs and docs, and list every question you'd have to ask.
- **Look for:**
  - undefined formats
  - missing defaults and limits
  - vague words ("about", "a few", "promptly")
  - unspecified ordering
  - responsibilities split unclearly between components
  - behaviour left to "the code"
- An agent implementing this will take it literally, so every ambiguity becomes a coin flip.

**B: Tester.**
- For each requirement ID, sketch its acceptance check in one line. Flag:
  - requirements with no observable outcome or threshold
  - normative statements that have no requirement ID
  - behaviour that can't be checked without the live GitHub API or a Mac, where the fake GitHub server and the fake clock could check it if the spec said more

**C: Simplifier.**
- What could be removed, merged or deferred without losing a user requirement (the U numbers)?
- **Look for:**
  - over-specification
  - features nobody asked for
  - the same rule stated in two files (pick one source of truth)
  - `(proposal)` items that add more complexity than value
- These are improvement opportunities, not just problems. Say what gets simpler and what's lost.

## Targeted lenses (after pass 3, on request)

| Lens | Look for |
|---|---|
| Security of the socket and the token | What another local user or process can read or trigger; the socket's and the files' permissions; where the token is held, logged or written; anything that could change something on GitHub |
| Quota cost | Points per hour in each cadence tier and in the worst case; requests nobody is waiting for; what a forced refresh can be made to cost; behaviour when other tools drain the same budget |
| Agent UX and output size | What an agent reads in each state and each failure; output that costs tokens without informing; stale data that looks fresh; outcomes an agent would misread (a timeout taken for success) |
| Operability | What the owner needs to diagnose a problem: logs, status, the quota left, recovery steps |
| Portability | What ties the design to macOS or to Unix sockets; what Linux packaging and a Windows transport would have to change |

## Not a finding

- **Style nitpicks,** unless they could cause a misreading.
- **Stale text in `docs/01-direction.md` or the decision log.** Note it as out of scope.
- **Something marked `(proposal)` or an open decision,** unless the content of the proposal is wrong.
- **"Consider adding X"** with no concrete failure or benefit behind it.
