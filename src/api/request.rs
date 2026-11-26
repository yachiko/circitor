//! `Request`'s spelling of the routes (T43).
//!
//! The one place paths, parameter names and body keys are spelt (API-1). `Params::parse` decodes what
//! `path_and_query` encodes.

use std::fmt::Write;

use super::types::{Ask, Request};
use crate::model::PrName;

/// Everything outside `[A-Za-z0-9._~-]` is percent-encoded, byte by byte (API-1). `+` is a plus.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'-') {
            out.push(char::from(b));
        } else {
            // Writing to a `String` can't fail.
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `/v1/prs/{owner}/{repo}/{number}`, every segment encoded.
fn pr_path(pr: &PrName) -> String {
    format!(
        "/v1/prs/{}/{}/{}",
        encode(&pr.owner),
        encode(&pr.repo),
        pr.number
    )
}

/// A query being built: a name and a value at a time, in the order given.
#[derive(Default)]
struct Query(String);

impl Query {
    fn value(&mut self, name: &str, value: &str) {
        self.0.push(if self.0.is_empty() { '?' } else { '&' });
        self.0.push_str(name);
        self.0.push('=');
        self.0.push_str(&encode(value));
    }

    fn opt(&mut self, name: &str, value: Option<&str>) {
        if let Some(v) = value {
            self.value(name, v);
        }
    }

    /// A flag is sent only when set, as `1`.
    fn flag(&mut self, name: &str, set: bool) {
        if set {
            self.value(name, "1");
        }
    }

    fn number(&mut self, name: &str, value: Option<u32>) {
        if let Some(n) = value {
            self.value(name, &n.to_string());
        }
    }

    fn finish(self) -> String {
        self.0
    }
}

/// A JSON string.
fn json_str(s: &str) -> String {
    // Serialising a `str` can't fail.
    serde_json::to_string(s).expect("a string always serialises")
}

/// The `Ask`'s words in the order API-9 gives them: `until`, `sha`, `rerun_since`, `fail_fast`, `kind`, `since`.
fn ask_query(q: &mut Query, ask: &Ask) {
    q.value("until", ask.until.as_str());
    q.opt("sha", ask.sha.as_deref());
    q.opt("rerun_since", ask.rerun_since.as_deref());
    q.flag("fail_fast", ask.fail_fast);
    q.opt("kind", ask.kind.as_deref());
    q.opt("since", ask.since.as_deref());
}

impl Request {
    pub fn method(&self) -> http::Method {
        match self {
            Request::List { .. }
            | Request::Show { .. }
            | Request::Changes { .. }
            | Request::Wait { .. }
            | Request::Status => http::Method::GET,
            Request::Notify { .. }
            | Request::Track { .. }
            | Request::Refresh
            | Request::Shutdown => http::Method::POST,
            Request::NotifyCancel { .. } | Request::Untrack { .. } => http::Method::DELETE,
        }
    }

    /// `/v1/prs?state=all&since=…`, `/v1/prs/{owner}/{repo}/{number}/wait?until=…&timeout=…` and so on,
    /// exactly as 04-api writes them; a flag is sent only when set, as `1`. A `wait`'s query holds `until`,
    /// then `sha`, `rerun_since`, `fail_fast`, `kind` and `since` as set, then `timeout` and `limit`.
    /// `NotifyCancel` is `DELETE /v1/prs/{owner}/{repo}/{number}/notify?socket=…`.
    pub fn path_and_query(&self) -> String {
        let mut q = Query::default();
        match self {
            Request::List { all, since, checks } => {
                if *all {
                    q.value("state", "all");
                }
                q.opt("since", since.as_deref());
                q.flag("checks", *checks);
                format!("/v1/prs{}", q.finish())
            }
            Request::Show { pr, fresh } => {
                q.flag("fresh", *fresh);
                format!("{}{}", pr_path(pr), q.finish())
            }
            Request::Changes { pr, since, limit } => {
                q.opt("since", since.as_deref());
                q.number("limit", *limit);
                match pr {
                    Some(pr) => format!("{}/changes{}", pr_path(pr), q.finish()),
                    None => format!("/v1/changes{}", q.finish()),
                }
            }
            Request::Wait {
                pr,
                cond,
                timeout,
                limit,
            } => {
                ask_query(&mut q, cond);
                q.number("timeout", Some(*timeout));
                q.number("limit", *limit);
                format!("{}/wait{}", pr_path(pr), q.finish())
            }
            Request::Notify { pr, .. } => format!("{}/notify", pr_path(pr)),
            Request::NotifyCancel { pr, socket } => {
                q.value("socket", socket);
                format!("{}/notify{}", pr_path(pr), q.finish())
            }
            Request::Track { .. } => "/v1/track".to_string(),
            Request::Untrack { pr } => format!(
                "/v1/track/{}/{}/{}",
                encode(&pr.owner),
                encode(&pr.repo),
                pr.number
            ),
            Request::Refresh => "/v1/refresh".to_string(),
            Request::Status => "/v1/status".to_string(),
            Request::Shutdown => "/v1/shutdown".to_string(),
        }
    }

    /// `track`: `{"pr":"owner/repo#12"}`. `notify`: one JSON object with `until`, then `sha`, `rerun_since`,
    /// `fail_fast`, `kind`, `since`, `for` and `limit` only as set, then `socket` and `token`. In a body
    /// `fail_fast` is a JSON boolean and `kind` one comma-separated string (NTF-2).
    pub fn body(&self) -> Option<String> {
        match self {
            Request::Track { pr } => Some(format!("{{\"pr\":{}}}", json_str(&pr.to_string()))),
            Request::Notify {
                cond,
                life,
                socket,
                token,
                limit,
                ..
            } => {
                // Built by hand, a key at a time, so they keep NTF-2's order (a `serde_json::Map` sorts them).
                let mut parts = vec![format!("\"until\":{}", json_str(cond.until.as_str()))];
                if let Some(v) = &cond.sha {
                    parts.push(format!("\"sha\":{}", json_str(v)));
                }
                if let Some(v) = &cond.rerun_since {
                    parts.push(format!("\"rerun_since\":{}", json_str(v)));
                }
                if cond.fail_fast {
                    parts.push("\"fail_fast\":true".to_string());
                }
                if let Some(v) = &cond.kind {
                    parts.push(format!("\"kind\":{}", json_str(v)));
                }
                if let Some(v) = &cond.since {
                    parts.push(format!("\"since\":{}", json_str(v)));
                }
                if let Some(n) = life {
                    parts.push(format!("\"for\":{n}"));
                }
                if let Some(n) = limit {
                    parts.push(format!("\"limit\":{n}"));
                }
                parts.push(format!("\"socket\":{}", json_str(socket)));
                parts.push(match token {
                    Some(t) => format!("\"token\":{}", json_str(t)),
                    None => "\"token\":null".to_string(),
                });
                Some(format!("{{{}}}", parts.join(",")))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Params;
    use crate::api::types::Until;

    fn pr() -> PrName {
        PrName::parse("owner/repo#12").unwrap()
    }

    // API-1: every route spelt once, every value encoded.
    #[test]
    fn api1_request_spells_every_route() {
        let get = http::Method::GET;
        let list = Request::List {
            all: true,
            since: None,
            checks: true,
        };
        assert_eq!(list.method(), get);
        assert_eq!(list.path_and_query(), "/v1/prs?state=all&checks=1");
        assert_eq!(list.body(), None);
        let plain = Request::List {
            all: false,
            since: None,
            checks: false,
        };
        assert_eq!(plain.path_and_query(), "/v1/prs");

        let show = Request::Show {
            pr: pr(),
            fresh: true,
        };
        assert_eq!(show.method(), get);
        assert_eq!(show.path_and_query(), "/v1/prs/owner/repo/12?fresh=1");
        let show = Request::Show {
            pr: pr(),
            fresh: false,
        };
        assert_eq!(show.path_and_query(), "/v1/prs/owner/repo/12");

        let all = Request::Changes {
            pr: None,
            since: Some("k3x9a2bq-4".into()),
            limit: Some(20),
        };
        assert_eq!(all.method(), get);
        assert_eq!(
            all.path_and_query(),
            "/v1/changes?since=k3x9a2bq-4&limit=20"
        );
        let one = Request::Changes {
            pr: Some(pr()),
            since: None,
            limit: None,
        };
        assert_eq!(one.path_and_query(), "/v1/prs/owner/repo/12/changes");

        let mut cond = Ask::new(Until::ChecksDone);
        let wait = |cond: Ask, limit| Request::Wait {
            pr: pr(),
            cond,
            timeout: 600,
            limit,
        };
        let w = wait(cond.clone(), None);
        assert_eq!(w.method(), get);
        assert_eq!(
            w.path_and_query(),
            "/v1/prs/owner/repo/12/wait?until=checks-done&timeout=600"
        );
        cond.fail_fast = true;
        cond.sha = Some("abc1234".into());
        cond.rerun_since = Some("k3x9a2bq-1".into());
        cond.since = Some("k3x9a2bq-2".into());
        assert_eq!(
            wait(cond, Some(5)).path_and_query(),
            "/v1/prs/owner/repo/12/wait?until=checks-done&sha=abc1234&rerun_since=k3x9a2bq-1\
             &fail_fast=1&since=k3x9a2bq-2&timeout=600&limit=5"
        );
        let mut changed = Ask::new(Until::Changed);
        changed.kind = Some("activity,merged".into());
        assert_eq!(
            wait(changed, None).path_and_query(),
            "/v1/prs/owner/repo/12/wait?until=changed&kind=activity%2Cmerged&timeout=600"
        );

        let track = Request::Track { pr: pr() };
        assert_eq!(track.method(), http::Method::POST);
        assert_eq!(track.path_and_query(), "/v1/track");
        assert_eq!(track.body().as_deref(), Some(r#"{"pr":"owner/repo#12"}"#));
        let untrack = Request::Untrack { pr: pr() };
        assert_eq!(untrack.method(), http::Method::DELETE);
        assert_eq!(untrack.path_and_query(), "/v1/track/owner/repo/12");
        assert_eq!(untrack.body(), None);

        for (r, method, path) in [
            (Request::Refresh, http::Method::POST, "/v1/refresh"),
            (Request::Status, http::Method::GET, "/v1/status"),
            (Request::Shutdown, http::Method::POST, "/v1/shutdown"),
        ] {
            assert_eq!(r.method(), method);
            assert_eq!(r.path_and_query(), path);
            assert_eq!(r.body(), None);
        }

        // A `since` with a space and an `&` survives the trip.
        let odd = "a b&c=d+e%f";
        let r = Request::Changes {
            pr: None,
            since: Some(odd.into()),
            limit: None,
        };
        let pq = r.path_and_query();
        assert_eq!(pq, "/v1/changes?since=a%20b%26c%3Dd%2Be%25f");
        let (_, query) = pq.split_once('?').unwrap();
        let mut params = Params::parse(Some(query)).unwrap();
        assert_eq!(params.take("since").as_deref(), Some(odd));
        params.finish().unwrap();
    }

    // NTF-2, NTF-3: the notify request.
    #[test]
    fn ntf2_notify_request() {
        let socket = "/run/user/1000/cc-socks/1.sock";
        let mut cond = Ask::new(Until::Merged);
        let n = Request::Notify {
            pr: pr(),
            cond: cond.clone(),
            life: None,
            socket: socket.into(),
            token: None,
            limit: None,
        };
        assert_eq!(n.method(), http::Method::POST);
        assert_eq!(n.path_and_query(), "/v1/prs/owner/repo/12/notify");
        let v: serde_json::Value = serde_json::from_str(&n.body().unwrap()).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj["until"], "merged");
        assert_eq!(obj["socket"], socket);
        assert!(obj["token"].is_null());
        assert_eq!(obj.len(), 3);

        cond.fail_fast = true;
        cond.kind = Some("activity,review".into());
        let n = Request::Notify {
            pr: PrName::parse("o/r#3").unwrap(),
            cond,
            life: Some(60),
            socket: "/tmp/a\"b.sock".into(),
            token: Some("tok".into()),
            limit: Some(7),
        };
        let body = n.body().unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["fail_fast"], true);
        assert_eq!(v["kind"], "activity,review");
        assert_eq!(v["for"], 60);
        assert_eq!(v["limit"], 7);
        assert_eq!(v["token"], "tok");
        assert_eq!(v["socket"], "/tmp/a\"b.sock");
        assert!(v.get("sha").is_none());
        assert!(!body.contains('\n'));

        let c = Request::NotifyCancel {
            pr: pr(),
            socket: socket.into(),
        };
        assert_eq!(c.method(), http::Method::DELETE);
        assert_eq!(
            c.path_and_query(),
            "/v1/prs/owner/repo/12/notify?socket=%2Frun%2Fuser%2F1000%2Fcc-socks%2F1.sock"
        );
        assert_eq!(c.body(), None);
    }
}
