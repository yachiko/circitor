//! Decoding: a pure function over the status, the headers and the body (POL-24, POL-25, MOD-2, MOD-3).
//!
//! OPS-9: no body and no message from GitHub is kept, logged or returned. The secondary-limit check
//! is the one place a message is read: it's compared and dropped.

use std::time::Duration;

use http::HeaderMap;
use serde_json::Value;

use super::query::{self, Slot};
use crate::clock::Time;
use crate::github::{
    Answer, Failure, Found, FoundPr, LookupAnswer, NodeAnswer, Query, Rate, Unavailable, Want,
};
use crate::model::{
    CheckItem, CheckKind, ChecksRead, Counts, ItemKind, ItemRead, ItemsRead, Mergeable, NodeId,
    Phase, PrName, PrRead, ReadState, Review,
};

/// POL-25: the phrase that names a secondary rate limit, compared in any case. Assumed (docs/02 §2,
/// §6 item 6): kept in one place.
pub(crate) const SECONDARY_PHRASE: &str = "secondary rate limit";

/// MOD-2's own words for a PR's error, and the plan's (P10) for a node of another type.
pub(crate) const UNLISTED_STATE: &str = "UNLISTED_STATE";
pub(crate) const UNDECODABLE: &str = "UNDECODABLE";
pub(crate) const UNTYPED: &str = "UNTYPED";
pub(crate) const NOT_A_PULL_REQUEST: &str = "NOT_A_PULL_REQUEST";
const NOT_FOUND: &str = "NOT_FOUND";

/// The body as it arrived: `None` when it was cut short or couldn't be read.
// TST-12: the production decoder the fixtures in `testdata/github/` are decoded with.
pub(crate) fn decode(
    query: &Query,
    status: u16,
    headers: &HeaderMap,
    body: Option<&[u8]>,
) -> Result<Answer, Failure> {
    let failure = |kind| Failure::Unavailable {
        kind,
        status: Some(status),
    };
    // POL-25 before POL-19 and POL-24; POL-20's 401 first of all.
    if status == 401 {
        return Err(Failure::Unauthorized);
    }
    let json: Option<Value> = body.and_then(|b| serde_json::from_slice(b).ok());
    let data = json
        .as_ref()
        .and_then(|j| j.get("data"))
        .filter(|d| d.is_object());

    // POL-25:exhausted: no `data`, and `x-ratelimit-remaining: 0`.
    if data.is_none() && header_u64(headers, "x-ratelimit-remaining") == Some(0) {
        return Err(Failure::Exhausted {
            reset_at: header_reset(headers),
        });
    }

    // POL-25:a-secondary-limit: a 403, or a 200 with an error, naming a secondary rate limit.
    let secondary = match status {
        403 => match &json {
            Some(j) => names_secondary(j),
            None => body.is_some_and(|b| contains_phrase(&String::from_utf8_lossy(b))),
        },
        200 => json.as_ref().is_some_and(errors_name_secondary),
        _ => false,
    };
    if secondary {
        return Err(Failure::Secondary {
            retry_after: header_u64(headers, "retry-after").map(Duration::from_secs),
        });
    }

    // POL-24: any other status is handled as POL-19.
    if status != 200 {
        return Err(failure(Unavailable::Http));
    }
    let Some(json) = &json else {
        return Err(failure(Unavailable::BadBody));
    };
    let Some(data) = data else {
        return Err(failure(Unavailable::NoData));
    };

    let errors = Errors::read(json);
    let plan = query::plan(query);

    let rate = if errors.any_under(&["rateLimit"]) {
        None
    } else {
        data.get("rateLimit").and_then(|r| decode_rate(r, headers))
    };

    let found = match &query.find {
        Some(_) if !errors.any_under(&["viewer"]) => data.get("viewer").and_then(decode_found),
        _ => None,
    };

    let nodes = query
        .nodes
        .iter()
        .zip(&plan.slots)
        .map(|(node, slot)| decode_node(data, &errors, &node.want, slot))
        .collect();

    let lookup = query.lookup.as_ref().map(|_| decode_lookup(data, &errors));

    Ok(Answer {
        rate,
        date: header_date(headers),
        found,
        nodes,
        lookup,
    })
}

