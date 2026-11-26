---
id: T63
title: "Follow-up to T10, T11, T19, T46 and T48: a subagent's notify, in words (D18)"
kind: code
milestone: M5
size: S
status: todo
depends_on: [T10, T11, T19, T35, T46, T48, T49, T57, T59, T60]
requirements: [NTF-7, NTF-11:a-subagent-is-told-nothing, NTF-11:otherwise-it-prints, NTF-12:a-subagent-s-notify-reaches-the-main-conversatio, CLI-11, TST-7, TST-17]
owns: [src/notify/text.rs, src/notify/text/, tests/golden/notify/, src/cli/render/, tests/golden/render/, src/cli/notify/, tests/golden/notify_cmd/, src/cli/guide/, skills/, AGENTS.section.md, README.md, tests/golden/cli/, tests/scn_notify.rs]
shares: []
---

# T63 · Follow-up to T10, T11, T19, T46 and T48: a subagent's notify, in words (D18)

## Outcome
An agent can tell from circitor's own words where a subagent's notify goes. The confirmation says it was asked here or by a subagent (NTF-7). `registered`'s third line says it went to the main conversation, and that a subagent uses `wait` and never cancels (NTF-11). The guide and the README say the same (CLI-11, NTF-12). No behaviour changes: the daemon, the API and `--cancel` stay as they are (D18).

## Context
The owner's field test (2026-11-26, `notify-findings.md` issue 1, kept on the `docs/notify-findings` branch): a subagent's `notify` registered the main conversation, because a subagent's shell has its session's socket and token (docs/07). The confirmation and the notice arrived there. The CLI's own line then told the subagent, which saw neither, to `--cancel`, and a cancel drops every notify of that socket on the PR, the main conversation's too (NTF-3). circitor can't tell a subagent from its session (D18's note), so only the text changes.

## Read first
- `plan/conventions.md`.
- `specs/08-notify.md`: NTF-7 (the confirmation and the paragraph after its examples), NTF-11 ("A subagent is told nothing" and "Otherwise it prints"), NTF-12's last bullet.
- `specs/05-cli.md`: CLI-11. `specs/07-testing.md`: TST-7, TST-17. `specs/decisions.md`: D18.
- `src/notify/text.rs`: `confirmation` (around line 55) and `watching`. `src/cli/render/mod.rs`: the third line (around line 415). `src/cli/guide/guide.md` and `tests.rs`: `cli11_guide_is_one_text`.

## Scope
- **The confirmation** (NTF-7, T46's): `circitor: watching <watching>, asked here or by a subagent. If you can read this, notices reach this session.` `text::watching` doesn't change: `registered:` doesn't carry the clause.
- **`registered`'s third line** (NTF-11, T10's): exactly `a confirmation was posted to this session's main conversation; if none arrives there, notices are held or refused: run circitor notify <pr> --cancel, then use circitor wait. A subagent never sees it: it uses circitor wait, and never cancels.`
- **The guide** (CLI-11, T11's), in `guide.md`, `skills/circitor/SKILL.md` and `AGENTS.section.md` alike. The "If none arrives" bullet becomes the main conversation's. It says a confirmation may come from one of its subagents. A new bullet says a subagent uses `wait`, never `notify` or `notify --cancel`, because its notice goes to the main conversation and its cancel drops the main conversation's notifies on that PR. Keep it under 120 lines.
- **The README** (NTF-12, T19's): one bullet in "Notices in Claude Code": a subagent's notify reaches the main conversation, not the subagent, so a subagent uses `circitor wait`.
- **Golden files** (TST-7): rewrite those that hold either text, with `CIRCITOR_UPDATE_GOLDEN=1`, and read every diff. Expect `tests/golden/notify/confirmation_*`, `tests/golden/render/registered*`, `tests/golden/notify_cmd/registered*`, `tests/golden/cli/notify_registered*` and `tests/golden/cli/notice_confirmation_*`. Also update the expected confirmation in `tests/scn_notify.rs` (around line 288), and any string asserted in `src/cli/notify/tests.rs` and `src/cli/render/tests.rs`.
- **TST-17's test**, in `src/cli/guide/tests.rs`: `tst17_guide_sends_a_subagent_to_wait` checks that `GUIDE` names subagents, sends them to `wait`, and tells them never to cancel. `cli11_guide_is_one_text` already proves that the skill and the `AGENTS.md` section are the same text.

## Out of scope
- Any change to `--cancel`, NTF-3 or the daemon (D18: text only).
- Detecting a subagent: there's nothing to detect (D18).
- `docs/`: recording the subagent measurement is T64's.
- The `stopped` notice, `stop`'s words and the spawner (issue 2): T65 (D19).

## Acceptance
- `cargo test` names that pass: `tst17_guide_sends_a_subagent_to_wait`, `cli11_guide_is_one_text`, the confirmation and registered golden tests in `src/notify/text/tests.rs`, `src/cli/render/tests.rs` and `src/cli/notify/tests.rs`, and `cargo test --features test-endpoint --test golden_cli --test scn_notify`.
- `grep -rn 'notices are held or refused here' src tests skills AGENTS.section.md README.md` finds nothing.
- `make verify` passes.

## Open questions
- NTF-11's subagent sentence is a (proposal) until T64 records the measurement. Build it as written.
