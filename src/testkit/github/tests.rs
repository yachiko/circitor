//! T14's acceptance tests: hand-written queries against scripted worlds.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::cost::measure;
use super::query::prepare;
use super::*;
use crate::clock::Time;
use crate::testkit::{MARKER_CHECK, MARKER_GITHUB_TEXT, MARKER_LOGIN, MARKER_TITLE};

fn t(s: &str) -> Time {
    Time::parse(s).unwrap()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn millis(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn req(q: &str) -> Vec<u8> {
    json!({ "query": q }).to_string().into_bytes()
}

fn req_vars(q: &str, vars: Value) -> Vec<u8> {
    json!({ "query": q, "variables": vars })
        .to_string()
        .into_bytes()
}

fn body(w: &World, q: &str) -> Value {
    execute(w, &req(q)).unwrap().body
}

fn start() -> Time {
    t("2026-11-24T22:00:00Z")
}

fn world() -> World {
    World::new(MARKER_LOGIN, start())
}

/// The record as circitor reads it (POL-6), as one fragment.
const RECORD: &str = "fragment R on PullRequest { id number title url state isDraft createdAt updatedAt \
    closedAt mergedAt repository { nameWithOwner } author { login } baseRefName headRefName headRefOid \
    mergeable mergeStateStatus reviewDecision comments { totalCount } reviews { totalCount } \
    reviewThreads { totalCount } commits(last: 1) { nodes { commit { oid statusCheckRollup { state \
    contexts(first: 100) { totalCount pageInfo { hasNextPage endCursor } nodes { __typename \
    ... on CheckRun { databaseId name status conclusion startedAt completedAt detailsUrl } \
    ... on StatusContext { context state createdAt } } } } } } } }";

fn read(w: &World, pr: PrId) -> Value {
    let q = format!(
        "query {{ node(id: \"{}\") {{ ...R }} }} {RECORD}",
        w.node_id(pr)
    );
    let b = body(w, &q);
    assert!(b.get("errors").is_none(), "{b}");
    b["data"]["node"].clone()
}

fn rollup(pr: &Value) -> Value {
    pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["state"].clone()
}

fn contexts(pr: &Value) -> Value {
    pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"].clone()
}

/// Each check as `name:status:conclusion` (a status as `context:state`).
fn checks(pr: &Value) -> Vec<String> {
    contexts(pr)["nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| match c["__typename"].as_str() {
                    Some("CheckRun") => format!(
                        "{}:{}:{}",
                        c["name"].as_str().unwrap_or("?"),
                        c["status"].as_str().unwrap_or("?"),
                        c["conclusion"].as_str().unwrap_or("null")
                    ),
                    _ => format!(
                        "{}:{}",
                        c["context"].as_str().unwrap_or("?"),
                        c["state"].as_str().unwrap_or("?")
                    ),
                })
                .collect()
        })
        .unwrap_or_default()
}

const OPEN_LIST: &str = "query($after: String) { viewer { login pullRequests(states: OPEN, first: 100, \
    after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount \
    pageInfo { hasNextPage endCursor } nodes { id number updatedAt } } } }";

fn open_page(w: &World, after: Option<&str>) -> Value {
    let b = execute(w, &req_vars(OPEN_LIST, json!({ "after": after })))
        .unwrap()
        .body;
    b["data"]["viewer"]["pullRequests"].clone()
}

fn open_ids(w: &World) -> Vec<String> {
    open_page(w, None)["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_owned())
        .collect()
}

fn ids_query(ids: &[&str], fragment_body: &str) -> String {
    let list: Vec<String> = ids.iter().map(|i| format!("\"{i}\"")).collect();
    format!(
        "query {{ nodes(ids: [{}]) {{ ... on PullRequest {{ {fragment_body} }} }} }}",
        list.join(", ")
    )
}

// ----- Cost --------------------------------------------------------------------------------

/// docs/02 §2, "Measured cost": each file's cost and node count.
const MEASURED: &[(&str, u32, u64)] = &[
    ("viewer-full-10", 1, 620),
    ("viewer-full-20", 1, 1240),
    ("viewer-full-30", 1, 1860),
    ("viewer-full-50", 2, 3100),
    ("viewer-full-100", 3, 6200),
    ("search-full-50", 2, 3100),
    ("viewer-light-100", 1, 200),
    ("viewer-ids-100", 1, 100),
    ("nodes-full-2", 1, 124),
    ("nodes-full-40", 1, 2480),
    ("ids-list-and-nodes-2", 1, 224),
    ("nodes-40-comment-ids", 2, 2880),
    ("nodes-40-comment-and-review-ids", 2, 3280),
    ("nodes-40-comment-ids-and-threads", 6, 4480),
    ("nodes-40-timeline", 2, 3280),
];

// TST-3: each row of the measured table gets its measured cost.
#[test]
fn tst3_measured_costs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cost");
    let frag = std::fs::read_to_string(dir.join("frag.graphql")).unwrap();
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        if name == "frag" {
            continue;
        }
        let row = MEASURED.iter().find(|(n, ..)| *n == name);
        let &(_, cost, nodes) = row.unwrap_or_else(|| panic!("{name} isn't a row of the table"));
        let mut text = std::fs::read_to_string(&path).unwrap();
        if text.contains("...PR") {
            text.push_str(&frag);
        }
        let request = req(&text);
        assert_eq!(cost_of(&request), Ok(cost), "{name}");
        let m = measure(&prepare(&request).unwrap()).unwrap();
        assert_eq!(m.nodes, nodes, "{name}: nodes");
        seen.push(name);
    }
    assert_eq!(seen.len(), MEASURED.len(), "one file per row: {seen:?}");
}

/// POL-6's record, for the shapes circitor sends.
const POLL_RECORD: &str = "fragment Record on PullRequest { id number title url state isDraft createdAt \
    updatedAt closedAt mergedAt repository { name owner { login } } author { login } baseRefName \
    headRefName headRefOid mergeable mergeStateStatus reviewDecision comments { totalCount } \
    reviews { totalCount } reviewThreads { totalCount } commits(last: 1) { nodes { commit { oid \
    statusCheckRollup { state contexts(first: 100) { totalCount pageInfo { hasNextPage endCursor } \
    nodes { __typename ... on CheckRun { databaseId name status conclusion detailsUrl } \
    ... on StatusContext { context state targetUrl } } } } } } } }";

