#![cfg(feature = "test-endpoint")]
//! T20: the fake GitHub's HTTP server, driven by a plain `reqwest` client and hand-written queries.

use std::sync::Arc;
use std::time::Duration;

use circitor::clock::{Clock, Mono, SettleProbe, SharedClock, SystemClock, TestClock, Time};
use circitor::testkit::github::{AskedOf, cost_of};
use circitor::testkit::server::{Aim, FakeGitHub, Fault, LimitStatus, Times};
use circitor::testkit::{MARKER_GITHUB_TEXT, MARKER_LOGIN, MARKER_TOKEN, MARKER_TOKEN_2};
use serde_json::{Value, json};

fn t0() -> Time {
    Time::parse("2026-11-24T22:00:00Z").unwrap()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn setup() -> (Arc<TestClock>, FakeGitHub) {
    let clock = TestClock::new(t0());
    let gh = FakeGitHub::start(clock.clone());
    (clock, gh)
}

/// A plain client: no proxy, no redirect followed.
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn body_of(query: &str, vars: &Value) -> Vec<u8> {
    json!({ "query": query, "variables": vars })
        .to_string()
        .into_bytes()
}

async fn send(
    c: &reqwest::Client,
    url: &str,
    query: &str,
    vars: Value,
    token: &str,
    pid: Option<&str>,
) -> reqwest::Result<reqwest::Response> {
    let mut r = c
        .post(url)
        .header("authorization", format!("bearer {token}"))
        .header("content-type", "application/json")
        .body(body_of(query, &vars));
    if let Some(pid) = pid {
        r = r.header("x-circitor-test-pid", pid);
    }
    r.send().await
}

async fn post(c: &reqwest::Client, gh: &FakeGitHub, query: &str) -> reqwest::Response {
    send(c, &gh.url(), query, json!({}), MARKER_TOKEN, None)
        .await
        .unwrap()
}

async fn json_of(r: reqwest::Response) -> Value {
    serde_json::from_slice(&r.bytes().await.unwrap()).unwrap()
}

fn header(r: &reqwest::Response, name: &str) -> Option<String> {
    r.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_owned())
}

fn date_of(r: &reqwest::Response) -> Option<Time> {
    header(r, "date").map(|d| {
        let parsed = chrono::DateTime::parse_from_rfc2822(&d).unwrap();
        Time::from_unix_millis(parsed.timestamp_millis())
    })
}

/// Yields (the IO driver runs between yields) until `cond` holds.
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..100_000 {
        if cond() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("never happened: {what}");
}

const VIEWER: &str = "query { viewer { login } rateLimit { cost remaining resetAt } }";

const FIND: &str = "query($after: String, $ids: [ID!]!) { viewer { pullRequests(states: OPEN, first: 1, \
    after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) { pageInfo { hasNextPage endCursor } \
    nodes { id } } } nodes(ids: $ids) { ... on PullRequest { number } } }";

fn nodes_query(ids: &[&str]) -> String {
    let list: Vec<String> = ids.iter().map(|i| format!("\"{i}\"")).collect();
    format!(
        "query {{ nodes(ids: [{}]) {{ ... on PullRequest {{ number }} }} }}",
        list.join(", ")
    )
}

const LOOKUP: &str =
    "query { repository(owner: \"o\", name: \"r\") { pullRequest(number: 1) { number } } }";

// TST-3: `Date` from GitHub's clock, and the five `x-ratelimit-*` headers; `Date` can be left out.
#[tokio::test]
async fn tst3_headers() {
    let (clock, gh) = setup();
    let c = http();
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 200);
    assert_eq!(date_of(&r), Some(clock.wall()));
    assert_eq!(header(&r, "x-ratelimit-limit").as_deref(), Some("5000"));
    assert_eq!(header(&r, "x-ratelimit-remaining").as_deref(), Some("4999"));
    assert_eq!(header(&r, "x-ratelimit-used").as_deref(), Some("1"));
    let reset = t0().plus(secs(3600)).unix_millis() / 1000;
    assert_eq!(header(&r, "x-ratelimit-reset"), Some(reset.to_string()));
    assert_eq!(
        header(&r, "x-ratelimit-resource").as_deref(),
        Some("graphql")
    );
    gh.without_date_header();
    clock.advance(secs(1)).await;
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 200);
    assert!(header(&r, "date").is_none());
    assert!(header(&r, "x-ratelimit-remaining").is_some());
    gh.check();
}

