//! The GraphQL engine: walks a checked query's selection sets and answers each field from the world.
//!
//! Only what circitor's queries reach has a resolver (`SUPPORTED`); any other field the schema has
//! is `NoResolver`, found before anything is answered, so a new query field fails loudly.

use std::collections::BTreeSet;

use apollo_compiler::Node;
use apollo_compiler::executable::{Field, SelectionSet};
use apollo_compiler::schema::Type;
use serde_json::{Map, Value, json};

use super::cost::{Measure, measure};
use super::query::{Prepared, prepare, sub_sets};
use super::world::{Check, PrState, World};
use super::{Executed, Rejected, valid_schema};
use crate::testkit::MARKER_GITHUB_TEXT;

/// The fields the fake answers, by the type the selection is on (an object, an interface or a
/// union). `*Connection` and `*Edge` types are answered generically.
const SUPPORTED: &[(&str, &[&str])] = &[
    (
        "Query",
        &["viewer", "rateLimit", "nodes", "node", "repository"],
    ),
    (
        "RateLimit",
        &["cost", "remaining", "limit", "resetAt", "nodeCount", "used"],
    ),
    ("User", &["id", "login", "pullRequests"]),
    ("Actor", &["login"]),
    ("RepositoryOwner", &["id", "login"]),
    ("Node", &["id"]),
    (
        "Repository",
        &["id", "name", "nameWithOwner", "owner", "pullRequest", "url"],
    ),
    (
        "PullRequest",
        &[
            "id",
            "number",
            "title",
            "url",
            "state",
            "isDraft",
            "merged",
            "closed",
            "createdAt",
            "updatedAt",
            "closedAt",
            "mergedAt",
            "repository",
            "author",
            "baseRefName",
            "headRefName",
            "headRefOid",
            "mergeable",
            "mergeStateStatus",
            "reviewDecision",
            "autoMergeRequest",
            "mergeQueueEntry",
            "mergeCommit",
            "comments",
            "reviews",
            "reviewThreads",
            "commits",
        ],
    ),
    (
        "PageInfo",
        &["hasNextPage", "hasPreviousPage", "startCursor", "endCursor"],
    ),
    ("PullRequestCommit", &["id", "commit"]),
    (
        "Commit",
        &["id", "oid", "committedDate", "statusCheckRollup"],
    ),
    ("StatusCheckRollup", &["id", "state", "contexts"]),
    (
        "CheckRun",
        &[
            "id",
            "databaseId",
            "name",
            "status",
            "conclusion",
            "detailsUrl",
            "startedAt",
            "completedAt",
        ],
    ),
    (
        "StatusContext",
        &["id", "context", "state", "targetUrl", "createdAt"],
    ),
    (
        "IssueComment",
        &[
            "id",
            "databaseId",
            "author",
            "createdAt",
            "updatedAt",
            "url",
        ],
    ),
    (
        "PullRequestReview",
        &[
            "id",
            "databaseId",
            "author",
            "state",
            "submittedAt",
            "createdAt",
            "updatedAt",
            "url",
            "comments",
        ],
    ),
    ("PullRequestReviewThread", &["id", "comments"]),
    (
        "PullRequestReviewComment",
        &[
            "id",
            "databaseId",
            "author",
            "createdAt",
            "pullRequestReview",
            "url",
        ],
    ),
    // Always `null` here (auto-merge and the merge queue aren't measured), but the full record asks.
    ("AutoMergeRequest", &["enabledAt"]),
    ("MergeQueueEntry", &["position", "state"]),
];

fn supported(ty: &str, field: &str) -> bool {
    if field == "__typename" {
        return true;
    }
    if ty.ends_with("Connection") {
        return matches!(field, "totalCount" | "nodes" | "edges" | "pageInfo");
    }
    if ty.ends_with("Edge") {
        return matches!(field, "cursor" | "node");
    }
    SUPPORTED
        .iter()
        .any(|(t, fields)| *t == ty && fields.contains(&field))
}