// ---- headers ----

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    header_str(headers, name).and_then(|v| v.parse().ok())
}

/// `x-ratelimit-reset`, in epoch seconds.
fn header_reset(headers: &HeaderMap) -> Option<Time> {
    header_u64(headers, "x-ratelimit-reset")
        .and_then(|s| i64::try_from(s).ok())
        .and_then(|s| s.checked_mul(1_000))
        .map(Time::from_unix_millis)
}

/// The `Date` header (MOD-6), an HTTP date.
fn header_date(headers: &HeaderMap) -> Option<Time> {
    let value = header_str(headers, "date")?;
    let t = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some(Time::from_unix_millis(t.timestamp_millis()))
}

// ---- the secondary limit ----

fn contains_phrase(message: &str) -> bool {
    message.to_lowercase().contains(SECONDARY_PHRASE)
}

/// A REST-style `message`, or any error's `message`.
fn names_secondary(json: &Value) -> bool {
    json.get("message")
        .and_then(Value::as_str)
        .is_some_and(contains_phrase)
        || errors_name_secondary(json)
}

fn errors_name_secondary(json: &Value) -> bool {
    json.get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| {
            errors.iter().any(|e| {
                e.get("message")
                    .and_then(Value::as_str)
                    .is_some_and(contains_phrase)
            })
        })
}

// ---- partial errors (POL-21) ----

/// One path step: a key or an index.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    Key(String),
    Index(u64),
}

/// The errors' paths and types; never their messages (OPS-9).
struct Errors(Vec<(Vec<Step>, Option<String>)>);

