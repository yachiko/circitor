#![cfg(feature = "test-endpoint")]
//! T41: the hand-written golden files hold the API's and the model's types to the specs (API-19).
//! The files are never rewritten from the types: every comparison goes through `golden::compare`.

use std::collections::BTreeSet;
use std::path::Path;

use circitor::api::types::{
    CancelBody, ChangesBody, Envelope, ErrorBody, ListBody, NotifyBody, RecordBody, RefreshBody,
    StatusBody, WaitBody,
};
use circitor::model::Record;
use circitor::testkit::golden;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

type Check = fn(&str);

/// `file` decodes into `T`, and `T`'s pretty JSON plus a newline equals the file (a string comparison).
fn agree<T: Serialize + DeserializeOwned>(file: &str) {
    let text = std::fs::read_to_string(root().join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let value: T = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut again = serde_json::to_string_pretty(&value).expect("serialises");
    again.push('\n');
    golden::compare(file, &again);
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The one table: every golden file of `testdata/api/` and `testdata/model/`, and the type it holds.
fn table() -> Vec<(&'static str, Check)> {
    vec![
        ("testdata/api/list.json", agree::<Envelope<ListBody>>),
        ("testdata/api/list-reset.json", agree::<Envelope<ListBody>>),
        ("testdata/api/show.json", agree::<Envelope<RecordBody>>),
        ("testdata/api/track.json", agree::<Envelope<RecordBody>>),
        (
            "testdata/api/untrack-null.json",
            agree::<Envelope<RecordBody>>,
        ),
        ("testdata/api/changes.json", agree::<Envelope<ChangesBody>>),
        (
            "testdata/api/wait-satisfied.json",
            agree::<Envelope<WaitBody>>,
        ),
        (
            "testdata/api/wait-timeout-no-record.json",
            agree::<Envelope<WaitBody>>,
        ),
        ("testdata/api/wait-reset.json", agree::<Envelope<WaitBody>>),
        (
            "testdata/api/notify-registered.json",
            agree::<Envelope<NotifyBody>>,
        ),
        (
            "testdata/api/notify-already.json",
            agree::<Envelope<NotifyBody>>,
        ),
        (
            "testdata/api/notify-cancelled.json",
            agree::<Envelope<CancelBody>>,
        ),
        ("testdata/api/refresh.json", agree::<Envelope<RefreshBody>>),
        ("testdata/api/status.json", agree::<Envelope<StatusBody>>),
        (
            "testdata/api/status-problem.json",
            agree::<Envelope<StatusBody>>,
        ),
        ("testdata/api/error-bad_request.json", agree::<ErrorBody>),
        ("testdata/api/error-not_found.json", agree::<ErrorBody>),
        ("testdata/api/error-unknown_route.json", agree::<ErrorBody>),
        ("testdata/api/error-auth.json", agree::<ErrorBody>),
        ("testdata/api/error-starting.json", agree::<ErrorBody>),
        (
            "testdata/api/error-github_unavailable.json",
            agree::<ErrorBody>,
        ),
        ("testdata/api/error-rate_limited.json", agree::<ErrorBody>),
        ("testdata/api/error-limit.json", agree::<ErrorBody>),
        ("testdata/api/error-no_session.json", agree::<ErrorBody>),
        ("testdata/model/record-open.json", agree::<Record>),
        ("testdata/model/record-merged.json", agree::<Record>),
    ]
}

fn json(file: &str) -> Value {
    let text = std::fs::read_to_string(root().join("testdata").join(file))
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("an object").keys().cloned().collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn api_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root().join("testdata/api"))
        .expect("testdata/api")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// API-19: each file decodes into its type and equals the type's pretty JSON, key order included.
#[test]
fn api19_files_and_types_agree() {
    for (file, check) in table() {
        check(file);
    }
}

// API-19: the envelope's three keys, then exactly the row of the route.
#[test]
fn api19_every_file_has_its_rows_keys() {
    let envelope = ["version", "cursor", "daemon"];
    let rows: &[(&str, &[&str])] = &[
        ("list", &["records", "untracked", "pending", "reset"]),
        ("show", &["record"]),
        ("track", &["record"]),
        ("untrack", &["record"]),
        (
            "changes",
            &["events", "records", "pending", "more_events", "reset"],
        ),
        (
            "wait",
            &[
                "outcome",
                "reason",
                "record",
                "events",
                "records",
                "more_events",
                "reset",
            ],
        ),
        (
            "notify-registered",
            &[
                "registered",
                "replaced",
                "expires_at",
                "outcome",
                "reason",
                "record",
                "events",
                "records",
                "more_events",
                "reset",
            ],
        ),
        (
            "notify-already",
            &[
                "registered",
                "replaced",
                "expires_at",
                "outcome",
                "reason",
                "record",
                "events",
                "records",
                "more_events",
                "reset",
            ],
        ),
        ("notify-cancelled", &["cancelled"]),
        ("refresh", &["outcome", "cost"]),
        (
            "status",
            &[
                "mode",
                "started_at",
                "last_poll",
                "cost_last_hour",
                "github",
                "tracked",
                "waiters",
                "subscriptions",
                "owners",
                "token_source",
                "gh_path",
                "paths",
                "warnings",
            ],
        ),
    ];
    let mut seen = 0;
    for name in api_files() {
        let v = json(&format!("api/{name}"));
        if name.starts_with("error-") {
            assert_eq!(keys(&v), set(&["version", "error"]), "{name}");
            assert_eq!(
                keys(&v["error"]),
                set(&["code", "message", "until"]),
                "{name}"
            );
            seen += 1;
            continue;
        }
        let stem = name.trim_end_matches(".json");
        let row = rows
            .iter()
            .find(|(prefix, _)| {
                stem == *prefix
                    || (stem.starts_with(prefix)
                        && stem[prefix.len()..].starts_with('-')
                        && !prefix.contains('-'))
            })
            .unwrap_or_else(|| panic!("{name}: no row"));
        let mut want = set(&envelope);
        want.extend(row.1.iter().map(|s| s.to_string()));
        assert_eq!(keys(&v), want, "{name}");
        seen += 1;
    }
    assert_eq!(seen, api_files().len());
}

// API-19: lists are written when empty, flags when false, and options as null.
#[test]
fn api19_empty_lists_and_flags_are_written() {
    let v = json("api/list.json");
    assert_eq!(v["untracked"], serde_json::json!([]));
    assert_eq!(v["pending"], serde_json::json!([]));
    assert_eq!(v["reset"], false);

    let v = json("api/wait-timeout-no-record.json");
    assert!(v["reason"].is_null() && v["record"].is_null());
    assert_eq!(v["events"], serde_json::json!([]));
    assert_eq!(v["records"], serde_json::json!([]));
    assert_eq!(v["more_events"], false);
    assert_eq!(v["reset"], false);

    assert!(json("api/untrack-null.json")["record"].is_null());

    let v = json("api/notify-registered.json");
    assert!(v["outcome"].is_null() && v["reason"].is_null());
    assert_eq!(v["events"], serde_json::json!([]));
    assert_eq!(v["records"], serde_json::json!([]));
    assert_eq!(v["more_events"], false);
    assert_eq!(v["reset"], false);
    assert!(json("api/notify-already.json")["expires_at"].is_null());

    let v = json("api/status.json");
    assert_eq!(v["subscriptions"], 0);
    assert_eq!(v["warnings"], serde_json::json!([]));
    for k in ["problem", "message", "until"] {
        assert!(v["daemon"][k].is_null(), "daemon.{k}");
    }
}

// API-3: one file per code, `until` only for the two that carry it.
#[test]
fn api3_one_error_file_per_code() {
    let codes = [
        "bad_request",
        "not_found",
        "unknown_route",
        "auth",
        "starting",
        "github_unavailable",
        "rate_limited",
        "limit",
        "no_session",
    ];
    for code in codes {
        let v = json(&format!("api/error-{code}.json"));
        assert_eq!(v["error"]["code"], code);
        let has_until = !v["error"]["until"].is_null();
        assert_eq!(
            has_until,
            code == "github_unavailable" || code == "rate_limited",
            "{code}"
        );
    }
    let found = api_files()
        .iter()
        .filter(|n| n.starts_with("error-"))
        .count();
    assert_eq!(found, codes.len());
}

// MOD-15, MOD-12: every kind is in `changes.json`, with the items' optional keys as MOD-12 says.
#[test]
fn mod15_changes_holds_every_kind() {
    let v = json("api/changes.json");
    let mut kinds = BTreeSet::new();
    let mut items = Vec::new();
    for event in v["events"].as_array().expect("events") {
        assert!(event.get("id").is_none(), "an event has no id");
        for kind in event["kinds"].as_array().expect("kinds") {
            kinds.insert(kind["kind"].as_str().expect("kind").to_string());
            if kind["kind"] == "activity" {
                items.extend(kind["items"].as_array().expect("items").clone());
            }
        }
    }
    let all = [
        "status",
        "head",
        "checks",
        "merged",
        "closed",
        "reopened",
        "gone",
        "draft",
        "mergeable",
        "review",
        "activity",
        "edited",
        "tracked",
        "untracked",
    ];
    assert_eq!(kinds, set(&all));
    let review = items
        .iter()
        .find(|i| i["kind"] == "review")
        .expect("a review item");
    assert_eq!(review["inline"], 1);
    assert!(
        review["fetch_inline"]
            .as_str()
            .expect("fetch_inline")
            .ends_with("/comments")
    );
    let comment = items
        .iter()
        .find(|i| i["kind"] == "comment")
        .expect("a comment item");
    assert!(comment.get("inline").is_none() && comment.get("review_state").is_none());
    assert!(comment.get("fetch_inline").is_none());
}

// API-6, API-19: `checks.items` only where the spec puts them.
#[test]
fn api6_list_leaves_items_out_and_show_has_them() {
    let v = json("api/list.json");
    let records = v["records"].as_array().expect("records");
    assert_eq!(records.len(), 2);
    for r in records {
        assert!(r["checks"].get("items").is_none());
        assert!(r["checks"].get("failed_items").is_some());
    }
    for file in [
        "show",
        "wait-satisfied",
        "notify-registered",
        "notify-already",
    ] {
        let v = json(&format!("api/{file}.json"));
        assert!(v["record"]["checks"].get("items").is_some(), "{file}");
    }
    let v = json("api/wait-reset.json");
    assert!(v["record"]["checks"].get("items").is_some());
    let records = v["records"].as_array().expect("records");
    assert!(!records.is_empty());
    for r in records {
        assert!(r["checks"].get("items").is_none());
    }
}

fn has_key(v: &Value, names: &[&str]) -> bool {
    match v {
        Value::Object(m) => m
            .iter()
            .any(|(k, v)| names.contains(&k.as_str()) || has_key(v, names)),
        Value::Array(a) => a.iter().any(|v| has_key(v, names)),
        _ => false,
    }
}

// NTF-9: no answer holds a session's socket or token. API-5's `paths.socket` is the daemon's own
// socket, not a session's, so it's the one key set aside.
#[test]
fn ntf9_no_answer_holds_a_session() {
    for name in api_files() {
        let mut v = json(&format!("api/{name}"));
        if let Some(m) = v.as_object_mut() {
            m.remove("paths");
        }
        assert!(!has_key(&v, &["socket", "token"]), "{name}");
    }
}

#[test]
fn every_file_is_in_the_table() {
    let named: BTreeSet<String> = table().iter().map(|(f, _)| f.to_string()).collect();
    let mut found = BTreeSet::new();
    for dir in ["testdata/api", "testdata/model"] {
        for e in std::fs::read_dir(root().join(dir)).expect(dir) {
            found.insert(format!(
                "{dir}/{}",
                e.expect("entry").file_name().to_string_lossy()
            ));
        }
    }
    assert_eq!(found, named);
}

// P6: GitHub's public schema, as published.
#[test]
fn p6_schema_validates() {
    let path = root().join("testdata/github/schema.graphql");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let schema = apollo_compiler::Schema::parse_and_validate(source, "schema.graphql")
        .unwrap_or_else(|e| panic!("the schema doesn't validate: {e}"));
    assert!(schema.get_object("PullRequest").is_some());
    let commit = schema.get_object("Commit").expect("Commit");
    assert!(commit.fields.contains_key("statusCheckRollup"));
    let query = schema.get_object("Query").expect("Query");
    assert!(query.fields.contains_key("nodes") && query.fields.contains_key("viewer"));
}