fn check_supported(
    prep: &Prepared,
    set: &SelectionSet,
    seen: &mut BTreeSet<String>,
) -> Result<(), Rejected> {
    use apollo_compiler::executable::Selection;
    for sel in &set.selections {
        match sel {
            Selection::Field(f) => {
                if !supported(set.ty.as_str(), f.name.as_str()) {
                    return Err(Rejected::NoResolver(format!("{}.{}", set.ty, f.name)));
                }
                check_supported(prep, &f.selection_set, seen)?;
            }
            Selection::InlineFragment(i) => check_supported(prep, &i.selection_set, seen)?,
            Selection::FragmentSpread(s) => {
                if seen.insert(s.fragment_name.to_string())
                    && let Some(frag) = prep.doc.fragments.get(&s.fragment_name)
                {
                    check_supported(prep, &frag.selection_set, seen)?;
                }
            }
        }
    }
    Ok(())
}

/// Parses `request` (the POST body: `query`, `variables`), checks it against the schema, answers only
/// the fields asked from the world as it is now, and costs it. Paging is by cursor over the world as it
/// is at this call.
pub fn execute(world: &World, request: &[u8]) -> Result<Executed, Rejected> {
    // TST-3: parsed and checked against the schema, then refused unless the fake resolves every field.
    let prep = prepare(request)?;
    check_supported(&prep, &prep.op.selection_set, &mut BTreeSet::new())?;
    let measure = measure(&prep)?;
    let mut ex = Exec {
        world,
        prep: &prep,
        measure,
        errors: Vec::new(),
        node_ids: Vec::new(),
        mergeable_read: BTreeSet::new(),
        runs_read: BTreeSet::new(),
        no_resolver: None,
    };
    let mut path = Vec::new();
    let data = ex.select(&Obj::Query, &[&prep.op.selection_set], &mut path);
    if let Some(missing) = ex.no_resolver {
        return Err(Rejected::NoResolver(missing));
    }
    let mut body = Map::new();
    body.insert("data".into(), data.map_or(Value::Null, Value::Object));
    if !ex.errors.is_empty() {
        body.insert("errors".into(), Value::Array(ex.errors));
    }
    let prs: Vec<usize> = ex.mergeable_read.into_iter().collect();
    let runs: Vec<(usize, usize)> = ex.runs_read.into_iter().collect();
    world.spend_reads(&prs, &runs);
    Ok(Executed {
        body: Value::Object(body),
        cost: measure.points(),
        node_ids: ex.node_ids,
    })
}

/// An object being answered.
enum Obj {
    Query,
    RateLimit,
    /// A user by login: the viewer, an author, an owner.
    User(String),
    Repo(usize),
    Pr(usize),
    PrCommit(usize),
    Commit(usize),
    Rollup(usize),
    Run(usize, usize),
    Status(usize, usize),
    Comment(usize, usize),
    Review(usize, usize),
    Thread(usize, usize),
    ReviewComment(usize, usize, usize),
    Conn(Box<Conn>),
    Edge(String, String, Box<Obj>),
    PageInfo(PageInfo),
}

struct Conn {
    ty: String,
    total: usize,
    items: Vec<(String, Obj)>,
    page: PageInfo,
}

#[derive(Clone)]
struct PageInfo {
    has_next: bool,
    has_prev: bool,
    start: Option<String>,
    end: Option<String>,
}

impl Obj {
    fn type_name(&self) -> &str {
        match self {
            Obj::Query => "Query",
            Obj::RateLimit => "RateLimit",
            Obj::User(_) => "User",
            Obj::Repo(_) => "Repository",
            Obj::Pr(_) => "PullRequest",
            Obj::PrCommit(_) => "PullRequestCommit",
            Obj::Commit(_) => "Commit",
            Obj::Rollup(_) => "StatusCheckRollup",
            Obj::Run(..) => "CheckRun",
            Obj::Status(..) => "StatusContext",
            Obj::Comment(..) => "IssueComment",
            Obj::Review(..) => "PullRequestReview",
            Obj::Thread(..) => "PullRequestReviewThread",
            Obj::ReviewComment(..) => "PullRequestReviewComment",
            Obj::Conn(c) => &c.ty,
            Obj::Edge(ty, ..) => ty,
            Obj::PageInfo(_) => "PageInfo",
        }
    }
}