impl Errors {
    fn read(json: &Value) -> Errors {
        let list = json
            .get("errors")
            .and_then(Value::as_array)
            .map(|errors| {
                errors
                    .iter()
                    .map(|e| {
                        let path = e
                            .get("path")
                            .and_then(Value::as_array)
                            .map(|p| {
                                p.iter()
                                    .filter_map(|s| match s {
                                        Value::String(k) => Some(Step::Key(k.clone())),
                                        Value::Number(n) => n.as_u64().map(Step::Index),
                                        _ => None,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let type_ = e.get("type").and_then(Value::as_str).map(str::to_string);
                        (path, type_)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Errors(list)
    }

    /// The errors whose path points at `at` or inside it, or at a field that holds it.
    fn at<'a>(&'a self, at: &'a [Step]) -> impl Iterator<Item = &'a Option<String>> + 'a {
        self.0.iter().filter_map(move |(path, type_)| {
            let n = path.len().min(at.len());
            (!path.is_empty() && path[..n] == at[..n]).then_some(type_)
        })
    }

    fn any_under(&self, keys: &[&str]) -> bool {
        let at: Vec<Step> = keys.iter().map(|k| Step::Key((*k).to_string())).collect();
        self.at(&at).next().is_some()
    }
}

/// POL-21: of the errors on a part, `NOT_FOUND` if one is, else the first one's type.
enum PartError {
    NotFound,
    Other(String),
}

fn part_error<'a>(mut types: impl Iterator<Item = &'a Option<String>>) -> Option<PartError> {
    let first = types.next()?;
    let mut all = std::iter::once(first).chain(types);
    if all.any(|t| t.as_deref() == Some(NOT_FOUND)) {
        return Some(PartError::NotFound);
    }
    // MOD-2: an error with no `type` is `UNTYPED`.
    Some(PartError::Other(
        first.clone().unwrap_or_else(|| UNTYPED.to_string()),
    ))
}

// ---- the parts ----

fn decode_rate(rate: &Value, headers: &HeaderMap) -> Option<Rate> {
    let cost = u32::try_from(rate.get("cost")?.as_u64()?).ok()?;
    let remaining = u32::try_from(rate.get("remaining")?.as_u64()?).ok()?;
    let reset_at = match rate.get("resetAt").and_then(Value::as_str) {
        Some(t) => Time::parse(t).ok(),
        None => None,
    }
    .or_else(|| header_reset(headers));
    Some(Rate {
        cost,
        remaining,
        reset_at,
    })
}

fn decode_found(viewer: &Value) -> Option<Found> {
    let login = viewer.get("login")?.as_str()?.to_string();
    let list = viewer.get("pullRequests")?;
    let prs = list
        .get("nodes")?
        .as_array()?
        .iter()
        .map(|pr| {
            let number = pr.get("number")?.as_u64()?;
            Some(FoundPr {
                id: NodeId(str_field(pr, "id")?),
                pr: pr_name(pr.get("repository")?, number)?,
                updated_at: time_field(pr, "updatedAt").ok()?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Found {
        login,
        prs,
        next: next_page(list.get("pageInfo")?)?,
    })
}

fn decode_node(data: &Value, errors: &Errors, want: &Want, slot: &Slot) -> NodeAnswer {
    let mut at = vec![Step::Key(slot.alias.clone())];
    let mut value = data.get(&slot.alias);
    if let Some(i) = slot.index {
        at.push(Step::Index(i as u64));
        value = value.and_then(|list| list.get(i));
    }
    let value = value.filter(|v| !v.is_null());
    match (value, part_error(errors.at(&at))) {
        (None, Some(PartError::NotFound)) => NodeAnswer::NotFound,
        (_, Some(PartError::NotFound)) => NodeAnswer::Error {
            type_: NOT_FOUND.to_string(),
        },
        (_, Some(PartError::Other(type_))) => NodeAnswer::Error { type_ },
        // MOD-2: `null` with no error.
        (None, None) => error(UNDECODABLE),
        (Some(node), None) => match pr_node(node) {
            Err(type_) => NodeAnswer::Error { type_ },
            Ok(node) => match want {
                Want::Record => match decode_pr(node) {
                    Ok(read) => NodeAnswer::Record {
                        read: Box::new(read),
                        items: None,
                    },
                    Err(type_) => NodeAnswer::Error { type_ },
                },
                Want::RecordAndItems => match decode_pr(node).and_then(|r| Ok((r, items(node)?))) {
                    Ok((read, items)) => NodeAnswer::Record {
                        read: Box::new(read),
                        items: Some(items),
                    },
                    Err(type_) => NodeAnswer::Error { type_ },
                },
                Want::Items => match items(node) {
                    Ok(items) => NodeAnswer::Items(items),
                    Err(type_) => NodeAnswer::Error { type_ },
                },
                Want::ChecksPage { .. } => match checks(node) {
                    Some(page) => NodeAnswer::ChecksPage(page),
                    None => error(UNDECODABLE),
                },
            },
        },
    }
}

fn error(type_: &str) -> NodeAnswer {
    NodeAnswer::Error {
        type_: type_.to_string(),
    }
}

/// POL-16: `NOT_FOUND` on the repository or the PR is `NotFound`; any other error is `Error`.
fn decode_lookup(data: &Value, errors: &Errors) -> LookupAnswer {
    let pr = data
        .get("repository")
        .filter(|r| !r.is_null())
        .and_then(|r| r.get("pullRequest"))
        .filter(|p| !p.is_null());
    let path = [
        Step::Key("repository".into()),
        Step::Key("pullRequest".into()),
    ];
    // An error on `["repository"]` alone also points at the PR.
    match (pr, part_error(errors.at(&path))) {
        (None, Some(PartError::NotFound)) => LookupAnswer::NotFound,
        (_, Some(PartError::NotFound)) => LookupAnswer::Error {
            type_: NOT_FOUND.to_string(),
        },
        (_, Some(PartError::Other(type_))) => LookupAnswer::Error { type_ },
        (None, None) => LookupAnswer::Error {
            type_: UNDECODABLE.to_string(),
        },
        (Some(node), None) => match pr_node(node).and_then(|n| Ok((decode_pr(n)?, items(n)?))) {
            Ok((read, items)) => LookupAnswer::Found {
                read: Box::new(read),
                items,
            },
            Err(type_) => LookupAnswer::Error { type_ },
        },
    }
}

/// A node of another type is `NOT_A_PULL_REQUEST`.
fn pr_node(node: &Value) -> Result<&Value, String> {
    match node.get("__typename").and_then(Value::as_str) {
        Some("PullRequest") => Ok(node),
        Some(_) => Err(NOT_A_PULL_REQUEST.to_string()),
        None => Err(UNDECODABLE.to_string()),
    }
}

// ---- the record (MOD-2) ----

fn undecodable() -> String {
    UNDECODABLE.to_string()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_string)
}

fn req_str(v: &Value, key: &str) -> Result<String, String> {
    str_field(v, key).ok_or_else(undecodable)
}

/// A time or `null`; a field missing or not a time can't be decoded.
fn time_field(v: &Value, key: &str) -> Result<Option<Time>, String> {
    match v.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Time::parse(s).map(Some).map_err(|_| undecodable()),
        _ => Err(undecodable()),
    }
}

/// A login, or `None` for a deleted account (`author: null`).
fn login(v: &Value) -> Result<Option<String>, String> {
    match v.get("author") {
        Some(Value::Null) => Ok(None),
        Some(author) => author
            .get("login")
            .and_then(Value::as_str)
            .map(|l| Some(l.to_string()))
            .ok_or_else(undecodable),
        None => Err(undecodable()),
    }
}

fn total(v: &Value, key: &str) -> Result<u32, String> {
    v.get(key)
        .and_then(|c| c.get("totalCount"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(undecodable)
}

fn pr_name(repository: &Value, number: u64) -> Option<PrName> {
    let slug = repository.get("nameWithOwner")?.as_str()?;
    PrName::parse(&format!("{slug}#{number}")).ok()
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

fn decode_pr(pr: &Value) -> Result<PrRead, String> {
    // MOD-2: a `state` this file doesn't list is the PR's error.
    let state = match pr.get("state").and_then(Value::as_str) {
        Some("OPEN") => ReadState::Open,
        Some("MERGED") => ReadState::Merged,
        Some("CLOSED") => ReadState::Closed,
        Some(_) => return Err(UNLISTED_STATE.to_string()),
        None => return Err(undecodable()),
    };
    let number = pr
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(undecodable)?;
    let review = match pr.get("reviewDecision") {
        Some(Value::Null) => Review::from_github(None),
        Some(Value::String(s)) => Review::from_github(Some(s)),
        _ => return Err(undecodable()),
    };
    Ok(PrRead {
        id: NodeId(req_str(pr, "id")?),
        pr: pr
            .get("repository")
            .and_then(|r| pr_name(r, number))
            .ok_or_else(undecodable)?,
        url: req_str(pr, "url")?,
        title: req_str(pr, "title")?,
        author: login(pr)?,
        state,
        draft: pr
            .get("isDraft")
            .and_then(Value::as_bool)
            .ok_or_else(undecodable)?,
        base: req_str(pr, "baseRefName")?,
        head: req_str(pr, "headRefName")?,
        head_sha: req_str(pr, "headRefOid")?,
        created_at: time_field(pr, "createdAt")?,
        updated_at: time_field(pr, "updatedAt")?,
        merged_at: time_field(pr, "mergedAt")?,
        closed_at: time_field(pr, "closedAt")?,
        // MOD-2: an unlisted value passes through, lowered.
        mergeable: Mergeable::from_github(&req_str(pr, "mergeable")?),
        merge_state: lower(&req_str(pr, "mergeStateStatus")?),
        review,
        counts: Counts {
            comments: total(pr, "commentCount")?,
            reviews: total(pr, "reviewCount")?,
            review_threads: total(pr, "reviewThreads")?,
        },
        checks: checks(pr).ok_or_else(undecodable)?,
    })
}

// ---- the checks (MOD-3) ----

/// The last commit's rollup. A `null` rollup, or no commit, is no checks (docs/02 §4).
fn checks(pr: &Value) -> Option<ChecksRead> {
    let commits = pr.get("commits")?.get("nodes")?.as_array()?;
    let rollup = match commits.first() {
        None => &Value::Null,
        Some(c) => c.get("commit")?.get("statusCheckRollup")?,
    };
    if rollup.is_null() {
        return Some(ChecksRead {
            items: Vec::new(),
            total: 0,
            rollup: None,
            next_page: None,
        });
    }
    let state = rollup.get("state")?.as_str()?;
    let contexts = rollup.get("contexts")?;
    let items = contexts
        .get("nodes")?
        .as_array()?
        .iter()
        .map(check_item)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    Some(ChecksRead {
        items,
        total: u32::try_from(contexts.get("totalCount")?.as_u64()?).ok()?,
        rollup: Some(lower(state)),
        next_page: next_page(contexts.get("pageInfo")?)?,
    })
}

/// `endCursor` while `hasNextPage`.
fn next_page(info: &Value) -> Option<Option<String>> {
    if info.get("hasNextPage")?.as_bool()? {
        Some(Some(str_field(info, "endCursor")?))
    } else {
        Some(None)
    }
}

fn opt_str(v: &Value, key: &str) -> Option<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(_) => None,
    }
}

/// MOD-3's table. `Some(None)`: a context of an unknown `__typename`, skipped. `None`: can't be decoded.
fn check_item(node: &Value) -> Option<Option<CheckItem>> {
    match node.get("__typename")?.as_str()? {
        "CheckRun" => {
            // MOD-3: `status`, lowered; anything else counts as `queued`.
            let phase = match node.get("status")?.as_str()? {
                "QUEUED" => Phase::Queued,
                "IN_PROGRESS" => Phase::InProgress,
                "COMPLETED" => Phase::Completed,
                _ => Phase::Queued,
            };
            // `null` until completed, whatever `conclusion` says.
            let result = match phase {
                Phase::Completed => opt_str(node, "conclusion")?.map(|c| lower(&c)),
                _ => None,
            };
            let id = match node.get("databaseId") {
                None | Some(Value::Null) => None,
                Some(n) => Some(n.as_u64()?),
            };
            Some(Some(CheckItem {
                kind: CheckKind::Run,
                id,
                name: str_field(node, "name")?,
                phase,
                result,
                url: opt_str(node, "detailsUrl")?,
            }))
        }
        "StatusContext" => {
            let state = node.get("state")?.as_str()?;
            // MOD-3: `completed` unless `PENDING` or `EXPECTED`.
            let (phase, result) = match state {
                "PENDING" | "EXPECTED" => (Phase::InProgress, None),
                other => (Phase::Completed, Some(lower(other))),
            };
            Some(Some(CheckItem {
                kind: CheckKind::Status,
                id: None,
                name: str_field(node, "context")?,
                phase,
                result,
                url: opt_str(node, "targetUrl")?,
            }))
        }
        _ => Some(None),
    }
}

// ---- the items (POL-12, MOD-12) ----

fn item_id(node: &Value) -> Result<u64, String> {
    node.get("databaseId")
        .and_then(Value::as_u64)
        .ok_or_else(undecodable)
}

fn items(pr: &Value) -> Result<ItemsRead, String> {
    let list = |key: &str| -> Result<&Vec<Value>, String> {
        pr.get(key)
            .and_then(|c| c.get("nodes"))
            .and_then(Value::as_array)
            .ok_or_else(undecodable)
    };
    let comments = list("lastComments")?
        .iter()
        .map(|c| {
            Ok(ItemRead {
                kind: ItemKind::Comment,
                id: item_id(c)?,
                author: login(c)?,
                at: time_field(c, "createdAt")?,
                review_state: None,
                inline: None,
            })
        })
        .collect::<Result<_, String>>()?;
    let reviews = list("lastReviews")?
        .iter()
        .map(|r| {
            Ok(ItemRead {
                kind: ItemKind::Review,
                id: item_id(r)?,
                author: login(r)?,
                at: time_field(r, "submittedAt")?,
                review_state: Some(lower(&req_str(r, "state")?)),
                inline: Some(total(r, "comments")?),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(ItemsRead { comments, reviews })
}
