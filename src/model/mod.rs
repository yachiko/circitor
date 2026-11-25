//! The PR model (`plan/contracts/model.md`). T02 declares the types and builds `PrName`,
//! `Mergeable`, `Review`, `EventKind::word` and `short_sha`; T05 builds the merge.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::clock::{Mono, Time};

mod merge;
mod name;

pub use merge::{apply_gone, apply_read, counts_rose};
pub use name::{NameError, PrName};

/// GitHub's GraphQL node ID. The key of a tracked PR; never shown (MOD-1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Closed,
    /// circitor's own (MOD-11).
    Gone,
}

/// `Ord`: the stronger is greater (STA-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Asked,
    Authored,
    Manual,
}

/// `yes`, `no`, `unknown`; anything else GitHub sends is kept, lowered (MOD-2).
/// Serialises as the lower-case string and deserialises any string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mergeable {
    Yes,
    No,
    Unknown,
    Other(String),
}

impl Mergeable {
    /// GitHub's `mergeable`: `MERGEABLE` is `Yes`, `CONFLICTING` `No`, `UNKNOWN` `Unknown` (MOD-2).
    pub fn from_github(value: &str) -> Mergeable {
        match value {
            "MERGEABLE" => Mergeable::Yes,
            "CONFLICTING" => Mergeable::No,
            "UNKNOWN" => Mergeable::Unknown,
            other => Mergeable::Other(other.to_lowercase()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Mergeable::Yes => "yes",
            Mergeable::No => "no",
            Mergeable::Unknown => "unknown",
            Mergeable::Other(s) => s,
        }
    }
}

impl Serialize for Mergeable {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Mergeable {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Mergeable, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "yes" => Mergeable::Yes,
            "no" => Mergeable::No,
            "unknown" => Mergeable::Unknown,
            _ => Mergeable::Other(s),
        })
    }
}

/// `approved`, `changes_requested`, `review_required`, `none`; anything else kept, lowered.
/// Serialises as the lower-case string and deserialises any string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Review {
    Approved,
    ChangesRequested,
    ReviewRequired,
    None,
    Other(String),
}

impl Review {
    /// GitHub's `reviewDecision`; `null` is `None` (MOD-2).
    pub fn from_github(value: Option<&str>) -> Review {
        match value {
            None => Review::None,
            Some("APPROVED") => Review::Approved,
            Some("CHANGES_REQUESTED") => Review::ChangesRequested,
            Some("REVIEW_REQUIRED") => Review::ReviewRequired,
            Some(other) => Review::Other(other.to_lowercase()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Review::Approved => "approved",
            Review::ChangesRequested => "changes_requested",
            Review::ReviewRequired => "review_required",
            Review::None => "none",
            Review::Other(s) => s,
        }
    }
}

impl Serialize for Review {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Review {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Review, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "approved" => Review::Approved,
            "changes_requested" => Review::ChangesRequested,
            "review_required" => Review::ReviewRequired,
            "none" => Review::None,
            _ => Review::Other(s),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Run,
    Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Queued,
    InProgress,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksState {
    None,
    Pending,
    Failing,
    Passing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Merged,
    Closed,
    Gone,
    Conflict,
    ChecksFailing,
    ChecksPending,
    ChangesRequested,
    Draft,
    ReviewRequired,
    Blocked,
    Unknown,
    Ready,
}

/// MOD-3.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckItem {
    pub kind: CheckKind,
    /// A run's `databaseId`; `null` for a status.
    pub id: Option<u64>,
    pub name: String,
    pub phase: Phase,
    /// Lowered; `null` until completed.
    pub result: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FailedItem {
    pub name: String,
    pub url: Option<String>,
}

/// MOD-5 to MOD-7.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checks {
    pub state: ChecksState,
    pub settled: bool,
    pub truncated: bool,
    pub total: u32,
    pub completed: u32,
    pub failed: u32,
    pub failed_items: Vec<FailedItem>,
    /// Always `Some` in the store. The API leaves it out of `list` unless `checks=1` (API-6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<CheckItem>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub comments: u32,
    pub reviews: u32,
    pub review_threads: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordError {
    #[serde(rename = "type")]
    pub type_: String,
    pub since: Time,
}

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
    pub created_at: Option<Time>,
    pub updated_at: Option<Time>,
    pub merged_at: Option<Time>,
    pub closed_at: Option<Time>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Comment,
    Review,
}

/// MOD-12. `fetch` and `fetch_inline` are built from the PR's name when the item is reported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// `comment`, `review`.
    pub kind: ItemKind,
    pub id: u64,
    pub author: Option<String>,
    pub at: Time,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<u32>,
    pub fetch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_inline: Option<String>,
}

/// MOD-15. `kinds` is a list of objects tagged by `kind`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub rev: u64,
    /// The name when the event was made.
    pub pr: PrName,
    pub at: Time,
    pub kinds: Vec<EventKind>,
    /// Which PR, across a rename; never serialised.
    #[serde(skip, default)]
    pub id: NodeId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    Status {
        from: Status,
        to: Status,
    },
    /// The first 7 characters of each SHA.
    Head {
        from: String,
        to: String,
    },
    Checks {
        state: ChecksState,
        total: u32,
        completed: u32,
        failed: u32,
        failed_items: Vec<FailedItem>,
    },
    Merged,
    Closed,
    Reopened,
    Gone,
    Draft {
        draft: bool,
    },
    Mergeable {
        mergeable: Mergeable,
        merge_state: String,
    },
    Review {
        review: Review,
    },
    Activity {
        counts: Counts,
        items: Vec<Item>,
        more: bool,
    },
    /// Of `pr`, `title`, `base`, in that order.
    Edited {
        fields: Vec<String>,
    },
    Tracked {
        source: Source,
    },
    Untracked {
        source: Source,
    },
}

