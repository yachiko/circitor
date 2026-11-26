//! Naming a PR (CLI-2; T11). Pure: running `git` and the requests are T27's.

use crate::model::{PrName, PrState, Record};

#[derive(Clone, Debug, PartialEq)]
pub enum PrArg {
    Full(PrName),
    Number(u64),
    Bare,
}

const FORMS: &str = "use owner/repo#12, the PR's URL, or its number";

/// `owner/repo#12`, a URL, `12`, nothing.
pub fn pr_arg(arg: Option<&str>) -> Result<PrArg, String> {
    // CLI-2
    let Some(arg) = arg else {
        return Ok(PrArg::Bare);
    };
    if let Some(rest) = arg.strip_prefix('#')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!(
            "{arg:?} isn't a PR name: write {rest}, because a shell reads #{rest} as a comment; {FORMS}"
        ));
    }
    if !arg.is_empty() && arg.bytes().all(|b| b.is_ascii_digit()) {
        return match arg.parse::<u64>() {
            Ok(n) if n > 0 => Ok(PrArg::Number(n)),
            _ => Err(format!("{arg:?} isn't a PR number; {FORMS}")),
        };
    }
    if let Ok(name) = PrName::parse(arg) {
        return Ok(PrArg::Full(name));
    }
    if let Ok(name) = PrName::from_url(arg) {
        return Ok(PrArg::Full(name));
    }
    Err(format!("{arg:?} isn't a PR name; {FORMS}"))
}

/// github.com only: https, ssh, `git@`.
pub fn repo_of_origin(url: &str) -> Option<(String, String)> {
    // CLI-2
    const PREFIXES: [&str; 3] = [
        "https://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ];
    let url = url.trim();
    let rest = PREFIXES.iter().find_map(|p| {
        let head = url.get(..p.len())?;
        head.eq_ignore_ascii_case(p).then(|| &url[p.len()..])
    })?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, repo) = rest.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (ok(owner) && ok(repo)).then(|| (owner.to_string(), repo.to_string()))
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
    // CLI-2: a login of `None` matches nothing.
    let Some(login) = login else {
        return BranchMatch::None;
    };
    let mine: Vec<&Record> = records
        .iter()
        .filter(|r| {
            r.pr.owner.eq_ignore_ascii_case(owner)
                && r.pr.repo.eq_ignore_ascii_case(repo)
                && r.head == branch
                && r.author
                    .as_deref()
                    .is_some_and(|a| a.eq_ignore_ascii_case(login))
        })
        .collect();
    let mut open: Vec<PrName> = mine
        .iter()
        .filter(|r| r.state == PrState::Open)
        .map(|r| r.pr.clone())
        .collect();
    match open.len() {
        0 => {}
        1 => return BranchMatch::Open(open.remove(0)),
        _ => {
            open.sort();
            return BranchMatch::Several(open);
        }
    }
    if !ended_too {
        return BranchMatch::None;
    }
    mine.iter()
        .filter(|r| r.state != PrState::Open)
        .max_by_key(|r| {
            let at = if r.state == PrState::Gone {
                r.fetched_at
            } else {
                r.closed_at.or(r.merged_at).unwrap_or(r.fetched_at)
            };
            (at, r.pr.clone())
        })
        .map_or(BranchMatch::None, |r| BranchMatch::Ended(r.pr.clone()))
}

#[cfg(test)]
mod tests;