/// A field's answer before it's completed against its type.
enum Out {
    Leaf(Value),
    Obj(Obj),
    List(Vec<Result<Out, FieldErr>>),
}

/// A field error: GitHub's `type` and a message.
struct FieldErr {
    type_: String,
    message: String,
}

// TST-11: every error message holds the GitHub-text marker.
fn err(type_: &str, message: String) -> FieldErr {
    FieldErr {
        type_: type_.to_owned(),
        message: format!("{MARKER_GITHUB_TEXT}: {message}"),
    }
}

/// A null in a non-null place: it goes up to the nearest nullable parent (GraphQL's rule).
struct Null;

struct Exec<'a> {
    world: &'a World,
    prep: &'a Prepared,
    measure: Measure,
    errors: Vec<Value>,
    node_ids: Vec<String>,
    mergeable_read: BTreeSet<usize>,
    runs_read: BTreeSet<(usize, usize)>,
    no_resolver: Option<String>,
}

fn time(t: crate::clock::Time) -> Value {
    Value::String(t.rfc3339())
}

fn opt_time(t: Option<crate::clock::Time>) -> Value {
    t.map_or(Value::Null, time)
}

fn leaf(v: impl Into<Value>) -> Result<Out, FieldErr> {
    Ok(Out::Leaf(v.into()))
}

fn object(o: Obj) -> Result<Out, FieldErr> {
    Ok(Out::Obj(o))
}

fn null() -> Result<Out, FieldErr> {
    Ok(Out::Leaf(Value::Null))
}

fn user(login: &Option<String>) -> Result<Out, FieldErr> {
    match login {
        Some(l) => object(Obj::User(l.clone())),
        None => null(),
    }
}

