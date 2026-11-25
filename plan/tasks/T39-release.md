---
id: T39
title: "Owner: decide D5, set up the release tool, publish to the tap"
kind: manual
milestone: M5
size: M
status: todo
depends_on: [T19, T36, T38]
requirements: [OPS-10, U12]
owns: []
shares: []
---

# T39 · Owner: decide D5, set up the release tool, publish to the tap

## Outcome
D5 is decided, releases are built for the four targets, and `brew install yachiko/tap/circitor` installs a binary that runs.

## Context
circitor ships through the owner's Homebrew tap, with prebuilt binaries from GitHub releases (OPS-10, U12). How is the one decision the specs leave open: **D5, formula or cask, and which release tool**. This task is **the owner's**: decide D5, set the tool up, cut the first release, and see `brew install yachiko/tap/circitor` give a binary that runs. T19 wrote the README's install line for the recommended answer; T40's acceptance checks install what this task publishes.

## Read first
- `specs/decisions.md`: D5 and its recommendation. `specs/06-operations.md#shipping`: OPS-10, OPS-11, OPS-13, OPS-15; OPS-18 (no `brew services` in v1).
- `specs/00-overview.md`: U12, and the non-goal "`brew services`".
- `docs/04-platform.md` §2 (the tap, tap trust, quarantine), §3 (`dist`, GoReleaser) and §8 items 1 and 7.
- `README.md` (T19): the install line and what it says about D5.
- `.github/workflows/` (T03): the CI the release workflow sits beside. `Cargo.toml`, `build.rs`, `src/version.rs` (T01): where the version and the commit come from.
- The tap, `yachiko/homebrew-tap`: how the other tools' workflows write to it and sign their commits.

## Scope
**OPS-10 is shared with T19.** T19's share: the README's `brew install yachiko/tap/circitor` line. **This task's share: D5 itself, the release tool, the first release and the tap's entry.** **U12's share here:** shipping through Homebrew, macOS first, with the Linux archives built too; T01 has "Rust, one binary".

- **Close docs/04 §8 item 7 first:** whether GoReleaser builds and releases a Rust binary. Read its documentation, and record the answer with its source in `docs/references.md` and docs/04 §3 (CLAUDE.md: a fact changes only with a source or a dated entry). The recommendation says to check this before deciding.
- **Decide D5.** The options: **a formula written by `dist`** (the recommendation: the maintained path for Rust, and a formula isn't quarantined); **a cask written by GoReleaser** (one tool across the tap, but a cask is quarantined, so its binary must be notarised: an Apple Developer account and signing in CI); or a formula by another route. Move D5 to "Resolved" in `specs/decisions.md` with the date, and have OPS-10 and the README follow.
- **Set up the tool** for OPS-13's four targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`. The release binary is built **without** `test-endpoint` (TST-1). The workflow publishes on a version tag only.
- **The tap:** the release writes circitor's formula (or cask) to `yachiko/homebrew-tap`. Its credential is a repository secret, set by the owner, as the tap's other tools do it. **No `service do` block** and nothing for `brew services` (OPS-10).
- **The first release:** tag `v0.1.0` (OPS-11: `MAJOR.MINOR.PATCH`, with `+<commit>` as build metadata in what `circitor version` prints).
- **Check it:** on the Mac, `brew install yachiko/tap/circitor`, then `circitor version`. On Linux, the archive's binary runs.

## Interfaces
- **Consumes:** the remote and a green CI on Linux and macOS (T38, the run that counts), T36's suite passing on the commit to be tagged, T19's README.
- **Provides:** the release workflow and its configuration, GitHub releases with four archives and their checksums, and `circitor` in the tap. T40 step 1 installs it.

## Out of scope
- The README's other sections, the plist and the systemd unit: T19. Only the install line changes here, and only if D5 isn't the formula.
- The CI workflow: T03. The first CI run: T38.
- Whether the installed binary behaves: T40 (TST-13). Here it only has to install and print its version.
- `brew services`, a menu bar app, Windows, a `gh` extension: non-goals in v1.
- Notarisation, unless D5 is decided as a cask.

## Acceptance
The owner ticks each line.

- [ ] **GoReleaser and Rust checked**, and written into `docs/references.md` and docs/04 §3 with the date.
- [ ] **D5 decided** and moved to "Resolved" in `specs/decisions.md`: formula or cask, the tool, the date, one line of why. OPS-10's "(decision needed, D5)" is gone.
- [ ] **The repository's releases are downloadable without a login** (it's public, or the decision says how the tap reaches a private one).
- [ ] **The tool is set up.** For `dist`: `dist init` with the four targets and the Homebrew installer pointing at `yachiko/homebrew-tap`; `dist plan` lists four archives and a formula. The files it wrote are committed.
- [ ] **The release build has no test code:** `cargo build --release && strings target/release/circitor | grep -c CIRCITOR_TEST_` prints 0.
- [ ] **The tap's secret is set** in the repository's settings, never in a file: `gh secret list` names it.
- [ ] **A dry run passed** before the tag: the tool's own check on a pull request or a pre-release tag builds all four targets.
- [ ] **Tagged and released:** `git tag v0.1.0 && git push origin v0.1.0`; `gh run watch`; `gh release view v0.1.0` lists four archives and checksums.
- [ ] **The tap has it:** `gh api repos/yachiko/homebrew-tap/contents/Formula/circitor.rb --jq .name` (or `Casks/`), and its commit is signed as the tap's others are.
- [ ] **No `service` block:** `grep -c 'service do' Formula/circitor.rb` in the tap prints 0.
- [ ] **On the Mac:** `brew install yachiko/tap/circitor` succeeds with the full name and no `brew trust` step; `circitor version` prints `0.1.0+<the tag's short commit>`; `codesign -dv "$(which circitor)"` is noted (signed ad hoc, or not at all); no Gatekeeper prompt appeared, or its text is noted.
- [ ] **On Linux:** download the `x86_64-unknown-linux-gnu` archive, check its checksum, run `./circitor version`.
- [ ] **`brew uninstall circitor` leaves nothing running:** `circitor stop` first; `pgrep -x circitor` finds nothing.
- [ ] **The README's install line matches what was published** (T19). A cask needs another line: a follow-up to T19.
- [ ] **Recorded:** the decision, the release URL, the tap commit and the two version lines, given to the orchestrator for this task's Evidence. What the Mac showed about an unsigned binary goes into T40's docs/05 entry (docs/04 §8 item 1).