const POLL_ITEMS: &str = "fragment Items on PullRequest { \
    lastComments: comments(last: 20) { nodes { databaseId author { login } createdAt } } \
    lastReviews: reviews(last: 20) { nodes { databaseId author { login } submittedAt state \
    comments { totalCount } } } }";

fn forty_ids() -> Vec<String> {
    (0..40).map(|i| format!("PR_x{i}")).collect()
}

// TST-3, POL-4, POL-7, POL-12: what circitor's own requests cost.
#[test]
fn tst3_circitor_shapes_cost() {
    let ids = forty_ids();
    let poll = format!(
        "query($ids: [ID!]!) {{ viewer {{ login pullRequests(states: OPEN, first: 100, \
         orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ pageInfo {{ hasNextPage endCursor }} \
         nodes {{ id number updatedAt repository {{ name owner {{ login }} }} }} }} }} \
         nodes(ids: $ids) {{ ...Record }} rateLimit {{ cost remaining resetAt }} }} {POLL_RECORD}"
    );
    assert_eq!(cost_of(&req_vars(&poll, json!({ "ids": ids }))), Ok(1));

    let first = format!(
        "query($ids: [ID!]!) {{ nodes(ids: $ids) {{ ...Record ...Items }} rateLimit {{ cost }} }} \
         {POLL_RECORD} {POLL_ITEMS}"
    );
    assert_eq!(cost_of(&req_vars(&first, json!({ "ids": ids }))), Ok(2));

    let items = format!(
        "query($ids: [ID!]!) {{ nodes(ids: $ids) {{ ...Items }} rateLimit {{ cost }} }} {POLL_ITEMS}"
    );
    assert_eq!(cost_of(&req_vars(&items, json!({ "ids": ids }))), Ok(1));

    // Cost follows the page asked for, never the rows returned (docs/02 §2): an empty world.
    let w = world();
    let answered = execute(&w, &req_vars(&poll, json!({ "ids": ids }))).unwrap();
    assert_eq!(answered.cost, 1);
    assert_eq!(answered.body["data"]["rateLimit"]["cost"], 1);
    assert_eq!(answered.node_ids, ids);
}

// ----- The schema, and only what is asked ----------------------------------------------------

// TST-3: a field the schema lacks, a mutation, a broken document, a field with no resolver.
#[test]
fn tst3_schema_is_checked() {
    let w = world();
    match execute(
        &w,
        &req("query { node(id: \"x\") { ... on PullRequest { nope } } }"),
    ) {
        Err(Rejected::NotInSchema(m)) => {
            assert!(m.contains("nope") && m.contains("PullRequest"), "{m}")
        }
        other => panic!("{other:?}"),
    }
    let mutation =
        "mutation { addComment(input: {subjectId: \"x\", body: \"y\"}) { clientMutationId } }";
    assert_eq!(execute(&w, &req(mutation)).err(), Some(Rejected::Mutation));
    assert_eq!(describe(&req(mutation)).err(), Some(Rejected::Mutation));
    assert!(matches!(
        execute(&w, &req("{ viewer {")),
        Err(Rejected::Unparsable(_))
    ));
    assert!(matches!(
        execute(&w, b"not json"),
        Err(Rejected::Unparsable(_))
    ));
    assert_eq!(
        execute(&w, &req("{ viewer { bio } }")).err(),
        Some(Rejected::NoResolver("User.bio".into()))
    );
    // A page over GitHub's 100 is refused.
    assert!(matches!(
        execute(
            &w,
            &req("{ viewer { pullRequests(first: 101) { totalCount } } }")
        ),
        Err(Rejected::NotInSchema(_))
    ));
}

// TST-3: the answer holds only the fields asked, under the keys asked.
#[test]
fn tst3_only_what_is_asked() {
    let mut w = world();
    assert_eq!(
        body(&w, "{ viewer { login } }"),
        json!({ "data": { "viewer": { "login": MARKER_LOGIN } } })
    );
    assert_eq!(
        body(&w, "{ me: viewer { name: login } }"),
        json!({ "data": { "me": { "name": MARKER_LOGIN } } })
    );
    let repo = w.repo("o", "r");
    let pr = w.open_pr(repo);
    w.run(pr, "build");
    w.status(pr, "ci/legacy", "PENDING");
    let q = format!(
        "{{ node(id: \"{}\") {{ __typename ... on PullRequest {{ number commits(last: 1) {{ nodes {{ commit \
         {{ statusCheckRollup {{ contexts(first: 10) {{ nodes {{ __typename }} }} }} }} }} }} }} }} }}",
        w.node_id(pr)
    );
    let b = body(&w, &q);
    assert_eq!(
        b,
        json!({ "data": { "node": { "__typename": "PullRequest", "number": 1, "commits": { "nodes": [
            { "commit": { "statusCheckRollup": { "contexts": { "nodes": [
                { "__typename": "CheckRun" }, { "__typename": "StatusContext" }
            ] } } } }
        ] } } } })
    );
}

// ----- docs/02 §4 ------------------------------------------------------------------------------

