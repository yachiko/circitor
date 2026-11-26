//! New items (MOD-12, STA-5, POL-12): which of a read's comments and reviews are new against
//! what was seen, how `seen` moves, and an item's paths.

use crate::clock::Time;
use crate::model::{
    Counts, EventKind, Item, ItemKind, ItemRead, ItemsRead, PrName, Seen, SeenKind,
};

/// POL-12 (proposal): how many of each kind one request reads, the newest.
pub(crate) const ITEMS_READ: usize = 20;

/// MOD-12, U5: who, when and an ID, never text. An item as reported, with its REST paths built
/// from the PR's current name.
/// A review always has `inline`; `fetch_inline` only when it is above 0.
pub(crate) fn item(read: &ItemRead, at: Time, pr: &PrName) -> Item {
    let (o, r, n, id) = (&pr.owner, &pr.repo, pr.number, read.id);
    match read.kind {
        ItemKind::Comment => Item {
            kind: ItemKind::Comment,
            id,
            author: read.author.clone(),
            at,
            review_state: None,
            inline: None,
            fetch: format!("repos/{o}/{r}/issues/comments/{id}"),
            fetch_inline: None,
        },
        ItemKind::Review => {
            let inline = read.inline.unwrap_or(0);
            let fetch = format!("repos/{o}/{r}/pulls/{n}/reviews/{id}");
            Item {
                kind: ItemKind::Review,
                id,
                author: read.author.clone(),
                at,
                review_state: read.review_state.clone(),
                inline: Some(inline),
                fetch_inline: (inline > 0).then(|| format!("{fetch}/comments")),
                fetch,
            }
        }
    }
}

/// STA-5: new if later than the newest seen, or at that time with an ID not seen. With nothing
/// seen, every item is new.
fn is_new(seen: &SeenKind, at: Time, id: u64) -> bool {
    match seen.at {
        None => true,
        Some(s) => at > s || (at == s && !seen.ids.contains(&id)),
    }
}

/// The new items of one kind, as items. An item with no time is ignored (MOD-12).
fn fresh_of(seen: &SeenKind, read: &[ItemRead], pr: &PrName) -> Vec<Item> {
    read.iter()
        .filter_map(|i| i.at.map(|at| (i, at)))
        .filter(|(i, at)| is_new(seen, *at, i.id))
        .map(|(i, at)| item(i, at, pr))
        .collect()
}

/// STA-5: `seen` moves to the newest `at` read and the IDs at it; at an equal `at` it adds the
/// IDs; it never moves back.
fn advance_kind(seen: &mut SeenKind, read: &[ItemRead]) {
    let Some(newest) = read.iter().filter_map(|i| i.at).max() else {
        return;
    };
    let at_newest = read.iter().filter(|i| i.at == Some(newest)).map(|i| i.id);
    match seen.at {
        Some(s) if s > newest => return,
        Some(s) if s == newest => seen.ids.extend(at_newest),
        _ => {
            seen.at = Some(newest);
            seen.ids = at_newest.collect();
        }
    }
    seen.ids.sort_unstable();
    seen.ids.dedup();
}

/// The new items of one read, per kind.
pub(crate) struct Fresh {
    pub comments: Vec<Item>,
    pub reviews: Vec<Item>,
}

impl Fresh {
    pub(crate) fn of(seen: &Seen, read: &ItemsRead, pr: &PrName) -> Fresh {
        Fresh {
            comments: fresh_of(&seen.comment, &read.comments, pr),
            reviews: fresh_of(&seen.review, &read.reviews, pr),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.comments.is_empty() && self.reviews.is_empty()
    }

    /// POL-12: all of a kind's read items are new (the read holds the newest `ITEMS_READ`).
    pub(crate) fn all_comments_new(&self) -> bool {
        self.comments.len() >= ITEMS_READ
    }

    pub(crate) fn all_reviews_new(&self) -> bool {
        self.reviews.len() >= ITEMS_READ
    }

    /// MOD-15: an `activity` kind, its items oldest `at` first, comments before reviews at the
    /// same `at`, then by `id`.
    pub(crate) fn activity(self, counts: Counts, more: bool) -> EventKind {
        let mut items: Vec<Item> = self.comments.into_iter().chain(self.reviews).collect();
        items.sort_by_key(|i| (i.at, i.kind == ItemKind::Review, i.id));
        EventKind::Activity {
            counts,
            items,
            more,
        }
    }
}

/// STA-5: `seen` after a read of the items.
pub(crate) fn advance(seen: &mut Seen, read: &ItemsRead) {
    advance_kind(&mut seen.comment, &read.comments);
    advance_kind(&mut seen.review, &read.reviews);
}
