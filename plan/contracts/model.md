# Seam: the PR model

**Purpose.** The record every component shares, exactly as the API serialises it (MOD-1 to MOD-16, API-19), the read GitHub's client hands over, and the one function that merges a read into a record and says what visibly changed.

**Producer:** T02 declares the types; T05 builds the behaviour. **Consumers:** T07 (builds `PrRead`), T15 (calls `apply_read`), T09, T10, T22, T23, T27 (read `Record` and `Event`), T14 (never: the fake GitHub builds its own JSON).

## Declarations (`src/model/`)

```rust
/// `owner/repo#number` (MOD-1). `Eq`, `Hash` and `Ord` ignore case; `Display` keeps GitHub's spelling.
/// `Ord` is the order "of `pr`" (MOD-15, STA-2, API-6): owner, then repo, both lower-cased, then the
/// number as a number (plan decision P10). Serialises as the string.
#[derive(Clone, Debug)]
pub struct PrName { pub owner: String, pub repo: String, pub number: u64 }
impl PrName {
    pub fn parse(s: &str) -> Result<PrName, NameError>;          // "owner/repo#12" only
    pub fn from_url(s: &str) -> Result<PrName, NameError>;       // https://github.com/owner/repo/pull/12[/…][?…][#…]
    pub fn slug(&self) -> String;                                // "owner/repo"
}

/// GitHub's GraphQL node ID. The key of a tracked PR; never shown (MOD-1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)] pub struct NodeId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum PrState { Open, Merged, Closed, Gone }                  // `Gone` is circitor's own (MOD-11)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Source { Asked, Authored, Manual }                      // `Ord`: the stronger is greater (STA-1)

