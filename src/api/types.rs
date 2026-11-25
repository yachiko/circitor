//! The JSON both sides share (API-2 to API-5, API-19, NTF-2, NTF-3). Built by T02.
//!
//! None of these is `deny_unknown_fields`: a CLI reads a newer daemon's answers (CLI-9). Only
//! `checks.items` and an item's review-only keys are ever left out (API-19).

use http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::clock::Time;
use crate::daemon::Mode;
use crate::model::{Event, PrName, Record};
use crate::poller::{PollOutcome, ProblemCode, Tier};
use crate::status::Warning;

/// API-3, in its table's order. `http()`: 400, 404, 404, 503, 503, 503, 503, 409, 409. `Limit` is STA-3's
/// places, API-14's waiters and NTF-2's subscriptions; `NoSession` is NTF-2's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    NotFound,
    UnknownRoute,
    Auth,
    Starting,
    GithubUnavailable,
    RateLimited,
    Limit,
    NoSession,
}

impl ErrorCode {
    /// API-3's HTTP status for the code.
    pub fn http(self) -> StatusCode {
        // API-3: the table's HTTP column.
        match self {
            ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
            ErrorCode::NotFound | ErrorCode::UnknownRoute => StatusCode::NOT_FOUND,
            ErrorCode::Auth
            | ErrorCode::Starting
            | ErrorCode::GithubUnavailable
            | ErrorCode::RateLimited => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Limit | ErrorCode::NoSession => StatusCode::CONFLICT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub until: Option<Time>,
}

/// API-3's body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub version: String,
    pub error: ApiError,
}

/// API-4's `daemon`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DaemonBlock {
    pub tier: Tier,
    pub last_poll_at: Option<Time>,
    pub problem: Option<ProblemCode>,
    pub message: Option<String>,
    pub until: Option<Time>,
}

/// Every successful body: `version`, `cursor`, `daemon`, then the route's keys (API-2, API-4, API-19).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub version: String,
    pub cursor: String,
    pub daemon: DaemonBlock,
    #[serde(flatten)]
    pub body: T,
}

// API-19: each route's keys in its row's order; a list or a flag is never left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListBody {
    pub records: Vec<Record>,
    pub untracked: Vec<PrName>,
    pub pending: Vec<PrName>,
    pub reset: bool,
}

/// show, track: always `Some`; untrack: may be `null`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordBody {
    pub record: Option<Record>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangesBody {
    pub events: Vec<Event>,
    pub records: Vec<Record>,
    pub pending: Vec<PrName>,
    pub more_events: bool,
    pub reset: bool,
}

/// API-11.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Satisfied,
    Ended,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Merged,
    Closed,
    Gone,
    HeadMoved,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitBody {
    pub outcome: Outcome,
    pub reason: Option<Reason>,
    pub record: Option<Record>,
    pub events: Vec<Event>,
    pub records: Vec<Record>,
    pub more_events: bool,
    pub reset: bool,
}

/// `POST …/notify` (NTF-2, API-19's row, in its order). Kept: `registered: true`, `expires_at`, `outcome`
/// and `reason` `null`, the record, the events since `since` (none without it). Already known:
/// `registered: false`, `expires_at: null`, and the rest as the `wait` it stands for. `replaced` either way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotifyBody {
    pub registered: bool,
    pub replaced: bool,
    pub expires_at: Option<Time>,
    pub outcome: Option<Outcome>,
    pub reason: Option<Reason>,
    pub record: Option<Record>,
    pub events: Vec<Event>,
    pub records: Vec<Record>,
    pub more_events: bool,
    pub reset: bool,
}

impl NotifyBody {
    /// The `wait` answer it stands for; `None` when `registered`, and when it has no `outcome`.
    pub fn as_wait(&self) -> Option<WaitBody> {
        if self.registered {
            return None;
        }
        Some(WaitBody {
            outcome: self.outcome?,
            reason: self.reason,
            record: self.record.clone(),
            events: self.events.clone(),
            records: self.records.clone(),
            more_events: self.more_events,
            reset: self.reset,
        })
    }
}