// TST-3: GitHub's clock follows the wall time forward, never back, with a skew.
#[tokio::test]
async fn tst3_githubs_clock() {
    let (clock, gh) = setup();
    let c = http();
    clock.advance(secs(90)).await;
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(date_of(&r), Some(t0().plus(secs(90))));
    clock.set_wall_back(secs(3600)).await;
    clock.advance(secs(1)).await;
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(date_of(&r), Some(t0().plus(secs(90))));
    gh.check();

    let (clock, gh) = setup();
    gh.set_skew(secs(300));
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(date_of(&r), Some(clock.wall().plus(secs(300))));
    gh.world(|w| w.after(secs(30), |w| w.set_login("later-login")));
    clock.advance(secs(31)).await;
    let v = json_of(post(&c, &gh, VIEWER).await).await;
    assert_eq!(v["data"]["viewer"]["login"], "later-login");
    gh.check();
}

// TST-3: every fault of "it can be told to misbehave", one case each.
#[tokio::test]
async fn tst3_faults() {
    let (clock, gh) = setup();
    let c = http();
    let next = || clock.advance(secs(1));

    // A 500 with a short text page.
    gh.fault(Fault::Status(500), Times::Requests(1));
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 500);
    assert!(r.text().await.unwrap().contains(MARKER_GITHUB_TEXT));
    next().await;

    // A hang that ends only when the client drops the request.
    gh.fault(Fault::Hang, Times::Requests(1));
    let url = gh.url();
    let hung = {
        let c = c.clone();
        tokio::spawn(async move { send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, None).await })
    };
    gh.until_held(1).await;
    clock.settle().await;
    assert!(!hung.is_finished());
    hung.abort();
    assert!(hung.await.unwrap_err().is_cancelled());
    until("the hang ends", || gh.held() == 0).await;
    next().await;

    // A dropped connection: no status.
    gh.fault(Fault::Drop, Times::Requests(1));
    let e = send(&c, &gh.url(), VIEWER, json!({}), MARKER_TOKEN, None)
        .await
        .unwrap_err();
    assert!(e.status().is_none());
    next().await;

    // A 401 with a REST-style body.
    gh.fault(Fault::Unauthorized, Times::Requests(1));
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 401);
    let v = json_of(r).await;
    assert_eq!(v["status"], "401");
    assert!(v.get("data").is_none() && v.get("errors").is_none());
    next().await;

    // A 200 without `data`.
    gh.fault(Fault::NoData, Times::Requests(1));
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 200);
    assert!(json_of(r).await.get("data").is_none());
    next().await;

    // A body that isn't JSON.
    gh.fault(Fault::NotJson, Times::Requests(1));
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 200);
    let bytes = r.bytes().await.unwrap();
    assert!(serde_json::from_slice::<Value>(&bytes).is_err());
    next().await;

    // A body shorter than its `Content-Length`.
    gh.fault(Fault::CutShort, Times::Requests(1));
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 200);
    assert!(r.content_length().unwrap() > 0);
    assert!(r.bytes().await.is_err());
    next().await;

    // A 302 whose target counts its hits.
    gh.fault(Fault::Redirect, Times::Requests(1));
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 302);
    let to = header(&r, "location").unwrap();
    assert!(to.starts_with("http://127.0.0.1:"));
    assert_eq!(gh.redirect_hits(), 0);
    let followed = c.get(&to).send().await.unwrap();
    assert_eq!(followed.status(), 200);
    assert_eq!(gh.redirect_hits(), 1);
    next().await;

    // A refused connection, then listening again on the same port.
    gh.refuse().await;
    let e = send(&c, &gh.url(), VIEWER, json!({}), MARKER_TOKEN, None)
        .await
        .unwrap_err();
    assert!(e.is_connect(), "{e}");
    gh.listen().await;
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(r.status(), 200);
    next().await;

    // Exhausted, with and without `x-ratelimit-reset`, and no `data`.
    for (status, reset, code) in [
        (LimitStatus::Forbidden, true, 403),
        (LimitStatus::Ok, false, 200),
        (LimitStatus::Forbidden, false, 403),
        (LimitStatus::Ok, true, 200),
    ] {
        gh.fault(Fault::Exhausted { status, reset }, Times::Requests(1));
        let r = post(&c, &gh, VIEWER).await;
        assert_eq!(r.status(), code);
        assert_eq!(header(&r, "x-ratelimit-remaining").as_deref(), Some("0"));
        assert_eq!(header(&r, "x-ratelimit-reset").is_some(), reset);
        let v = json_of(r).await;
        assert!(v.get("data").is_none(), "{v}");
        next().await;
    }

    // A secondary limit as a 403 and as a 200, with `Retry-After: 30` and without.
    for (status, retry_after, code) in [
        (LimitStatus::Forbidden, Some(30), 403),
        (LimitStatus::Forbidden, None, 403),
        (LimitStatus::Ok, Some(30), 200),
        (LimitStatus::Ok, None, 200),
    ] {
        gh.fault(
            Fault::Secondary {
                status,
                retry_after,
            },
            Times::Requests(1),
        );
        let r = post(&c, &gh, VIEWER).await;
        assert_eq!(r.status(), code);
        assert_eq!(
            header(&r, "retry-after"),
            retry_after.map(|s| s.to_string())
        );
        let v = json_of(r).await;
        assert!(v.get("data").is_none());
        let message = v["message"]
            .as_str()
            .or(v["errors"][0]["message"].as_str())
            .unwrap();
        assert!(message.contains("secondary rate limit"), "{message}");
        next().await;
    }

    // A fault set for 2 requests is gone at the third.
    gh.fault(Fault::Status(502), Times::Requests(2));
    assert_eq!(post(&c, &gh, VIEWER).await.status(), 502);
    next().await;
    assert_eq!(post(&c, &gh, VIEWER).await.status(), 502);
    next().await;
    assert_eq!(post(&c, &gh, VIEWER).await.status(), 200);
    gh.check();
}

