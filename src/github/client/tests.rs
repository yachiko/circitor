//! T07's tests: the builder against GitHub's schema, the request against a loopback stub, and the
//! decoding against the fixtures in `testdata/github/`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use apollo_compiler::executable::{OperationType, Selection, SelectionSet};
use apollo_compiler::validation::Valid;
use apollo_compiler::{ExecutableDocument, Schema};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::decode::{self, NOT_A_PULL_REQUEST, UNDECODABLE, UNLISTED_STATE, UNTYPED};
use super::{Client, request_body};
use crate::clock::Time;
use crate::github::{
    Answer, Failure, Find, GitHub, LookupAnswer, NodeAnswer, NodeQuery, Query, Unavailable, Want,
};
use crate::model::{
    CheckItem, CheckKind, ItemKind, Mergeable, NodeId, Phase, PrName, ReadState, Review,
};
use crate::testkit::{MARKER_GITHUB_TEXT, MARKER_TOKEN};
use crate::token::Token;

// ---- helpers ----

/// The want of the node at an index.
type WantOf = fn(usize) -> Want;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn t(s: &str) -> Time {
    Time::parse(s).unwrap()
}

/// GitHub's schema, parsed once per test process (it's about 1.5 MB).
fn schema() -> &'static Valid<Schema> {
    static SCHEMA: OnceLock<Valid<Schema>> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let path = root().join("testdata/github/schema.graphql");
        let source = std::fs::read_to_string(&path).unwrap();
        Schema::parse_and_validate(source, "schema.graphql").unwrap()
    })
}

/// Parses and validates the body's document, and checks it's one `query` whose variables all have values.
fn validate(body: &Value) -> Valid<ExecutableDocument> {
    let source = body["query"].as_str().unwrap();
    let doc = ExecutableDocument::parse_and_validate(schema(), source, "query.graphql")
        .unwrap_or_else(|e| panic!("doesn't validate: {e}\n{source}"));
    let ops: Vec<_> = doc.operations.iter().collect();
    assert_eq!(ops.len(), 1, "{source}");
    assert_eq!(ops[0].operation_type, OperationType::Query, "{source}");
    let vars = body["variables"].as_object().unwrap();
    for v in &ops[0].variables {
        if v.ty.is_non_null() {
            assert!(
                vars.contains_key(v.name.as_str()),
                "${} has no value",
                v.name
            );
        }
    }
    assert_eq!(vars.len(), ops[0].variables.len(), "{source}");
    doc
}

fn node(i: usize, want: Want) -> NodeQuery {
    NodeQuery {
        id: NodeId(format!("PR_node{i}")),
        want,
    }
}

fn page(i: usize) -> Want {
    Want::ChecksPage {
        after: format!("cursor{i}"),
    }
}

fn name() -> PrName {
    PrName::parse("yachiko/experiment#3").unwrap()
}

fn nodes_of(want: fn(usize) -> Want, n: usize) -> Query {
    Query {
        nodes: (0..n).map(|i| node(i, want(i))).collect(),
        ..Query::default()
    }
}

/// Every shape the builder can make.
fn shapes() -> Vec<(String, Query)> {
    let wants: [(&str, WantOf); 4] = [
        ("record", |_| Want::Record),
        ("record_and_items", |_| Want::RecordAndItems),
        ("items", |_| Want::Items),
        ("checks_page", page),
    ];
    let mut out = Vec::new();
    for (label, want) in wants {
        for n in [1, 40] {
            out.push((format!("{label} x{n}"), nodes_of(want, n)));
        }
    }
    let mixed = |n: usize| -> Vec<NodeQuery> {
        (0..n)
            .map(|i| node(i, (wants[i % 4].1)(i)))
            .collect::<Vec<_>>()
    };
    out.push((
        "all four mixed x4".into(),
        Query {
            nodes: mixed(4),
            ..Query::default()
        },
    ));
    out.push((
        "all four mixed x40".into(),
        Query {
            nodes: mixed(40),
            ..Query::default()
        },
    ));
    out.push((
        "find".into(),
        Query {
            find: Some(Find::default()),
            ..Query::default()
        },
    ));
    out.push((
        "find after".into(),
        Query {
            find: Some(Find {
                after: Some("Y3Vyc29y".into()),
            }),
            ..Query::default()
        },
    ));
    out.push((
        "lookup".into(),
        Query {
            lookup: Some(name()),
            ..Query::default()
        },
    ));
    out.push((
        "all three parts".into(),
        Query {
            find: Some(Find {
                after: Some("Y3Vyc29y".into()),
            }),
            nodes: mixed(8),
            lookup: Some(name()),
        },
    ));
    out
}

