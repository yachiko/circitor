#![allow(unused_variables, dead_code)] // stub: T46 removes this
//! The words of every notice (NTF-7, NTF-8; T46). Pure. **These functions are the only place a
//! notice's words exist.** Durations are `cli::render::duration`, the outcome line is
//! `cli::render::outcome_line`, the flags are `cli::render::flags`: called, never copied.

use std::time::Duration;

use crate::api::{Ask, Outcome, Reason};
use crate::model::{PrName, Record, Status};
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
pub fn watching(pr: &PrName, cond: &Ask, life: Duration) -> String {
    unimplemented!("T46")
}

/// NTF-7: `circitor: watching <watching>. If you can read this, notices reach this session.`
pub fn confirmation(told: &Told) -> String {
    unimplemented!("T46")
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
    unimplemented!("T46")
}

/// `circitor: notify for <pr> expired after <life>; it's <status><stale>. Ask again: <ask again>` (NTF-8).
pub fn expired(told: &Told, status: Status, stale: &str) -> String {
    unimplemented!("T46")
}

/// `circitor: can't read <pr> from GitHub (<why>); notify is cancelled. Details: circitor status` (NTF-8).
pub fn problem(told: &Told, why: ProblemWhy) -> String {
    unimplemented!("T46")
}

/// `circitor: the daemon is stopping; notify for <pr> is cancelled. Ask again: <ask again>`, then
/// ` --no-spawn` when `no_spawn` (a stop someone asked for), and nothing after it otherwise (NTF-10).
pub fn stopped(told: &Told, no_spawn: bool) -> String {
    unimplemented!("T46")
}

/// `circitor notify <pr> `, `cli::render::flags(&told.cond)`, ` --for <seconds>` only if it was given,
/// then ` --since <told.cursor>`: the subscription's cursor, as CLI-6's `again:` sets it (NTF-8).
pub fn ask_again(told: &Told) -> String {
    unimplemented!("T46")
}