// TST-3: a fault aimed at requests with or without a finding, at one node, at a lookup.
#[tokio::test]
async fn tst3_aimed_faults() {
    let (clock, gh) = setup();
    let c = http();
    let (a, b, id_a, id_b) = gh.world(|w| {
        let repo = w.repo("o", "r");
        let a = w.open_pr(repo);
        let b = w.open_pr(repo);
        let (ia, ib) = (w.node_id(a).to_owned(), w.node_id(b).to_owned());
        (a, b, ia, ib)
    });
    let _ = b;
    let nodes = nodes_query(&[&id_a, &id_b]);
    let find_vars = json!({ "ids": [] });
    let find = |c: &reqwest::Client, url: String, vars: Value| {
        let c = c.clone();
        async move {
            send(&c, &url, FIND, vars, MARKER_TOKEN, None)
                .await
                .unwrap()
        }
    };

    gh.fault_aimed(Fault::Status(500), Aim::WithFinding, Times::UntilCleared);
    assert_eq!(post(&c, &gh, &nodes).await.status(), 200);
    clock.advance(secs(1)).await;
    assert_eq!(find(&c, gh.url(), find_vars.clone()).await.status(), 500);
    clock.advance(secs(1)).await;
    gh.clear_faults();

    gh.fault_aimed(Fault::Status(500), Aim::WithoutFinding, Times::UntilCleared);
    assert_eq!(find(&c, gh.url(), find_vars.clone()).await.status(), 200);
    clock.advance(secs(1)).await;
    assert_eq!(post(&c, &gh, &nodes).await.status(), 500);
    clock.advance(secs(1)).await;
    gh.clear_faults();

    // `NOT_FOUND` aimed at one node leaves the other node of the same request.
    gh.pr_error(a, "NOT_FOUND", Aim::node(&id_a), Times::Requests(1));
    let v = json_of(post(&c, &gh, &nodes).await).await;
    assert_eq!(v["data"]["nodes"][0], Value::Null);
    assert_eq!(v["data"]["nodes"][1]["number"], 2);
    assert_eq!(v["errors"][0]["type"], "NOT_FOUND");
    assert_eq!(v["errors"][0]["path"], json!(["nodes", 0]));
    clock.advance(secs(1)).await;
    let v = json_of(post(&c, &gh, &nodes).await).await;
    assert!(v.get("errors").is_none(), "{v}");
    clock.advance(secs(1)).await;

    // `FORBIDDEN` aimed at a lookup puts the error on `repository.pullRequest`.
    gh.pr_error(a, "FORBIDDEN", Aim::lookup("o", "r", 1), Times::Requests(1));
    // Not at a request by node.
    let v = json_of(post(&c, &gh, &nodes).await).await;
    assert!(v.get("errors").is_none(), "{v}");
    clock.advance(secs(1)).await;
    let v = json_of(post(&c, &gh, LOOKUP).await).await;
    assert_eq!(v["errors"][0]["type"], "FORBIDDEN");
    assert_eq!(v["errors"][0]["path"], json!(["repository", "pullRequest"]));
    assert_eq!(v["data"]["repository"]["pullRequest"], Value::Null);
    clock.advance(secs(1)).await;
    let v = json_of(post(&c, &gh, LOOKUP).await).await;
    assert_eq!(v["data"]["repository"]["pullRequest"]["number"], 1);
    gh.check();
}