/// `DELETE …/notify` (NTF-3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CancelBody {
    pub cancelled: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefreshBody {
    pub outcome: PollOutcome,
    pub cost: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShutdownBody {}

/// API-5, in its order. `cursor` and `daemon` come from the envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusBody {
    /// `spawned`, `foreground`.
    pub mode: Mode,
    pub started_at: Time,
    pub last_poll: Option<LastPoll>,
    pub cost_last_hour: u32,
    pub github: GithubInfo,
    pub tracked: TrackedCounts,
    pub waiters: u32,
    /// NTF-9: open ones; `View::subscriptions`.
    pub subscriptions: u32,
    pub owners: Vec<String>,
    /// `gh`, `GH_TOKEN`, `GITHUB_TOKEN`.
    pub token_source: String,
    /// API-5: the `gh` that was run last, or as configured before any run.
    pub gh_path: String,
    pub paths: PathsInfo,
    pub warnings: Vec<Warning>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastPoll {
    pub at: Time,
    pub outcome: PollOutcome,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GithubInfo {
    pub remaining: Option<u32>,
    pub reset_at: Option<Time>,
    pub login: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackedCounts {
    pub authored: u32,
    pub manual: u32,
    pub asked: u32,
}

/// `state`: state.json.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathsInfo {
    pub config: String,
    pub state: String,
    pub socket: String,
    pub log: String,
}

/// What any CLI reads of any daemon's status (API-2): only these two keys, everything else ignored.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct StatusLite {
    pub version: String,
    pub mode: String,
}

/// `merged`, `checks-done`, `changed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Until {
    Merged,
    ChecksDone,
    Changed,
}

impl Until {
    pub fn as_str(self) -> &'static str {
        match self {
            Until::Merged => "merged",
            Until::ChecksDone => "checks-done",
            Until::Changed => "changed",
        }
    }

    /// The three words exactly; anything else is `None`.
    pub fn parse(s: &str) -> Option<Until> {
        match s {
            "merged" => Some(Until::Merged),
            "checks-done" => Some(Until::ChecksDone),
            "changed" => Some(Until::Changed),
            _ => None,
        }
    }
}

/// A condition in the client's own words, unchecked (API-10, D8): the CLI's flags, a `wait`'s query and a
/// `notify`'s body all carry these six, and `again:` and `Ask again:` print them back (CLI-6, NTF-8).
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub until: Until,
    pub sha: Option<String>,
    pub rerun_since: Option<String>,
    /// `--fail-fast`, `fail_fast` (D8).
    pub fail_fast: bool,
    /// `--kind`, `kind`: the comma-separated list as given (D8).
    pub kind: Option<String>,
    pub since: Option<String>,
}

impl Ask {
    /// Everything else unset.
    pub fn new(until: Until) -> Ask {
        Ask {
            until,
            sha: None,
            rerun_since: None,
            fail_fast: false,
            kind: None,
            since: None,
        }
    }
}

/// Every request the CLI sends. The one place paths, parameter names and body keys are spelt.
/// Its methods are T43's (`request.rs`).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    List {
        all: bool,
        since: Option<String>,
        checks: bool,
    },
    Show {
        pr: PrName,
        fresh: bool,
    },
    Changes {
        pr: Option<PrName>,
        since: Option<String>,
        limit: Option<u32>,
    },
    Wait {
        pr: PrName,
        cond: Ask,
        timeout: u32,
        limit: Option<u32>,
    },
    /// NTF-2. `life` is `for`, sent only when `--for` was given; `token` is sent as a string or `null`.
    Notify {
        pr: PrName,
        cond: Ask,
        life: Option<u32>,
        socket: String,
        token: Option<String>,
        limit: Option<u32>,
    },
    /// NTF-3.
    NotifyCancel {
        pr: PrName,
        socket: String,
    },
    Track {
        pr: PrName,
    },
    Untrack {
        pr: PrName,
    },
    Refresh,
    Status,
    Shutdown,
}