/// The field paths a selection selects, by field name (`a.b.c`), through fragments.
fn paths(doc: &ExecutableDocument, set: &SelectionSet, prefix: &str, out: &mut Vec<String>) {
    for sel in &set.selections {
        match sel {
            Selection::Field(f) => {
                // The top-level fields by their alias (`record`, `items`), the rest by name.
                let p = if prefix.is_empty() {
                    f.alias.as_ref().unwrap_or(&f.name).to_string()
                } else {
                    format!("{prefix}.{}", f.name)
                };
                out.push(p.clone());
                paths(doc, &f.selection_set, &p, out);
            }
            Selection::InlineFragment(i) => paths(doc, &i.selection_set, prefix, out),
            Selection::FragmentSpread(s) => {
                let frag = doc.fragments.get(&s.fragment_name).unwrap();
                paths(doc, &frag.selection_set, prefix, out);
            }
        }
    }
}

/// A fixture file: the query it answers, and the response.
struct Fixture {
    query: Query,
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
}

fn load(path: &Path) -> Fixture {
    let text = std::fs::read_to_string(path).unwrap();
    let v: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let request = &v["request"];
    let nodes = request["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let want = match w.as_str().unwrap() {
                "record" => Want::Record,
                "record_and_items" => Want::RecordAndItems,
                "items" => Want::Items,
                "checks_page" => page(i),
                other => panic!("{}: unknown want {other}", path.display()),
            };
            node(i, want)
        })
        .collect();
    let query = Query {
        find: request["find"].as_bool().unwrap().then(Find::default),
        nodes,
        lookup: request["lookup"].as_bool().unwrap().then(name),
    };
    let mut headers = HeaderMap::new();
    for (k, val) in v["headers"].as_object().unwrap() {
        let k = k.to_ascii_lowercase();
        assert!(
            k == "date" || k == "retry-after" || k.starts_with("x-ratelimit-"),
            "{}: header {k}",
            path.display()
        );
        headers.insert(
            HeaderName::from_bytes(k.as_bytes()).unwrap(),
            HeaderValue::from_str(val.as_str().unwrap()).unwrap(),
        );
    }
    let body = match &v["body"] {
        Value::String(s) => s.clone().into_bytes(),
        other => serde_json::to_vec(other).unwrap(),
    };
    Fixture {
        query,
        status: u16::try_from(v["status"].as_u64().unwrap()).unwrap(),
        headers,
        body,
    }
}

fn decode_fixture(name: &str) -> Result<Answer, Failure> {
    let f = load(&root().join(format!("testdata/github/responses/synthetic-{name}.json")));
    decode::decode(&f.query, f.status, &f.headers, Some(&f.body))
}

fn ok(name: &str) -> Answer {
    decode_fixture(name).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

fn record(answer: &NodeAnswer) -> &crate::model::PrRead {
    match answer {
        NodeAnswer::Record { read, .. } => read,
        other => panic!("not a record: {other:?}"),
    }
}

fn err(type_: &str) -> NodeAnswer {
    NodeAnswer::Error {
        type_: type_.to_string(),
    }
}

// ---- the stub ----

/// One request as the stub saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A loopback HTTP/1.1 server answering every request with `response` (raw bytes, so a body can be
/// cut short), recording each one. Returns the `/graphql` URL.
async fn stub(response: Vec<u8>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let head_end = loop {
                let n = sock.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break None;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break Some(i);
                }
            };
            let Some(head_end) = head_end else { continue };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let mut lines = head.split("\r\n");
            let mut first = lines.next().unwrap().split(' ');
            let method = first.next().unwrap().to_string();
            let path = first.next().unwrap().to_string();
            let headers: Vec<(String, String)> = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                .collect();
            let len: usize = headers
                .iter()
                .find(|(k, _)| k == "content-length")
                .map(|(_, v)| v.parse().unwrap())
                .unwrap_or(0);
            let mut body = buf[head_end + 4..].to_vec();
            while body.len() < len {
                let n = sock.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            log.lock().unwrap().push(Seen {
                method,
                path,
                headers,
                body,
            });
            let _ = sock.write_all(&response).await;
            let _ = sock.shutdown().await;
        }
    });
    (format!("http://127.0.0.1:{port}/graphql"), seen)
}

fn response(status: &str, headers: &[(&str, &str)], body: &str) -> Vec<u8> {
    let mut r = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        r.push_str(&format!("{k}: {v}\r\n"));
    }
    r.push_str("\r\n");
    r.push_str(body);
    r.into_bytes()
}

fn token() -> Token {
    Token::new(MARKER_TOKEN.to_string())
}

fn find_query() -> Query {
    Query {
        find: Some(Find::default()),
        nodes: vec![node(0, Want::Record)],
        ..Query::default()
    }
}

async fn send_to(raw: Vec<u8>) -> (Result<Answer, Failure>, Vec<Seen>) {
    let (url, seen) = stub(raw).await;
    let result = Client::for_tests(&url).send(&token(), &find_query()).await;
    let seen = seen.lock().unwrap().clone();
    (result, seen)
}