// TST-3: by default the world behaves as docs/02 §4 records: PR C's timeline (c1.log), PR B's
// merge, and what moves `updatedAt`.
#[test]
fn tst3_defaults_follow_docs02() {
    let mut w = World::new(MARKER_LOGIN, t("2026-11-24T22:25:51Z"));
    let repo = w.repo("yachiko", "experiment");
    let c = w.open_pr(repo);
    w.at(t("2026-11-24T22:26:00Z"), move |w| {
        let pass = w.run(c, "pass");
        let fail = w.run(c, "fail");
        let skipped = w.run(c, "skipped");
        let slow = w.run(c, "slow");
        w.finish(skipped, "SKIPPED");
        for r in [pass, fail, slow] {
            w.start(r);
        }
        w.at(t("2026-11-24T22:26:03Z"), move |w| {
            w.finish(pass, "SUCCESS");
            w.finish(fail, "FAILURE");
        });
        w.at(t("2026-11-24T22:27:20Z"), move |w| {
            w.finish(slow, "CANCELLED")
        });
    });
    w.at(t("2026-11-24T22:26:33Z"), move |w| {
        w.status(c, "circitor/manual", "PENDING")
    });
    w.at(t("2026-11-24T22:26:49Z"), move |w| {
        w.status(c, "circitor/manual", "FAILURE")
    });

    let upd = json!("2026-11-24T22:25:51Z");
    w.advance_to(t("2026-11-24T22:25:54Z"));
    let r = read(&w, c);
    assert_eq!(
        (r["mergeable"].clone(), r["mergeStateStatus"].clone()),
        (json!("MERGEABLE"), json!("CLEAN"))
    );
    assert_eq!(
        r["commits"]["nodes"][0]["commit"]["statusCheckRollup"],
        Value::Null
    );
    assert_eq!(r["updatedAt"], upd);

    w.advance_to(t("2026-11-24T22:26:01Z"));
    let r = read(&w, c);
    assert_eq!(rollup(&r), "PENDING");
    assert_eq!(r["mergeStateStatus"], "UNSTABLE");
    assert_eq!(
        checks(&r),
        [
            "pass:IN_PROGRESS:null",
            "fail:IN_PROGRESS:null",
            "skipped:COMPLETED:SKIPPED",
            "slow:IN_PROGRESS:null"
        ]
    );

    w.advance_to(t("2026-11-24T22:26:04Z"));
    let r = read(&w, c);
    assert_eq!(
        rollup(&r),
        "FAILURE",
        "FAILURE at once, with slow still running"
    );
    assert_eq!(checks(&r)[3], "slow:IN_PROGRESS:null");

    w.advance_to(t("2026-11-24T22:26:36Z"));
    let r = read(&w, c);
    assert_eq!(checks(&r)[4], "circitor/manual:PENDING");
    assert_eq!(contexts(&r)["nodes"][4]["__typename"], "StatusContext");
    w.advance_to(t("2026-11-24T22:26:50Z"));
    assert_eq!(checks(&read(&w, c))[4], "circitor/manual:FAILURE");

    w.advance_to(t("2026-11-24T22:27:22Z"));
    let r = read(&w, c);
    assert_eq!(checks(&r)[3], "slow:COMPLETED:CANCELLED");
    assert_eq!(rollup(&r), "FAILURE");
    assert_eq!(
        (r["mergeable"].clone(), r["mergeStateStatus"].clone()),
        (json!("MERGEABLE"), json!("UNSTABLE"))
    );
    assert_eq!(r["updatedAt"], upd, "no check moves updatedAt");

    // PR B: squash-merged.
    let b = w.open_pr(repo);
    w.advance_to(t("2026-11-24T22:28:00Z"));
    assert!(open_ids(&w).contains(&w.node_id(b).to_owned()));
    w.merge(b);
    let r = read(&w, b);
    assert_eq!(r["state"], "MERGED");
    assert_eq!(r["mergeable"], "UNKNOWN");
    assert_eq!(r["mergeStateStatus"], "UNKNOWN");
    assert_eq!(r["mergedAt"], r["closedAt"]);
    assert_eq!(r["mergedAt"], "2026-11-24T22:28:00Z");
    assert!(
        !open_ids(&w).contains(&w.node_id(b).to_owned()),
        "gone from the open list at once"
    );

    // What moves updatedAt: a push, a comment, a draft change, a close; a check doesn't.
    let p = w.open_pr(repo);
    let mut now = w.now();
    let mut step = |w: &mut World, act: &dyn Fn(&mut World), moves: bool| {
        let before = read(w, p)["updatedAt"].clone();
        now = now.plus(secs(10));
        w.advance_to(now);
        act(w);
        let after = read(w, p)["updatedAt"].clone();
        if moves {
            assert_eq!(after, json!(now.rfc3339()));
        } else {
            assert_eq!(after, before);
        }
    };
    step(
        &mut w,
        &|w| {
            w.push(p);
        },
        true,
    );
    step(
        &mut w,
        &|w| {
            w.comment(p);
        },
        true,
    );
    step(&mut w, &|w| w.set_draft(p, true), true);
    step(&mut w, &|w| w.set_draft(p, false), true);
    step(
        &mut w,
        &|w| {
            let r = w.run(p, "ci");
            w.finish(r, "SUCCESS");
            w.status(p, "legacy", "SUCCESS");
        },
        false,
    );
    step(&mut w, &|w| w.close(p), true);
    let r = read(&w, p);
    assert_eq!(
        (r["mergeable"].clone(), r["mergeStateStatus"].clone()),
        (json!("MERGEABLE"), json!("CLEAN")),
        "a closed PR keeps its last"
    );

    // A review moves it, but late (PR E: about 10 s).
    let e = w.open_pr(repo);
    let created = w.now();
    w.advance_to(created.plus(secs(9)));
    w.review(e, "COMMENTED", 1);
    let r = read(&w, e);
    assert_eq!(r["reviews"]["totalCount"], 1, "the count is right at once");
    assert_eq!(r["reviewThreads"]["totalCount"], 1);
    w.advance_to(created.plus(secs(12)));
    assert_eq!(read(&w, e)["updatedAt"], json!(created.rfc3339()));
    w.advance_to(created.plus(secs(19)));
    assert_eq!(
        read(&w, e)["updatedAt"],
        json!(created.plus(secs(9)).rfc3339())
    );
}