/// `yes`, `no`, `unknown`; anything else GitHub sends is kept, lowered (MOD-2).
#[derive(Clone, Debug, PartialEq, Eq)] pub enum Mergeable { Yes, No, Unknown, Other(String) }
/// `approved`, `changes_requested`, `review_required`, `none`; anything else kept, lowered.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum Review { Approved, ChangesRequested, ReviewRequired, None, Other(String) }
// Both serialise as the lower-case string and deserialise any string. `Mergeable::from_github("MERGEABLE")`
// is `Yes`, `"CONFLICTING"` `No`; `Review::from_github(None)` is `None`.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum CheckKind { Run, Status }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Phase { Queued, InProgress, Completed }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ChecksState { None, Pending, Failing, Passing }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Status { Merged, Closed, Gone, Conflict, ChecksFailing, ChecksPending, ChangesRequested, Draft,
                  ReviewRequired, Blocked, Unknown, Ready }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckItem {                                           // MOD-3
    pub kind: CheckKind,
    pub id: Option<u64>,                                         // a run's `databaseId`; `null` for a status
    pub name: String,
    pub phase: Phase,
    pub result: Option<String>,                                  // lowered; `null` until completed
    pub url: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FailedItem { pub name: String, pub url: Option<String> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checks {                                              // MOD-5 to MOD-7
    pub state: ChecksState,
    pub settled: bool,
    pub truncated: bool,
    pub total: u32, pub completed: u32, pub failed: u32,
    pub failed_items: Vec<FailedItem>,
    /// Always `Some` in the store. The API leaves it out of `list` unless `checks=1` (API-6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<CheckItem>>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts { pub comments: u32, pub reviews: u32, pub review_threads: u32 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordError { #[serde(rename = "type")] pub type_: String, pub since: Time }

/// MOD-2, field for field and in this order. Every key is always present in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub pr: PrName,
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub state: PrState,
    pub draft: bool,
    pub base: String,
    pub head: String,
    pub head_sha: String,
    pub created_at: Option<Time>, pub updated_at: Option<Time>,
    pub merged_at: Option<Time>, pub closed_at: Option<Time>,
    pub mergeable: Mergeable,
    pub merge_state: String,
    pub review: Review,
    pub checks: Checks,
    pub counts: Counts,
    pub status: Status,
    pub source: Source,
    pub rev: u64,
    pub fetched_at: Time,
    /// Always `false` in the store. Whoever builds an answer sets it with `poller::cadence::stale` (MOD-16).
    pub stale: bool,
    pub error: Option<RecordError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ItemKind { Comment, Review }

/// MOD-12. `fetch` and `fetch_inline` are built from the PR's name when the item is reported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub kind: ItemKind,                                          // `comment`, `review`
    pub id: u64,
    pub author: Option<String>,
    pub at: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub review_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub inline: Option<u32>,
    pub fetch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub fetch_inline: Option<String>,
}

/// MOD-15. `kinds` is a list of objects tagged by `kind` (plan decision P10).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub rev: u64,
    pub pr: PrName,                                              // the name when the event was made
    pub at: Time,
    pub kinds: Vec<EventKind>,
    #[serde(skip, default)] pub id: NodeId,                      // which PR, across a rename; never serialised
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    Status { from: Status, to: Status },
    Head { from: String, to: String },                           // the first 7 characters of each SHA
    Checks { state: ChecksState, total: u32, completed: u32, failed: u32, failed_items: Vec<FailedItem> },
    Merged, Closed, Reopened, Gone,
    Draft { draft: bool },
    Mergeable { mergeable: Mergeable, merge_state: String },
    Review { review: Review },
    Activity { counts: Counts, items: Vec<Item>, more: bool },
    Edited { fields: Vec<String> },                              // of `pr`, `title`, `base`, in that order
    Tracked { source: Source },
    Untracked { source: Source },
}

// ---- what the GitHub client hands over (T07 builds these; nothing derived) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ReadState { Open, Merged, Closed }
#[derive(Clone, Debug, PartialEq)]
pub struct PrRead {
    pub id: NodeId,
    pub pr: PrName,                                              // from `repository.nameWithOwner` and `number`
    pub url: String, pub title: String, pub author: Option<String>,
    pub state: ReadState, pub draft: bool,
    pub base: String, pub head: String, pub head_sha: String,
    pub created_at: Option<Time>, pub updated_at: Option<Time>,
    pub merged_at: Option<Time>, pub closed_at: Option<Time>,
    pub mergeable: Mergeable, pub merge_state: String, pub review: Review,
    pub counts: Counts,
    pub checks: ChecksRead,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ChecksRead {
    pub items: Vec<CheckItem>,                                   // this page's, mapped as MOD-3 says
    pub total: u32,                                              // GitHub's `totalCount`; 0 with a `null` rollup
    pub rollup: Option<String>,                                  // the rollup's `state`, lowered; `None` when `null`
    pub next_page: Option<String>,                               // `endCursor` while `hasNextPage`
}
impl ChecksRead { pub fn append_page(&mut self, page: ChecksRead); }   // POL-8: items added, `total`, `rollup`, `next_page` replaced
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRead {
    pub kind: ItemKind, pub id: u64, pub author: Option<String>,
    pub at: Option<Time>,                                        // `createdAt`; a review's `submittedAt`
    pub review_state: Option<String>, pub inline: Option<u32>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemsRead { pub comments: Vec<ItemRead>, pub reviews: Vec<ItemRead> }   // the last 20 of each

// ---- the merge (T05) ----

/// STA-5: per kind, the newest `at` seen and the IDs at that time. Serialises as STA-6's `seen` object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Seen { pub comment: SeenKind, pub review: SeenKind }
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SeenKind { pub at: Option<Time>, pub ids: Vec<u64> }

/// What the merge remembers about a PR between reads, beside the record. The store keeps it (STA-4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Carry {
    pub applied_seq: u64,                                        // MOD-2: the request number last applied
    pub settle_start: Option<Mono>,                              // MOD-6
    pub known: Option<Known>,                                    // MOD-8, MOD-9
    pub seen: Seen,                                              // STA-5
}
#[derive(Clone, Debug, PartialEq)]
pub struct Known { pub mergeable: Mergeable, pub merge_state: String, pub head_sha: String, pub base: String, pub read_at: Time }

#[derive(Clone, Debug, PartialEq)]
pub struct ReadMeta {
    pub seq: u64,                                                // the number the gate gave the request (MOD-2)
    pub sent_wall: Time, pub sent_mono: Mono,                    // when the request was sent
    pub github_date: Option<Time>,                               // the response's `Date` header (MOD-6)
}
#[derive(Clone, Copy, Debug)]
pub enum ItemsOutcome<'a> { NotAsked, Read(&'a ItemsRead), Failed }   // POL-12, POL-4
#[derive(Clone, Debug)]
pub enum Before<'a> {
    /// First sight (found, read through): the items are a baseline, the event is `tracked` (STA-5).
    New { source: Source },
    /// Loaded from the state file, first refresh (STA-8, MOD-15): `saved_open` says whether it was saved open.
    Loaded { source: Source, saved_open: bool },
    Known(&'a Record),
}
#[derive(Clone, Debug)]
pub enum Merged {
    /// Sent before the read last applied (MOD-2): nothing changes.
    Older,
    Applied { record: Record, carry: Carry, kinds: Vec<EventKind> },   // `kinds` empty: no visible change
}
/// STA-4's merge. Pure: no clock, no I/O. `record.rev` is the old one (0 for a new record); the store sets it.
pub fn apply_read(before: Before<'_>, carry: &Carry, read: &PrRead, items: ItemsOutcome<'_>, meta: &ReadMeta) -> Merged;
/// MOD-11, POL-21: an open record becomes `gone`. `None` if it isn't open.
pub fn apply_gone(record: &Record) -> Option<(Record, Vec<EventKind>)>;
/// POL-12: whether `read` shows `counts.comments` or `counts.reviews` above the record's.
pub fn counts_rose(record: &Record, read: &PrRead) -> bool;
pub fn short_sha(sha: &str) -> &str;                              // the first 7 characters; built by T02 (T10 needs it before T05)
```

## Semantics

- **`apply_read` holds every rule of 01-model that compares two reads:** a newer read wins (MOD-2), each check read is believed (MOD-4), the summary (MOD-5, MOD-7), settling (MOD-6, on `meta.sent_mono`, with the shortcut on `meta.github_date` against `read.updated_at`), the kept `mergeable` and `merge_state` (MOD-8, MOD-9, on `meta.sent_wall`), `review` (MOD-10), `status` (MOD-11), new items against `carry.seen` (MOD-12, STA-5), and which differences are visible and what kinds they make (MOD-14, MOD-15).
- **The caller gives one complete read.** For a PR with several check pages (POL-8) the engine first joins them with `ChecksRead::append_page`; `checks.truncated` is `read.checks.items.len() < read.checks.total`.
- **Request numbers.** `Older` is `meta.seq < carry.applied_seq`; the gate numbers from 1. **A first read (`New`, `Loaded`) without `ItemsOutcome::Read`** is applied with `seen` left as it was and no `activity`; it doesn't happen while a first refresh asks `RecordAndItems`.
- **`ItemsOutcome::Failed`** keeps the record's old `counts.comments` and `counts.reviews` (POL-12), so the next refresh sees the rise again. `NotAsked` with a rise is the engine's bug: `apply_read` then also keeps the old counts.
- **`Before::New`** makes exactly `[Tracked]`. **`Before::Loaded`** makes `[Tracked]`, plus `Merged` or `Closed` when `saved_open` and it has ended, plus `Activity` for the items newer than `carry.seen` (MOD-15). **`Before::Known`** makes whatever changed; a `state` going from `closed` or `gone` to `open` makes `Reopened`, and starts settling again (MOD-6).
- **`record.source`** is copied from `before`. Changing it is the store's (`track`, `untrack`, POL-5), and takes no revision (MOD-14).
- **`record.fetched_at`** is `meta.sent_wall`. **`record.error`** is `None` after an applied read; **`record.stale`** is always `false` here.
- **An item with no time** (a review not yet submitted) is ignored: not seen, not new. (plan decision P10; MOD-12 doesn't say)
- **Event order inside `kinds`** is the order of `EventKind`'s variants above, so output is stable.
- **Serde names are the API's.** `testdata/api/` holds the golden JSON (see `api.md`); T41's tests keep the types and the files equal.

## The fake

None needed: the types are plain data. `testdata/model/` (T41) holds two `Record`s as JSON, one with checks and one merged, for tasks that only render or serve records.

## Don't assume

- That `PrRead` is GitHub's JSON. It's already mapped and lowered by T07; the fake GitHub never sees it.
- That a `Record` in the store has `stale` set, or that `rev` in `Merged::Applied` is new.
- That `PrName` compares by spelling: `Yachiko/Circitor#12` equals `yachiko/circitor#12`.
