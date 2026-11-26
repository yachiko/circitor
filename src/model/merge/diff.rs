//! What visibly changed between two records (MOD-14), as event kinds in `EventKind`'s declared
//! order (MOD-15).

use crate::model::{CheckItem, CheckKind, Checks, EventKind, PrState, Record, short_sha};

/// MOD-4, MOD-14: how a check is told apart between reads: a run by `id`, a status by `name`,
/// and a run with no `id` by `name` too.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Key<'a> {
    Run(u64),
    RunNamed(&'a str),
    Status(&'a str),
}

fn key(item: &CheckItem) -> Key<'_> {
    match (item.kind, item.id) {
        (CheckKind::Run, Some(id)) => Key::Run(id),
        (CheckKind::Run, None) => Key::RunNamed(&item.name),
        (CheckKind::Status, _) => Key::Status(&item.name),
    }
}

fn keys(checks: &Checks) -> Vec<Key<'_>> {
    let mut keys: Vec<Key<'_>> = checks.items.iter().flatten().map(key).collect();
    keys.sort();
    keys
}

/// MOD-14's checks fields, and which items are in the list. Not `settled`, a `url`, or a move
/// between `queued` and `in_progress`.
fn checks_changed(old: &Checks, new: &Checks) -> bool {
    old.state != new.state
        || old.total != new.total
        || old.completed != new.completed
        || old.failed != new.failed
        || old.truncated != new.truncated
        || old.failed_items != new.failed_items
        || keys(old) != keys(new)
}

/// MOD-14, MOD-15: the kinds of what changed from `old` to `new`, `activity` as the caller made it.
pub(crate) fn changes(old: &Record, new: &Record, activity: Option<EventKind>) -> Vec<EventKind> {
    let mut kinds = Vec::new();
    if old.status != new.status {
        kinds.push(EventKind::Status {
            from: old.status,
            to: new.status,
        });
    }
    if old.head_sha != new.head_sha {
        kinds.push(EventKind::Head {
            from: short_sha(&old.head_sha).to_string(),
            to: short_sha(&new.head_sha).to_string(),
        });
    }
    if checks_changed(&old.checks, &new.checks) {
        let c = &new.checks;
        kinds.push(EventKind::Checks {
            state: c.state,
            total: c.total,
            completed: c.completed,
            failed: c.failed,
            failed_items: c.failed_items.clone(),
        });
    }
    if old.state != new.state {
        match new.state {
            PrState::Merged => kinds.push(EventKind::Merged),
            PrState::Closed => kinds.push(EventKind::Closed),
            PrState::Open => kinds.push(EventKind::Reopened),
            PrState::Gone => kinds.push(EventKind::Gone),
        }
    }
    if old.draft != new.draft {
        kinds.push(EventKind::Draft { draft: new.draft });
    }
    // MOD-8: on a merged PR the change to `unknown` comes with `merged`, with no `mergeable` kind.
    if new.state != PrState::Merged
        && (old.mergeable != new.mergeable || old.merge_state != new.merge_state)
    {
        kinds.push(EventKind::Mergeable {
            mergeable: new.mergeable.clone(),
            merge_state: new.merge_state.clone(),
        });
    }
    if old.review != new.review {
        kinds.push(EventKind::Review {
            review: new.review.clone(),
        });
    }
    kinds.extend(activity);
    // MOD-1: `pr` compares without case, so a change of case alone isn't one.
    let fields: Vec<String> = [
        ("pr", old.pr != new.pr),
        ("title", old.title != new.title),
        ("base", old.base != new.base),
    ]
    .into_iter()
    .filter(|(_, changed)| *changed)
    .map(|(name, _)| name.to_string())
    .collect();
    if !fields.is_empty() {
        kinds.push(EventKind::Edited { fields });
    }
    kinds
}