// TST-3: changes take effect at their times on the world's clock, in the order scripted.
#[test]
fn tst3_scripted_times() {
    let mut w = world();
    let repo = w.repo("o", "r");
    let pr = w.open_pr(repo);
    w.after(secs(3), move |w| w.set_draft(pr, true));
    w.at(start().plus(secs(5)), move |w| w.set_title(pr, "first"));
    w.at(start().plus(secs(5)), move |w| w.set_title(pr, "second"));

    w.advance_to(start().plus(millis(2900)));
    assert_eq!(read(&w, pr)["isDraft"], false);
    w.advance_to(start().plus(secs(3)));
    let r = read(&w, pr);
    assert_eq!(r["isDraft"], true);
    assert_eq!(
        r["updatedAt"],
        json!(start().plus(secs(3)).rfc3339()),
        "applied at its own time"
    );
    w.advance_to(start().plus(secs(5)));
    assert_eq!(read(&w, pr)["title"], "second");

    // GitHub's clock never goes back.
    w.advance_to(start());
    assert_eq!(w.now(), start().plus(secs(5)));
    // A change due at once in a scheduled change applies in that same advance.
    w.at(start().plus(secs(7)), move |w| {
        w.after(Duration::ZERO, move |w| w.set_title(pr, "nested"))
    });
    w.advance_to(start().plus(secs(8)));
    assert_eq!(read(&w, pr)["title"], "nested");
}

// TST-3: the misbehaviours that change an answer.
#[test]
fn tst3_world_misbehaviours() {
    let mut w = World::new(MARKER_LOGIN, t("2026-11-24T22:09:23Z"));
    let repo = w.repo("yachiko", "experiment");
    let pr = w.open_pr(repo);
    // UNKNOWN for exactly 3 reads, then the value.
    w.unknown_for(pr, 3);
    for _ in 0..3 {
        let r = read(&w, pr);
        assert_eq!(
            (r["mergeable"].clone(), r["mergeStateStatus"].clone()),
            (json!("UNKNOWN"), json!("UNKNOWN"))
        );
    }
    assert_eq!(read(&w, pr)["mergeable"], "MERGEABLE");
    // A request that doesn't ask for it spends no read.
    w.unknown_for(pr, 1);
    body(
        &w,
        &format!("{{ node(id: \"{}\") {{ id }} }}", w.node_id(pr)),
    );
    assert_eq!(read(&w, pr)["mergeable"], "UNKNOWN");
    assert_eq!(read(&w, pr)["mergeable"], "MERGEABLE");

    // PR A: after a push the old head and its old rollup for 3 s, then the new head, rollup null.
    const OLD: &str = "dce9dbe0000000000000000000000000000000aa";
    const NEW: &str = "cf54ec40000000000000000000000000000000bb";
    w.push_sha(pr, OLD);
    let old_updated = w.now();
    for name in ["fast", "slow"] {
        let r = w.run(pr, name);
        w.finish(r, "SUCCESS");
    }
    w.advance_to(t("2026-11-24T22:13:50.600Z"));
    w.push_lagged(pr, Some(NEW), secs(3));
    w.advance_to(t("2026-11-24T22:13:51Z"));
    let r = read(&w, pr);
    assert_eq!(r["headRefOid"], OLD);
    assert_eq!(rollup(&r), "SUCCESS");
    assert_eq!(r["updatedAt"], json!(old_updated.rfc3339()));
    w.advance_to(t("2026-11-24T22:13:54Z"));
    let r = read(&w, pr);
    assert_eq!(r["headRefOid"], NEW);
    assert_eq!(
        r["commits"]["nodes"][0]["commit"]["statusCheckRollup"],
        Value::Null
    );
    assert_eq!(r["updatedAt"], "2026-11-24T22:13:50Z");

    // A run read IN_PROGRESS, then QUEUED, then IN_PROGRESS (docs/02 §4, observed).
    let run = w.run(pr, "late");
    w.start(run);
    assert_eq!(checks(&read(&w, pr)), ["late:IN_PROGRESS:null"]);
    w.misread(run, "QUEUED");
    assert_eq!(checks(&read(&w, pr)), ["late:QUEUED:null"]);
    assert_eq!(checks(&read(&w, pr)), ["late:IN_PROGRESS:null"]);
}