// TST-3: `hold_next` holds one request until released, then answers from the world as it is then.
#[tokio::test]
async fn tst3_hold_next() {
    let (clock, gh) = setup();
    let c = http();
    let mut hold = gh.hold_next();
    let url = gh.url();
    let held = {
        let c = c.clone();
        tokio::spawn(async move {
            let r = send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, None)
                .await
                .unwrap();
            json_of(r).await
        })
    };
    hold.arrived().await;
    assert!(hold.is_holding());
    clock.settle().await;
    assert!(!held.is_finished());
    assert_eq!(gh.held(), 1);
    gh.set_login("changed-meanwhile");
    hold.release();
    let v = held.await.unwrap();
    assert_eq!(v["data"]["viewer"]["login"], "changed-meanwhile");
    assert_eq!(gh.held(), 0);
    clock.advance(secs(1)).await;
    // The request after it isn't held.
    let v = json_of(post(&c, &gh, VIEWER).await).await;
    assert_eq!(v["data"]["viewer"]["login"], "changed-meanwhile");
    gh.check();
}

// TST-3, POL-20: only the tokens named are accepted; the log keeps each `Authorization`.
#[tokio::test]
async fn tst3_tokens() {
    let (clock, gh) = setup();
    let c = http();
    gh.accept_only(&[MARKER_TOKEN]);
    let r = send(&c, &gh.url(), VIEWER, json!({}), MARKER_TOKEN, None)
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    clock.advance(secs(1)).await;
    let r = send(&c, &gh.url(), VIEWER, json!({}), MARKER_TOKEN_2, None)
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(json_of(r).await["status"], "401");
    let log = gh.requests();
    assert_eq!(
        log[0].authorization.as_deref(),
        Some(format!("bearer {MARKER_TOKEN}").as_str())
    );
    assert_eq!(
        log[1].authorization.as_deref(),
        Some(format!("bearer {MARKER_TOKEN_2}").as_str())
    );
    assert_eq!(log[1].cost, 0);
    gh.check();
}

// TST-3: the test sets `remaining` and the reset time, makes them lag, and sets the login.
#[tokio::test]
async fn tst3_rate_controls() {
    let (clock, gh) = setup();
    let c = http();
    let reset = t0().plus(secs(1800));
    gh.set_rate(400, reset);
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(header(&r, "x-ratelimit-remaining").as_deref(), Some("399"));
    assert_eq!(
        header(&r, "x-ratelimit-reset"),
        Some((reset.unix_millis() / 1000).to_string())
    );
    let v = json_of(r).await;
    assert_eq!(v["data"]["rateLimit"]["cost"], 1);
    assert_eq!(v["data"]["rateLimit"]["remaining"], 399);
    assert_eq!(v["data"]["rateLimit"]["resetAt"], reset.rfc3339());

    // With a lag of 2, an answer reports the value from two requests before.
    gh.set_rate_lag(2);
    let mut seen = Vec::new();
    for _ in 0..4 {
        clock.advance(secs(1)).await;
        let r = post(&c, &gh, VIEWER).await;
        let h: u32 = header(&r, "x-ratelimit-remaining")
            .unwrap()
            .parse()
            .unwrap();
        let v = json_of(r).await;
        assert_eq!(v["data"]["rateLimit"]["remaining"], h);
        seen.push(h);
    }
    // True values after each: 398, 397, 396, 395.
    assert_eq!(seen, [399, 399, 398, 397]);
    gh.set_rate_lag(0);
    clock.advance(secs(1)).await;
    let r = post(&c, &gh, VIEWER).await;
    assert_eq!(header(&r, "x-ratelimit-remaining").as_deref(), Some("394"));

    // MOD-11: the login.
    gh.set_login("someone-else");
    clock.advance(secs(1)).await;
    let v = json_of(post(&c, &gh, VIEWER).await).await;
    assert_eq!(v["data"]["viewer"]["login"], "someone-else");
    gh.check();
}

