#![allow(unused_variables, dead_code)] // stub: T06 removes this

use std::path::Path;

use super::Config;

/// One line naming the key or the file (OPS-1). `Display` is the message printed on stderr. T06 fills it.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {}

/// A missing file is `Config::default()`. Anything else wrong is an error (OPS-1).
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    unimplemented!("T06")
}