// TST-3, docs/02 §5: the error shapes.
#[test]
fn tst3_error_shapes() {
    let mut w = world();
    let repo = w.repo("o", "r");
    let a = w.open_pr(repo);
    let b = w.open_pr(repo);

    // One unknown ID among two: null at its index, one NOT_FOUND at the key and the index.
    let q = format!(
        "{{ prs: nodes(ids: [\"{}\", \"PR_nope\"]) {{ ... on PullRequest {{ number }} }} }}",
        w.node_id(a)
    );
    let r = body(&w, &q);
    assert_eq!(r["data"]["prs"], json!([{ "number": 1 }, null]));
    let errors = r["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["type"], "NOT_FOUND");
    assert_eq!(errors[0]["path"], json!(["prs", 1]));

    // A PR number that doesn't exist, and a repository that doesn't.
    let r = body(
        &w,
        "{ repository(owner: \"o\", name: \"r\") { pullRequest(number: 9999) { id } } }",
    );
    assert_eq!(r["data"], json!({ "repository": { "pullRequest": null } }));
    assert_eq!(r["errors"][0]["type"], "NOT_FOUND");
    assert_eq!(r["errors"][0]["path"], json!(["repository", "pullRequest"]));
    let r = body(
        &w,
        "{ repository(owner: \"o\", name: \"nope\") { pullRequest(number: 1) { id } } }",
    );
    assert_eq!(r["data"], json!({ "repository": null }));
    assert_eq!(r["errors"][0]["type"], "NOT_FOUND");
    assert_eq!(r["errors"][0]["path"], json!(["repository"]));

    // A scripted FORBIDDEN on one node leaves the others; by name too.
    w.node_error(b, Some("FORBIDDEN"));
    let both = ids_query(&[w.node_id(a), w.node_id(b)], "number");
    let r = body(&w, &both);
    assert_eq!(r["data"]["nodes"], json!([{ "number": 1 }, null]));
    assert_eq!(r["errors"][0]["type"], "FORBIDDEN");
    assert_eq!(r["errors"][0]["path"], json!(["nodes", 1]));
    let r = body(
        &w,
        "{ repository(owner: \"o\", name: \"r\") { pullRequest(number: 2) { id } } }",
    );
    assert_eq!(r["errors"][0]["type"], "FORBIDDEN");
    assert_eq!(r["errors"][0]["path"], json!(["repository", "pullRequest"]));
    w.node_error(b, None);
    assert!(body(&w, &both).get("errors").is_none());

    // A scripted error inside a node: its path goes into it. On a non-null field it empties the
    // node (GraphQL's rule); on a nullable one only the field is null.
    w.field_error(a, "commits", Some("INTERNAL"));
    let r = body(
        &w,
        &ids_query(
            &[w.node_id(a), w.node_id(b)],
            "number commits(last: 1) { totalCount }",
        ),
    );
    assert_eq!(r["data"]["nodes"][0], Value::Null);
    assert_eq!(r["data"]["nodes"][1]["number"], 2);
    assert_eq!(r["errors"][0]["type"], "INTERNAL");
    assert_eq!(r["errors"][0]["path"], json!(["nodes", 0, "commits"]));
    w.field_error(a, "commits", None);
    w.field_error(a, "reviewDecision", Some("FORBIDDEN"));
    let r = body(&w, &ids_query(&[w.node_id(a)], "number reviewDecision"));
    assert_eq!(
        r["data"]["nodes"][0],
        json!({ "number": 1, "reviewDecision": null })
    );
    assert_eq!(
        r["errors"][0]["path"],
        json!(["nodes", 0, "reviewDecision"])
    );
    assert!(r["errors"][0]["locations"].is_array());
}

// TST-3: paging by cursor over the world as it is at each call.
#[test]
fn tst3_paging() {
    let mut w = world();
    let repo = w.repo("o", "r");
    let prs = w.open_prs(repo, 150, secs(1));
    let page1 = open_page(&w, None);
    assert_eq!(page1["nodes"].as_array().unwrap().len(), 100);
    assert_eq!(page1["pageInfo"]["hasNextPage"], true);
    assert_eq!(page1["totalCount"], 150);
    let times: Vec<&str> = page1["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["updatedAt"].as_str().unwrap())
        .collect();
    let mut sorted = times.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(times, sorted, "newest updatedAt first");
    assert_eq!(page1["nodes"][0]["id"], w.node_id(prs[0]));
    let cursor = page1["pageInfo"]["endCursor"].as_str().unwrap().to_owned();
    let page2 = open_page(&w, Some(&cursor));
    assert_eq!(page2["nodes"].as_array().unwrap().len(), 50);
    assert_eq!(page2["pageInfo"]["hasNextPage"], false);
    // A PR closed between the two calls shifts the second page.
    w.close(prs[10]);
    let shifted = open_page(&w, Some(&cursor));
    assert_eq!(shifted["nodes"].as_array().unwrap().len(), 49);
    assert_eq!(shifted["nodes"][0]["id"], w.node_id(prs[101]));

    // 187 checks are two pages, totalCount 187.
    let pr = w.open_pr(repo);
    w.runs(pr, 187, "COMPLETED", Some("SUCCESS"));
    let r = read(&w, pr);
    assert_eq!(contexts(&r)["totalCount"], 187);
    assert_eq!(contexts(&r)["nodes"].as_array().unwrap().len(), 100);
    assert_eq!(contexts(&r)["pageInfo"]["hasNextPage"], true);
    let after = contexts(&r)["pageInfo"]["endCursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let page = check_page(&w, pr, &after);
    assert_eq!(page["nodes"].as_array().unwrap().len(), 87);
    assert_eq!(page["pageInfo"]["hasNextPage"], false);

    // comments(last: 20) of 25 gives the newest 20.
    let ids: Vec<u64> = (0..25).map(|_| w.comment(pr)).collect();
    let r = body(
        &w,
        &ids_query(
            &[w.node_id(pr)],
            "comments(last: 20) { totalCount nodes { databaseId } }",
        ),
    );
    let got: Vec<u64> = r["data"]["nodes"][0]["comments"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["databaseId"].as_u64().unwrap())
        .collect();
    assert_eq!(got, ids[5..]);
    assert_eq!(r["data"]["nodes"][0]["comments"]["totalCount"], 25);
}

/// A further page of checks, as POL-8 asks for it.
fn check_page(w: &World, pr: PrId, after: &str) -> Value {
    let q = "query($id: ID!, $after: String) { node(id: $id) { ... on PullRequest { commits(last: 1) { nodes { \
             commit { statusCheckRollup { contexts(first: 100, after: $after) { totalCount \
             pageInfo { hasNextPage endCursor } nodes { ... on CheckRun { name status conclusion } } } } } } } } } }";
    let b = execute(
        w,
        &req_vars(q, json!({ "id": w.node_id(pr), "after": after })),
    )
    .unwrap()
    .body;
    b["data"]["node"]["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"].clone()
}

// TST-3: one small case per line of `contracts/testkit.md`'s checklist, built with the world's methods.
#[test]
fn tst3_checklist() {
    let mut w = world();
    let repo = w.repo("o", "r");

    // A push with the old head served for N seconds; a force-push back to an earlier SHA.
    let pr = w.open_pr(repo);
    let first = w.head(pr).to_owned();
    let ok = w.run(pr, "build");
    w.finish(ok, "SUCCESS");
    let second = w.push_lagged(pr, None, secs(5));
    assert_eq!(read(&w, pr)["headRefOid"], first.as_str());
    w.advance_to(w.now().plus(secs(5)));
    assert_eq!(read(&w, pr)["headRefOid"], second.as_str());
    w.push_sha(pr, &first);
    let r = read(&w, pr);
    assert_eq!(r["headRefOid"], first.as_str());
    assert_eq!(checks(&r), ["build:COMPLETED:SUCCESS"], "its old checks");
    let r = body(&w, &ids_query(&[w.node_id(pr)], "commits { totalCount }"));
    assert_eq!(r["data"]["nodes"][0]["commits"]["totalCount"], 3);

    // Runs with a databaseId, any status and conclusion (FOO), statuses posted again, a re-run.
    let pr = w.open_pr(repo);
    let foo = w.run(pr, "odd");
    w.finish(foo, "FOO");
    let r = read(&w, pr);
    assert_eq!(checks(&r), ["odd:COMPLETED:FOO"]);
    assert_eq!(
        contexts(&r)["nodes"][0]["databaseId"],
        w.run_database_id(foo)
    );
    assert_eq!(rollup(&r), "FAILURE");
    w.rerun(foo);
    let r = read(&w, pr);
    assert_eq!(checks(&r), ["odd:QUEUED:null"]);
    assert_eq!(
        contexts(&r)["nodes"][0]["databaseId"],
        w.run_database_id(foo)
    );
    for state in ["PENDING", "SUCCESS", "PENDING"] {
        w.status(pr, "ci/x", state);
        assert_eq!(checks(&read(&w, pr))[1], format!("ci/x:{state}"));
    }
    w.misread(foo, "IN_PROGRESS");
    assert_eq!(checks(&read(&w, pr))[0], "odd:IN_PROGRESS:null");
    assert_eq!(checks(&read(&w, pr))[0], "odd:QUEUED:null");

    // 100, 101, 187, 300, 301, 400 checks, with the unread tail's state and the rollup chosen.
    for (n, pages) in [(100, 1), (101, 2), (187, 2), (300, 3), (301, 4), (400, 4)] {
        let pr = w.open_pr(repo);
        w.runs(pr, n.min(300), "COMPLETED", Some("SUCCESS"));
        if n > 300 {
            w.runs(pr, n - 300, "IN_PROGRESS", None);
            w.set_rollup(pr, Some("SUCCESS"));
        }
        let r = read(&w, pr);
        assert_eq!(contexts(&r)["totalCount"], n);
        let mut count = 1;
        let mut page = contexts(&r);
        let mut last = page.clone();
        while page["pageInfo"]["hasNextPage"] == true {
            let after = page["pageInfo"]["endCursor"].as_str().unwrap().to_owned();
            page = check_page(&w, pr, &after);
            last = page.clone();
            count += 1;
        }
        assert_eq!(count, pages, "{n} checks");
        if n > 300 {
            assert_eq!(rollup(&r), "SUCCESS", "the rollup as chosen");
            assert_eq!(
                last["nodes"][0]["status"], "IN_PROGRESS",
                "the tail as chosen"
            );
        }
    }

    // updatedAt set against GitHub's clock.
    let pr = w.open_pr(repo);
    let two_min_ago = w.now().minus(secs(120));
    w.set_updated_at(pr, two_min_ago);
    assert_eq!(read(&w, pr)["updatedAt"], json!(two_min_ago.rfc3339()));

    // UNKNOWN for N reads on every PR at once; a base change; CONFLICTING with DIRTY.
    let mut w2 = world();
    let repo2 = w2.repo("o", "r");
    let all = w2.open_prs(repo2, 3, secs(1));
    w2.unknown_for_all(3);
    let ids: Vec<&str> = all.iter().map(|p| w2.node_id(*p)).collect();
    let q = ids_query(&ids, "mergeable");
    for _ in 0..3 {
        assert_eq!(
            body(&w2, &q)["data"]["nodes"],
            json!(vec![json!({"mergeable": "UNKNOWN"}); 3])
        );
    }
    assert_eq!(
        body(&w2, &q)["data"]["nodes"],
        json!(vec![json!({"mergeable": "MERGEABLE"}); 3])
    );
    w2.set_base(all[0], "develop");
    w2.unknown_for(all[0], 1);
    let r = read(&w2, all[0]);
    assert_eq!(
        (r["baseRefName"].clone(), r["mergeable"].clone()),
        (json!("develop"), json!("UNKNOWN"))
    );
    w2.set_mergeable(all[1], "CONFLICTING");
    let r = read(&w2, all[1]);
    assert_eq!(
        (r["mergeable"].clone(), r["mergeStateStatus"].clone()),
        (json!("CONFLICTING"), json!("DIRTY"))
    );

    // Merge, close, reopen.
    let pr = w.open_pr(repo);
    w.close(pr);
    assert_eq!(read(&w, pr)["state"], "CLOSED");
    assert_eq!(read(&w, pr)["mergedAt"], Value::Null);
    w.reopen(pr);
    let r = read(&w, pr);
    assert_eq!(
        (r["state"].clone(), r["closedAt"].clone()),
        (json!("OPEN"), Value::Null)
    );
    assert!(open_ids(&w).contains(&w.node_id(pr).to_owned()));
    w.merge(pr);
    assert_eq!(read(&w, pr)["state"], "MERGED");

    // Comments and reviews with databaseId, author, time, state and inline count; a thread reply.
    let pr = w.open_pr(repo);
    let c = w.comment_by(pr, Some("alice"));
    let rv = w.review_by(pr, Some("bob"), "CHANGES_REQUESTED", 2);
    let reply = w.thread_reply(pr);
    let items = "comments(last: 20) { nodes { databaseId author { login } createdAt } } \
                 reviews(last: 20) { totalCount nodes { databaseId author { login } submittedAt state comments { totalCount } } } \
                 reviewThreads { totalCount }";
    let r = body(&w, &ids_query(&[w.node_id(pr)], items));
    let n = &r["data"]["nodes"][0];
    assert_eq!(n["comments"]["nodes"][0]["databaseId"], c);
    assert_eq!(n["comments"]["nodes"][0]["author"]["login"], "alice");
    assert_eq!(
        n["comments"]["nodes"][0]["createdAt"],
        json!(w.now().rfc3339())
    );
    assert_eq!(
        n["reviews"]["totalCount"], 2,
        "a thread reply is a review of its own"
    );
    assert_eq!(n["reviews"]["nodes"][0]["databaseId"], rv);
    assert_eq!(n["reviews"]["nodes"][0]["state"], "CHANGES_REQUESTED");
    assert_eq!(n["reviews"]["nodes"][0]["comments"]["totalCount"], 2);
    assert_eq!(n["reviews"]["nodes"][1]["databaseId"], reply);
    assert_eq!(n["reviews"]["nodes"][1]["state"], "COMMENTED");
    assert_eq!(n["reviews"]["nodes"][1]["comments"]["totalCount"], 1);
    assert_eq!(n["reviewThreads"]["totalCount"], 2);

    // A repository renamed and transferred keeps its node IDs.
    let pr = w.open_pr(repo);
    let id = w.node_id(pr).to_owned();
    let number = w.number(pr);
    w.rename_repo(repo, "renamed");
    w.transfer_repo(repo, "elsewhere");
    let r = read(&w, pr);
    assert_eq!(
        (r["id"].clone(), r["repository"]["nameWithOwner"].clone()),
        (json!(id), json!("elsewhere/renamed"))
    );
    let by_name = |w: &World, owner: &str, name: &str| {
        body(
            w,
            &format!(
                "{{ repository(owner: \"{owner}\", name: \"{name}\") {{ pullRequest(number: {number}) {{ id }} }} }}"
            ),
        )
    };
    assert_eq!(
        by_name(&w, "elsewhere", "renamed")["data"]["repository"]["pullRequest"]["id"],
        json!(id)
    );
    assert_eq!(by_name(&w, "o", "r")["errors"][0]["type"], "NOT_FOUND");
    w.transfer_repo(repo, "o");
    w.rename_repo(repo, "r");

    // A PR hidden and returned: off the list, NOT_FOUND by node and by name, back open or merged.
    let pr = w.open_pr(repo);
    let id = w.node_id(pr).to_owned();
    let number = w.number(pr);
    w.hide(pr);
    assert!(!open_ids(&w).contains(&id));
    let r = body(&w, &ids_query(&[&id], "number"));
    assert_eq!(
        (
            r["data"]["nodes"][0].clone(),
            r["errors"][0]["type"].clone()
        ),
        (Value::Null, json!("NOT_FOUND"))
    );
    let r = body(
        &w,
        &format!(
            "{{ repository(owner: \"o\", name: \"r\") {{ pullRequest(number: {number}) {{ id }} }} }}"
        ),
    );
    assert_eq!(r["errors"][0]["type"], "NOT_FOUND");
    w.unhide(pr);
    assert!(open_ids(&w).contains(&id));
    assert_eq!(read(&w, pr)["id"], json!(id));
    w.hide(pr);
    w.merge(pr);
    w.unhide(pr);
    assert_eq!(read(&w, pr)["state"], "MERGED");
    // A repository hidden and returned.
    w.hide_repo(repo);
    assert!(open_ids(&w).is_empty());
    assert_eq!(
        body(&w, "{ repository(owner: \"o\", name: \"r\") { id } }")["errors"][0]["type"],
        "NOT_FOUND"
    );
    w.show_repo(repo);
    assert!(!open_ids(&w).is_empty());

    // An error type on one node until cleared.
    let pr = w.open_pr(repo);
    w.node_error(pr, Some("SERVICE_UNAVAILABLE"));
    for _ in 0..2 {
        assert_eq!(
            body(&w, &ids_query(&[w.node_id(pr)], "number"))["errors"][0]["type"],
            "SERVICE_UNAVAILABLE"
        );
    }
    w.node_error(pr, None);
    assert_eq!(read(&w, pr)["number"], w.number(pr));

    // Bulk worlds of 41, 150, 200 and 205 PRs with distinct updatedAt.
    for n in [41, 150, 200, 205] {
        let mut b = world();
        let r = b.repo("o", "bulk");
        let prs = b.open_prs(r, n, secs(1));
        let mut times: Vec<Time> = prs.iter().map(|p| b.updated_at(*p)).collect();
        times.sort();
        times.dedup();
        assert_eq!(times.len(), n);
        assert_eq!(open_page(&b, None)["totalCount"], n);
    }

    // PRs by other authors and of other owners, a head branch name, two PRs on one branch.
    let mut w3 = world();
    let mine = w3.repo("o", "r");
    let theirs = w3.repo("other-org", "lib");
    let a = w3.open_pr(mine);
    let b = w3.open_pr(mine);
    let c = w3.open_pr(theirs);
    w3.set_head_ref(a, "feature");
    w3.set_head_ref(b, "feature");
    w3.set_author(b, Some("someone-else"));
    let open = open_ids(&w3);
    assert_eq!(open.len(), 2);
    assert!(open.contains(&w3.node_id(a).to_owned()) && open.contains(&w3.node_id(c).to_owned()));
    let r = body(
        &w3,
        &ids_query(
            &[w3.node_id(a), w3.node_id(b), w3.node_id(c)],
            "headRefName author { login } repository { nameWithOwner }",
        ),
    );
    assert_eq!(r["data"]["nodes"][0]["headRefName"], "feature");
    assert_eq!(r["data"]["nodes"][1]["headRefName"], "feature");
    assert_eq!(r["data"]["nodes"][1]["author"]["login"], "someone-else");
    assert_eq!(
        r["data"]["nodes"][2]["repository"]["nameWithOwner"],
        "other-org/lib"
    );

    // A change set to repeat: 26 PRs flipping draft every 30 s.
    let mut w4 = world();
    let r4 = w4.repo("o", "r");
    let prs = w4.open_prs(r4, 26, secs(1));
    let flipping = prs.clone();
    w4.every(start().plus(secs(30)), secs(30), move |w| {
        for p in &flipping {
            let draft = w.is_draft(*p);
            w.set_draft(*p, !draft);
        }
    });
    let ids: Vec<&str> = prs.iter().map(|p| w4.node_id(*p)).collect();
    let q = ids_query(&ids, "isDraft");
    for (at, draft) in [(29, false), (30, true), (59, true), (60, false), (90, true)] {
        w4.advance_to(start().plus(secs(at)));
        assert_eq!(
            body(&w4, &q)["data"]["nodes"],
            json!(vec![json!({"isDraft": draft}); 26]),
            "at {at} s"
        );
    }
}

// TST-3: what a request asked, for T20's request log.
#[test]
fn tst3_what_was_asked() {
    let ids = ["PR_a", "PR_b"];
    let poll = format!(
        "query($ids: [ID!]!, $after: String) {{ viewer {{ login pullRequests(states: OPEN, first: 100, after: $after, \
         orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ id }} }} }} \
         nodes(ids: $ids) {{ ...Record }} items: nodes(ids: [\"PR_c\"]) {{ ...Items }} }} {POLL_RECORD} {POLL_ITEMS}"
    );
    let asked = describe(&req_vars(&poll, json!({ "ids": ids }))).unwrap();
    assert_eq!(asked.finding, Some(None));
    assert_eq!(
        asked.nodes,
        [
            ("PR_a".to_owned(), AskedOf::Record),
            ("PR_b".to_owned(), AskedOf::Record),
            ("PR_c".to_owned(), AskedOf::Items)
        ]
    );
    assert_eq!(asked.lookup, None);
    let asked = describe(&req_vars(
        &poll,
        json!({ "ids": ["PR_a"], "after": "Y3Vyc29y:99" }),
    ))
    .unwrap();
    assert_eq!(asked.finding, Some(Some("Y3Vyc29y:99".to_owned())));

    let first = format!(
        "query {{ nodes(ids: [\"PR_a\"]) {{ ...Record ...Items }} }} {POLL_RECORD} {POLL_ITEMS}"
    );
    assert_eq!(
        describe(&req(&first)).unwrap().nodes,
        [("PR_a".to_owned(), AskedOf::RecordAndItems)]
    );
    let page = "query { node(id: \"PR_a\") { ... on PullRequest { commits(last: 1) { nodes { commit { statusCheckRollup { \
                contexts(first: 100, after: \"Y3Vyc29y:99\") { nodes { __typename } } } } } } } } }";
    let asked = describe(&req(page)).unwrap();
    assert_eq!(asked.nodes, [("PR_a".to_owned(), AskedOf::ChecksPage)]);
    assert_eq!(asked.finding, None);

    let lookup = format!(
        "query($o: String!, $r: String!, $n: Int!) {{ repository(owner: $o, name: $r) {{ pullRequest(number: $n) \
         {{ ...Record ...Items }} }} }} {POLL_RECORD} {POLL_ITEMS}"
    );
    let asked = describe(&req_vars(
        &lookup,
        json!({ "o": "yachiko", "r": "experiment", "n": 13 }),
    ))
    .unwrap();
    assert_eq!(
        asked.lookup,
        Some(("yachiko".to_owned(), "experiment".to_owned(), 13))
    );
    assert!(asked.nodes.is_empty() && asked.finding.is_none());
}

// ----- Markers -------------------------------------------------------------------------------

fn walk<'v>(v: &'v Value, key: Option<&str>, found: &mut Vec<(String, &'v Value)>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                walk(v, Some(k), found);
            }
        }
        Value::Array(a) => {
            for v in a {
                walk(v, key, found);
            }
        }
        _ => {
            if let Some(k) = key {
                found.push((k.to_owned(), v));
            }
        }
    }
}