// TST-3, TST-6: every request is logged with its times, cost, finding, nodes and lookup.
#[tokio::test]
async fn tst3_request_log() {
    let (clock, gh) = setup();
    let c = http();
    let (id_a, id_b) = gh.world(|w| {
        let repo = w.repo("o", "r");
        let a = w.open_pr(repo);
        let b = w.open_pr(repo);
        (w.node_id(a).to_owned(), w.node_id(b).to_owned())
    });
    let mut sent: Vec<Vec<u8>> = Vec::new();

    // A finding with two nodes.
    let vars = json!({ "ids": [id_a, id_b] });
    let r = send(&c, &gh.url(), FIND, vars.clone(), MARKER_TOKEN, None)
        .await
        .unwrap();
    sent.push(body_of(FIND, &vars));
    let v = json_of(r).await;
    let cursor = v["data"]["viewer"]["pullRequests"]["pageInfo"]["endCursor"]
        .as_str()
        .unwrap()
        .to_owned();
    clock.advance(secs(1)).await;

    // Its second page.
    let vars = json!({ "ids": [], "after": cursor });
    send(&c, &gh.url(), FIND, vars.clone(), MARKER_TOKEN, None)
        .await
        .unwrap();
    sent.push(body_of(FIND, &vars));
    clock.advance(secs(1)).await;

    // An items request, a check page, a lookup.
    let items = format!(
        "query {{ nodes(ids: [\"{id_a}\"]) {{ ... on PullRequest {{ comments(last: 10) {{ nodes {{ databaseId }} }} }} }} }}"
    );
    let page = format!(
        "query {{ node(id: \"{id_b}\") {{ ... on PullRequest {{ commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ \
         contexts(first: 100, after: \"Y3Vyc29y:99\") {{ nodes {{ __typename }} }} }} }} }} }} }} }} }}"
    );
    for q in [items.as_str(), page.as_str(), LOOKUP] {
        post(&c, &gh, q).await;
        sent.push(body_of(q, &json!({})));
        clock.advance(secs(1)).await;
    }

    // A faulted request: cost 0.
    gh.fault(Fault::Status(500), Times::Requests(1));
    post(&c, &gh, VIEWER).await;

    let log = gh.requests();
    assert_eq!(log.len(), 6);
    for (i, r) in log.iter().enumerate() {
        let at = secs(i as u64);
        assert_eq!(r.index, i);
        assert_eq!(r.mono, Mono::ZERO.plus(at), "{r:?}");
        assert_eq!(r.wall, t0().plus(at), "{r:?}");
    }
    for (r, body) in log.iter().zip(&sent) {
        assert_eq!(r.cost, cost_of(body).unwrap(), "{r:?}");
        assert!(r.cost > 0);
        assert!(r.fault.is_none());
    }
    assert_eq!(log[0].finding, Some(None));
    assert_eq!(
        log[0].nodes,
        [
            (id_a.clone(), AskedOf::Record),
            (id_b.clone(), AskedOf::Record)
        ]
    );
    assert_eq!(log[1].finding, Some(Some(cursor)));
    assert!(log[1].nodes.is_empty());
    assert_eq!(log[2].finding, None);
    assert_eq!(log[2].nodes, [(id_a.clone(), AskedOf::Items)]);
    assert_eq!(log[3].nodes, [(id_b.clone(), AskedOf::ChecksPage)]);
    assert_eq!(log[4].lookup, Some(("o".into(), "r".into(), 1)));
    assert!(log[4].nodes.is_empty() && log[4].finding.is_none());
    assert_eq!(log[5].cost, 0);
    assert_eq!(log[5].fault, Some(Fault::Status(500)));
    assert_eq!(gh.points(), log.iter().map(|r| r.cost).sum::<u32>());
    assert_eq!(
        gh.points(),
        sent.iter().map(|b| cost_of(b).unwrap()).sum::<u32>()
    );
    gh.check();
}