impl<'a> Exec<'a> {
    fn select(
        &mut self,
        obj: &Obj,
        sets: &[&'a SelectionSet],
        path: &mut Vec<Value>,
    ) -> Result<Map<String, Value>, Null> {
        let schema = valid_schema();
        let concrete = obj.type_name().to_owned();
        let applies = |t: &str| t == concrete || schema.is_subtype(t, &concrete);
        let grouped = self.prep.collect(sets, &applies);
        let mut out = Map::new();
        for (key, fields) in grouped {
            let f = fields[0];
            path.push(Value::String(key.to_string()));
            let value = if f.name == "__typename" {
                Ok(Value::String(concrete.clone()))
            } else {
                let ty = &f.definition.ty;
                match self.resolve(obj, f) {
                    Ok(o) => self.complete(ty, o, &fields, path),
                    Err(e) => {
                        self.push_error(e, f, path);
                        if ty.is_non_null() {
                            Err(Null)
                        } else {
                            Ok(Value::Null)
                        }
                    }
                }
            };
            path.pop();
            match value {
                Ok(v) => {
                    out.insert(key.to_string(), v);
                }
                Err(Null) => return Err(Null),
            }
        }
        Ok(out)
    }

    fn complete(
        &mut self,
        ty: &Type,
        out: Out,
        fields: &[&'a Node<Field>],
        path: &mut Vec<Value>,
    ) -> Result<Value, Null> {
        let value = match out {
            Out::Leaf(v) => Ok(v),
            Out::Obj(o) => {
                let subs = sub_sets(fields);
                self.select(&o, &subs, path).map(Value::Object)
            }
            Out::List(items) => {
                let item_ty = ty.item_type();
                let mut list = Vec::with_capacity(items.len());
                let mut failed = false;
                for (i, item) in items.into_iter().enumerate() {
                    path.push(json!(i));
                    let v = match item {
                        Ok(o) => self.complete(item_ty, o, fields, path),
                        Err(e) => {
                            self.push_error(e, fields[0], path);
                            if item_ty.is_non_null() {
                                Err(Null)
                            } else {
                                Ok(Value::Null)
                            }
                        }
                    };
                    path.pop();
                    match v {
                        Ok(v) => list.push(v),
                        Err(Null) => {
                            failed = true;
                            break;
                        }
                    }
                }
                if failed {
                    Err(Null)
                } else {
                    Ok(Value::Array(list))
                }
            }
        };
        match value {
            Ok(Value::Null) | Err(Null) if ty.is_non_null() => Err(Null),
            Ok(v) => Ok(v),
            Err(Null) => Ok(Value::Null),
        }
    }

    fn push_error(&mut self, e: FieldErr, field: &Node<Field>, path: &[Value]) {
        let mut entry = Map::new();
        entry.insert("type".into(), Value::String(e.type_));
        entry.insert("path".into(), Value::Array(path.to_vec()));
        if let Some(range) = field.line_column_range(&self.prep.doc.sources) {
            entry.insert(
                "locations".into(),
                json!([{"line": range.start.line, "column": range.start.column}]),
            );
        }
        entry.insert("message".into(), Value::String(e.message));
        self.errors.push(Value::Object(entry));
    }

    fn resolve(&mut self, obj: &Obj, f: &'a Node<Field>) -> Result<Out, FieldErr> {
        let w = self.world;
        let name = f.name.as_str();
        match obj {
            Obj::Query => self.query_field(f),
            Obj::RateLimit => {
                let (remaining, reset_at) = w.rate();
                let cost = self.measure.points();
                let left = remaining.saturating_sub(cost);
                match name {
                    "cost" => leaf(cost),
                    "remaining" => leaf(left),
                    "limit" => leaf(5000),
                    "used" => leaf(5000u32.saturating_sub(left)),
                    "resetAt" => leaf(time(reset_at)),
                    "nodeCount" => leaf(self.measure.nodes),
                    _ => self.missing(obj, f),
                }
            }
            Obj::User(login) => match name {
                "login" => leaf(login.clone()),
                "id" => leaf(format!("U_fake_{login}")),
                "pullRequests" => self.user_pull_requests(login, f),
                _ => self.missing(obj, f),
            },
            Obj::Repo(r) => {
                let repo = &w.repos[*r];
                match name {
                    "id" => leaf(repo.id.clone()),
                    "name" => leaf(repo.name.clone()),
                    "nameWithOwner" => leaf(format!("{}/{}", repo.owner, repo.name)),
                    "url" => leaf(format!("https://github.com/{}/{}", repo.owner, repo.name)),
                    "owner" => object(Obj::User(repo.owner.clone())),
                    "pullRequest" => {
                        let number = self.prep.arg_i64(f, "number").unwrap_or(0);
                        let found = w.prs.iter().position(|p| {
                            p.repo == *r && i64::try_from(p.number).is_ok_and(|n| n == number)
                        });
                        match found {
                            Some(i) if w.pr_visible(i) => match &w.prs[i].node_error {
                                Some(t) => Err(err(
                                    t,
                                    format!("The pull request #{number} can't be read."),
                                )),
                                None => object(Obj::Pr(i)),
                            },
                            _ => Err(err(
                                "NOT_FOUND",
                                format!(
                                    "Could not resolve to a PullRequest with the number of {number}."
                                ),
                            )),
                        }
                    }
                    _ => self.missing(obj, f),
                }
            }
            Obj::Pr(i) => self.pr_field(*i, obj, f),
            Obj::PrCommit(c) => match name {
                "id" => leaf(format!("PURC_fake{c}")),
                "commit" => object(Obj::Commit(*c)),
                _ => self.missing(obj, f),
            },
            Obj::Commit(c) => {
                let commit = &w.commits[*c];
                match name {
                    "id" => leaf(format!("C_fake{c}")),
                    "oid" => leaf(commit.oid.clone()),
                    "committedDate" => leaf(time(commit.committed_date)),
                    "statusCheckRollup" => match w.rollup_state(*c) {
                        Some(_) => object(Obj::Rollup(*c)),
                        None => null(),
                    },
                    _ => self.missing(obj, f),
                }
            }
            Obj::Rollup(c) => match name {
                "id" => leaf(format!("SCR_fake{c}")),
                "state" => leaf(w.rollup_state(*c)),
                "contexts" => {
                    let checks = &w.commits[*c].checks;
                    let items = (0..checks.len())
                        .map(|i| match checks[i] {
                            Check::Run(_) => Obj::Run(*c, i),
                            Check::Status(_) => Obj::Status(*c, i),
                        })
                        .collect();
                    self.connection(f, items)
                }
                _ => self.missing(obj, f),
            },
            Obj::Run(c, i) => {
                let Some(Check::Run(r)) = w.commits[*c].checks.get(*i) else {
                    return null();
                };
                let misread = r
                    .misreads
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .front()
                    .cloned();
                if matches!(name, "status" | "conclusion" | "completedAt") {
                    self.runs_read.insert((*c, *i));
                }
                let repo = &w.repos[w.commits[*c].repo];
                match (name, misread) {
                    ("status", Some(s)) => leaf(s),
                    ("conclusion" | "completedAt", Some(_)) => null(),
                    ("id", _) => leaf(format!("CR_fake{}", r.database_id)),
                    ("databaseId", _) => leaf(r.database_id),
                    ("name", _) => leaf(r.name.clone()),
                    ("status", None) => leaf(r.status.clone()),
                    ("conclusion", None) => leaf(r.conclusion.clone()),
                    ("detailsUrl", _) => leaf(format!(
                        "https://github.com/{}/{}/actions/runs/1/job/{}",
                        repo.owner, repo.name, r.database_id
                    )),
                    ("startedAt", _) => leaf(opt_time(r.started_at)),
                    ("completedAt", None) => leaf(opt_time(r.completed_at)),
                    _ => self.missing(obj, f),
                }
            }
            Obj::Status(c, i) => {
                let Some(Check::Status(s)) = w.commits[*c].checks.get(*i) else {
                    return null();
                };
                match name {
                    "id" => leaf(format!("SC_fake{c}_{i}")),
                    "context" => leaf(s.context.clone()),
                    "state" => leaf(s.state.clone()),
                    "targetUrl" => null(),
                    "createdAt" => leaf(time(s.created_at)),
                    _ => self.missing(obj, f),
                }
            }
            Obj::Comment(p, i) => {
                let pr = &w.prs[*p];
                let c = &pr.comments[*i];
                match name {
                    "id" => leaf(format!("IC_fake{}", c.database_id)),
                    "databaseId" => leaf(c.database_id),
                    "author" => user(&c.author),
                    "createdAt" | "updatedAt" => leaf(time(c.created_at)),
                    "url" => leaf(format!(
                        "{}#issuecomment-{}",
                        self.pr_url(*p),
                        c.database_id
                    )),
                    _ => self.missing(obj, f),
                }
            }
            Obj::Review(p, i) => {
                let pr = &w.prs[*p];
                let r = &pr.reviews[*i];
                match name {
                    "id" => leaf(format!("PRR_fake{}", r.database_id)),
                    "databaseId" => leaf(r.database_id),
                    "author" => user(&r.author),
                    "state" => leaf(r.state.clone()),
                    "submittedAt" | "createdAt" | "updatedAt" => leaf(time(r.submitted_at)),
                    "url" => leaf(format!(
                        "{}#pullrequestreview-{}",
                        self.pr_url(*p),
                        r.database_id
                    )),
                    "comments" => {
                        let items = pr
                            .threads
                            .iter()
                            .enumerate()
                            .flat_map(|(t, th)| {
                                th.comments
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, c)| c.review == *i)
                                    .map(move |(k, _)| Obj::ReviewComment(*p, t, k))
                            })
                            .collect();
                        self.connection(f, items)
                    }
                    _ => self.missing(obj, f),
                }
            }
            Obj::Thread(p, t) => match name {
                "id" => leaf(format!("PRRT_fake{p}_{t}")),
                "comments" => {
                    let n = w.prs[*p].threads[*t].comments.len();
                    let items = (0..n).map(|k| Obj::ReviewComment(*p, *t, k)).collect();
                    self.connection(f, items)
                }
                _ => self.missing(obj, f),
            },
            Obj::ReviewComment(p, t, k) => {
                let c = &w.prs[*p].threads[*t].comments[*k];
                match name {
                    "id" => leaf(format!("PRRC_fake{}", c.database_id)),
                    "databaseId" => leaf(c.database_id),
                    "author" => user(&c.author),
                    "createdAt" => leaf(time(c.created_at)),
                    "pullRequestReview" => object(Obj::Review(*p, c.review)),
                    "url" => leaf(format!("{}#discussion_r{}", self.pr_url(*p), c.database_id)),
                    _ => self.missing(obj, f),
                }
            }
            Obj::Conn(c) => match name {
                "totalCount" => leaf(c.total),
                "pageInfo" => object(Obj::PageInfo(c.page.clone())),
                "nodes" => Ok(Out::List(
                    c.items
                        .iter()
                        .map(|(_, o)| Ok(Out::Obj(o.reborrow())))
                        .collect(),
                )),
                "edges" => {
                    let ty = f.definition.ty.inner_named_type().to_string();
                    Ok(Out::List(
                        c.items
                            .iter()
                            .map(|(cursor, o)| {
                                Ok(Out::Obj(Obj::Edge(
                                    ty.clone(),
                                    cursor.clone(),
                                    Box::new(o.reborrow()),
                                )))
                            })
                            .collect(),
                    ))
                }
                _ => self.missing(obj, f),
            },
            Obj::Edge(_, cursor, node) => match name {
                "cursor" => leaf(cursor.clone()),
                "node" => object(node.reborrow()),
                _ => self.missing(obj, f),
            },
            Obj::PageInfo(p) => match name {
                "hasNextPage" => leaf(p.has_next),
                "hasPreviousPage" => leaf(p.has_prev),
                "startCursor" => leaf(p.start.clone()),
                "endCursor" => leaf(p.end.clone()),
                _ => self.missing(obj, f),
            },
        }
    }