impl EventKind {
    /// The `kind` tag: `status`, `head`, … (MOD-15). Built by T02; `api::condition` matches `kind` on it (API-10).
    pub fn word(&self) -> &'static str {
        match self {
            EventKind::Status { .. } => "status",
            EventKind::Head { .. } => "head",
            EventKind::Checks { .. } => "checks",
            EventKind::Merged => "merged",
            EventKind::Closed => "closed",
            EventKind::Reopened => "reopened",
            EventKind::Gone => "gone",
            EventKind::Draft { .. } => "draft",
            EventKind::Mergeable { .. } => "mergeable",
            EventKind::Review { .. } => "review",
            EventKind::Activity { .. } => "activity",
            EventKind::Edited { .. } => "edited",
            EventKind::Tracked { .. } => "tracked",
            EventKind::Untracked { .. } => "untracked",
        }
    }
}

// ---- what the GitHub client hands over (T07 builds these; nothing derived) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadState {
    Open,
    Merged,
    Closed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrRead {
    pub id: NodeId,
    /// From `repository.nameWithOwner` and `number`.
    pub pr: PrName,
    pub url: String,
    pub title: String,
    pub author: Option<String>,
    pub state: ReadState,
    pub draft: bool,
    pub base: String,
    pub head: String,
    pub head_sha: String,
    pub created_at: Option<Time>,
    pub updated_at: Option<Time>,
    pub merged_at: Option<Time>,
    pub closed_at: Option<Time>,
    pub mergeable: Mergeable,
    pub merge_state: String,
    pub review: Review,
    pub counts: Counts,
    pub checks: ChecksRead,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChecksRead {
    /// This page's, mapped as MOD-3 says.
    pub items: Vec<CheckItem>,
    /// GitHub's `totalCount`; 0 with a `null` rollup.
    pub total: u32,
    /// The rollup's `state`, lowered; `None` when `null`.
    pub rollup: Option<String>,
    /// `endCursor` while `hasNextPage`.
    pub next_page: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemRead {
    pub kind: ItemKind,
    pub id: u64,
    pub author: Option<String>,
    /// `createdAt`; a review's `submittedAt`.
    pub at: Option<Time>,
    pub review_state: Option<String>,
    pub inline: Option<u32>,
}

/// The last 20 of each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemsRead {
    pub comments: Vec<ItemRead>,
    pub reviews: Vec<ItemRead>,
}

// ---- the merge (T05) ----

/// STA-5: per kind, the newest `at` seen and the IDs at that time. Serialises as STA-6's `seen` object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub comment: SeenKind,
    pub review: SeenKind,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SeenKind {
    pub at: Option<Time>,
    pub ids: Vec<u64>,
}

/// What the merge remembers about a PR between reads, beside the record. The store keeps it (STA-4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Carry {
    /// MOD-2: the request number last applied.
    pub applied_seq: u64,
    /// MOD-6.
    pub settle_start: Option<Mono>,
    /// MOD-8, MOD-9.
    pub known: Option<Known>,
    /// STA-5.
    pub seen: Seen,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Known {
    pub mergeable: Mergeable,
    pub merge_state: String,
    pub head_sha: String,
    pub base: String,
    pub read_at: Time,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReadMeta {
    /// The number the gate gave the request (MOD-2).
    pub seq: u64,
    /// When the request was sent.
    pub sent_wall: Time,
    pub sent_mono: Mono,
    /// The response's `Date` header (MOD-6).
    pub github_date: Option<Time>,
}

/// POL-12, POL-4.
#[derive(Clone, Copy, Debug)]
pub enum ItemsOutcome<'a> {
    NotAsked,
    Read(&'a ItemsRead),
    Failed,
}

#[derive(Clone, Debug)]
pub enum Before<'a> {
    /// First sight (found, read through): the items are a baseline, the event is `tracked` (STA-5).
    New {
        source: Source,
    },
    /// Loaded from the state file, first refresh (STA-8, MOD-15): `saved_open` says whether it was saved open.
    Loaded {
        source: Source,
        saved_open: bool,
    },
    Known(&'a Record),
}

// The contract's shape: `Applied` carries a whole record by value; boxing it is a contract change.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Merged {
    /// Sent before the read last applied (MOD-2): nothing changes.
    Older,
    /// `kinds` empty: no visible change.
    Applied {
        record: Record,
        carry: Carry,
        kinds: Vec<EventKind>,
    },
}

/// The first 7 characters; a shorter string whole. Built by T02 (T10 needs it before T05).
pub fn short_sha(sha: &str) -> &str {
    match sha.char_indices().nth(7) {
        Some((i, _)) => &sha[..i],
        None => sha,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A record with every field filled, every time in whole seconds.
    pub(crate) fn sample_record() -> Record {
        let t = |s: &str| Time::parse(s).unwrap();
        Record {
            pr: PrName::parse("yachiko/circitor#12").unwrap(),
            url: "https://github.com/yachiko/circitor/pull/12".into(),
            title: "A title".into(),
            author: Some("someone".into()),
            state: PrState::Open,
            draft: true,
            base: "main".into(),
            head: "feature".into(),
            head_sha: "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b".into(),
            created_at: Some(t("2026-11-24T20:00:00Z")),
            updated_at: Some(t("2026-11-24T21:00:00Z")),
            merged_at: Some(t("2026-11-24T21:30:00Z")),
            closed_at: Some(t("2026-11-24T21:31:00Z")),
            mergeable: Mergeable::Other("odd".into()),
            merge_state: "blocked".into(),
            review: Review::ChangesRequested,
            checks: Checks {
                state: ChecksState::Failing,
                settled: true,
                truncated: true,
                total: 2,
                completed: 2,
                failed: 1,
                failed_items: vec![FailedItem {
                    name: "test".into(),
                    url: Some("https://x/2".into()),
                }],
                items: Some(vec![
                    CheckItem {
                        kind: CheckKind::Run,
                        id: Some(7),
                        name: "test".into(),
                        phase: Phase::Completed,
                        result: Some("failure".into()),
                        url: Some("https://x/2".into()),
                    },
                    CheckItem {
                        kind: CheckKind::Status,
                        id: None,
                        name: "ci/legacy".into(),
                        phase: Phase::InProgress,
                        result: None,
                        url: None,
                    },
                ]),
            },
            counts: Counts {
                comments: 3,
                reviews: 2,
                review_threads: 1,
            },
            status: Status::ChecksFailing,
            source: Source::Manual,
            rev: 41,
            fetched_at: t("2026-11-24T22:09:23Z"),
            stale: true,
            error: Some(RecordError {
                type_: "FORBIDDEN".into(),
                since: t("2026-11-24T22:08:00Z"),
            }),
        }
    }

    /// An event at `rev`, of one `tracked` kind, with no `id`.
    pub(crate) fn sample_event(rev: u64) -> Event {
        Event {
            rev,
            pr: PrName::parse("yachiko/circitor#12").unwrap(),
            at: Time::parse("2026-11-24T22:09:23Z").unwrap(),
            kinds: vec![EventKind::Tracked {
                source: Source::Authored,
            }],
            id: NodeId::default(),
        }
    }

    #[test]
    fn short_sha_is_seven_characters() {
        assert_eq!(short_sha("0123456789abcdef"), "0123456");
        assert_eq!(short_sha("0123456"), "0123456");
        assert_eq!(short_sha("abc"), "abc");
        assert_eq!(short_sha(""), "");
    }

    // MOD-2: the values lowered, anything unlisted kept.
    #[test]
    fn mod2_mergeable_and_review() {
        let words: Vec<String> = ["MERGEABLE", "CONFLICTING", "UNKNOWN", "FOO"]
            .iter()
            .map(|g| serde_json::to_string(&Mergeable::from_github(g)).unwrap())
            .collect();
        assert_eq!(words, ["\"yes\"", "\"no\"", "\"unknown\"", "\"foo\""]);
        assert_eq!(Review::from_github(None), Review::None);
        assert_eq!(
            serde_json::to_string(&Review::from_github(None)).unwrap(),
            "\"none\""
        );
        assert_eq!(Review::from_github(Some("APPROVED")), Review::Approved);
        assert_eq!(
            Review::from_github(Some("CHANGES_REQUESTED")),
            Review::ChangesRequested
        );
        assert_eq!(
            Review::from_github(Some("REVIEW_REQUIRED")),
            Review::ReviewRequired
        );
        assert_eq!(
            Review::from_github(Some("ODD_ONE")),
            Review::Other("odd_one".into())
        );
        for (json, m) in [
            ("\"yes\"", Mergeable::Yes),
            ("\"what\"", Mergeable::Other("what".into())),
        ] {
            assert_eq!(serde_json::from_str::<Mergeable>(json).unwrap(), m);
        }
        for (json, r) in [
            ("\"changes_requested\"", Review::ChangesRequested),
            ("\"none\"", Review::None),
            (
                "\"anything at all\"",
                Review::Other("anything at all".into()),
            ),
        ] {
            assert_eq!(serde_json::from_str::<Review>(json).unwrap(), r);
        }
    }

    /// One of each of MOD-15's fourteen kinds.
    pub(crate) fn every_kind() -> Vec<EventKind> {
        let at = Time::parse("2026-11-24T22:09:23Z").unwrap();
        vec![
            EventKind::Status {
                from: Status::ChecksPending,
                to: Status::Ready,
            },
            EventKind::Head {
                from: "1a2b3c4".into(),
                to: "9f8e7d6".into(),
            },
            EventKind::Checks {
                state: ChecksState::Failing,
                total: 5,
                completed: 5,
                failed: 1,
                failed_items: vec![FailedItem {
                    name: "build".into(),
                    url: Some("https://x/1".into()),
                }],
            },
            EventKind::Merged,
            EventKind::Closed,
            EventKind::Reopened,
            EventKind::Gone,
            EventKind::Draft { draft: false },
            EventKind::Mergeable {
                mergeable: Mergeable::No,
                merge_state: "dirty".into(),
            },
            EventKind::Review {
                review: Review::Approved,
            },
            EventKind::Activity {
                counts: Counts {
                    comments: 2,
                    reviews: 1,
                    review_threads: 1,
                },
                items: vec![Item {
                    kind: ItemKind::Review,
                    id: 42,
                    author: Some("someone".into()),
                    at,
                    review_state: Some("commented".into()),
                    inline: Some(1),
                    fetch: "repos/o/r/pulls/1/reviews/42".into(),
                    fetch_inline: Some("repos/o/r/pulls/1/reviews/42/comments".into()),
                }],
                more: true,
            },
            EventKind::Edited {
                fields: vec!["title".into()],
            },
            EventKind::Tracked {
                source: Source::Authored,
            },
            EventKind::Untracked {
                source: Source::Asked,
            },
        ]
    }

    // MOD-15: each kind is an object tagged by its word.
    #[test]
    fn mod15_kind_words() {
        let kinds = every_kind();
        let words: Vec<&str> = kinds.iter().map(EventKind::word).collect();
        assert_eq!(
            words,
            [
                "status",
                "head",
                "checks",
                "merged",
                "closed",
                "reopened",
                "gone",
                "draft",
                "mergeable",
                "review",
                "activity",
                "edited",
                "tracked",
                "untracked"
            ]
        );
        for kind in &kinds {
            let json = serde_json::to_value(kind).unwrap();
            assert_eq!(json["kind"], kind.word());
        }
    }
}