// POL-1: a mutation fails the test.
#[tokio::test]
#[should_panic(expected = "POL-1: a mutation")]
async fn tst3_rules_pol1_mutation() {
    let (_clock, gh) = setup();
    let c = http();
    post(
        &c,
        &gh,
        "mutation { addStar(input: {starrableId: \"x\"}) { clientMutationId } }",
    )
    .await;
    gh.check();
}

// TST-3: a field the schema lacks fails the test.
#[tokio::test]
#[should_panic(expected = "a query the schema refuses")]
async fn tst3_rules_not_in_schema() {
    let (_clock, gh) = setup();
    let c = http();
    post(&c, &gh, "query { viewer { nope } }").await;
    gh.check();
}

// POL-2: a second request while a hung one is open fails the test.
#[tokio::test]
#[should_panic(expected = "while 1 other was in flight")]
async fn tst3_rules_pol2_in_flight() {
    let (_clock, gh) = setup();
    let c = http();
    gh.fault(Fault::Hang, Times::Requests(1));
    let url = gh.url();
    let _hung = {
        let c = c.clone();
        tokio::spawn(async move { send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, None).await })
    };
    gh.until_held(1).await;
    post(&c, &gh, VIEWER).await;
    gh.check();
}

// POL-2: a request 0.9 s after the last one ended fails the test.
#[tokio::test]
#[should_panic(expected = "under 1 s")]
async fn tst3_rules_pol2_gap() {
    let (clock, gh) = setup();
    let c = http();
    post(&c, &gh, VIEWER).await;
    clock.advance(Duration::from_millis(900)).await;
    post(&c, &gh, VIEWER).await;
    gh.check();
}

// POL-2: exactly 1 s apart passes; a hung request ends when its client drops; `new_daemon` resets.
#[tokio::test]
async fn tst3_rules_hold() {
    let (clock, gh) = setup();
    let c = http();
    post(&c, &gh, VIEWER).await;
    clock.advance(secs(1)).await;
    post(&c, &gh, VIEWER).await;
    clock.advance(secs(1)).await;

    gh.fault(Fault::Hang, Times::Requests(1));
    let url = gh.url();
    let hung = {
        let c = c.clone();
        tokio::spawn(async move { send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, None).await })
    };
    gh.until_held(1).await;
    clock.advance(secs(5)).await;
    hung.abort();
    let _ = hung.await;
    until("the hang ends", || gh.held() == 0).await;
    clock.advance(secs(1)).await;
    post(&c, &gh, VIEWER).await;

    gh.new_daemon();
    post(&c, &gh, VIEWER).await;
    assert_eq!(gh.requests().len(), 5);
    gh.check();
}

// TST-3, TST-9: under `start_real` the rules are held per `x-circitor-test-pid`.
#[tokio::test]
async fn tst3_real_processes_are_told_apart_by_pid() {
    let clock: SharedClock = SystemClock::new();
    let gh = FakeGitHub::start_real(clock.clone());
    let c = http();
    let url = gh.url();
    send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, Some("101"))
        .await
        .unwrap();
    circitor::clock::sleep(&clock, Duration::from_millis(100)).await;
    send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, Some("102"))
        .await
        .unwrap();
    assert!(gh.broken().is_empty(), "{:?}", gh.broken());
    circitor::clock::sleep(&clock, Duration::from_millis(100)).await;
    send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, Some("102"))
        .await
        .unwrap();
    let broken = gh.take_broken();
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert!(broken[0].contains("under 1 s") && broken[0].contains("pid 102"));
    let pids: Vec<Option<String>> = gh.requests().into_iter().map(|r| r.pid).collect();
    assert_eq!(
        pids,
        [Some("101".into()), Some("102".into()), Some("102".into())]
    );
    gh.check();
}