    fn missing(&mut self, obj: &Obj, f: &Node<Field>) -> Result<Out, FieldErr> {
        if self.no_resolver.is_none() {
            self.no_resolver = Some(format!("{}.{}", obj.type_name(), f.name));
        }
        null()
    }

    fn pr_url(&self, i: usize) -> String {
        let p = &self.world.prs[i];
        let r = &self.world.repos[p.repo];
        format!(
            "https://github.com/{}/{}/pull/{}",
            r.owner, r.name, p.number
        )
    }

    fn query_field(&mut self, f: &'a Node<Field>) -> Result<Out, FieldErr> {
        let w = self.world;
        match f.name.as_str() {
            "viewer" => object(Obj::User(w.login().to_owned())),
            "rateLimit" => object(Obj::RateLimit),
            "nodes" => {
                let ids = self
                    .prep
                    .arg(f, "ids")
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let items = ids
                    .iter()
                    .map(|id| {
                        let id = id.as_str().unwrap_or_default().to_owned();
                        self.node_ids.push(id.clone());
                        self.node_by_id(&id).map(Out::Obj)
                    })
                    .collect();
                Ok(Out::List(items))
            }
            "node" => {
                let id = self.prep.arg_str(f, "id").unwrap_or_default();
                self.node_ids.push(id.clone());
                self.node_by_id(&id).map(Out::Obj)
            }
            "repository" => {
                let owner = self.prep.arg_str(f, "owner").unwrap_or_default();
                let name = self.prep.arg_str(f, "name").unwrap_or_default();
                let found = w.repos.iter().position(|r| {
                    !r.hidden
                        && r.owner.eq_ignore_ascii_case(&owner)
                        && r.name.eq_ignore_ascii_case(&name)
                });
                match found {
                    Some(r) => object(Obj::Repo(r)),
                    None => Err(err(
                        "NOT_FOUND",
                        format!(
                            "Could not resolve to a Repository with the name '{owner}/{name}'."
                        ),
                    )),
                }
            }
            _ => self.missing(&Obj::Query, f),
        }
    }

