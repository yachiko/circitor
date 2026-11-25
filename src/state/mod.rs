//! The state store and the state file (`plan/contracts/state.md`). T02's and final: T15 builds
//! `store`, T16 `file`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::clock::Time;
use crate::model::{NodeId, PrName, PrState, Seen, Source};

pub mod file;
pub mod store;

pub use file::{Loaded, RealFs, Saver, SaverDeps, StateFs, load};
pub use store::{
    Applied, Class, Named, PrView, Store, StoreDeps, Subscribed, Target, Timing, Tracked,
    Untracked, View, Waiting,
};

/// STA-3's 200, API-14's 64, NTF-2's 64.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("limit")]
pub struct Limit;

/// STA-3.
pub const MAX_PLACES: usize = 200;
/// API-14.
pub const MAX_WAITERS: u32 = 64;
/// NTF-2; counted apart from the waiters.
pub const MAX_SUBSCRIPTIONS: u32 = 64;
/// STA-10; also `limit`'s range (API-8).
pub const MAX_EVENTS: usize = 500;

/// STA-9. `Display` is `<epoch>-<rev>`; `FromStr` takes exactly that (8 of `[a-z0-9]`, `-`, digits) or fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: String,
    pub rev: u64,
}

/// Not in STA-9's form.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a cursor")]
pub struct CursorError;

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.epoch, self.rev)
    }
}

impl FromStr for Cursor {
    type Err = CursorError;

    // STA-9: `<epoch>-<rev>`, the epoch 8 lower-case letters and digits, the rev digits only.
    fn from_str(s: &str) -> Result<Cursor, CursorError> {
        let (epoch, rev) = s.split_once('-').ok_or(CursorError)?;
        let epoch_ok = epoch.len() == 8
            && epoch
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        let rev_ok = !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_digit());
        if !epoch_ok || !rev_ok {
            return Err(CursorError);
        }
        let rev = rev.parse::<u64>().map_err(|_| CursorError)?;
        Ok(Cursor {
            epoch: epoch.to_string(),
            rev,
        })
    }
}

/// What `state.json` holds, version 1 (STA-6), without its `version`: reading and writing that key,
/// and the object around `prs`, are `state::file`'s own (T16). Serde names are the file's.
/// **No subscription is ever in it** (STA-6, NTF-9): there's no field for one, and none is added.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileContent {
    pub prs: Vec<FileEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub pr: PrName,
    pub id: NodeId,
    /// Never `asked`.
    pub source: Source,
    pub login: Option<String>,
    pub ended: Option<Ended>,
    pub seen: Seen,
}

/// `merged`, `closed` or `gone`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ended {
    pub state: PrState,
    pub seen_at: Time,
}

/// What the saver reads (T16), and the store provides (T15).
pub trait FileSource: Send + Sync + 'static {
    fn file_content(&self) -> FileContent;
    /// Changes whenever `file_content()` would return something else.
    fn file_changed(&self) -> tokio::sync::watch::Receiver<u64>;
}

/// What the routes call after a `track` or an `untrack` (STA-7). T16's saver implements it.
#[async_trait::async_trait]
pub trait Flush: Send + Sync + 'static {
    /// Writes now if the content changed since the last write, whatever the 5 s spacing, and returns
    /// when the write succeeded or failed. A failure is the saver's to report and retry.
    async fn flush(&self);
}

/// The fake: does nothing.
pub struct NoFlush;

#[async_trait::async_trait]
impl Flush for NoFlush {
    async fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // STA-9: a cursor is `<epoch>-<rev>` and nothing else.
    #[test]
    fn sta9_cursor_form() {
        for s in ["k3x9a2bq-42", "k3x9a2bq-0"] {
            let c: Cursor = s.parse().unwrap();
            assert_eq!(c.epoch, "k3x9a2bq");
            assert_eq!(c.to_string(), s);
        }
        let max = format!("k3x9a2bq-{}", u64::MAX);
        assert_eq!(max.parse::<Cursor>().unwrap().rev, u64::MAX);
        let beyond = format!("k3x9a2bq-{}0", u64::MAX);
        for bad in [
            "k3x9a2b-1",
            "k3x9a2bqz-1",
            "K3x9a2bq-1",
            "k3x9a2bq1",
            "k3x9a2bq-",
            "k3x9a2bq-x",
            "k3x9a2bq-+1",
            "k3x9a2bq--1",
            "k3x9a2bq-1-2",
            "",
            beyond.as_str(),
        ] {
            assert!(bad.parse::<Cursor>().is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn no_flush_does_nothing() {
        NoFlush.flush().await;
    }
}