## Traps
- **A formula's URL into a private repository's release answers 404 to Homebrew.** Decide public or private (T38) before the tag, not after.
- **A cask is quarantined; a formula isn't** (docs/04 §2). A cask without notarisation installs a binary macOS refuses to run. That's the whole weight of D5.
- **Since Homebrew 6.0.0 a third-party tap needs trust:** the full name `yachiko/tap/circitor` trusts that one item; the short name fails without `brew trust` (docs/04 §2). Test the line the README gives.
- **`dist init` edits `Cargo.toml`** (or adds `dist-workspace.toml`) and writes its own workflow. `Cargo.toml` is T01's and the plan says no later task edits it: this is the owner's change, made on purpose; run `make verify` after it.
- **Don't let the release job run the tests' feature.** `--features test-endpoint` in a release build ships `CIRCITOR_TEST_GITHUB` to users (TST-1).
- **The version comes from `Cargo.toml`, the commit from `git`** at build time (`build.rs`). A release built from an archive without `.git` prints `+unknown`: check the version line of the released binary, not of a local build.
- **`brew upgrade` only reaches a running daemon through the CLI** (CLI-9, OPS-15): a later release is noticed when the next command replaces the older daemon. Nothing in the formula should try to restart it.
- **Apple Silicon is expected to need at least an ad-hoc signature** (docs/04 §2, assumed). A binary linked on a macOS runner usually has one; a cross-compiled one may not.
- The tap's credential can write to every formula in the tap. Scope it as narrowly as the tap's other tools do.

## Open questions
- **D5 is open**, and this task is where it's decided. Until then T19's README carries the formula form, marked as depending on D5.
- **Whether GoReleaser builds Rust** (docs/04 §8 item 7) and **whether an unsigned binary from a formula runs on Apple Silicon** (item 1) are unverified. The first is checked here by reading; the second here and in T40 by running.
- **This task's `owns` is empty**, though it adds a release workflow and the tool's configuration, and edits `Cargo.toml`, `specs/decisions.md`, OPS-10's wording and docs/04. They're the owner's own edits; tell the orchestrator which files changed.
- **TST-13 says its checks come "before a release", and its first step installs from the tap.** Built: `v0.1.0` is published here and accepted in T40; the owner may prefer a pre-release tag first.
- D6 doesn't touch this task.
