#![allow(unused_variables, dead_code)] // stub: T10 removes this
//! Rendering: pure, text only (CLI-3, CLI-5, CLI-6; T10).

use std::path::Path;
use std::time::Duration;

use crate::api::{
    ApiError, Ask, ChangesBody, Envelope, ErrorCode, ListBody, NotifyBody, Outcome, Reason,
    RecordBody, RefreshBody, StatusBody, Until, WaitBody,
};
use crate::clock::Time;
use crate::model::{PrName, Record};
use crate::poller::PollOutcome;

#[derive(Clone, Debug)]
pub struct RenderCtx {
    pub now: Time,
    pub tz: chrono::FixedOffset,
    pub cli_version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaitAsk {
    pub pr: PrName,
    pub cond: Ask,
    /// The deadline in force: `--timeout`, or `DEFAULT_WAIT_TIMEOUT`.
    pub timeout: u32,
    /// `again:` repeats `--timeout` only if it was given (CLI-6).
    pub timeout_given: bool,
    /// And `--json` likewise.
    pub json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoAnswer {
    NoTimeLeft,
    DaemonStopped,
    NotAnswering,
}

pub fn list(e: &Envelope<ListBody>, with_since: bool, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

pub fn show(e: &Envelope<RecordBody>, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

/// `pr`: the PR asked about, for `more events: circitor changes <pr> --since …` (CLI-3, D9); `None` over every PR.
pub fn changes(e: &Envelope<ChangesBody>, pr: Option<&PrName>, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

pub fn wait(e: &Envelope<WaitBody>, ask: &WaitAsk, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

/// CLI-6's outcome line alone, with no newline and no stale mark. **`wait` prints it, `already` prints
/// it, and `notify::text::outcome` puts it in a notice (NTF-8): one function, so the three can't drift.**
/// `sha` is the `--sha` given. `Outcome::Timeout` has no outcome line: an empty string.
pub fn outcome_line(
    outcome: Outcome,
    reason: Option<Reason>,
    record: Option<&Record>,
    until: Until,
    sha: Option<&str>,
) -> String {
    unimplemented!("T10")
}

/// A condition as the flags that ask for it: `--until checks-done`, then `--sha`, `--rerun-since`,
/// `--fail-fast` and `--kind`, each only if it was given and as it was. Never `--since`.
pub fn flags(cond: &Ask) -> String {
    unimplemented!("T10")
}

/// CLI-3's durations: the largest unit and the next, of `d`, `h`, `m`, `s`.
pub fn duration(d: Duration) -> String {
    unimplemented!("T10")
}

/// NTF-11's "already known", in NTF-11's order.
pub fn already(
    e: &Envelope<NotifyBody>,
    pr: &PrName,
    cond: &Ask,
    life: Option<u32>,
    ctx: &RenderCtx,
) -> String {
    unimplemented!("T10")
}

/// NTF-11's registered output, whole, around its first line (`registered: ` and `notify::text::watching`,
/// which T48 builds and passes in).
pub fn registered(e: &Envelope<NotifyBody>, first: &str, pr: &PrName, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

/// CLI-3's stale mark alone, with its leading space: ` (stale, 4m old)`, ` (stale, 4m old, error FORBIDDEN)`,
/// or "" when `record.stale` is false.
pub fn stale_mark(record: &Record, now: Time) -> String {
    unimplemented!("T10")
}

/// CLI-6's three endings without an answer; `cursor` is the `--since` given or the first `circitor-cursor`.
pub fn wait_no_answer(
    kind: NoAnswer,
    ask: &WaitAsk,
    cursor: Option<&str>,
    socket: &Path,
) -> String {
    unimplemented!("T10")
}

/// `track`, `untrack`. With `record: null` (API-15) it names `pr`.
pub fn tracked(e: &Envelope<RecordBody>, pr: &PrName, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

pub fn refresh(e: &Envelope<RefreshBody>, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

pub fn status(e: &Envelope<StatusBody>, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

pub fn stop(was_running: bool) -> String {
    unimplemented!("T10")
}

/// An API-3 error as CLI-5's one stderr line; `older_daemon` adds CLI-9's hint.
pub fn api_error(e: &ApiError, older_daemon: bool, ctx: &RenderCtx) -> String {
    unimplemented!("T10")
}

/// CLI-5's code for a wait's answer: 0, 3, 4 or 8. `notify` never calls it (NTF-11).
pub fn wait_code(body: &WaitBody, until: Until) -> i32 {
    unimplemented!("T10")
}

/// `no_session` is 11 (CLI-5).
pub fn error_code(code: ErrorCode, older_daemon: bool) -> i32 {
    unimplemented!("T10")
}

/// `refresh`'s code: 0 for `polled` and `shared`, else the code of the API-3 error of that name (API-16).
pub fn refresh_code(outcome: PollOutcome) -> i32 {
    unimplemented!("T10")
}

/// CLI-6's three endings: 3, 6, 6.
pub fn no_answer_code(kind: NoAnswer) -> i32 {
    unimplemented!("T10")
}
