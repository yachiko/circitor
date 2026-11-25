#![allow(unused_variables, dead_code)] // stub: T11 removes this
//! Naming a PR (CLI-2; T11).

use crate::model::{PrName, Record};

#[derive(Clone, Debug, PartialEq)]
pub enum PrArg {
    Full(PrName),
    Number(u64),
    Bare,
}

/// `owner/repo#12`, a URL, `12`, nothing.
pub fn pr_arg(arg: Option<&str>) -> Result<PrArg, String> {
    unimplemented!("T11")
}

/// github.com only: https, ssh, `git@`.
pub fn repo_of_origin(url: &str) -> Option<(String, String)> {
    unimplemented!("T11")
}

#[derive(Clone, Debug, PartialEq)]
pub enum BranchMatch {
    Open(PrName),
    Ended(PrName),
    None,
    Several(Vec<PrName>),
}

/// CLI-2's choice among the records of a `list`: head branch, repository, author, open before ended.
pub fn match_branch(
    records: &[Record],
    owner: &str,
    repo: &str,
    branch: &str,
    login: Option<&str>,
    ended_too: bool,
) -> BranchMatch {
    unimplemented!("T11")
}
