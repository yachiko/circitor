//! The pieces every text is made of: the record line, durations, titles, events, the tail (CLI-3).

use std::fmt::Write as _;
use std::time::Duration;

use serde::Serialize;

use super::RenderCtx;
use crate::api::{DaemonBlock, Envelope};
use crate::clock::Time;
use crate::model::{CheckItem, Event, EventKind, Item, ItemKind, Phase, PrName, Record, short_sha};

/// CLI-3: `list` without `--since` prints this many records at most. (proposal: 50)
pub const LIST_MAX: usize = 50;

/// A value's JSON word, for the enums whose serde name is the word CLI-3 prints.
pub(super) fn word<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// CLI-3: the largest unit and the next, a zero next unit left out.
pub fn duration(d: Duration) -> String {
    const UNITS: [(char, u64); 4] = [('d', 86_400), ('h', 3_600), ('m', 60), ('s', 1)];
    let secs = d.as_secs();
    if secs == 0 {
        return "0s".to_string();
    }
    let i = UNITS.iter().position(|&(_, n)| secs >= n).unwrap_or(3);
    let (c, n) = UNITS[i];
    let mut out = format!("{}{c}", secs / n);
    if let Some(&(c2, n2)) = UNITS.get(i + 1) {
        let next = (secs % n) / n2;
        if next > 0 {
            let _ = write!(out, "{next}{c2}");
        }
    }
    out
}