fn http(status: u16) -> Failure {
    Failure::Unavailable {
        kind: Unavailable::Http,
        status: Some(status),
    }
}

// ---- the builder ----

// POL-1: every shape the builder can make validates against GitHub's schema and is one `query`.
#[test]
fn pol1_every_query_validates_and_is_a_query() {
    for (label, query) in shapes() {
        let body = request_body(&query);
        let doc = validate(&body);
        let source = body["query"].as_str().unwrap();
        assert!(source.starts_with("query"), "{label}");
        assert!(!source.contains("mutation"), "{label}");
        assert!(
            source.contains("rateLimit { cost remaining resetAt }"),
            "{label}"
        );
        // One answer slot per node, in order.
        assert_eq!(
            super::query::plan(&query).slots.len(),
            query.nodes.len(),
            "{label}"
        );
        drop(doc);
    }
    // `after` only when given.
    let shape = |label: &str| shapes().into_iter().find(|(l, _)| l == label).unwrap().1;
    let plain = request_body(&shape("find"));
    assert!(!plain["query"].as_str().unwrap().contains("after"));
    let after = request_body(&shape("find after"));
    assert_eq!(after["variables"]["findAfter"], "Y3Vyc29y");
}

// POL-6, MOD-3: the record selection has every GitHub field of MOD-2 and MOD-3, `contexts(first: 100)`,
// no `isRequired` (D6), and no item IDs.
#[test]
fn pol6_record_selection() {
    let body = request_body(&nodes_of(|_| Want::Record, 2));
    let doc = validate(&body);
    let source = body["query"].as_str().unwrap();
    let op = doc.operations.iter().next().unwrap();
    let mut all = Vec::new();
    paths(&doc, &op.selection_set, "", &mut all);
    let pr = "record";
    let checks = "record.commits.nodes.commit.statusCheckRollup";
    let mut want: Vec<String> = [
        "id",
        "number",
        "repository.nameWithOwner",
        "url",
        "title",
        "author.login",
        "state",
        "isDraft",
        "baseRefName",
        "headRefName",
        "headRefOid",
        "createdAt",
        "updatedAt",
        "mergedAt",
        "closedAt",
        "mergeable",
        "mergeStateStatus",
        "reviewDecision",
        "comments.totalCount",
        "reviews.totalCount",
        "reviewThreads.totalCount",
    ]
    .iter()
    .map(|f| format!("{pr}.{f}"))
    .collect();
    want.extend(
        [
            "state",
            "contexts.totalCount",
            "contexts.pageInfo.hasNextPage",
            "contexts.pageInfo.endCursor",
            "contexts.nodes.__typename",
            "contexts.nodes.databaseId",
            "contexts.nodes.name",
            "contexts.nodes.status",
            "contexts.nodes.conclusion",
            "contexts.nodes.detailsUrl",
            "contexts.nodes.context",
            "contexts.nodes.state",
            "contexts.nodes.targetUrl",
        ]
        .iter()
        .map(|f| format!("{checks}.{f}")),
    );
    for w in &want {
        assert!(all.contains(w), "missing {w}");
    }
    assert!(source.contains("contexts(first: 100)"));
    assert!(source.contains("commits(last: 1)"));
    // MOD-3:whether-a-check-is-required (D6).
    assert!(!all.iter().any(|p| p.ends_with("isRequired")));
    assert!(!source.contains("isRequired"));
    // POL-6: no comment or review IDs.
    assert!(
        !all.iter()
            .any(|p| p.contains("comments.nodes") || p.contains("reviews.nodes")),
        "{all:?}"
    );
    // No query asks `isRequired`, whatever its shape (D6).
    for (label, q) in shapes() {
        let b = request_body(&q);
        assert!(
            !b["query"].as_str().unwrap().contains("isRequired"),
            "{label}"
        );
    }
}

// POL-12, MOD-12: the item selections; a review's inline count.
#[test]
fn pol12_item_selection() {
    let body = request_body(&nodes_of(|_| Want::Items, 1));
    let doc = validate(&body);
    let op = doc.operations.iter().next().unwrap();
    let mut all = Vec::new();
    paths(&doc, &op.selection_set, "", &mut all);
    for w in [
        "items.comments.nodes.databaseId",
        "items.comments.nodes.author.login",
        "items.comments.nodes.createdAt",
        "items.reviews.nodes.databaseId",
        "items.reviews.nodes.author.login",
        "items.reviews.nodes.state",
        "items.reviews.nodes.submittedAt",
        "items.reviews.nodes.comments.totalCount",
    ] {
        assert!(all.contains(&w.to_string()), "missing {w}");
    }
    let source = body["query"].as_str().unwrap();
    assert!(source.contains("comments(last: 20)") && source.contains("reviews(last: 20)"));
    // Items alone ask nothing of the record.
    assert!(!all.iter().any(|p| p.contains("statusCheckRollup")));
}