    fn node_by_id(&self, id: &str) -> Result<Obj, FieldErr> {
        let w = self.world;
        let not_found = || {
            err(
                "NOT_FOUND",
                format!("Could not resolve to a node with the global id of '{id}'"),
            )
        };
        if let Some(i) = w.prs.iter().position(|p| p.id == id) {
            if !w.pr_visible(i) {
                return Err(not_found());
            }
            return match &w.prs[i].node_error {
                Some(t) => Err(err(t, format!("The node '{id}' can't be read."))),
                None => Ok(Obj::Pr(i)),
            };
        }
        if let Some(r) = w.repos.iter().position(|r| r.id == id && !r.hidden) {
            return Ok(Obj::Repo(r));
        }
        Err(not_found())
    }

    fn user_pull_requests(&mut self, login: &str, f: &'a Node<Field>) -> Result<Out, FieldErr> {
        let w = self.world;
        let states: Option<Vec<String>> = self.prep.arg(f, "states").map(|v| match v {
            Value::Array(a) => a
                .iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect(),
            Value::String(s) => vec![s],
            _ => Vec::new(),
        });
        let order = self.prep.arg(f, "orderBy");
        let field = order
            .as_ref()
            .and_then(|o| o.get("field"))
            .and_then(Value::as_str)
            .unwrap_or("CREATED_AT")
            .to_owned();
        let desc = order
            .as_ref()
            .and_then(|o| o.get("direction"))
            .and_then(Value::as_str)
            == Some("DESC");
        let mut prs: Vec<usize> = (0..w.prs.len())
            .filter(|&i| {
                let p = &w.prs[i];
                w.pr_visible(i)
                    && p.author.as_deref() == Some(login)
                    && states
                        .as_ref()
                        .is_none_or(|s| s.iter().any(|s| s == p.state.as_str()))
            })
            .collect();
        let key = |i: &usize| match field.as_str() {
            "UPDATED_AT" => w.view_updated_at(*i),
            _ => w.prs[*i].created_at,
        };
        prs.sort_by(|a, b| {
            let o = key(a).cmp(&key(b)).then(a.cmp(b));
            if desc { o.reverse() } else { o }
        });
        self.connection(f, prs.into_iter().map(Obj::Pr).collect())
    }

