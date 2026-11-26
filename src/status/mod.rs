//! The `Warnings` registry behind `status`'s `warnings` (API-5; `plan/contracts/log.md`).
//! T02 declared; T04 built.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::clock::{SharedClock, Time};
use crate::log::Logger;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    QuotaCapped,
    QuotaReserve,
    PrLimit,
    PrFailing,
    StateFileSetAside,
    StateFileNewer,
    StateFileUnwritable,
    /// OPS-9, API-5: a log line couldn't be written.
    LogUnwritable,
}

impl WarningCode {
    /// Its name in JSON and in a log line.
    fn name(self) -> &'static str {
        match self {
            WarningCode::QuotaCapped => "quota_capped",
            WarningCode::QuotaReserve => "quota_reserve",
            WarningCode::PrLimit => "pr_limit",
            WarningCode::PrFailing => "pr_failing",
            WarningCode::StateFileSetAside => "state_file_set_aside",
            WarningCode::StateFileNewer => "state_file_newer",
            WarningCode::StateFileUnwritable => "state_file_unwritable",
            WarningCode::LogUnwritable => "log_unwritable",
        }
    }

    /// Its place in the enum, for `list`'s order.
    fn rank(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub code: WarningCode,
    pub message: String,
    pub since: Time,
}

struct Raised {
    message: String,
    since: Time,
}

#[derive(Clone)]
pub struct Warnings {
    clock: SharedClock,
    logger: Logger,
    raised: Arc<Mutex<HashMap<(WarningCode, String), Raised>>>,
}

impl Warnings {
    pub fn new(clock: SharedClock, logger: Logger) -> Warnings {
        Warnings {
            clock,
            logger,
            raised: Arc::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<(WarningCode, String), Raised>> {
        self.raised.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Raises `code` for `key`, or keeps it: `since` stays that of the first raise, `message` is replaced.
    /// `key` tells apart several warnings of one code (`pr_failing`: the PR's node ID); "" otherwise.
    pub fn raise(&self, code: WarningCode, key: &str, message: String) {
        let first = {
            let mut raised = self.lock();
            match raised.get_mut(&(code, key.to_string())) {
                Some(r) => {
                    r.message = message;
                    false
                }
                None => {
                    let since = self.clock.wall();
                    raised.insert((code, key.to_string()), Raised { message, since });
                    true
                }
            }
        };
        // OPS-9: every warning, once when raised. The key (a node ID for `pr_failing`) is never logged.
        if first {
            self.logger
                .warn(format_args!("warning raised code={}", code.name()));
        }
    }

    /// Nothing when it isn't raised.
    pub fn clear(&self, code: WarningCode, key: &str) {
        let was = self.lock().remove(&(code, key.to_string())).is_some();
        // OPS-9: and once when cleared.
        if was {
            self.logger
                .info(format_args!("warning cleared code={}", code.name()));
        }
    }

    /// By `since`, then code, then key.
    pub fn list(&self) -> Vec<Warning> {
        let raised = self.lock();
        let mut all: Vec<(&(WarningCode, String), &Raised)> = raised.iter().collect();
        all.sort_by(|((ca, ka), ra), ((cb, kb), rb)| {
            ra.since
                .cmp(&rb.since)
                .then(ca.rank().cmp(&cb.rank()))
                .then(ka.cmp(kb))
        });
        // The key is never in a `Warning` (MOD-1: a node ID is never shown).
        all.into_iter()
            .map(|((code, _), r)| Warning {
                code: *code,
                message: r.message.clone(),
                since: r.since,
            })
            .collect()
    }

    /// Since when `code` has been raised for `key`; `None` when it isn't (the notifier's `pr_failing`, NTF-4).
    pub fn since(&self, code: WarningCode, key: &str) -> Option<Time> {
        self.lock().get(&(code, key.to_string())).map(|r| r.since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, TestClock};
    use crate::log::{BufferSink, Level};
    use std::time::Duration;

    fn setup() -> (Arc<TestClock>, Warnings, BufferSink) {
        let clock = TestClock::new(Time::parse("2026-11-24T22:00:00Z").unwrap());
        let sink = BufferSink::default();
        let logger = Logger::new(clock.clone(), Arc::new(sink.clone()), Level::Debug);
        (clock.clone(), Warnings::new(clock, logger), sink)
    }

    const KEY: &str = "PR_kwDOMARKER123";

    // OPS-9: a warning logs once when raised and once when cleared; the key is never logged.
    #[tokio::test]
    async fn ops9_a_warning_logs_once_raised_and_once_cleared() {
        let (clock, w, sink) = setup();
        let code = WarningCode::PrFailing;
        assert_eq!(w.since(code, KEY), None);
        let first = clock.wall();
        w.raise(code, KEY, "one".into());
        clock.advance(Duration::from_secs(10)).await;
        w.raise(code, KEY, "two".into());
        clock.advance(Duration::from_secs(10)).await;
        w.raise(code, KEY, "three".into());
        assert_eq!(
            w.list(),
            vec![Warning {
                code,
                message: "three".into(),
                since: first
            }]
        );
        assert_eq!(w.since(code, KEY), Some(first));
        assert_eq!(w.since(code, "another"), None);
        assert_eq!(w.since(WarningCode::QuotaCapped, KEY), None);
        w.clear(code, KEY);
        w.clear(code, KEY);
        assert_eq!(w.since(code, KEY), None);
        assert!(w.list().is_empty());
        let text = sink.contents();
        assert_eq!(
            text,
            "2026-11-24T22:00:00Z warn warning raised code=pr_failing\n\
             2026-11-24T22:00:20Z info warning cleared code=pr_failing\n"
        );
        assert!(!text.contains(KEY));
        // Clearing what was never raised does nothing.
        w.clear(WarningCode::QuotaCapped, "");
        assert_eq!(sink.contents(), text);
    }

    // API-5: the list is ordered by `since`, then by the code's place, then by key.
    #[tokio::test]
    async fn status_list_order() {
        let (clock, w, _) = setup();
        let t0 = clock.wall();
        w.raise(WarningCode::PrFailing, "b", "pr b".into());
        w.raise(WarningCode::PrFailing, "a", "pr a".into());
        w.raise(WarningCode::LogUnwritable, "", "log".into());
        w.raise(WarningCode::QuotaReserve, "", "reserve".into());
        clock.advance(Duration::from_secs(1)).await;
        let t1 = clock.wall();
        w.raise(WarningCode::QuotaCapped, "", "capped".into());
        let got: Vec<(WarningCode, String, Time)> = w
            .list()
            .into_iter()
            .map(|x| (x.code, x.message, x.since))
            .collect();
        assert_eq!(
            got,
            vec![
                (WarningCode::QuotaReserve, "reserve".into(), t0),
                (WarningCode::PrFailing, "pr a".into(), t0),
                (WarningCode::PrFailing, "pr b".into(), t0),
                (WarningCode::LogUnwritable, "log".into(), t0),
                (WarningCode::QuotaCapped, "capped".into(), t1),
            ]
        );
    }

    #[test]
    fn code_names_match_json() {
        use WarningCode::*;
        for code in [
            QuotaCapped,
            QuotaReserve,
            PrLimit,
            PrFailing,
            StateFileSetAside,
            StateFileNewer,
            StateFileUnwritable,
            LogUnwritable,
        ] {
            assert_eq!(
                serde_json::to_string(&code).unwrap(),
                format!("\"{}\"", code.name())
            );
        }
    }
}
