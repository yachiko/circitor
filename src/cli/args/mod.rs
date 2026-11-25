#![allow(unused_variables, dead_code)] // stub: T11 removes this
//! The command line (T11).

use crate::api::Ask;
use crate::config::Env;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    List {
        all: bool,
        since: Option<String>,
        json: bool,
    },
    Show {
        pr: Option<String>,
        fresh: bool,
        json: bool,
    },
    /// `Ask`: `api.md`; `--fail-fast` and `--kind` are in it (D8).
    Wait {
        pr: Option<String>,
        cond: Ask,
        timeout: Option<u32>,
        json: bool,
    },
    /// NTF-11.
    Notify {
        pr: Option<String>,
        what: NotifyWhat,
        json: bool,
    },
    Changes {
        pr: Option<String>,
        since: Option<String>,
        json: bool,
    },
    Track {
        pr: String,
        json: bool,
    },
    Untrack {
        pr: String,
        json: bool,
    },
    Refresh {
        json: bool,
    },
    Status {
        json: bool,
    },
    Stop,
    Daemon {
        spawned: bool,
        check_config: bool,
    },
    Guide,
    Version,
    Help(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum NotifyWhat {
    /// `life`: `--for`, 60 to 86 400.
    Register {
        cond: Ask,
        life: Option<u32>,
    },
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub command: Command,
    pub no_spawn: bool,
}

/// `Err` is a usage error: its text for stderr, exit code 2 (CLI-5).
pub fn parse(args: &[String], env: &Env) -> Result<Parsed, String> {
    unimplemented!("T11")
}