/// CLI-3: in double quotes, `"` and `\` escaped, control characters as `\u{..}`. Never cut.
pub(super) fn title(t: &str) -> String {
    let mut out = String::with_capacity(t.len() + 2);
    out.push('"');
    for c in t.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:02x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// CLI-3: ` (stale, 4m old)` or ` (stale, 4m old, error FORBIDDEN)`.
pub fn stale_mark(record: &Record, now: Time) -> String {
    if !record.stale {
        return String::new();
    }
    let age = duration(now.since(record.fetched_at));
    match &record.error {
        Some(e) => format!(" (stale, {age} old, error {})", e.type_),
        None => format!(" (stale, {age} old)"),
    }
}

/// CLI-3's first example. The stale mark goes last.
pub(super) fn record_line(r: &Record, now: Time) -> String {
    format!(
        "{}  {}  {}  checks {}/{}, {} failed  review {}  {}{}",
        r.pr,
        word(&r.status),
        short_sha(&r.head_sha),
        r.checks.completed,
        r.checks.total,
        r.checks.failed,
        r.review.as_str(),
        title(&r.title),
        stale_mark(r, now),
    )
}

pub(super) fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

/// CLI-3: a reset is never silent.
pub(super) fn reset_lines(out: &mut String, reset: bool, records: &[Record], now: Time) {
    if !reset {
        return;
    }
    push_line(out, "reset: cursor too old, current state follows");
    for r in records {
        push_line(out, &record_line(r, now));
    }
}

fn gh_api(path: &str) -> String {
    format!("gh api {path}")
}

fn item_line(pr: &PrName, i: &Item) -> String {
    let who = i.author.as_deref().unwrap_or("ghost");
    match i.kind {
        ItemKind::Comment => format!("{pr}  new comment by {who}  {}", gh_api(&i.fetch)),
        ItemKind::Review => {
            let mut s = format!("{pr}  new review");
            if let Some(state) = &i.review_state {
                let _ = write!(s, " {state}");
            }
            let _ = write!(s, " by {who}");
            let inline = i.inline.unwrap_or(0);
            if inline > 0 {
                let _ = write!(s, ", {inline} inline");
            }
            let _ = write!(s, "  {}", gh_api(&i.fetch));
            if inline > 0
                && let Some(f) = &i.fetch_inline
            {
                let _ = write!(s, "  {}", gh_api(f));
            }
            s
        }
    }
}

fn kind_lines(out: &mut String, pr: &PrName, kind: &EventKind) {
    let line = match kind {
        EventKind::Status { from, to } => format!("status {} -> {}", word(from), word(to)),
        EventKind::Head { from, to } => format!("head {from} -> {to}"),
        EventKind::Checks {
            state,
            total,
            completed,
            failed,
            failed_items,
        } => match word(state).as_str() {
            "failing" => {
                let mut s = format!("checks failing ({failed} of {total})");
                let items: Vec<String> = failed_items
                    .iter()
                    .map(|f| match &f.url {
                        Some(u) => format!("{} {u}", f.name),
                        None => f.name.clone(),
                    })
                    .collect();
                if !items.is_empty() {
                    let _ = write!(s, ": {}", items.join(", "));
                }
                s
            }
            "pending" => format!("checks pending ({completed} of {total}, {failed} failed)"),
            "passing" => format!("checks passing ({completed}/{total})"),
            other => format!("checks {other}"),
        },
        EventKind::Merged => "merged".to_string(),
        EventKind::Untracked { .. } => "untracked".to_string(),
        EventKind::Closed => "closed".to_string(),
        EventKind::Reopened => "reopened".to_string(),
        EventKind::Gone => "gone".to_string(),
        EventKind::Draft { draft } => format!("draft {draft}"),
        EventKind::Mergeable {
            mergeable,
            merge_state,
        } => format!("mergeable {} ({merge_state})", mergeable.as_str()),
        EventKind::Review { review } => format!("review {}", review.as_str()),
        EventKind::Activity { items, more, .. } => {
            for i in items {
                push_line(out, &item_line(pr, i));
            }
            if *more {
                push_line(
                    out,
                    &format!(
                        "{pr}  more new items than shown  {}  {}",
                        gh_api(&format!(
                            "repos/{}/{}/issues/{}/comments",
                            pr.owner, pr.repo, pr.number
                        )),
                        gh_api(&format!(
                            "repos/{}/{}/pulls/{}/reviews",
                            pr.owner, pr.repo, pr.number
                        )),
                    ),
                );
            }
            return;
        }
        EventKind::Edited { fields } => format!("edited {}", fields.join(", ")),
        EventKind::Tracked { source } => format!("tracked ({})", word(source)),
    };
    push_line(out, &format!("{pr}  {line}"));
}

/// CLI-3: one line per kind of an event, in `kinds`' order, and one per item.
pub(super) fn event_lines(out: &mut String, events: &[Event]) {
    for e in events {
        for k in &e.kinds {
            kind_lines(out, &e.pr, k);
        }
    }
}

/// CLI-3 and D9: the PR is named after an answer about one PR.
pub(super) fn more_events_line(pr: Option<&PrName>, cursor: &str) -> String {
    match pr {
        Some(pr) => format!("more events: circitor changes {pr} --since {cursor}"),
        None => format!("more events: circitor changes --since {cursor}"),
    }
}

fn local(t: Time, ctx: &RenderCtx) -> String {
    t.inner().with_timezone(&ctx.tz).format("%H:%M").to_string()
}

/// `for 12m (until 14:05 local)`; just the time when it has passed.
pub(super) fn until_words(until: Time, ctx: &RenderCtx) -> String {
    let at = local(until, ctx);
    if until > ctx.now {
        format!("for {} (until {at} local)", duration(until.since(ctx.now)))
    } else {
        format!("(until {at} local)")
    }
}

/// `try again in 12m, until 14:05 local`; `try again now` when the time has passed.
pub(super) fn try_again(until: Time, ctx: &RenderCtx) -> String {
    if until > ctx.now {
        format!(
            "try again in {}, until {} local",
            duration(until.since(ctx.now)),
            local(until, ctx)
        )
    } else {
        "try again now".to_string()
    }
}

/// CLI-3: a daemon problem is never silent.
pub(super) fn problem_line(d: &DaemonBlock, ctx: &RenderCtx) -> Option<String> {
    let code = word(d.problem.as_ref()?);
    let mut s = format!("problem: {code}");
    if let Some(u) = d.until {
        let _ = write!(s, " {}", until_words(u, ctx));
    }
    if let Some(m) = &d.message {
        let _ = write!(s, ", {m}");
    }
    Some(s)
}

pub(super) fn push_problem<T>(out: &mut String, e: &Envelope<T>, ctx: &RenderCtx) {
    if let Some(l) = problem_line(&e.daemon, ctx) {
        push_line(out, &l);
    }
}

pub(super) fn push_cursor<T>(out: &mut String, e: &Envelope<T>) {
    push_line(out, &format!("cursor: {}", e.cursor));
}

pub(super) fn pending_lines(out: &mut String, pending: &[PrName]) {
    for p in pending {
        push_line(out, &format!("pending {p}"));
    }
}

/// A check that is done and didn't pass (MOD-5).
pub(super) fn failed_check(c: &CheckItem) -> bool {
    c.phase == Phase::Completed
        && !matches!(
            c.result.as_deref(),
            Some("success") | Some("skipped") | Some("neutral")
        )
}
