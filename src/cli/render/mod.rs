//! Rendering: pure, text only (CLI-3, CLI-5, CLI-6; T10).
//!
//! No clock, no environment, no file, no terminal check: time is `ctx.now`, and nothing is coloured
//! (CLI-12). Every line ends in `\n`.

use std::fmt::Write as _;
use std::path::Path;

use crate::api::{
    ApiError, Ask, ChangesBody, Envelope, ErrorCode, ListBody, NotifyBody, Outcome, Reason,
    RecordBody, RefreshBody, StatusBody, Until, WaitBody,
};
use crate::cli::exit;
use crate::clock::Time;
use crate::model::{ChecksState, PrName, Record, short_sha};
use crate::poller::PollOutcome;

mod parts;
#[cfg(test)]
mod tests;

pub use parts::{LIST_MAX, duration, stale_mark};
use parts::{
    event_lines, failed_check, more_events_line, pending_lines, push_cursor, push_line,
    push_problem, record_line, reset_lines, try_again, word,
};

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

/// CLI-3: `list`.
pub fn list(e: &Envelope<ListBody>, with_since: bool, ctx: &RenderCtx) -> String {
    let b = &e.body;
    let mut out = String::new();
    // CLI-3: a reset is never silent; its records are the list.
    if b.reset {
        push_line(&mut out, "reset: cursor too old, current state follows");
    }
    // CLI-3: without `--since` at most LIST_MAX records, then how many are left out.
    let shown = if with_since {
        b.records.len()
    } else {
        b.records.len().min(LIST_MAX)
    };
    for r in &b.records[..shown] {
        push_line(&mut out, &record_line(r, ctx.now));
    }
    let left = b.records.len() - shown;
    if left > 0 {
        let s = if left == 1 { "" } else { "s" };
        push_line(
            &mut out,
            &format!("{left} more PR{s} (all are in circitor list --json)"),
        );
    }
    pending_lines(&mut out, &b.pending);
    for p in &b.untracked {
        push_line(&mut out, &format!("untracked {p}"));
    }
    push_problem(&mut out, e, ctx);
    push_cursor(&mut out, e);
    out
}

/// CLI-3: `show`.
pub fn show(e: &Envelope<RecordBody>, ctx: &RenderCtx) -> String {
    let mut out = String::new();
    if let Some(r) = &e.body.record {
        push_line(&mut out, &record_line(r, ctx.now));
        show_lines(&mut out, r);
    }
    push_problem(&mut out, e, ctx);
    out
}

fn show_lines(out: &mut String, r: &Record) {
    let mut passed = 0u32;
    match &r.checks.items {
        Some(items) => {
            for c in items {
                let state = if failed_check(c) {
                    c.result.clone().unwrap_or_default()
                } else if c.phase != crate::model::Phase::Completed {
                    match c.phase {
                        crate::model::Phase::Queued => "queued".to_string(),
                        _ => "running".to_string(),
                    }
                } else {
                    passed += 1;
                    continue;
                };
                let url = c
                    .url
                    .as_deref()
                    .map(|u| format!(" {u}"))
                    .unwrap_or_default();
                push_line(out, &format!("check {state}: {}{url}", c.name));
            }
        }
        None => {
            for f in &r.checks.failed_items {
                let url = f
                    .url
                    .as_deref()
                    .map(|u| format!(" {u}"))
                    .unwrap_or_default();
                push_line(out, &format!("check failed: {}{url}", f.name));
            }
            passed = r
                .checks
                .completed
                .saturating_sub(r.checks.failed_items.len() as u32);
        }
    }
    push_line(out, &format!("passed checks: {passed}"));
    push_line(
        out,
        &format!(
            "mergeable {}, merge_state {}",
            r.mergeable.as_str(),
            r.merge_state
        ),
    );
    push_line(
        out,
        &format!(
            "counts: comments {}, reviews {}, review_threads {}",
            r.counts.comments, r.counts.reviews, r.counts.review_threads
        ),
    );
    if r.mergeable.as_str() == "unknown" {
        push_line(
            out,
            &format!(
                "mergeable unknown: GitHub is computing it; circitor wait {} --until changed",
                r.pr
            ),
        );
    }
}

