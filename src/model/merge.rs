//! STA-4's merge (T05): one read into a record, by every rule of 01-model, and what visibly changed.

mod derive;
mod diff;
mod items;

#[cfg(test)]
mod tests;

use super::{
    Before, Carry, ChecksRead, EventKind, ItemsOutcome, Merged, PrRead, PrState, ReadMeta,
    ReadState, Record,
};
use items::{Fresh, ITEMS_READ};

impl ChecksRead {
    /// POL-8: items added, `total`, `rollup`, `next_page` replaced.
    pub fn append_page(&mut self, page: ChecksRead) {
        self.items.extend(page.items);
        self.total = page.total;
        self.rollup = page.rollup;
        self.next_page = page.next_page;
    }
}

fn state_of(read: ReadState) -> PrState {
    match read {
        ReadState::Open => PrState::Open,
        ReadState::Merged => PrState::Merged,
        ReadState::Closed => PrState::Closed,
    }
}

/// STA-4's merge. Pure: no clock, no I/O. `record.rev` is the old one (0 for a new record); the store sets it.
pub fn apply_read(
    before: Before<'_>,
    carry: &Carry,
    read: &PrRead,
    items: ItemsOutcome<'_>,
    meta: &ReadMeta,
) -> Merged {
    // MOD-2: a newer read wins; requests are ordered by the gate's number.
    if meta.seq < carry.applied_seq {
        return Merged::Older;
    }
    let old = match before {
        Before::Known(r) => Some(r),
        Before::New { .. } | Before::Loaded { .. } => None,
    };
    let (source, rev) = match before {
        Before::New { source } | Before::Loaded { source, .. } => (source, 0),
        Before::Known(r) => (r.source, r.rev),
    };
    let mut next = carry.clone();
    next.applied_seq = meta.seq;

    // MOD-6: the settle start is the read that first read the record, first showed this head
    // (also back to an earlier one), or first showed it open again after a close or `gone`.
    let empty = read.checks.items.is_empty();
    let settle_start = match (old, carry.settle_start) {
        (Some(r), Some(start))
            if r.head_sha == read.head_sha
                && !(matches!(r.state, PrState::Closed | PrState::Gone)
                    && read.state == ReadState::Open) =>
        {
            Some((r, start))
        }
        _ => None,
    };
    let settled = match settle_start {
        // Once true, it stays true until the next settle start; it moves only here.
        Some((r, start)) => {
            r.checks.settled || meta.sent_mono.since(start) >= derive::settle_time(empty)
        }
        None => {
            next.settle_start = Some(meta.sent_mono);
            // MOD-6's shortcut: for a record first seen or loaded only.
            old.is_none() && derive::settled_at_once(read, meta)
        }
    };

    // MOD-8, MOD-9: on the wall clock the request was sent at.
    let (mergeable, merge_state, known) =
        derive::mergeability(read, carry.known.as_ref(), meta.sent_wall);
    next.known = known;

    // MOD-12, STA-5, POL-12: the counts and the new items.
    let rose = old.is_some_and(|r| counts_rose(r, read));
    let mut counts = read.counts;
    let mut activity = None;
    match items {
        ItemsOutcome::Read(got) => {
            let fresh = Fresh::of(&carry.seen, got, &read.pr);
            items::advance(&mut next.seen, got);
            match before {
                // STA-5: first sight is a baseline.
                Before::New { .. } => {}
                Before::Loaded { .. } => {
                    if !fresh.is_empty() {
                        let more = fresh.all_comments_new() || fresh.all_reviews_new();
                        activity = Some(fresh.activity(counts, more));
                    }
                }
                Before::Known(r) => {
                    if rose {
                        let limit = ITEMS_READ as u32;
                        let more = (fresh.all_comments_new()
                            && counts.comments.saturating_sub(r.counts.comments) > limit)
                            || (fresh.all_reviews_new()
                                && counts.reviews.saturating_sub(r.counts.reviews) > limit);
                        activity = Some(fresh.activity(counts, more));
                    }
                }
            }
        }
        ItemsOutcome::Failed | ItemsOutcome::NotAsked => {
            // POL-12: without the items, a rise isn't taken yet, so the next refresh asks again.
            if let Some(r) = old
                && (matches!(items, ItemsOutcome::Failed) || rose)
            {
                counts.comments = r.counts.comments;
                counts.reviews = r.counts.reviews;
            }
        }
    }

    // MOD-2: the record, field for field from the read. U4: it holds the state, the checks, the
    // review state and whether it can merge. MOD-1: `pr` takes GitHub's spelling.
    let mut record = Record {
        pr: read.pr.clone(),
        url: read.url.clone(),
        title: read.title.clone(),
        author: read.author.clone(),
        state: state_of(read.state),
        draft: read.draft,
        base: read.base.clone(),
        head: read.head.clone(),
        head_sha: read.head_sha.clone(),
        created_at: read.created_at,
        updated_at: read.updated_at,
        merged_at: read.merged_at,
        closed_at: read.closed_at,
        mergeable,
        merge_state,
        // MOD-10: `reviewDecision`, as T07 lowered it.
        review: read.review.clone(),
        checks: derive::checks(&read.checks, settled),
        counts,
        status: super::Status::Unknown,
        source,
        rev,
        fetched_at: meta.sent_wall,
        stale: false,
        error: None,
    };
    record.status = derive::status(&record);

    // MOD-14, MOD-15.
    let kinds = match before {
        Before::New { source } => vec![EventKind::Tracked { source }],
        Before::Loaded { source, saved_open } => {
            let mut kinds = Vec::new();
            if saved_open {
                match record.state {
                    PrState::Merged => kinds.push(EventKind::Merged),
                    PrState::Closed => kinds.push(EventKind::Closed),
                    PrState::Open | PrState::Gone => {}
                }
            }
            kinds.extend(activity);
            kinds.push(EventKind::Tracked { source });
            kinds
        }
        Before::Known(r) => diff::changes(r, &record, activity),
    };
    Merged::Applied {
        record,
        carry: next,
        kinds,
    }
}

/// MOD-11, POL-21: an open record becomes `gone`. `None` if it isn't open.
pub fn apply_gone(record: &Record) -> Option<(Record, Vec<EventKind>)> {
    if record.state != PrState::Open {
        return None;
    }
    // MOD-11: every other field stays, `fetched_at` and `error` included.
    let mut gone = record.clone();
    gone.state = PrState::Gone;
    gone.status = derive::status(&gone);
    let kinds = diff::changes(record, &gone, None);
    Some((gone, kinds))
}

/// POL-12: whether `read` shows `counts.comments` or `counts.reviews` above the record's.
pub fn counts_rose(record: &Record, read: &PrRead) -> bool {
    read.counts.comments > record.counts.comments || read.counts.reviews > record.counts.reviews
}