// TST-11: the fake's own markers in every title, error message, check name and login it makes.
#[test]
fn tst11_markers() {
    let mut w = world();
    let repo = w.repo("o", "r");
    let pr = w.open_pr(repo);
    let named = w.open_pr(repo);
    w.unnamed_run(pr);
    w.runs(pr, 2, "QUEUED", None);
    w.run(named, "build");
    w.comment(pr);
    w.review(pr, "COMMENTED", 1);
    w.thread_reply(pr);
    let q = format!(
        "query {{ viewer {{ login }} nodes(ids: [\"{}\", \"{}\", \"PR_nope\"]) {{ ...R ...Items }} }} {RECORD} {POLL_ITEMS}",
        w.node_id(pr),
        w.node_id(named)
    );
    let b = body(&w, &q);
    let mut leaves = Vec::new();
    walk(&b, None, &mut leaves);
    let of = |k: &str| {
        leaves
            .iter()
            .filter(|(key, _)| key == k)
            .map(|(_, v)| v.as_str().unwrap_or(""))
            .collect::<Vec<_>>()
    };
    assert_eq!(of("title").len(), 2);
    assert!(of("title").iter().all(|t| t.contains(MARKER_TITLE)));
    assert_eq!(of("message").len(), 1);
    assert!(of("message").iter().all(|m| m.contains(MARKER_GITHUB_TEXT)));
    assert!(of("login").len() >= 5);
    assert!(
        of("login").iter().all(|l| l.contains(MARKER_LOGIN)),
        "{:?}",
        of("login")
    );
    let names = of("name");
    assert_eq!(names.len(), 4);
    assert_eq!(names.iter().filter(|n| n.contains(MARKER_CHECK)).count(), 3);
    assert!(
        names.contains(&"build"),
        "a check the test names is used as given"
    );
    // A title and a login the test gives are used as given.
    w.set_title(pr, "My title");
    w.set_author(pr, Some("alice"));
    let r = read(&w, pr);
    assert_eq!(
        (r["title"].clone(), r["author"]["login"].clone()),
        (json!("My title"), json!("alice"))
    );
}

// TST-3: rateLimit answers the rate T20 sets and spends, and this request's cost.
#[test]
fn tst3_rate_limit() {
    let mut w = world();
    let reset = start().plus(secs(1800));
    w.set_rate(4000, reset);
    let b = body(
        &w,
        "{ rateLimit { cost remaining limit used resetAt nodeCount } }",
    );
    assert_eq!(
        b["data"]["rateLimit"],
        json!({ "cost": 1, "remaining": 3999, "limit": 5000, "used": 1001, "resetAt": reset.rfc3339(), "nodeCount": 0 })
    );
    w.spend(1);
    assert_eq!(w.rate(), (3999, reset));
    w.set_login("renamed-user");
    assert_eq!(
        body(&w, "{ viewer { login } }")["data"]["viewer"]["login"],
        "renamed-user"
    );
    assert_eq!(w.login(), "renamed-user");
}

// TST-3: T20 keeps the world behind a lock shared across its tasks.
#[test]
fn tst3_world_is_send() {
    fn send<T: Send>() {}
    send::<World>();
}