/// `pr`: the PR asked about, for `more events: circitor changes <pr> --since …` (CLI-3, D9); `None` over every PR.
pub fn changes(e: &Envelope<ChangesBody>, pr: Option<&PrName>, ctx: &RenderCtx) -> String {
    let b = &e.body;
    let mut out = String::new();
    reset_lines(&mut out, b.reset, &b.records, ctx.now);
    event_lines(&mut out, &b.events);
    pending_lines(&mut out, &b.pending);
    if b.more_events {
        push_line(&mut out, &more_events_line(pr, &e.cursor));
    }
    push_problem(&mut out, e, ctx);
    push_cursor(&mut out, e);
    out
}

/// CLI-6: the head of a `wait`'s answer, as `wait` and `already` both print it: the reset and its
/// records, the events, and the outcome line (never a timeout's) behind `prefix`.
fn answer(out: &mut String, w: &WaitBody, cond: &Ask, prefix: &str, ctx: &RenderCtx) {
    reset_lines(out, w.reset, &w.records, ctx.now);
    event_lines(out, &w.events);
    if w.outcome != Outcome::Timeout {
        let line = outcome_line(
            w.outcome,
            w.reason,
            w.record.as_ref(),
            cond.until,
            cond.sha.as_deref(),
        );
        let mark = w
            .record
            .as_ref()
            .map(|r| stale_mark(r, ctx.now))
            .unwrap_or_default();
        push_line(out, &format!("{prefix}{line}{mark}"));
    }
}

pub fn wait(e: &Envelope<WaitBody>, ask: &WaitAsk, ctx: &RenderCtx) -> String {
    let w = &e.body;
    let mut out = String::new();
    answer(&mut out, w, &ask.cond, "", ctx);
    if w.outcome == Outcome::Timeout {
        push_line(&mut out, &timeout_line(w.record.as_ref(), ask, ctx));
    }
    if w.more_events {
        push_line(&mut out, &more_events_line(Some(&ask.pr), &e.cursor));
    }
    if w.outcome == Outcome::Timeout {
        push_line(&mut out, &again_wait(ask, Some(&e.cursor)));
    }
    push_problem(&mut out, e, ctx);
    push_cursor(&mut out, e);
    out
}

/// CLI-6: `timeout after 100s: <pr> <status>`, the head pair when `--sha` isn't the head, or `no record yet`.
fn timeout_line(record: Option<&Record>, ask: &WaitAsk, ctx: &RenderCtx) -> String {
    let mut s = format!("timeout after {}s: {}", ask.timeout, ask.pr);
    let Some(r) = record else {
        s.push_str(" no record yet");
        return s;
    };
    let other_head = ask
        .cond
        .sha
        .as_deref()
        .filter(|sha| !sha.is_empty() && !starts_with_nocase(&r.head_sha, sha));
    match other_head {
        Some(sha) => {
            let _ = write!(
                s,
                " head {}, not {}",
                short_sha(&r.head_sha),
                short_sha(sha)
            );
        }
        None => {
            let _ = write!(
                s,
                " {} ({}/{}, {} failed)",
                word(&r.status),
                r.checks.completed,
                r.checks.total,
                r.checks.failed
            );
        }
    }
    s.push_str(&stale_mark(r, ctx.now));
    s
}

