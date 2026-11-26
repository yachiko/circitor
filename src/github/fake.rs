//! `ScriptedGitHub` and `answers` (T43).

use std::sync::{Arc, Mutex};

use crate::clock::Activity;
use crate::token::Token;

use super::{Answer, Failure, GitHub, Query};

type Handler = Box<dyn FnMut(&Query) -> Result<Answer, Failure> + Send + 'static>;

/// What `hold` leaves for the next `send`: the activity to excuse it from, and the release to wait for.
struct Hold {
    activity: Activity,
    released: tokio::sync::oneshot::Receiver<()>,
}

/// A `GitHub` that answers by script. It doesn't check queries against a schema, compute costs,
/// or enforce POL-2: the fake GitHub server (T14, T20) does.
pub struct ScriptedGitHub {
    handler: Mutex<Handler>,
    calls: Mutex<Vec<Query>>,
    hold: Mutex<Option<Hold>>,
}

impl ScriptedGitHub {
    pub fn new(
        handler: impl FnMut(&Query) -> Result<Answer, Failure> + Send + 'static,
    ) -> Arc<ScriptedGitHub> {
        Arc::new(ScriptedGitHub {
            handler: Mutex::new(Box::new(handler)),
            calls: Mutex::new(Vec::new()),
            hold: Mutex::new(None),
        })
    }

    /// Every query sent, in order.
    pub fn calls(&self) -> Vec<Query> {
        lock(&self.calls).clone()
    }

    /// The next `send` waits until the returned `Release` is dropped, and is excused from the work in
    /// flight meanwhile (`clock.md`).
    pub fn hold(&self, activity: &Activity) -> Release {
        let (release, released) = tokio::sync::oneshot::channel();
        *lock(&self.hold) = Some(Hold {
            activity: activity.clone(),
            released,
        });
        Release {
            inner: Some(release),
        }
    }
}

/// A std lock, taken and released inside one function; a poisoned one still holds its value.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[async_trait::async_trait]
impl GitHub for ScriptedGitHub {
    async fn send(&self, _token: &Token, query: &Query) -> Result<Answer, Failure> {
        lock(&self.calls).push(query.clone());
        let hold = lock(&self.hold).take();
        if let Some(hold) = hold {
            // The excuse lives in this future: a dropped request stops excusing.
            let _excused = hold.activity.excuse();
            // A dropped `Release` is the signal; so is a sender that's gone.
            let _ = hold.released.await;
        }
        (lock(&self.handler))(query)
    }
}

/// Dropping it lets the held `send` answer.
pub struct Release {
    inner: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Release {
    fn drop(&mut self) {
        drop(self.inner.take());
    }
}

/// Helpers that build plain values for tests.
pub mod answers {
    use crate::clock::Time;
    use crate::github::{Answer, NodeAnswer, Rate};
    use crate::model::{ChecksRead, Counts, Mergeable, NodeId, PrName, PrRead, ReadState, Review};

    /// The fixed time every value counts from: TST-5's 2026-11-24T22:00:00Z.
    fn t0() -> Time {
        Time::parse("2026-11-24T22:00:00Z").expect("a fixed, valid time")
    }

    /// A plain open PR read, named `name` (`"o/r#1"`). Its node ID is made from the name, so two calls agree.
    pub fn pr_read(name: &str) -> PrRead {
        // A fake's input: a test that passes a bad name is wrong at once.
        let pr = PrName::parse(name).expect("a PR name such as o/r#1");
        let url = format!(
            "https://github.com/{}/{}/pull/{}",
            pr.owner, pr.repo, pr.number
        );
        PrRead {
            id: NodeId(format!("PR_{}", name.to_ascii_lowercase())),
            pr,
            url,
            title: "A pull request".to_string(),
            author: Some("someone".to_string()),
            state: ReadState::Open,
            draft: false,
            base: "main".to_string(),
            head: "feature".to_string(),
            head_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
            created_at: Some(t0()),
            updated_at: Some(t0()),
            merged_at: None,
            closed_at: None,
            mergeable: Mergeable::Yes,
            merge_state: "clean".to_string(),
            review: Review::None,
            counts: Counts::default(),
            checks: ChecksRead {
                items: Vec::new(),
                total: 0,
                rollup: None,
                next_page: None,
            },
        }
    }

    /// An answer holding `nodes`, with a plain rate block.
    pub fn answer_with(nodes: Vec<NodeAnswer>) -> Answer {
        Answer {
            rate: Some(rate(1, 4_999)),
            date: Some(t0()),
            found: None,
            nodes,
            lookup: None,
        }
    }

    /// A rate block, resetting an hour on.
    pub fn rate(cost: u32, remaining: u32) -> Rate {
        Rate {
            cost,
            remaining,
            reset_at: Some(t0().plus(std::time::Duration::from_secs(3_600))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Find, NodeAnswer};
    use crate::testkit::MARKER_TOKEN;
    use futures::FutureExt;

    // Held sends, answers and calls.
    #[tokio::test]
    async fn scripted_github_answers_and_holds() {
        let token = Token::new(MARKER_TOKEN.to_string());
        let mut n = 0u32;
        let github = ScriptedGitHub::new(move |_q| {
            n += 1;
            Ok(answers::answer_with(vec![NodeAnswer::Error {
                type_: format!("N{n}"),
            }]))
        });
        let first = Query::default();
        let second = Query {
            find: Some(Find { after: None }),
            ..Query::default()
        };
        let a = github.send(&token, &first).await.unwrap();
        github.send(&token, &second).await.unwrap();
        assert_eq!(github.calls(), vec![first.clone(), second]);
        assert_eq!(a.nodes, vec![NodeAnswer::Error { type_: "N1".into() }]);

        // Under a hold: pending, excused, then released.
        let activity = Activity::default();
        let _busy = activity.begin();
        let release = github.hold(&activity);
        let mut send = Box::pin(github.send(&token, &first));
        assert!((&mut send).now_or_never().is_none());
        assert_eq!(activity.in_flight(), 0);
        drop(release);
        let answer = send.await.unwrap();
        assert_eq!(activity.in_flight(), 1);
        assert_eq!(answer.nodes, vec![NodeAnswer::Error { type_: "N3".into() }]);
        // The send after it isn't held.
        assert!(github.send(&token, &first).now_or_never().is_some());

        // A held send dropped early leaves nothing excused.
        let release = github.hold(&activity);
        let mut send = Box::pin(github.send(&token, &first));
        assert!((&mut send).now_or_never().is_none());
        assert_eq!(activity.in_flight(), 0);
        drop(send);
        assert_eq!(activity.in_flight(), 1);
        drop(release);

        assert_eq!(answers::pr_read("o/r#1"), answers::pr_read("o/r#1"));
        assert_ne!(answers::pr_read("o/r#1").id, answers::pr_read("o/r#2").id);
        let r = answers::rate(3, 40);
        assert_eq!((r.cost, r.remaining), (3, 40));
    }
}
