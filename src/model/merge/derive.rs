//! The derivations over one read or one record: the checks summary (MOD-5, MOD-7), settling's
//! times (MOD-6), the kept mergeability (MOD-8, MOD-9) and `status` (MOD-11).

use std::time::Duration;

use crate::clock::Time;
use crate::model::{
    CheckItem, Checks, ChecksRead, ChecksState, FailedItem, Known, Mergeable, Phase, PrRead,
    PrState, ReadMeta, ReadState, Record, Review, Status,
};

/// MOD-5: the results that pass. Every other result of a completed check fails.
pub(crate) const PASSING_RESULTS: [&str; 3] = ["success", "skipped", "neutral"];
/// MOD-11 (proposal, D6): the merge states that read `blocked`.
pub(crate) const BLOCKED_MERGE_STATES: [&str; 2] = ["blocked", "behind"];
/// MOD-6 (proposal): the settle time of a list with checks.
pub(crate) const SETTLE_WITH_CHECKS: Duration = Duration::from_secs(20);
/// MOD-6 (proposal): the settle time of an empty list.
pub(crate) const SETTLE_EMPTY: Duration = Duration::from_secs(60);
/// MOD-6 (proposal): the margin of the shortcut, above the settle time, by GitHub's clock.
pub(crate) const SHORTCUT_MARGIN: Duration = Duration::from_secs(5);
/// MOD-8 (proposal): how long a known `yes` or `no` is kept through `UNKNOWN`, on the wall clock.
pub(crate) const KEEP_KNOWN: Duration = Duration::from_secs(60);
/// MOD-8, MOD-9: what a merged PR reads.
pub(crate) const MERGE_STATE_UNKNOWN: &str = "unknown";

/// MOD-5: whether a completed check's result passes. A completed check with no result fails.
pub(crate) fn passes(result: Option<&str>) -> bool {
    result.is_some_and(|r| PASSING_RESULTS.contains(&r))
}

fn failed(item: &CheckItem) -> bool {
    item.phase == Phase::Completed && !passes(item.result.as_deref())
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// MOD-5, MOD-7: the summary of one complete read. The items are the read's: each read is believed (MOD-4).
pub(crate) fn checks(read: &ChecksRead, settled: bool) -> Checks {
    let items = &read.items;
    // MOD-7: fewer items read than GitHub's count.
    let truncated = items.len() < read.total as usize;
    let completed = items.iter().filter(|i| i.phase == Phase::Completed).count();
    let failed_items: Vec<FailedItem> = items
        .iter()
        .filter(|i| failed(i))
        .map(|i| FailedItem {
            name: i.name.clone(),
            url: i.url.clone(),
        })
        .collect();
    let running = completed < items.len();
    let state = if truncated {
        // MOD-7: the rollup decides only for a truncated list, in this order.
        let rollup = read.rollup.as_deref();
        if running || matches!(rollup, Some("pending") | Some("expected")) {
            ChecksState::Pending
        } else if rollup == Some("success") {
            ChecksState::Passing
        } else {
            ChecksState::Failing
        }
    } else if items.is_empty() {
        // MOD-5: from the items, never from the rollup.
        ChecksState::None
    } else if running {
        ChecksState::Pending
    } else if !failed_items.is_empty() {
        ChecksState::Failing
    } else {
        ChecksState::Passing
    };
    Checks {
        state,
        settled,
        truncated,
        total: if truncated {
            read.total
        } else {
            count(items.len())
        },
        completed: count(completed),
        failed: count(failed_items.len()),
        failed_items,
        items: Some(items.clone()),
    }
}

/// MOD-6: the settle time of the list as this read shows it.
pub(crate) fn settle_time(empty: bool) -> Duration {
    if empty {
        SETTLE_EMPTY
    } else {
        SETTLE_WITH_CHECKS
    }
}

/// MOD-6: a record first seen or loaded counts as settled at once when its `updated_at` is more
/// than the settle time plus the margin older than GitHub's `Date`. Without either time, never.
pub(crate) fn settled_at_once(read: &PrRead, meta: &ReadMeta) -> bool {
    match (meta.github_date, read.updated_at) {
        (Some(date), Some(updated)) => {
            date.since(updated) > settle_time(read.checks.items.is_empty()) + SHORTCUT_MARGIN
        }
        _ => false,
    }
}

/// MOD-8, MOD-9: the `mergeable` and `merge_state` the record takes, and what is known after it.
pub(crate) fn mergeability(
    read: &PrRead,
    known: Option<&Known>,
    sent_wall: Time,
) -> (Mergeable, String, Option<Known>) {
    if read.state == ReadState::Merged {
        // MOD-8: a merged PR reads `unknown` for both, and stays so.
        return (Mergeable::Unknown, MERGE_STATE_UNKNOWN.to_string(), None);
    }
    let open = read.state == ReadState::Open;
    match &read.mergeable {
        Mergeable::Yes | Mergeable::No if open => {
            let now = Known {
                mergeable: read.mergeable.clone(),
                merge_state: read.merge_state.clone(),
                head_sha: read.head_sha.clone(),
                base: read.base.clone(),
                read_at: sent_wall,
            };
            (read.mergeable.clone(), read.merge_state.clone(), Some(now))
        }
        Mergeable::Unknown if open => match known {
            // MOD-8: kept for one head and base, at most 60 s after the read that gave it.
            // A `sent_wall` before `read_at` (a clock set back) counts as now (POL-26).
            Some(k)
                if k.head_sha == read.head_sha
                    && k.base == read.base
                    && sent_wall.since(k.read_at) <= KEEP_KNOWN =>
            {
                (k.mergeable.clone(), k.merge_state.clone(), Some(k.clone()))
            }
            _ => (Mergeable::Unknown, read.merge_state.clone(), None),
        },
        // Only `yes` and `no` are kept, and only while open: anything else is taken as read.
        _ => (read.mergeable.clone(), read.merge_state.clone(), None),
    }
}

/// MOD-11: the first row that applies. For `status` only, an unlisted `mergeable` counts as
/// `unknown` and an unlisted `review` as `review_required`.
pub(crate) fn status(r: &Record) -> Status {
    match r.state {
        PrState::Merged => return Status::Merged,
        PrState::Closed => return Status::Closed,
        PrState::Gone => return Status::Gone,
        PrState::Open => {}
    }
    let checks = &r.checks;
    if r.mergeable == Mergeable::No {
        Status::Conflict
    } else if checks.state == ChecksState::Failing {
        Status::ChecksFailing
    } else if checks.state == ChecksState::Pending || !checks.settled {
        Status::ChecksPending
    } else if r.review == Review::ChangesRequested {
        Status::ChangesRequested
    } else if r.draft {
        Status::Draft
    } else if matches!(r.review, Review::ReviewRequired | Review::Other(_)) {
        Status::ReviewRequired
    } else if BLOCKED_MERGE_STATES.contains(&r.merge_state.as_str()) {
        Status::Blocked
    } else if matches!(r.mergeable, Mergeable::Unknown | Mergeable::Other(_)) {
        Status::Unknown
    } else {
        Status::Ready
    }
}