    fn pr_field(&mut self, i: usize, obj: &Obj, f: &'a Node<Field>) -> Result<Out, FieldErr> {
        let w = self.world;
        let p = &w.prs[i];
        let name = f.name.as_str();
        if let Some((_, t)) = p.field_errors.iter().find(|(field, _)| field == name) {
            return Err(err(t, format!("The field '{name}' can't be read.")));
        }
        match name {
            "id" => leaf(p.id.clone()),
            "number" => leaf(p.number),
            "title" => leaf(p.title.clone()),
            "url" => leaf(self.pr_url(i)),
            "state" => leaf(p.state.as_str()),
            "isDraft" => leaf(p.draft),
            "merged" => leaf(p.state == PrState::Merged),
            "closed" => leaf(p.state != PrState::Open),
            "createdAt" => leaf(time(p.created_at)),
            "updatedAt" => leaf(time(w.view_updated_at(i))),
            "closedAt" => leaf(opt_time(p.closed_at)),
            "mergedAt" => leaf(opt_time(p.merged_at)),
            "repository" => object(Obj::Repo(p.repo)),
            "author" => user(&p.author),
            "baseRefName" => leaf(p.base.clone()),
            "headRefName" => leaf(p.head_ref.clone()),
            "headRefOid" => leaf(w.commits[w.view_head(i)].oid.clone()),
            "mergeable" => {
                self.mergeable_read.insert(i);
                leaf(w.mergeability(i).0)
            }
            "mergeStateStatus" => {
                self.mergeable_read.insert(i);
                leaf(w.mergeability(i).1)
            }
            "reviewDecision" => leaf(p.review_decision.clone()),
            "autoMergeRequest" | "mergeQueueEntry" => null(),
            "mergeCommit" => match p.merge_commit {
                Some(c) => object(Obj::Commit(c)),
                None => null(),
            },
            "comments" => {
                let items = (0..p.comments.len()).map(|k| Obj::Comment(i, k)).collect();
                self.connection(f, items)
            }
            "reviews" => {
                let states: Option<Vec<String>> = self.prep.arg(f, "states").and_then(|v| {
                    v.as_array().map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(str::to_owned))
                            .collect()
                    })
                });
                let items = (0..p.reviews.len())
                    .filter(|&k| {
                        states
                            .as_ref()
                            .is_none_or(|s| s.contains(&p.reviews[k].state))
                    })
                    .map(|k| Obj::Review(i, k))
                    .collect();
                self.connection(f, items)
            }
            "reviewThreads" => {
                let items = (0..p.threads.len()).map(|k| Obj::Thread(i, k)).collect();
                self.connection(f, items)
            }
            "commits" => {
                let items = w
                    .view_history(i)
                    .iter()
                    .map(|&c| Obj::PrCommit(c))
                    .collect();
                self.connection(f, items)
            }
            _ => self.missing(obj, f),
        }
    }

    /// A page of `items` by `first`, `last`, `after` and `before`; cursors are positions in the
    /// list as it is at this call, so a change between two pages shifts them (TST-3).
    fn connection(&mut self, f: &Node<Field>, items: Vec<Obj>) -> Result<Out, FieldErr> {
        let total = items.len();
        let cursor = |name: &str| -> Result<Option<usize>, FieldErr> {
            match self.prep.arg_str(f, name) {
                None => Ok(None),
                Some(c) => parse_cursor(&c).map(Some).ok_or_else(|| {
                    err(
                        "INVALID_CURSOR_ARGUMENTS",
                        format!("`{c}` does not appear to be a valid cursor."),
                    )
                }),
            }
        };
        let after = cursor("after")?;
        let before = cursor("before")?;
        let mut start = after.map_or(0, |a| a + 1).min(total);
        let mut end = before.unwrap_or(total).min(total);
        if start > end {
            start = end;
        }
        if let Some(n) = self.prep.arg_i64(f, "first") {
            end = end.min(start + usize::try_from(n.max(0)).unwrap_or(0));
        }
        if let Some(n) = self.prep.arg_i64(f, "last") {
            start = start.max(end.saturating_sub(usize::try_from(n.max(0)).unwrap_or(0)));
        }
        let page = PageInfo {
            has_next: end < total,
            has_prev: start > 0,
            start: (start < end).then(|| make_cursor(start)),
            end: (start < end).then(|| make_cursor(end - 1)),
        };
        let items = items
            .into_iter()
            .enumerate()
            .skip(start)
            .take(end - start)
            .map(|(k, o)| (make_cursor(k), o))
            .collect();
        object(Obj::Conn(Box::new(Conn {
            ty: f.definition.ty.inner_named_type().to_string(),
            total,
            items,
            page,
        })))
    }
}