fn starts_with_nocase(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// CLI-6: `again: circitor wait <pr> <flags> [--timeout n] [--json] [--since <cursor>]`.
fn again_wait(ask: &WaitAsk, cursor: Option<&str>) -> String {
    let mut s = format!("again: circitor wait {} {}", ask.pr, flags(&ask.cond));
    if ask.timeout_given {
        let _ = write!(s, " --timeout {}", ask.timeout);
    }
    if ask.json {
        s.push_str(" --json");
    }
    if let Some(c) = cursor {
        let _ = write!(s, " --since {c}");
    }
    s
}

/// `checks.state` `failing`, or `pending` with a failure in (`--fail-fast`, D8).
fn failed_in(r: &Record) -> bool {
    match r.checks.state {
        ChecksState::Failing => true,
        ChecksState::Pending => r.checks.failed > 0,
        _ => false,
    }
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
    match outcome {
        Outcome::Timeout => String::new(),
        Outcome::Ended => match reason {
            Some(Reason::Merged) => "ended: merged".to_string(),
            Some(Reason::Closed) => "ended: closed".to_string(),
            Some(Reason::Gone) => "ended: gone".to_string(),
            Some(Reason::HeadMoved) => {
                let mut s = "ended: head moved".to_string();
                if let Some(r) = record {
                    let _ = write!(s, " to {}", short_sha(&r.head_sha));
                }
                if let Some(sha) = sha {
                    let _ = write!(s, ", expected {}", short_sha(sha).to_ascii_lowercase());
                }
                s
            }
            None => "ended".to_string(),
        },
        Outcome::Satisfied => match until {
            Until::Merged => "merged".to_string(),
            Until::Changed => "changed".to_string(),
            Until::ChecksDone => match record {
                None => "checks done".to_string(),
                Some(r) => checks_line(r),
            },
        },
    }
}

fn checks_line(r: &Record) -> String {
    let c = &r.checks;
    let sha = short_sha(&r.head_sha);
    let mut s = match c.state {
        ChecksState::None => return format!("no checks reported on {sha}"),
        ChecksState::Passing => {
            return format!("checks passed ({}/{}) on {sha}", c.completed, c.total);
        }
        ChecksState::Failing => format!("checks failed ({} of {}) on {sha}", c.failed, c.total),
        // `--fail-fast`: a failure is in, and others may still run (API-10, D8).
        ChecksState::Pending => format!(
            "checks failing ({} failed, {} of {} done) on {sha}",
            c.failed, c.completed, c.total
        ),
    };
    if c.truncated {
        s.push_str(" (truncated: unread checks may still run)");
    }
    s
}

/// CLI-6: a condition as the flags that ask for it. Never `--since`.
pub fn flags(cond: &Ask) -> String {
    let mut s = format!("--until {}", cond.until.as_str());
    if let Some(v) = &cond.sha {
        let _ = write!(s, " --sha {v}");
    }
    if let Some(v) = &cond.rerun_since {
        let _ = write!(s, " --rerun-since {v}");
    }
    if cond.fail_fast {
        s.push_str(" --fail-fast");
    }
    if let Some(v) = &cond.kind {
        let _ = write!(s, " --kind {v}");
    }
    s
}

/// NTF-11's "already known", in NTF-11's order.
pub fn already(
    e: &Envelope<NotifyBody>,
    pr: &PrName,
    cond: &Ask,
    life: Option<u32>,
    ctx: &RenderCtx,
) -> String {
    let b = &e.body;
    let mut out = String::new();
    match b.as_wait() {
        Some(w) => answer(&mut out, &w, cond, "already: ", ctx),
        None => {
            reset_lines(&mut out, b.reset, &b.records, ctx.now);
            event_lines(&mut out, &b.events);
        }
    }
    if b.replaced {
        push_line(&mut out, "replaced an earlier notify");
    }
    if b.more_events {
        push_line(&mut out, &more_events_line(Some(pr), &e.cursor));
    }
    if b.reset {
        // NTF-11: nothing since the reset is known to match.
        let mut s = format!("again: circitor notify {pr} {}", flags(cond));
        if let Some(l) = life {
            let _ = write!(s, " --for {l}");
        }
        let _ = write!(s, " --since {}", e.cursor);
        push_line(&mut out, &s);
    }
    push_problem(&mut out, e, ctx);
    push_cursor(&mut out, e);
    out
}

/// NTF-11's registered output, whole, around its first line (`registered: ` and `notify::text::watching`,
/// which T48 builds and passes in).
pub fn registered(e: &Envelope<NotifyBody>, first: &str, pr: &PrName, ctx: &RenderCtx) -> String {
    let b = &e.body;
    let mut out = String::new();
    reset_lines(&mut out, b.reset, &b.records, ctx.now);
    event_lines(&mut out, &b.events);
    push_line(&mut out, first);
    if let Some(r) = &b.record {
        push_line(&mut out, &record_line(r, ctx.now));
    }
    push_line(
        &mut out,
        &format!(
            "a confirmation was posted to this session; if none arrives, notices are held or refused here: run circitor notify {pr} --cancel, then use circitor wait"
        ),
    );
    if b.replaced {
        push_line(&mut out, "replaced an earlier notify");
    }
    if b.more_events {
        push_line(&mut out, &more_events_line(Some(pr), &e.cursor));
    }
    push_problem(&mut out, e, ctx);
    push_cursor(&mut out, e);
    out
}

/// CLI-6's three endings without an answer; `cursor` is the `--since` given or the first `circitor-cursor`.
pub fn wait_no_answer(
    kind: NoAnswer,
    ask: &WaitAsk,
    cursor: Option<&str>,
    socket: &Path,
) -> String {
    let mut out = String::new();
    let first = match kind {
        NoAnswer::NoTimeLeft => format!(
            "timeout after {}s: {} (no answer from the daemon)",
            ask.timeout, ask.pr
        ),
        NoAnswer::DaemonStopped => "daemon stopped".to_string(),
        NoAnswer::NotAnswering => format!("daemon not answering on {}", socket.display()),
    };
    push_line(&mut out, &first);
    push_line(&mut out, &again_wait(ask, cursor));
    if let Some(c) = cursor {
        push_line(&mut out, &format!("cursor: {c}"));
    }
    out
}

/// `track`, `untrack`. With `record: null` (API-15) it names `pr`.
pub fn tracked(e: &Envelope<RecordBody>, pr: &PrName, ctx: &RenderCtx) -> String {
    let mut out = String::new();
    match &e.body.record {
        Some(r) => push_line(&mut out, &record_line(r, ctx.now)),
        None => push_line(&mut out, &format!("{pr} not tracked")),
    }
    push_problem(&mut out, e, ctx);
    out
}

pub fn refresh(e: &Envelope<RefreshBody>, ctx: &RenderCtx) -> String {
    let mut out = String::new();
    let b = &e.body;
    let outcome = word(&b.outcome);
    if b.outcome == PollOutcome::Shared {
        push_line(&mut out, &format!("refresh: {outcome}"));
    } else {
        let s = if b.cost == 1 { "" } else { "s" };
        push_line(
            &mut out,
            &format!("refresh: {outcome}, {} point{s}", b.cost),
        );
    }
    push_problem(&mut out, e, ctx);
    out
}

fn or_none<T: std::fmt::Display>(v: &Option<T>) -> String {
    v.as_ref()
        .map_or_else(|| "none".to_string(), |v| v.to_string())
}

/// API-5: one `key: value` line per key, in its order.
pub fn status(e: &Envelope<StatusBody>, ctx: &RenderCtx) -> String {
    let b = &e.body;
    let mut out = String::new();
    let mut kv = |k: &str, v: String| push_line(&mut out, &format!("{k}: {v}"));
    kv("version", e.version.clone());
    kv("mode", word(&b.mode));
    kv("started_at", b.started_at.to_string());
    kv(
        "last_poll",
        b.last_poll.as_ref().map_or("none".to_string(), |l| {
            format!("{} {}", l.at, word(&l.outcome))
        }),
    );
    kv("cost_last_hour", b.cost_last_hour.to_string());
    kv(
        "github",
        format!(
            "remaining {}, reset_at {}, login {}",
            or_none(&b.github.remaining),
            or_none(&b.github.reset_at),
            or_none(&b.github.login)
        ),
    );
    kv(
        "tracked",
        format!(
            "authored {}, manual {}, asked {}",
            b.tracked.authored, b.tracked.manual, b.tracked.asked
        ),
    );
    kv("waiters", b.waiters.to_string());
    kv("subscriptions", b.subscriptions.to_string());
    kv(
        "owners",
        if b.owners.is_empty() {
            "none".to_string()
        } else {
            b.owners.join(", ")
        },
    );
    kv("token_source", b.token_source.clone());
    kv("gh_path", b.gh_path.clone());
    kv(
        "paths",
        format!(
            "config {}, state {}, socket {}, log {}",
            b.paths.config, b.paths.state, b.paths.socket, b.paths.log
        ),
    );
    kv(
        "warnings",
        if b.warnings.is_empty() {
            "none".to_string()
        } else {
            b.warnings
                .iter()
                .map(|w| format!("{} since {}: {}", word(&w.code), w.since, w.message))
                .collect::<Vec<_>>()
                .join("; ")
        },
    );
    push_problem(&mut out, e, ctx);
    if ctx.cli_version != e.version {
        push_line(&mut out, &format!("cli_version: {}", ctx.cli_version));
    }
    out
}

pub fn stop(was_running: bool) -> String {
    if was_running {
        "stopped\n"
    } else {
        "not running\n"
    }
    .to_string()
}

/// CLI-5: one line, `<code>: <message>`; the three codes that exit 9 end with when to try again.
/// CLI-9: an older daemon that wasn't replaced adds its hint to `bad_request` and `unknown_route`.
pub fn api_error(e: &ApiError, older_daemon: bool, ctx: &RenderCtx) -> String {
    let mut s = format!("{}: {}", word(&e.code), e.message);
    if error_code(e.code, false) == exit::UNAVAILABLE {
        match e.until {
            Some(u) => {
                let _ = write!(s, " ({})", try_again(u, ctx));
            }
            None => s.push_str(" (try again in a minute)"),
        }
    }
    if older_daemon && matches!(e.code, ErrorCode::BadRequest | ErrorCode::UnknownRoute) {
        s.push_str(
            " (the daemon is older than this CLI: run circitor stop, or restart its service, to replace it)",
        );
    }
    s.push('\n');
    s
}

/// CLI-5's code for a wait's answer: 0, 3, 4 or 8.
pub fn wait_code(body: &WaitBody, until: Until) -> i32 {
    match body.outcome {
        Outcome::Timeout => exit::TIMEOUT,
        Outcome::Ended => exit::ENDED,
        Outcome::Satisfied => {
            let failed = until == Until::ChecksDone && body.record.as_ref().is_some_and(failed_in);
            if failed {
                exit::CHECKS_FAILED
            } else {
                exit::OK
            }
        }
    }
}

/// `no_session` is 11 (CLI-5).
pub fn error_code(code: ErrorCode, older_daemon: bool) -> i32 {
    match code {
        ErrorCode::BadRequest if older_daemon => exit::ERROR,
        ErrorCode::BadRequest => exit::USAGE,
        ErrorCode::NotFound => exit::NOT_FOUND,
        ErrorCode::UnknownRoute => exit::ERROR,
        ErrorCode::Auth => exit::AUTH,
        ErrorCode::Starting | ErrorCode::GithubUnavailable | ErrorCode::RateLimited => {
            exit::UNAVAILABLE
        }
        ErrorCode::Limit => exit::LIMIT,
        ErrorCode::NoSession => exit::NO_SESSION,
    }
}

/// `refresh`'s code: 0 for `polled` and `shared`, else the code of the API-3 error of that name (API-16).
pub fn refresh_code(outcome: PollOutcome) -> i32 {
    match outcome {
        PollOutcome::Polled | PollOutcome::Shared => exit::OK,
        PollOutcome::Auth => error_code(ErrorCode::Auth, false),
        PollOutcome::GithubUnavailable => error_code(ErrorCode::GithubUnavailable, false),
        PollOutcome::RateLimited => error_code(ErrorCode::RateLimited, false),
    }
}

/// CLI-6's three endings: 3, 6, 6.
pub fn no_answer_code(kind: NoAnswer) -> i32 {
    match kind {
        NoAnswer::NoTimeLeft => exit::TIMEOUT,
        NoAnswer::DaemonStopped | NoAnswer::NotAnswering => exit::NO_DAEMON,
    }
}
