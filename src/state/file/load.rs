//! Loading (STA-8).

use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;

use super::{StateFs, VERSION, bad_path};
use crate::log::Logger;
use crate::model::{PrState, Source};
use crate::state::{FileContent, FileEntry};
use crate::status::{WarningCode, Warnings};

pub enum Loaded {
    /// Also for a missing file: empty.
    Content(FileContent),
    /// Not JSON or no integer `version`: renamed to `state.json.bad`; start empty.
    SetAside,
    /// A newer `version`: left alone, never written; start empty.
    Newer,
}

/// The keys an entry must hold, `null` or not (STA-6). A key missing is an entry that doesn't decode.
const ENTRY_KEYS: [&str; 6] = ["pr", "id", "source", "login", "ended", "seen"];

/// STA-8. Skips and logs entries that don't decode; raises `state_file_set_aside` or `state_file_newer`.
pub fn load(fs: &dyn StateFs, path: &Path, logger: &Logger, warnings: &Warnings) -> Loaded {
    // STA-8: a missing file is normal.
    let bytes = match fs.read(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Loaded::Content(FileContent::default()),
        Err(e) => {
            // Open question of T16: STA-8 doesn't say what an unreadable file is.
            logger.error(format_args!("state file can't be read type={:?}", e.kind()));
            return Loaded::Content(FileContent::default());
        }
    };
    let root: Option<Value> = serde_json::from_slice(&bytes).ok();
    // STA-8: the `version` is read from the raw JSON before any entry is decoded.
    let version = root
        .as_ref()
        .and_then(|v| v.get("version"))
        .filter(|v| v.is_i64() || v.is_u64())
        .map(|v| v.as_u64());
    let (root, version) = match (root, version) {
        (Some(root), Some(version)) => (root, version),
        _ => return set_aside(fs, path, logger, warnings),
    };
    // STA-8: a file with a newer `version` is left alone. A negative one is an earlier one.
    if version.is_some_and(|v| v > VERSION) {
        logger.warn(format_args!(
            "state file newer version={}",
            version.unwrap_or_default()
        ));
        warnings.raise(
            WarningCode::StateFileNewer,
            "",
            "state.json was written by a newer circitor and is left alone".to_string(),
        );
        return Loaded::Newer;
    }
    // STA-8: a binary reads every earlier version, and ignores keys it doesn't know.
    let Some(list) = root.get("prs").and_then(Value::as_array) else {
        // Open question of T16: STA-8 sets aside only the two cases above.
        logger.warn("state file has no list of prs");
        return Loaded::Content(FileContent::default());
    };
    let mut seen_ids = HashSet::new();
    let mut prs = Vec::new();
    for (index, raw) in list.iter().enumerate() {
        match decode_entry(raw) {
            Some(entry) if seen_ids.insert(entry.id.clone()) => prs.push(entry),
            _ => {
                // STA-8: an entry is named by its index, never by its content.
                logger.warn(format_args!("state file entry skipped index={index}"));
            }
        }
    }
    Loaded::Content(FileContent { prs })
}

// STA-8: a key missing, an unknown `source`, `asked` and an `open` end don't decode.
fn decode_entry(raw: &Value) -> Option<FileEntry> {
    let object = raw.as_object()?;
    if ENTRY_KEYS.iter().any(|k| !object.contains_key(*k)) {
        return None;
    }
    let entry: FileEntry = serde_json::from_value(raw.clone()).ok()?;
    if entry.source == Source::Asked {
        return None;
    }
    if entry
        .ended
        .as_ref()
        .is_some_and(|e| e.state == PrState::Open)
    {
        return None;
    }
    Some(entry)
}

// STA-8: renamed to `state.json.bad`, replacing an earlier one; the warning and the log carry the time.
fn set_aside(fs: &dyn StateFs, path: &Path, logger: &Logger, warnings: &Warnings) -> Loaded {
    if let Err(e) = fs.rename(path, &bad_path(path)) {
        logger.error(format_args!(
            "state file can't be set aside type={:?}",
            e.kind()
        ));
    }
    logger.warn("state file set aside");
    warnings.raise(
        WarningCode::StateFileSetAside,
        "",
        "state.json wasn't valid and was renamed to state.json.bad".to_string(),
    );
    Loaded::SetAside
}