fn make_cursor(i: usize) -> String {
    format!("Y3Vyc29y:{i}")
}

fn parse_cursor(c: &str) -> Option<usize> {
    c.strip_prefix("Y3Vyc29y:")?.parse().ok()
}

impl Obj {
    /// A copy, for a connection's items handed out as nodes and as edges.
    fn reborrow(&self) -> Obj {
        match self {
            Obj::Query => Obj::Query,
            Obj::RateLimit => Obj::RateLimit,
            Obj::User(l) => Obj::User(l.clone()),
            Obj::Repo(r) => Obj::Repo(*r),
            Obj::Pr(i) => Obj::Pr(*i),
            Obj::PrCommit(c) => Obj::PrCommit(*c),
            Obj::Commit(c) => Obj::Commit(*c),
            Obj::Rollup(c) => Obj::Rollup(*c),
            Obj::Run(c, i) => Obj::Run(*c, *i),
            Obj::Status(c, i) => Obj::Status(*c, *i),
            Obj::Comment(p, i) => Obj::Comment(*p, *i),
            Obj::Review(p, i) => Obj::Review(*p, *i),
            Obj::Thread(p, i) => Obj::Thread(*p, *i),
            Obj::ReviewComment(p, t, k) => Obj::ReviewComment(*p, *t, *k),
            Obj::Conn(c) => Obj::Conn(Box::new(Conn {
                ty: c.ty.clone(),
                total: c.total,
                items: c
                    .items
                    .iter()
                    .map(|(k, o)| (k.clone(), o.reborrow()))
                    .collect(),
                page: c.page.clone(),
            })),
            Obj::Edge(t, c, o) => Obj::Edge(t.clone(), c.clone(), Box::new(o.reborrow())),
            Obj::PageInfo(p) => Obj::PageInfo(p.clone()),
        }
    }
}
