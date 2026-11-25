# Hardening log: 06-operations.md

| Pass | Date | Lens | Found (H/M/L) | Applied | Rejected | Deferred | Open | Parked |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-11-25 | consistency | 1/9/8 | 12 | 0 | 0 | 0 | 6 |
| 2 | 2026-11-25 | failure modes | 0/9/5 | 12 | 0 | 0 | 0 | 2 |
| 3 | 2026-11-25 | implementability | 0/6/8 | 12 | 0 | 0 | 0 | 2 |
| 4 | 2026-11-25 | planning findings | 0/3/4 (+1 decision record) | 8 | 0 | 0 | 0 | 0 |

## Pass 1: consistency (2026-11-25)

Reviewers: 3, with 36 raw findings; 18 after merging duplicates (the shutdown order, the logging rule, the launchd `PATH` and OPS-15 were each raised by two or three). Dropped in verification: 3 (2 already parked in other logs: the spend count lost at a restart, 02-polling H2-13; the spawned daemon's stderr on a rotated log, 05-cli H1-13; 1 out of scope).

### H1-1 · high · `checks.settle_s` could be set below the time checks take to appear
- Location: specs/06-operations.md OPS-2; specs/01-model.md MOD-6; docs/02 §4 (up to 9 s, verified)
- Status: applied: the range is 15–300 s, with a note under the table saying why

### H1-2 · medium · Shutdown closed the waiters while the socket still accepted, and always wrote the state file
- Location: specs/06-operations.md OPS-7, OPS-8; specs/05-cli.md CLI-10; specs/03-state.md STA-8
- Status: applied: OPS-7 stops accepting and removes the socket first, so a reconnecting waiter starts a new daemon; it writes the state file unless STA-8 left it alone; OPS-8's idle exit stops as OPS-7 does

### H1-3 · medium · A CLI with a different environment couldn't reach a running daemon
- Location: specs/06-operations.md OPS-4; specs/05-cli.md CLI-7; specs/03-state.md STA-11
- Status: applied: a CLI that can't connect tries the socket path in `circitor.lock` before spawning, with a warning naming both paths; CLI-7 follows. Covers the CLI half of 03-state H2-18.

### H1-4 · medium · Under launchd `gh` isn't on the `PATH`, and the error told the user to log in
- Location: specs/06-operations.md OPS-5, OPS-6; specs/04-api.md API-3; docs/04 §2
- Status: applied: `gh` is also looked for next to circitor's own binary; a `gh` that can't be run is reported as such, naming `gh_path`, in `status` and in API-3's `auth` message

### H1-5 · medium · "Never logged: anything from a response body" forbade what OPS-9 and POL-24 log
- Location: specs/06-operations.md OPS-9; specs/02-polling.md POL-24
- Status: applied: free text from a response is never logged; names, numbers, states, costs and error `type`s may be. The Logged list adds API-5's warnings, STA-7's failed write, POL-3's failed token read and POL-24's per-PR error.

### H1-6 · medium · The daemon's token depended on which shell spawned it
- Location: specs/06-operations.md OPS-5; specs/05-cli.md CLI-7; specs/04-api.md API-3; D4, U11, MOD-11
- Status: applied: a spawned daemon uses gh's login only, since CLI-7 starts it without `GH_TOKEN` and `GITHUB_TOKEN`; a foreground daemon still reads them; API-3's `auth` fix says so

### H1-7 · medium · A slow `gh auth token` could outlast the CLI's 8 s spawn wait
- Location: specs/06-operations.md OPS-5; specs/03-state.md STA-11; specs/05-cli.md CLI-7
- Status: applied: the token is read after the socket is bound; until the first read ends, the first poll and read-throughs wait for it, and `status` answers at once; STA-11's start order follows

### H1-8 · medium · The ranges let the cadences be inverted
- Location: specs/06-operations.md OPS-1, OPS-2; specs/02-polling.md POL-18, POL-23
- Status: applied: OPS-1 requires `hot_interval` ≤ `interval` ≤ `idle_interval`, checked like a range

### H1-9 · medium · OPS-14 relied on a socket rule unconfirmed on Linux
- Location: specs/06-operations.md OPS-14; specs/05-cli.md CLI-7; docs/03 §2
- Status: applied: the README gives `sandbox.excludedCommands` (CLI and daemon outside the sandbox) or `allowUnixSockets` (Linux rule assumed); CLI-7's exit-6 message names both

### H1-10 · medium · `keep_done` up to 30 days let ended PRs fill STA-3's 200 places
- Location: specs/06-operations.md OPS-2; specs/03-state.md STA-3
- Status: applied: the range stops at 7 days, with a note under the table

### H1-11 · low · OPS-15 said every old daemon is replaced; CLI-9 never replaces a foreground one
- Location: specs/06-operations.md OPS-15; specs/05-cli.md CLI-9
- Status: applied: a foreground daemon runs the old version until it's restarted, and the CLI warns until then

### H1-12 · low · OPS-10's "either works" ignored cask quarantine
- Location: specs/06-operations.md OPS-10; docs/04 §2; D5
- Status: applied: a cask needs a notarised binary, a formula doesn't; the README installs with the full name, which trusts the tap item

### H1-13 · low · OPS-17's targets can't be met as written
- Location: specs/06-operations.md OPS-17; specs/02-polling.md POL-2, POL-7; specs/04-api.md API-18
- Status: parked
- Note: a cold start with 200 PRs is 7 requests and 6 s of POL-2 gaps before any network time; "no CPU between polls" ignores STA-11's and POL-22's timers. Fix idea: "within 5 s with up to 40 PRs (latency isn't measured, docs/02)"; "no CPU between polls beyond its timers".

### H1-14 · low · What keeps a spawned daemon alive, and on which clock, isn't said
- Location: specs/06-operations.md OPS-8; specs/04-api.md API-5; specs/02-polling.md POL-13, POL-26
- Status: parked
- Note: API-5 says `status` counts no activity for POL-13, but OPS-8 says "no client request"; `stop` and refused requests are unclear; POL-26 doesn't name `exit_after`'s clock.

### H1-15 · low · OPS-12 says the skill doesn't reach pi (applied in pass 4: H4-6)
- Location: specs/06-operations.md OPS-12; docs/03 §1, §4
- Status: parked
- Note: pi and OpenCode v2 read `.agents/skills`; only OpenCode v1 needs the `AGENTS.md` section. pi puts only `name` and `description` in the prompt.

### H1-16 · low · "Talks to nothing else" overlooks `gh`'s token refresh
- Location: specs/06-operations.md OPS-16; docs/02 §1
- Status: parked
- Note: `gh auth token` may refresh an expiring token with github.com. Fix idea: "circitor itself connects only to `api.github.com`".

### H1-17 · low · OPS-4 states a temp-directory fact as settled, and applies macOS's socket limit on Linux unstated
- Location: specs/06-operations.md OPS-4; docs/04 §4, §8
- Status: parked
- Note: docs/04 says files "may be removed" and lists it as open item 3; 103 bytes is refused on Linux too (limit 107) without saying it's deliberate; the macOS open item cites §8, whose socket item is about `$TMPDIR`, not Application Support.

### H1-18 · low · `version` with the commit isn't pinned down for CLI-9's semver comparison
- Location: specs/06-operations.md OPS-11; specs/05-cli.md CLI-9
- Status: parked
- Note: `0.1.0 (abc1234)` doesn't parse as semver; `0.1.0+abc1234` makes two builds of one version equal. Fix idea: build metadata, compared by precedence; one that doesn't parse counts as older.

### Out of scope
- specs/05-cli.md CLI-1: `circitor daemon --spawned` and `--check-config` (OPS-1, OPS-6) aren't in the command table.

## Pass 2: failure modes (2026-11-25)

Reviewers: 3 (time and PR-state boundaries; infrastructure failure; lifecycle), with 23 raw findings; 15 after merging duplicates (requests dropped at a stop and `gh auth token`'s outcomes were each raised by all three; the 5 s stop, the Cellar path and a config error by two; the hung daemon was merged with the lock file's format). Dropped in verification: 1 already parked elsewhere (a config edit that never applies: 05-cli H2-21, 04-api H2-18).

Another session hardened 05-cli and 04-api during this pass and edited 06-operations. Its CLI-10 (a dropped wait starts no daemon; any other command is tried once more), CLI-7 (watching the child, request limits) and CLI-9 (`--check-config` before replacing) were kept. H2-1 and H2-3 were applied on top of them, and H2-7 shrank to what they didn't cover.

### H2-1 · medium · A stop drops requests that aren't waits, and only `wait` retried
- Location: specs/06-operations.md OPS-7; specs/04-api.md API-14; specs/05-cli.md CLI-10
- Status: applied: OPS-7 and API-14 close every open connection without an answer, waiters' and others' alike; the other session's CLI-10 repeats them. Covers 04-api H2-13.

### H2-2 · medium · OPS-7's 5 s wasn't enforced, and repeated signals and `SIGHUP` weren't covered
- Location: specs/06-operations.md OPS-7; specs/03-state.md STA-7, STA-11
- Status: applied: the stop has a 5 s deadline; the final write skips STA-7's spacing, `gh` is killed; past it, exit 1; a second signal does the same at once; `SIGHUP` stops it as `SIGTERM`; a failed final write exits 1

### H2-3 · medium · A config error wasn't fully defined, and in a spawned daemon it cost every call 8 s
- Location: specs/06-operations.md OPS-1; specs/05-cli.md CLI-7, CLI-9
- Status: applied with change: only a missing file means the defaults; unreadable, invalid TOML or a wrong type stops the daemon; the config is read first, its error on stderr. The CLI half was already done by the other session (CLI-7 watches the child, CLI-9 checks before replacing), so OPS-1 points to it instead of a separate pre-check.

### H2-4 · medium · A foreground daemon lost to a spawned one, so OPS-14's advice did nothing
- Location: specs/06-operations.md OPS-6, OPS-14; specs/03-state.md STA-11
- Status: applied: a foreground daemon asks a spawned holder to shut down and takes the lock; a second foreground daemon exits with an error naming the holder's socket; STA-11 follows. Covers 04-api's out-of-scope note on STA-11.

### H2-5 · medium · `gh auth token`'s outcomes weren't told apart, so a timeout said "log in" and a logout was ignored
- Location: specs/06-operations.md OPS-5; specs/02-polling.md POL-3; specs/04-api.md API-3, API-5
- Status: applied: three outcomes (a token; no login, which drops the token; a failure, which keeps it and never says log in); one read at a time; `gh`'s output never logged; the login logged and in `status`. Rests on docs/04 §8 item 4 (Keychain), so (proposal).

### H2-6 · medium · "Next to circitor's own binary" wasn't Homebrew's `bin`
- Location: specs/06-operations.md OPS-5
- Status: applied: Homebrew's `bin` by its fixed paths, never where the binary resolves to; `gh` looked up again at each read (proposal; Homebrew's layout isn't in docs/04)

### H2-7 · medium · The lock file's content had no format, and nothing could stop a hung daemon
- Location: specs/03-state.md STA-11; specs/06-operations.md OPS-4; specs/05-cli.md CLI-8
- Status: applied: once locked, the file is truncated and holds one line, the process ID and the socket path; a reader takes only a complete line; `stop` with no answer sends `SIGTERM`, then `SIGKILL` 5 s later, while that lock is held

### H2-8 · medium · Paths that already existed were trusted: the directory's mode, and whatever sat at the socket path
- Location: specs/06-operations.md OPS-3, OPS-4
- Status: applied: an existing directory of the user's is set to 0700, anyone else's stops the daemon (also the socket's directory); the socket is bound under umask 077; at start only a socket that refuses connections is removed. 04-api H2-19 (peer user ID) stays parked for the security lens.

### H2-9 · medium · Running under a service manager was offered, but nothing said how
- Location: specs/06-operations.md OPS-6, OPS-15
- Status: applied: new OPS-18 (proposal): the README's plist and unit run Homebrew's `bin/circitor`, restart on failure only, keep stderr, and name the restart command; OPS-15 cites it

### H2-10 · low · `checks.settle_s` up to 300 s made a default `wait` after a push time out
- Location: specs/06-operations.md OPS-2; specs/01-model.md MOD-6; specs/04-api.md API-12
- Status: applied: the range is 15–30 s, the note says three times it stays within API-12's 100 s

### H2-11 · low · The log didn't say why the daemon stopped or note a sleep, and its time zone wasn't stated
- Location: specs/06-operations.md OPS-9
- Status: applied: the stop's cause and a noticed sleep are logged; times are RFC 3339, UTC

### H2-12 · low · Nothing said what happens when a log line or a rotation can't be written
- Location: specs/06-operations.md OPS-9
- Status: applied: logging never stops the daemon; a failed line is dropped, a failed rotation retried at the next 5 MB; either raises a warning

### H2-13 · low · Proxies and TLS roots aren't stated, so circitor can fail where `gh` works
- Location: specs/06-operations.md OPS-16; specs/00-overview.md (reqwest with rustls)
- Status: parked
- Note: behind a proxy or a TLS-inspecting network `gh` works and circitor sits in POL-19's backoff; whether a spawned daemon has the proxy variables depends on the spawning shell. Fix idea: honour `HTTPS_PROXY` and `NO_PROXY`, trust the system's store, log a transport failure's kind.

### H2-14 · low · The skill and the `AGENTS.md` section go stale across upgrades
- Location: specs/06-operations.md OPS-12; specs/05-cli.md CLI-11
- Status: parked
- Note: copied once from the repository, they drift from the installed binary. Fix idea: keep them to "when to use circitor; run `circitor guide`", and add `circitor guide --skill`.

### Out of scope, pass 2
- specs/07-testing.md TST-9, TST-11: the stop's deadline, the foreground takeover, `stop` on a hung daemon, the three `gh` outcomes and a failing log write have no test yet.
- docs/04-platform.md: Homebrew's keg layout and what Rust's `current_exe` returns for a link aren't recorded; OPS-5 cites them as (proposal) until a source is added to references.md.

## Pass 3: implementability, testability, simplification (2026-11-25)

Reviewers: 3 (implementer, tester, simplifier), with 27 raw findings; 14 after merging duplicates (the foreground lock race, the `gh auth token` outcomes and the `CIRCITOR_SOCKET` directory rule were each raised by two or three; the relative socket path, `gh_path`, `--check-config` and the log's names by two). Dropped in verification: none.

Measured locally, no GitHub API call and no token printed (docs/05, 2026-11-25): `gh auth token` with no login exits 1, `no oauth token found for github.com` on stderr, nothing on stdout; it prints `GH_TOKEN` or `GITHUB_TOKEN` when set, and ignores an empty one. docs/02 §1 updated to match, on the owner's say.

Another session hardening 05-cli edited 06-operations during this pass: OPS-4's lock-file bullet and OPS-7's waiter sentence now point to CLI-7 and CLI-10 (part of H3-12, and the drift it found), and OPS-11 defines `version` as `MAJOR.MINOR.PATCH+<commit>`, which answers parked H1-18.

### H3-1 · medium · A foreground daemon could lose the lock to a fresh spawned one and exit 0, leaving its service down
- Location: specs/06-operations.md OPS-6; specs/03-state.md STA-11; specs/05-cli.md CLI-10
- Status: applied: the foreground daemon repeats the takeover until it holds the lock or 15 s pass, then exits 1, never 0; STA-11 retries every 100 ms

### H3-2 · medium · Nothing told "no login" apart from "a failure" of `gh auth token`
- Location: specs/06-operations.md OPS-5; docs/02 §1
- Status: applied: a token is stdout's one line, stderr ignored; no login is a non-zero exit with `no oauth token found` on stderr, read for that only; anything else is a failure. The local measurement went to docs/05 and docs/02 §1.

### H3-3 · medium · `CIRCITOR_SOCKET` stopped the daemon in `/tmp`, chmodded `$HOME`, and a relative path differed between CLI and daemon
- Location: specs/06-operations.md OPS-3, OPS-4; specs/05-cli.md CLI-7
- Status: applied: the directory rule covers circitor's own directories; a `CIRCITOR_SOCKET` directory is used as it is; the override must be absolute; an empty one, or an empty or relative XDG variable, counts as unset. A socket in a shared directory leans on the peer check parked as 04-api H2-19.

### H3-4 · medium · A stop that was asked for could exit 1, so a service manager restarted it
- Location: specs/06-operations.md OPS-7, OPS-18; specs/05-cli.md CLI-8
- Status: applied: every stop exits 0, problems logged, status 1 is for a crash; OPS-18's README says a daemon `stop` had to kill is restarted, and how to stop a service

### H3-5 · medium · "Answers" had no probe and no time limit in the daemon's own checks
- Location: specs/06-operations.md OPS-4, OPS-6; specs/03-state.md STA-11
- Status: applied: OPS-4 defines it (a complete `GET /v1/status` response within 2 s); STA-11 cites it

### H3-6 · medium · Before the first token read ended, the daemon both held requests and was in `auth`
- Location: specs/06-operations.md OPS-5; specs/02-polling.md POL-20; specs/04-api.md API-9
- Status: applied: until the first read ends the daemon isn't in `auth`, and every request that needs GitHub waits for it; POL-20's `auth` begins when the first read ends without a token

### H3-7 · low · `gh_path` didn't say how a path differs from a name, or whether a wrong path falls back
- Location: specs/06-operations.md OPS-2, OPS-5
- Status: applied: a `gh_path` with `/` is absolute and the only one tried; a name is looked up in `PATH`, then Homebrew's `bin`; `status` shows the path run. The simplifier's alternative (drop the fixed Homebrew paths, set `PATH` in OPS-18's files) wasn't taken.

### H3-8 · low · The daemon read `GH_TOKEN` and `GITHUB_TOKEN` itself, which `gh auth token` already does
- Location: specs/06-operations.md OPS-5; specs/04-api.md API-3
- Status: applied: the token source is `gh auth token` alone; the source in use is the variable set or gh's login; OPS-5 says `GH_CONFIG_DIR` and `XDG_CONFIG_HOME` still come from the spawning shell

### H3-9 · low · `--check-config` had no exit status or output, yet CLI-9 relied on it
- Location: specs/06-operations.md OPS-1; specs/05-cli.md CLI-9
- Status: applied: exit 0 when missing or valid, else exit 1 with the one-line message on stderr; CLI-9 checks in its own process

### H3-10 · low · The log's format, levels and old copy were unnamed, and rotating while running added machinery
- Location: specs/06-operations.md OPS-6, OPS-9; specs/05-cli.md CLI-7; specs/03-state.md STA-12
- Status: applied: `<time> <level> <message>`, four levels, `info` default; both modes log to stderr; the CLI, not the daemon, rotates a log over 5 000 000 bytes to `circitor.log.1` before starting a daemon (the daemon rotating at its start would break CLI-7's reading of the log's last lines); the retry and its warning are gone. Settles 05-cli H1-13.

### H3-11 · low · `checks.settle_s` was a constant with a 15-second dial
- Location: specs/06-operations.md OPS-2; specs/01-model.md MOD-6; specs/07-testing.md TST-8
- Status: applied: the key and its note are gone; MOD-6 states a fixed 20 s with the reasoning, as (proposal); TST-8's `no-ci` follows

### H3-12 · low · OPS-1 and OPS-15 restated CLI-9 and CLI-7
- Location: specs/06-operations.md OPS-1, OPS-15
- Status: applied: pointers to CLI-7 and CLI-9 in their place (the other session had already done OPS-4 and OPS-7)

### H3-13 · low · OPS-2 doesn't give each key's TOML type, and `owners` has no range
- Location: specs/06-operations.md OPS-1, OPS-2
- Status: parked
- Note: a wrong type is fatal (OPS-1) but no type is stated: `30.0` or `"30s"`, tables or dotted keys, an empty or duplicate login. Fix idea: a Type column (integer seconds; integer; an array of logins, 1–39 letters, digits and hyphens, duplicates without case are one; a string), and "`checks` and `quota` are tables".

### H3-14 · low · OPS-9's "Logged" list repeats itself and the source specs
- Location: specs/06-operations.md OPS-9; specs/02-polling.md POL-3
- Status: parked
- Note: quota warnings and the set-aside state file are API-5 warnings already; POL-3 says a failed periodic token read is logged once, OPS-9 doesn't say once. Fix idea: "every `daemon.problem` and API-5 warning, when it starts and ends, and whatever a requirement says is logged".

### Out of scope, pass 3
- specs/reviews/05-cli.md H1-13 (parked there) is settled by H3-10; that log isn't updated here.
- specs/07-testing.md: no test yet for the foreground takeover's retry and exit 1, `gh auth token`'s outcomes with a fake `gh` through `gh_path`, `--check-config`'s exit status, or the CLI's log rotation.

## Pass 4: planning findings (2026-11-25)

No reviewers: the findings came from building the implementation plan (`plan/README.md`, "Spec issues found while planning") and were verified against the spec text and the facts docs. Out of scope: what API-5's `paths.state` names (04-api).

### H4-1 · medium · OPS-4's umask gives the wrong mode
- Location: specs/06-operations.md OPS-4; specs/07-testing.md TST-9
- Status: applied: OPS-4 binds under a umask of 0177, and says a umask of 077 gives 0700 on Linux (a local bind, 2026-11-25; macOS not checked)

### H4-2 · medium · OPS-9 raises a warning that has no code
- Location: specs/06-operations.md OPS-9; specs/04-api.md API-5
- Status: applied: API-5 gains `log_unwritable`; OPS-9 names it

### H4-3 · low · `status` shows `gh`'s path, but API-5 has no key for it
- Location: specs/06-operations.md OPS-5; specs/04-api.md API-5
- Status: applied: API-5 gains `gh_path`

### H4-4 · medium · Whether `status` keeps a spawned daemon alive
- Location: specs/06-operations.md OPS-8; specs/04-api.md API-5
- Status: applied: OPS-8 runs on the monotonic clock and doesn't count `status`. Also settles part of H1-14.

### H4-5 · low · OPS-9 logs a stop cause the daemon can't see
- Location: specs/06-operations.md OPS-9; specs/04-api.md API-1, API-2, API-17; specs/05-cli.md CLI-9
- Status: applied: "replaced" and `stop` both log as a shutdown request

### H4-6 · low · OPS-12 says the skill doesn't reach pi
- Location: specs/06-operations.md OPS-12; docs/03-clients.md §1
- Status: applied: pi is named; also closes H1-15

### H4-7 · low · OPS-18's plist has no label and no stderr path
- Location: specs/06-operations.md OPS-18
- Status: applied: label `dev.circitor.daemon`, stderr at `~/Library/Logs/circitor.log` (proposal)

### H4-8 · decision · D5 and D6 decided by the owner
- Location: specs/decisions.md; specs/06-operations.md OPS-10
- Status: applied: both resolved on 2026-11-25; OPS-10 names a formula written by `dist`

