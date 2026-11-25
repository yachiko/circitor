#![allow(unused_variables, dead_code)] // stub: T48 removes this
//! `circitor notify` (NTF-11; T48).

use crate::cli::args::NotifyWhat;
use crate::cli::{CliEnv, Output};

/// docs/07.
pub const SOCKET_VAR: &str = "CLAUDE_CODE_MESSAGING_SOCKET";
pub const TOKEN_VAR: &str = "CLAUDE_CODE_MESSAGING_TOKEN";

/// `circitor notify`, whole (NTF-11). `commands::run` calls it for `Command::Notify` and returns what it returns.
pub async fn run(
    pr: Option<&str>,
    what: &NotifyWhat,
    json: bool,
    no_spawn: bool,
    env: &CliEnv,
) -> Output {
    unimplemented!("T48")
}