// TST-4: settled with no connection, under a hang and a hold; the excuse covers the engine's `Busy`.
#[tokio::test]
async fn tst4_probe_and_excuse() {
    let (clock, gh) = setup();
    let activity = clock.activity();
    let c = http();
    assert!(gh.settled());

    // A hang is held by script.
    gh.fault(Fault::Hang, Times::Requests(1));
    let url = gh.url();
    let hung = {
        let c = c.clone();
        let url = url.clone();
        tokio::spawn(async move { send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, None).await })
    };
    gh.until_held(1).await;
    assert!(gh.settled());
    assert_eq!(activity.in_flight(), 0);
    gh.clear_faults();
    // Cleared, the hang's connection is closed: the client sees no answer.
    assert!(hung.await.unwrap().is_err());
    until("the hang ends", || gh.held() == 0).await;
    clock.advance(secs(1)).await;

    // A held request, with the engine's `Busy` around it.
    let busy = activity.begin();
    let mut hold = gh.hold_next();
    let held = {
        let c = c.clone();
        let url = url.clone();
        tokio::spawn(async move { send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, None).await })
    };
    hold.arrived().await;
    assert!(gh.settled());
    assert_eq!(activity.in_flight(), 0);
    clock.advance(secs(5)).await;
    hold.release();
    let r = held.await.unwrap().unwrap();
    assert_eq!(r.status(), 200);
    drop(busy);
    assert_eq!(activity.in_flight(), 0);
    clock.advance(secs(1)).await;

    // The same with no `Busy` taken: the count never goes below 0.
    let mut hold = gh.hold_next();
    let held = {
        let c = c.clone();
        tokio::spawn(async move { send(&c, &url, VIEWER, json!({}), MARKER_TOKEN, None).await })
    };
    hold.arrived().await;
    assert_eq!(activity.in_flight(), 0);
    clock.advance(secs(5)).await;
    hold.release();
    assert_eq!(held.await.unwrap().unwrap().status(), 200);
    until("the answer is out", || gh.held() == 0).await;
    assert_eq!(activity.in_flight(), 0);
    // No excuse is left behind to hide real work.
    let work = activity.begin();
    assert_eq!(activity.in_flight(), 1);
    drop(work);
    assert!(gh.settled());
    gh.check();
}

// TST-9: on the system clock, two processes overlap freely, one is held to 1 s, `Date` is real.
#[tokio::test]
async fn tst9_start_real() {
    let clock: SharedClock = SystemClock::new();
    let gh = FakeGitHub::start_real(clock.clone());
    let url = gh.url();
    let (one, two) = (http(), http());

    let mut hold = gh.hold_next();
    let first = {
        let url = url.clone();
        tokio::spawn(
            async move { send(&one, &url, VIEWER, json!({}), MARKER_TOKEN, Some("1")).await },
        )
    };
    hold.arrived().await;
    let r = send(&two, &url, VIEWER, json!({}), MARKER_TOKEN, Some("2"))
        .await
        .unwrap();
    let date = date_of(&r).unwrap();
    let now = clock.wall();
    assert!(
        now.since(date) <= secs(2) && date.since(now) <= secs(2),
        "{date} vs {now}"
    );
    hold.release();
    assert_eq!(first.await.unwrap().unwrap().status(), 200);
    assert!(gh.broken().is_empty(), "{:?}", gh.broken());

    circitor::clock::sleep(&clock, Duration::from_millis(200)).await;
    send(&two, &url, VIEWER, json!({}), MARKER_TOKEN, Some("2"))
        .await
        .unwrap();
    let broken = gh.take_broken();
    assert_eq!(broken.len(), 1, "{broken:?}");
    assert!(broken[0].contains("under 1 s"));
    gh.check();
}

// TST-11: the 401's body and both limit messages hold `MARKER_GITHUB_TEXT`.
#[tokio::test]
async fn tst11_markers() {
    let (clock, gh) = setup();
    let c = http();
    for fault in [
        Fault::Unauthorized,
        Fault::Exhausted {
            status: LimitStatus::Forbidden,
            reset: true,
        },
        Fault::Exhausted {
            status: LimitStatus::Ok,
            reset: true,
        },
        Fault::Secondary {
            status: LimitStatus::Forbidden,
            retry_after: None,
        },
        Fault::Secondary {
            status: LimitStatus::Ok,
            retry_after: None,
        },
        Fault::Status(502),
        Fault::Redirect,
        Fault::NoData,
    ] {
        gh.fault(fault.clone(), Times::Requests(1));
        let text = post(&c, &gh, VIEWER).await.text().await.unwrap();
        assert!(text.contains(MARKER_GITHUB_TEXT), "{fault:?}: {text}");
        clock.advance(secs(1)).await;
    }
    // A refusal's answer too; the default login is the marker one.
    let v = json_of(post(&c, &gh, VIEWER).await).await;
    assert_eq!(v["data"]["viewer"]["login"], MARKER_LOGIN);
    gh.check();
}
