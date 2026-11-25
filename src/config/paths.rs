#![allow(unused_variables, dead_code)] // stub: T06 removes this

use std::path::Path;

use super::{Env, PathError, Paths, Platform};

/// Owned by someone else, can't be created or changed; names the path. T06 fills it.
#[derive(Debug, thiserror::Error)]
pub enum DirError {}

/// OPS-3, OPS-4. Touches no file: it only computes.
pub fn resolve(env: &Env, platform: Platform) -> Result<Paths, PathError> {
    unimplemented!("T06")
}

/// OPS-3: creates it with mode 0700, or sets an existing one of the user's to 0700; one owned by anyone else is an error naming it.
pub fn ensure_private_dir(path: &Path) -> Result<(), DirError> {
    unimplemented!("T06")
}
