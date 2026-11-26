//! The words of every notice (NTF-7, NTF-8; T46). Pure. **These functions are the only place a
//! notice's words exist.** Durations are `cli::render::duration`, the outcome line is
//! `cli::render::outcome_line`, the flags are `cli::render::flags`: called, never copied.
//!
//! NTF-8: nothing a GitHub user wrote is in a notice but the PR's name. These functions take no title,
//! no check's name, no login and no URL: the outcome line holds counts and short SHAs, and `status`,
//! the stale mark and the problem's code are circitor's own words. A notice ends without a newline.

use std::fmt::Write as _;
use std::time::Duration;

use crate::api::{Ask, Outcome, Reason};
use crate::cli::render::{duration, flags, outcome_line};
use crate::model::{PrName, Record, Status, short_sha};
use crate::state::Cursor;

use super::ProblemWhy;

/// What a notice says of its subscription, in the client's own words.
#[derive(Clone, Debug, PartialEq)]
pub struct Told {
    pub pr: PrName,
    pub cond: Ask,
    pub life: Duration,
    pub life_given: bool,
    pub cursor: Cursor,
}

/// NTF-7's one rule for the condition's words, which the confirmation and the CLI's `registered:` line
/// both hold: `<pr> until <until>`, then, each only where given and in this order, ` on <sha7>` (the
/// first 7 characters, lower case), ` after <rerun_since>`, ` fail-fast`, ` kind <kind>` and
/// ` since <since>`, each as given; then `, for <life>` (`render::duration`).
// NTF-7: the condition is written the same way in the confirmation and in `registered:`.
pub fn watching(pr: &PrName, cond: &Ask, life: Duration) -> String {
    let mut s = format!("{pr} until {}", cond.until.as_str());
    if let Some(sha) = &cond.sha {
        let _ = write!(s, " on {}", short_sha(sha).to_ascii_lowercase());
    }
    if let Some(rerun) = &cond.rerun_since {
        let _ = write!(s, " after {rerun}");
    }
    if cond.fail_fast {
        s.push_str(" fail-fast");
    }
    if let Some(kind) = &cond.kind {
        let _ = write!(s, " kind {kind}");
    }
    if let Some(since) = &cond.since {
        let _ = write!(s, " since {since}");
    }
    let _ = write!(s, ", for {}", duration(life));
    s
}

/// NTF-7: `circitor: watching <watching>. If you can read this, notices reach this session.`
pub fn confirmation(told: &Told) -> String {
    format!(
        "circitor: watching {}. If you can read this, notices reach this session.",
        watching(&told.pr, &told.cond, told.life)
    )
}

/// NTF-8: `circitor: <pr> <outcome line>. Details: circitor changes <pr> --since <cursor>`. With
/// `history: false` (the cursor is too old as the notice is built, STA-9):
/// `circitor: <pr> <outcome line>. History too old; details: circitor show <pr>`.
pub fn outcome(
    told: &Told,
    end: (Outcome, Option<Reason>),
    record: Option<&Record>,
    history: bool,
) -> String {
    let line = outcome_line(
        end.0,
        end.1,
        record,
        told.cond.until,
        told.cond.sha.as_deref(),
    );
    let pr = &told.pr;
    if history {
        format!(
            "circitor: {pr} {line}. Details: circitor changes {pr} --since {}",
            told.cursor
        )
    } else {
        // NTF-8, STA-9: a cursor too old as the notice is built names `show`.
        format!("circitor: {pr} {line}. History too old; details: circitor show {pr}")
    }
}

/// `circitor: notify for <pr> expired after <life>; it's <status><stale>. Ask again: <ask again>` (NTF-8).
pub fn expired(told: &Told, status: Status, stale: &str) -> String {
    format!(
        "circitor: notify for {} expired after {}; it's {}{stale}. Ask again: {}",
        told.pr,
        duration(told.life),
        status_word(status),
        ask_again(told)
    )
}

/// `circitor: can't read <pr> from GitHub (<why>); notify is cancelled. Details: circitor status` (NTF-8).
pub fn problem(told: &Told, why: ProblemWhy) -> String {
    format!(
        "circitor: can't read {} from GitHub ({}); notify is cancelled. Details: circitor status",
        told.pr,
        why.as_str()
    )
}

/// `circitor: the daemon is stopping; notify for <pr> is cancelled. Ask again: <ask again>`, then
/// ` --no-spawn` when `no_spawn` (a stop someone asked for), and nothing after it otherwise (NTF-10).
pub fn stopped(told: &Told, no_spawn: bool) -> String {
    let mut s = format!(
        "circitor: the daemon is stopping; notify for {} is cancelled. Ask again: {}",
        told.pr,
        ask_again(told)
    );
    // NTF-10: a stop the daemon chose itself posts the command without `--no-spawn`.
    if no_spawn {
        s.push_str(" --no-spawn");
    }
    s
}

/// `circitor notify <pr> `, `cli::render::flags(&told.cond)`, ` --for <seconds>` only if it was given,
/// then ` --since <told.cursor>`: the subscription's cursor, as CLI-6's `again:` sets it (NTF-8).
pub fn ask_again(told: &Told) -> String {
    let mut s = format!("circitor notify {} {}", told.pr, flags(&told.cond));
    if told.life_given {
        let _ = write!(s, " --for {}", told.life.as_secs());
    }
    let _ = write!(s, " --since {}", told.cursor);
    s
}

#[cfg(test)]
mod tests;

/// A status's JSON word (`checks_pending`), as CLI-3 prints it.
fn status_word(status: Status) -> String {
    match serde_json::to_value(status) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}