// ---- the request ----

// OPS-16, TST-1: POST /graphql, the bearer token, circitor's user agent, and exactly `request_body`.
#[tokio::test]
async fn ops16_the_request() {
    let (url, seen) = stub(response("200 OK", &[], r#"{"data":{}}"#)).await;
    let query = find_query();
    let _ = Client::for_tests(&url).send(&token(), &query).await;
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let r = &seen[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/graphql");
    assert_eq!(
        r.header("authorization"),
        Some(format!("bearer {MARKER_TOKEN}").as_str())
    );
    assert!(r.header("user-agent").unwrap().starts_with("circitor/"));
    assert_eq!(r.header("content-type"), Some("application/json"));
    let body: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(body, request_body(&query));
}

// TST-1: a test build's client sends this process's ID.
#[cfg(feature = "test-endpoint")]
#[tokio::test]
async fn tst1_test_build_sends_its_pid() {
    let (_, seen) = send_to(response("200 OK", &[], r#"{"data":{}}"#)).await;
    let pid = std::process::id().to_string();
    assert_eq!(seen[0].header("x-circitor-test-pid"), Some(pid.as_str()));
}

// TST-1: the endpoint of a test build is the loopback only.
#[cfg(feature = "test-endpoint")]
#[test]
fn tst1_endpoint() {
    use crate::config::Env;
    let env = |url: Option<&str>| Env {
        vars: url
            .map(|u| (super::TEST_GITHUB_VAR.to_string(), u.to_string()))
            .into_iter()
            .collect(),
        home: None,
    };
    let e = Client::from_env(&env(None), "1.2.3").err().unwrap();
    assert!(e.to_string().contains("CIRCITOR_TEST_GITHUB"), "{e}");
    let e = Client::from_env(&env(Some("")), "1.2.3").err().unwrap();
    assert!(e.to_string().contains("CIRCITOR_TEST_GITHUB"), "{e}");
    let c = Client::from_env(&env(Some("http://127.0.0.1:4567/graphql")), "1.2.3").unwrap();
    assert_eq!(c.endpoint, "http://127.0.0.1:4567/graphql");
    for (bad, host) in [
        ("https://api.github.com/graphql", "api.github.com"),
        ("http://localhost:4567/graphql", "localhost"),
        ("http://127.0.0.1/graphql", "127.0.0.1"),
        ("http://127.0.0.1:4567/other", "127.0.0.1"),
    ] {
        let e = Client::from_env(&env(Some(bad)), "1.2.3").err().unwrap();
        let text = e.to_string();
        assert!(
            text.contains("CIRCITOR_TEST_GITHUB") && text.contains(host),
            "{bad}: {text}"
        );
    }
}

// TST-1: `for_tests` panics on any host but the loopback.
#[test]
#[should_panic(expected = "api.github.com")]
fn tst1_for_tests_refuses_github() {
    let _ = Client::for_tests("https://api.github.com/graphql");
}

// ---- POL-25, POL-24: answers that fail as a whole ----

// POL-25, POL-24: each case.
#[tokio::test]
async fn pol25_pol24_classification() {
    let cases: Vec<(Vec<u8>, Failure)> = vec![
        (
            response(
                "401 Unauthorized",
                &[],
                r#"{"message":"Bad credentials","status":"401"}"#,
            ),
            Failure::Unauthorized,
        ),
        (
            response(
                "403 Forbidden",
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "1795561200"),
                ],
                r#"{"message":"API rate limit exceeded"}"#,
            ),
            Failure::Exhausted {
                reset_at: Some(t("2026-11-24T23:00:00Z")),
            },
        ),
        (
            response(
                "200 OK",
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "1795561200"),
                ],
                r#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#,
            ),
            Failure::Exhausted {
                reset_at: Some(t("2026-11-24T23:00:00Z")),
            },
        ),
        (
            response(
                "403 Forbidden",
                &[("x-ratelimit-remaining", "0")],
                r#"{"message":"API rate limit exceeded"}"#,
            ),
            Failure::Exhausted { reset_at: None },
        ),
        (
            response(
                "403 Forbidden",
                &[("retry-after", "30"), ("x-ratelimit-remaining", "4000")],
                r#"{"message":"You have exceeded a secondary rate limit. Please wait."}"#,
            ),
            Failure::Secondary {
                retry_after: Some(Duration::from_secs(30)),
            },
        ),
        (
            response(
                "200 OK",
                &[("x-ratelimit-remaining", "4000")],
                r#"{"errors":[{"message":"YOU HAVE HIT A SECONDARY RATE LIMIT"}]}"#,
            ),
            Failure::Secondary { retry_after: None },
        ),
        (
            response("403 Forbidden", &[], r#"{"message":"Forbidden"}"#),
            http(403),
        ),
        (
            response("500 Internal Server Error", &[], "oops"),
            http(500),
        ),
        (response("502 Bad Gateway", &[], "<html></html>"), http(502)),
        (
            response(
                "200 OK",
                &[("x-ratelimit-remaining", "4000")],
                r#"{"errors":[{"type":"INTERNAL","message":"Something went wrong"}]}"#,
            ),
            Failure::Unavailable {
                kind: Unavailable::NoData,
                status: Some(200),
            },
        ),
        (
            response("200 OK", &[], "not json"),
            Failure::Unavailable {
                kind: Unavailable::BadBody,
                status: Some(200),
            },
        ),
    ];
    for (raw, want) in cases {
        let text = String::from_utf8_lossy(&raw).to_string();
        let (got, _) = send_to(raw).await;
        assert_eq!(got, Err(want), "{text}");
    }
}

// POL-24: a body cut before its `Content-Length` is `BadBody`.
#[tokio::test]
async fn pol24_cut_body_is_bad_body() {
    let raw =
        b"HTTP/1.1 200 OK\r\ncontent-length: 500\r\nconnection: close\r\n\r\n{\"data\":{\"rate"
            .to_vec();
    let (got, _) = send_to(raw).await;
    assert_eq!(
        got,
        Err(Failure::Unavailable {
            kind: Unavailable::BadBody,
            status: Some(200)
        })
    );
}

// POL-24, OPS-16: a redirect is `Http` with its status, and is never followed.
#[tokio::test]
async fn pol24_redirect_not_followed() {
    let (url, seen) = stub(Vec::new()).await;
    let elsewhere = url.replace("/graphql", "/elsewhere");
    let (url2, seen2) = stub(response("302 Found", &[("location", &elsewhere)], "")).await;
    let got = Client::for_tests(&url2).send(&token(), &find_query()).await;
    assert_eq!(got, Err(http(302)));
    assert_eq!(seen2.lock().unwrap().len(), 1);
    assert!(
        seen.lock().unwrap().is_empty(),
        "the Location got a request"
    );
    drop(url);
}

// POL-24: a closed port is `Network`.
#[tokio::test]
async fn pol24_closed_port_is_network() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let got = Client::for_tests(&format!("http://127.0.0.1:{port}/graphql"))
        .send(&token(), &find_query())
        .await;
    assert_eq!(
        got,
        Err(Failure::Unavailable {
            kind: Unavailable::Network,
            status: None
        })
    );
}

// OPS-9: neither GitHub's text nor the token is kept in a `Failure` or an `Answer`.
#[tokio::test]
async fn ops9_no_text_kept() {
    let m = MARKER_GITHUB_TEXT;
    let bodies = [
        (
            "401 Unauthorized",
            vec![],
            format!(r#"{{"message":"Bad credentials {m}","status":"401"}}"#),
        ),
        (
            "403 Forbidden",
            vec![("retry-after", "30")],
            format!(r#"{{"message":"secondary rate limit {m}"}}"#),
        ),
        ("403 Forbidden", vec![], format!(r#"{{"message":"{m}"}}"#)),
        ("500 Internal Server Error", vec![], m.to_string()),
        (
            "200 OK",
            vec![],
            format!(r#"{{"errors":[{{"type":"INTERNAL","message":"{m}"}}]}}"#),
        ),
        (
            "200 OK",
            vec![],
            format!(
                r#"{{"data":{{"rateLimit":null,"viewer":null,"record":[null]}},"errors":[{{"type":"NOT_FOUND","path":["record",0],"message":"{m}"}},{{"type":"INTERNAL","path":["viewer"],"message":"{m}"}},{{"path":["rateLimit"],"message":"{m}"}}]}}"#
            ),
        ),
    ];
    for (status, headers, body) in bodies {
        let (got, _) = send_to(response(status, &headers, &body)).await;
        let debug = format!("{got:?}");
        assert!(!debug.contains(m), "{debug}");
        assert!(!debug.contains(MARKER_TOKEN), "{debug}");
    }
}

// ---- the fixtures ----

// POL-4: a poll with two PRs.
#[test]
fn fixture_poll_two_prs() {
    let a = ok("poll-two-prs");
    let found = a.found.unwrap();
    assert_eq!(found.login, "yachiko");
    assert_eq!(found.prs.len(), 2);
    assert_eq!(
        found.prs[0].pr,
        PrName::parse("yachiko/experiment#1").unwrap()
    );
    assert_eq!(found.prs[1].id, NodeId("PR_kwDOexperiment0003".into()));
    assert_eq!(found.prs[1].updated_at, Some(t("2026-11-24T22:25:51Z")));
    assert_eq!(found.next, None);
    assert_eq!(a.nodes.len(), 2);
    assert!(
        a.nodes
            .iter()
            .all(|n| matches!(n, NodeAnswer::Record { items: None, .. }))
    );
    let rate = a.rate.unwrap();
    assert_eq!(rate.cost, 1);
    assert_eq!(rate.remaining, 4989);
    assert_eq!(rate.reset_at, Some(t("2026-11-24T23:00:00Z")));
    assert_eq!(a.date, Some(t("2026-11-24T22:14:43Z")));
    assert_eq!(a.lookup, None);
}

// MOD-3: PR C at 22:26:36 (c1.log).
#[test]
fn mod3_fixture_pr_c() {
    let a = ok("pr-c-checks");
    let r = record(&a.nodes[0]);
    let item = |kind, id, name: &str, phase, result: Option<&str>| CheckItem {
        kind,
        id,
        name: name.to_string(),
        phase,
        result: result.map(str::to_string),
        url: None,
    };
    let items: Vec<CheckItem> = r
        .checks
        .items
        .iter()
        .map(|i| CheckItem {
            url: None,
            ..i.clone()
        })
        .collect();
    assert_eq!(
        items,
        vec![
            item(
                CheckKind::Run,
                Some(301),
                "pass",
                Phase::Completed,
                Some("success")
            ),
            item(
                CheckKind::Run,
                Some(302),
                "fail",
                Phase::Completed,
                Some("failure")
            ),
            item(
                CheckKind::Run,
                Some(303),
                "skipped",
                Phase::Completed,
                Some("skipped")
            ),
            item(CheckKind::Run, Some(304), "slow", Phase::InProgress, None),
            item(
                CheckKind::Status,
                None,
                "circitor/manual",
                Phase::InProgress,
                None
            ),
        ]
    );
    assert!(
        r.checks.items[0]
            .url
            .as_deref()
            .unwrap()
            .starts_with("https://")
    );
    assert_eq!(r.checks.rollup.as_deref(), Some("failure"));
    assert_eq!(r.checks.total, 5);
    assert_eq!(r.checks.next_page, None);
    assert_eq!(r.mergeable, Mergeable::Yes);
    assert_eq!(r.merge_state, "unstable");
    assert!(r.head_sha.starts_with("180b372"));
    assert_eq!(r.state, ReadState::Open);
    assert_eq!(r.title, "PR C");
    assert_eq!(r.author.as_deref(), Some("yachiko"));
    assert_eq!(r.review, Review::None);
}

// MOD-2: PR B merged (b1.log).
#[test]
fn mod2_fixture_pr_b_merged() {
    let a = ok("pr-b-merged");
    let r = record(&a.nodes[0]);
    assert_eq!(r.state, ReadState::Merged);
    assert_eq!(r.mergeable, Mergeable::Unknown);
    assert_eq!(r.merge_state, "unknown");
    assert!(r.merged_at.is_some());
    assert_eq!(r.merged_at, r.closed_at);
    assert_eq!(r.checks.rollup, None);
    assert_eq!(r.checks.total, 0);
    assert!(r.checks.items.is_empty());
}

// POL-21: docs/02 §5's unknown ID, and an error inside a node.
#[test]
fn pol21_fixture_partial_errors() {
    let a = ok("unknown-id");
    assert!(matches!(a.nodes[0], NodeAnswer::Record { .. }));
    assert_eq!(a.nodes[1], NodeAnswer::NotFound);
    let a = ok("forbidden-inside");
    assert_eq!(a.nodes[0], err("FORBIDDEN"));
    assert_eq!(record(&a.nodes[1]).title, "PR C");
    assert!(a.rate.is_some());
}

// POL-16: a lookup's answers.
#[test]
fn pol16_fixture_lookup() {
    assert_eq!(ok("lookup-no-pr").lookup, Some(LookupAnswer::NotFound));
    assert_eq!(ok("lookup-no-repo").lookup, Some(LookupAnswer::NotFound));
    assert_eq!(
        ok("lookup-forbidden").lookup,
        Some(LookupAnswer::Error {
            type_: "FORBIDDEN".into()
        })
    );
    match ok("lookup-found").lookup {
        Some(LookupAnswer::Found { read, items }) => {
            assert_eq!(read.pr, name());
            assert_eq!(items.comments.len(), 1);
            assert_eq!(items.reviews.len(), 1);
        }
        other => panic!("{other:?}"),
    }
}

// MOD-3, POL-8, POL-12: a null rollup, a checks page, and items.
#[test]
fn fixture_rollup_page_items() {
    let r = ok("null-rollup");
    let r = record(&r.nodes[0]);
    assert_eq!(
        (
            r.checks.total,
            r.checks.rollup.clone(),
            r.checks.items.len()
        ),
        (0, None, 0)
    );

    let a = ok("checks-page");
    match &a.nodes[0] {
        NodeAnswer::ChecksPage(page) => {
            assert_eq!(page.items.len(), 100);
            assert_eq!(page.total, 187);
            assert_eq!(page.next_page.as_deref(), Some("Y3Vyc29yOjIwMA=="));
            assert_eq!(page.rollup.as_deref(), Some("pending"));
        }
        other => panic!("{other:?}"),
    }

    let a = ok("items");
    let NodeAnswer::Items(items) = &a.nodes[0] else {
        panic!("{:?}", a.nodes[0]);
    };
    assert_eq!(items.comments.len(), 1);
    let c = &items.comments[0];
    assert_eq!(c.kind, ItemKind::Comment);
    assert_eq!((c.id, c.inline, c.review_state.clone()), (3601, None, None));
    assert_eq!(c.at, Some(t("2026-11-24T22:30:02Z")));
    let r = &items.reviews[0];
    assert_eq!(r.kind, ItemKind::Review);
    assert_eq!(r.id, 3701);
    assert_eq!(r.inline, Some(1));
    assert_eq!(r.review_state.as_deref(), Some("commented"));
    assert_eq!(r.author.as_deref(), Some("yachiko"));
    assert_eq!(r.at, Some(t("2026-11-24T22:31:29Z")));
    assert_eq!(items.reviews[1].at, None);

    let a = ok("first-refresh");
    match &a.nodes[0] {
        NodeAnswer::Record {
            read,
            items: Some(items),
        } => {
            assert_eq!(read.counts.comments, 1);
            assert_eq!(read.counts.reviews, 1);
            assert_eq!(read.counts.review_threads, 1);
            assert_eq!(items.reviews[0].inline, Some(1));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(a.rate.unwrap().cost, 2);
}

// POL-21: errors on `viewer` and on `rateLimit`; `resetAt` missing.
#[test]
fn fixture_viewer_rate_reset() {
    let a = ok("viewer-error");
    assert_eq!(a.found, None);
    assert!(matches!(a.nodes[0], NodeAnswer::Record { .. }));
    let a = ok("ratelimit-error");
    assert_eq!(a.rate, None);
    assert!(matches!(a.nodes[0], NodeAnswer::Record { .. }));
    let a = ok("no-reset-at");
    assert_eq!(a.rate.unwrap().reset_at, Some(t("2026-11-24T23:00:00Z")));
}

// POL-25, POL-24: the failure fixtures.
#[test]
fn fixture_failures() {
    assert_eq!(
        decode_fixture("bad-credentials"),
        Err(Failure::Unauthorized)
    );
    assert_eq!(
        decode_fixture("exhausted"),
        Err(Failure::Exhausted {
            reset_at: Some(t("2026-11-24T23:00:00Z"))
        })
    );
    assert_eq!(
        decode_fixture("secondary"),
        Err(Failure::Secondary {
            retry_after: Some(Duration::from_secs(30))
        })
    );
    assert_eq!(decode_fixture("bad-gateway"), Err(http(502)));
    assert_eq!(
        decode_fixture("not-json"),
        Err(Failure::Unavailable {
            kind: Unavailable::BadBody,
            status: Some(200)
        })
    );
}

// MOD-2:a-value-github-sends-that: unlisted values pass through, lowered; a check's phase is one of
// three; an odd node never fails the answer.
#[test]
fn mod2_unlisted_values() {
    let a = ok("unlisted-values");
    assert_eq!(a.nodes.len(), 6);
    let r = record(&a.nodes[0]);
    assert_eq!(r.merge_state, "has_hooks");
    assert_eq!(r.mergeable, Mergeable::Other("foo".into()));
    assert_eq!(r.author, None);
    assert_eq!(r.review, Review::None);
    assert_eq!(r.checks.rollup.as_deref(), Some("error"));
    let got: Vec<(&str, Phase, Option<&str>)> = r
        .checks
        .items
        .iter()
        .map(|i| (i.name.as_str(), i.phase, i.result.as_deref()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("waiting", Phase::Queued, None),
            ("startup", Phase::Completed, Some("startup_failure")),
            ("foo", Phase::Completed, Some("foo")),
            ("expected", Phase::InProgress, None),
            ("errored", Phase::Completed, Some("error")),
        ],
        "a context of an unknown type is skipped"
    );
    assert_eq!(r.checks.total, 6);
    assert_eq!(a.nodes[1], err(UNLISTED_STATE));
    assert_eq!(a.nodes[2], err(NOT_A_PULL_REQUEST));
    assert_eq!(a.nodes[3], err(UNDECODABLE));
    assert_eq!(a.nodes[4], err(UNTYPED));
    assert_eq!(record(&a.nodes[5]).state, ReadState::Merged);
    assert!(a.rate.is_some());
}

// MOD-2: a `null` node with no error can't be decoded; a check run's result waits for `completed`.
#[test]
fn mod2_null_without_error_and_early_conclusion() {
    let q = nodes_of(|_| Want::Record, 2);
    let mut pr: Value = serde_json::from_slice(
        &load(&root().join("testdata/github/responses/synthetic-pr-c-checks.json")).body,
    )
    .unwrap();
    let mut node = pr["data"]["record"][0].take();
    node["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"][3]["conclusion"] =
        json!("SUCCESS");
    let body = json!({"data": {"record": [Value::Null, node]}});
    let a = decode::decode(
        &q,
        200,
        &HeaderMap::new(),
        Some(&serde_json::to_vec(&body).unwrap()),
    )
    .unwrap();
    assert_eq!(a.nodes[0], err(UNDECODABLE));
    let r = record(&a.nodes[1]);
    assert_eq!(r.checks.items[3].phase, Phase::InProgress);
    assert_eq!(r.checks.items[3].result, None);
    assert_eq!(a.rate, None);
    assert_eq!(a.date, None);
}

// TST-12: every fixture decodes, synthetic and recorded; a missing `recorded/` is fine.
#[test]
fn tst12_every_fixture_decodes() {
    let mut count = 0;
    for dir in ["testdata/github/responses", "testdata/github/recorded"] {
        let Ok(entries) = std::fs::read_dir(root().join(dir)) else {
            assert_eq!(dir, "testdata/github/recorded", "{dir} is missing");
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if dir.ends_with("responses") {
                let file = path.file_name().unwrap().to_string_lossy().to_string();
                assert!(file.starts_with("synthetic-"), "{file}");
            }
            let f = load(&path);
            let got = decode::decode(&f.query, f.status, &f.headers, Some(&f.body));
            if let Ok(a) = &got {
                assert_eq!(a.nodes.len(), f.query.nodes.len(), "{}", path.display());
                assert!(a.found.is_none() || f.query.find.is_some());
                assert_eq!(a.lookup.is_some(), f.query.lookup.is_some());
            }
            // A 200 that should decode must: a fixture that comes back `BadBody` or `NoData` while
            // it holds `data` means a field was renamed.
            if f.status == 200
                && serde_json::from_slice::<Value>(&f.body).is_ok_and(|b| b.get("data").is_some())
            {
                assert!(got.is_ok(), "{}: {got:?}", path.display());
            }
            count += 1;
        }
    }
    assert!(count >= 20, "{count} fixtures");
}

// T37: the request bodies the owner records real responses with.
#[test]
#[ignore = "prints the request bodies for T37; asserts nothing"]
fn dump_queries() {
    let id = |s: &str| NodeId(s.to_string());
    let shapes: Vec<(&str, &str, Query)> = vec![
        (
            "a poll with a finding and two records",
            r#"{"find": true, "nodes": ["record", "record"], "lookup": false}"#,
            Query {
                find: Some(Find::default()),
                nodes: vec![
                    NodeQuery {
                        id: id("PR_A"),
                        want: Want::Record,
                    },
                    NodeQuery {
                        id: id("PR_B"),
                        want: Want::Record,
                    },
                ],
                lookup: None,
            },
        ),
        (
            "a first refresh with items",
            r#"{"find": false, "nodes": ["record_and_items"], "lookup": false}"#,
            Query {
                nodes: vec![NodeQuery {
                    id: id("PR_A"),
                    want: Want::RecordAndItems,
                }],
                ..Query::default()
            },
        ),
        (
            "the record by ID",
            r#"{"find": false, "nodes": ["record"], "lookup": false}"#,
            Query {
                nodes: vec![NodeQuery {
                    id: id("PR_A"),
                    want: Want::Record,
                }],
                ..Query::default()
            },
        ),
        (
            "a check page",
            r#"{"find": false, "nodes": ["checks_page"], "lookup": false}"#,
            Query {
                nodes: vec![NodeQuery {
                    id: id("PR_A"),
                    want: Want::ChecksPage {
                        after: "CURSOR".into(),
                    },
                }],
                ..Query::default()
            },
        ),
        (
            "items alone",
            r#"{"find": false, "nodes": ["items"], "lookup": false}"#,
            Query {
                nodes: vec![NodeQuery {
                    id: id("PR_A"),
                    want: Want::Items,
                }],
                ..Query::default()
            },
        ),
        (
            "a lookup by name",
            r#"{"find": false, "nodes": [], "lookup": true}"#,
            Query {
                lookup: Some(PrName::parse("yachiko/experiment#1").unwrap()),
                ..Query::default()
            },
        ),
    ];
    for (label, request, query) in shapes {
        println!("== {label}\n== request: {request}");
        println!("{}\n", request_body(&query));
    }
}