/// API-9; header names compare without case.
pub const CURSOR_HEADER: &str = "circitor-cursor";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::{every_kind, sample_event, sample_record};
    use crate::status::WarningCode;
    use serde::de::DeserializeOwned;

    /// The top-level keys of a JSON object, in the order written.
    fn keys(json: &str) -> Vec<String> {
        struct Keys(Vec<String>);
        impl<'de> Deserialize<'de> for Keys {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Keys, D::Error> {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = Keys;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("an object")
                    }
                    fn visit_map<A: serde::de::MapAccess<'de>>(
                        self,
                        mut m: A,
                    ) -> Result<Keys, A::Error> {
                        let mut keys = Vec::new();
                        while let Some(k) = m.next_key::<String>()? {
                            m.next_value::<serde::de::IgnoredAny>()?;
                            keys.push(k);
                        }
                        Ok(Keys(keys))
                    }
                }
                d.deserialize_map(V)
            }
        }
        serde_json::from_str::<Keys>(json).unwrap().0
    }

    fn time(s: &str) -> Time {
        Time::parse(s).unwrap()
    }

    fn daemon() -> DaemonBlock {
        DaemonBlock {
            tier: Tier::Active,
            last_poll_at: None,
            problem: None,
            message: None,
            until: None,
        }
    }

    fn full_daemon() -> DaemonBlock {
        DaemonBlock {
            tier: Tier::Idle,
            last_poll_at: Some(time("2026-11-24T22:00:00Z")),
            problem: Some(ProblemCode::RateLimited),
            message: Some("asks again at 22:30".into()),
            until: Some(time("2026-11-24T22:30:00Z")),
        }
    }

    fn envelope<T>(body: T, daemon: DaemonBlock) -> Envelope<T> {
        Envelope {
            version: "1.2.3+test".into(),
            cursor: "k3x9a2bq-42".into(),
            daemon,
            body,
        }
    }

    fn empty_status() -> StatusBody {
        StatusBody {
            mode: Mode::Spawned,
            started_at: time("2026-11-24T22:00:00Z"),
            last_poll: None,
            cost_last_hour: 0,
            github: GithubInfo {
                remaining: None,
                reset_at: None,
                login: None,
            },
            tracked: TrackedCounts {
                authored: 0,
                manual: 0,
                asked: 0,
            },
            waiters: 0,
            subscriptions: 0,
            owners: Vec::new(),
            token_source: "gh".into(),
            gh_path: "gh".into(),
            paths: PathsInfo {
                config: "c".into(),
                state: "s".into(),
                socket: "k".into(),
                log: "l".into(),
            },
            warnings: Vec::new(),
        }
    }

    fn head() -> Vec<&'static str> {
        vec!["version", "cursor", "daemon"]
    }

    fn check_keys<T: Serialize>(body: T, row: &[&str]) {
        let json = serde_json::to_string(&envelope(body, daemon())).unwrap();
        let mut want = head();
        want.extend_from_slice(row);
        assert_eq!(keys(&json), want, "{json}");
        assert!(!json.contains('\n'));
    }

    // API-19: every list and flag is present, in the row's order.
    #[test]
    fn api19_every_key_is_present_when_empty() {
        check_keys(
            ListBody {
                records: vec![],
                untracked: vec![],
                pending: vec![],
                reset: false,
            },
            &["records", "untracked", "pending", "reset"],
        );
        check_keys(RecordBody { record: None }, &["record"]);
        check_keys(
            ChangesBody {
                events: vec![],
                records: vec![],
                pending: vec![],
                more_events: false,
                reset: false,
            },
            &["events", "records", "pending", "more_events", "reset"],
        );
        check_keys(
            WaitBody {
                outcome: Outcome::Timeout,
                reason: None,
                record: None,
                events: vec![],
                records: vec![],
                more_events: false,
                reset: false,
            },
            &[
                "outcome",
                "reason",
                "record",
                "events",
                "records",
                "more_events",
                "reset",
            ],
        );
        check_keys(
            NotifyBody {
                registered: false,
                replaced: false,
                expires_at: None,
                outcome: None,
                reason: None,
                record: None,
                events: vec![],
                records: vec![],
                more_events: false,
                reset: false,
            },
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
        );
        check_keys(CancelBody { cancelled: 0 }, &["cancelled"]);
        check_keys(
            RefreshBody {
                outcome: PollOutcome::Polled,
                cost: 0,
            },
            &["outcome", "cost"],
        );
        check_keys(
            empty_status(),
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
        );
        check_keys(ShutdownBody {}, &[]);

        // The envelope's daemon block, the record and the error body keep every key too.
        let json = serde_json::to_string(&daemon()).unwrap();
        assert_eq!(
            keys(&json),
            ["tier", "last_poll_at", "problem", "message", "until"]
        );
        let json = serde_json::to_string(&ErrorBody {
            version: "v".into(),
            error: ApiError {
                code: ErrorCode::Starting,
                message: "m".into(),
                until: None,
            },
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"version":"v","error":{"code":"starting","message":"m","until":null}}"#
        );

        // `checks.items` and an item's review-only keys are the only keys ever left out.
        let mut record = sample_record();
        record.checks.items = None;
        record.author = None;
        record.error = None;
        let json = serde_json::to_value(&record).unwrap();
        let checks = json["checks"].as_object().unwrap();
        assert!(!checks.contains_key("items"));
        assert_eq!(checks.len(), 7);
        assert_eq!(json.as_object().unwrap().len(), 24);
        assert!(json["author"].is_null() && json["error"].is_null());
        let EventKind::Activity { items, .. } = &every_kind()[10] else {
            panic!()
        };
        let mut item = items[0].clone();
        item.review_state = None;
        item.inline = None;
        item.fetch_inline = None;
        item.author = None;
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 5);
        assert!(json["author"].is_null());
    }

    use crate::model::EventKind;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
        let json = serde_json::to_string(value).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, value, "{json}");
        // An unknown key is ignored (CLI-9).
        let mut obj: serde_json::Value = serde_json::from_str(&json).unwrap();
        obj.as_object_mut()
            .unwrap()
            .insert("from_a_newer_daemon".into(), serde_json::json!({"x": [1]}));
        let back: T = serde_json::from_value(obj).unwrap();
        assert_eq!(&back, value);
    }

    // API-19: every body decodes back, and ignores a key it doesn't know.
    #[test]
    fn api19_bodies_survive_a_round_trip() {
        let record = sample_record();
        let pr = record.pr.clone();
        let events: Vec<Event> = every_kind()
            .into_iter()
            .enumerate()
            .map(|(i, kind)| {
                let mut e = sample_event(i as u64 + 1);
                e.kinds = vec![kind];
                e
            })
            .collect();
        let d = full_daemon;

        round_trip(&envelope(
            ListBody {
                records: vec![record.clone()],
                untracked: vec![pr.clone()],
                pending: vec![pr.clone()],
                reset: true,
            },
            d(),
        ));
        round_trip(&envelope(
            RecordBody {
                record: Some(record.clone()),
            },
            d(),
        ));
        round_trip(&envelope(
            ChangesBody {
                events: events.clone(),
                records: vec![record.clone()],
                pending: vec![pr.clone()],
                more_events: true,
                reset: true,
            },
            d(),
        ));
        round_trip(&envelope(
            WaitBody {
                outcome: Outcome::Ended,
                reason: Some(Reason::HeadMoved),
                record: Some(record.clone()),
                events: events.clone(),
                records: vec![record.clone()],
                more_events: true,
                reset: true,
            },
            d(),
        ));
        round_trip(&envelope(
            NotifyBody {
                registered: false,
                replaced: true,
                expires_at: Some(time("2026-11-25T22:00:00Z")),
                outcome: Some(Outcome::Satisfied),
                reason: Some(Reason::Merged),
                record: Some(record.clone()),
                events: events.clone(),
                records: vec![record.clone()],
                more_events: true,
                reset: true,
            },
            d(),
        ));
        round_trip(&envelope(CancelBody { cancelled: 2 }, d()));
        round_trip(&envelope(
            RefreshBody {
                outcome: PollOutcome::RateLimited,
                cost: 3,
            },
            d(),
        ));
        round_trip(&envelope(ShutdownBody {}, d()));
        round_trip(&envelope(
            StatusBody {
                mode: Mode::Foreground,
                started_at: time("2026-11-24T22:00:00Z"),
                last_poll: Some(LastPoll {
                    at: time("2026-11-24T22:05:00Z"),
                    outcome: PollOutcome::Auth,
                }),
                cost_last_hour: 12,
                github: GithubInfo {
                    remaining: Some(4_990),
                    reset_at: Some(time("2026-11-24T23:00:00Z")),
                    login: Some("someone".into()),
                },
                tracked: TrackedCounts {
                    authored: 3,
                    manual: 2,
                    asked: 1,
                },
                waiters: 4,
                subscriptions: 5,
                owners: vec!["yachiko".into()],
                token_source: "GH_TOKEN".into(),
                gh_path: "/usr/bin/gh".into(),
                paths: PathsInfo {
                    config: "/c/config.toml".into(),
                    state: "/s/state.json".into(),
                    socket: "/r/circitor.sock".into(),
                    log: "/s/circitor.log".into(),
                },
                warnings: vec![
                    Warning {
                        code: WarningCode::PrFailing,
                        message: "o/r#1".into(),
                        since: time("2026-11-24T22:01:00Z"),
                    },
                    Warning {
                        code: WarningCode::LogUnwritable,
                        message: "m".into(),
                        since: time("2026-11-24T22:02:00Z"),
                    },
                ],
            },
            d(),
        ));
        for code in every_code() {
            round_trip(&ErrorBody {
                version: "1.2.3+test".into(),
                error: ApiError {
                    code,
                    message: "a message".into(),
                    until: Some(time("2026-11-24T22:30:00Z")),
                },
            });
        }

        // `Event.id` is never serialised: a filled one comes back empty.
        let mut e = sample_event(1);
        e.id = crate::model::NodeId("PR_kwDO".into());
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("PR_kwDO"));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, crate::model::NodeId::default());

        // `StatusLite` reads only its two keys from a whole status.
        let json = serde_json::to_string(&envelope(empty_status(), daemon())).unwrap();
        let lite: StatusLite = serde_json::from_str(&json).unwrap();
        assert_eq!(
            lite,
            StatusLite {
                version: "1.2.3+test".into(),
                mode: "spawned".into()
            }
        );
    }

    fn every_code() -> [ErrorCode; 9] {
        [
            ErrorCode::BadRequest,
            ErrorCode::NotFound,
            ErrorCode::UnknownRoute,
            ErrorCode::Auth,
            ErrorCode::Starting,
            ErrorCode::GithubUnavailable,
            ErrorCode::RateLimited,
            ErrorCode::Limit,
            ErrorCode::NoSession,
        ]
    }

    // API-3: each code's HTTP status.
    #[test]
    fn api3_http_status_per_code() {
        let statuses: Vec<u16> = every_code().iter().map(|c| c.http().as_u16()).collect();
        assert_eq!(statuses, [400, 404, 404, 503, 503, 503, 503, 409, 409]);
        let words: Vec<String> = every_code()
            .iter()
            .map(|c| serde_json::to_string(c).unwrap())
            .collect();
        assert_eq!(words[8], "\"no_session\"");
        assert_eq!(words[5], "\"github_unavailable\"");
    }

    // NTF-2: the notify answer's keys and the wait it stands for.
    #[test]
    fn ntf2_notify_body() {
        let record = sample_record();
        let registered = NotifyBody {
            registered: true,
            replaced: false,
            expires_at: Some(time("2026-11-25T22:00:00Z")),
            outcome: None,
            reason: None,
            record: Some(record.clone()),
            events: vec![],
            records: vec![],
            more_events: false,
            reset: false,
        };
        let json = serde_json::to_string(&registered).unwrap();
        assert_eq!(
            keys(&json),
            [
                "registered",
                "replaced",
                "expires_at",
                "outcome",
                "reason",
                "record",
                "events",
                "records",
                "more_events",
                "reset"
            ]
        );
        assert!(json.starts_with(r#"{"registered":true,"replaced":false,"expires_at":"2026-11-25T22:00:00Z","outcome":null,"reason":null,"#));
        assert_eq!(registered.as_wait(), None);

        let known = NotifyBody {
            registered: false,
            replaced: true,
            expires_at: None,
            outcome: Some(Outcome::Satisfied),
            reason: None,
            record: Some(record.clone()),
            events: vec![sample_event(3)],
            records: vec![],
            more_events: true,
            reset: false,
        };
        let json = serde_json::to_string(&known).unwrap();
        assert!(json.starts_with(r#"{"registered":false,"replaced":true,"expires_at":null,"outcome":"satisfied","reason":null,"#));
        assert_eq!(
            known.as_wait(),
            Some(WaitBody {
                outcome: Outcome::Satisfied,
                reason: None,
                record: Some(record),
                events: vec![sample_event(3)],
                records: vec![],
                more_events: true,
                reset: false,
            })
        );
        assert_eq!(
            serde_json::to_string(&CancelBody { cancelled: 2 }).unwrap(),
            r#"{"cancelled":2}"#
        );
    }

    #[test]
    fn until_words() {
        for (word, until) in [
            ("merged", Until::Merged),
            ("checks-done", Until::ChecksDone),
            ("changed", Until::Changed),
        ] {
            assert_eq!(until.as_str(), word);
            assert_eq!(Until::parse(word), Some(until));
        }
        assert_eq!(Until::parse("checks_done"), None);
        assert_eq!(Until::parse("Merged"), None);
        assert_eq!(Until::parse(""), None);
        assert_eq!(
            Ask::new(Until::Merged),
            Ask {
                until: Until::Merged,
                sha: None,
                rerun_since: None,
                fail_fast: false,
                kind: None,
                since: None
            }
        );
    }
}
