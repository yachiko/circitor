//! `FakeTokenSource` (T43).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::clock::SharedClock;
use crate::testkit::MARKER_TOKEN;

use super::{Token, TokenRead, TokenSource, TokenSourceKind};

struct Settings {
    read: TokenRead,
    delay: Option<Duration>,
    kind: TokenSourceKind,
    gh_path: String,
}

/// Its default read is `Token(MARKER_TOKEN)`; `kind()` is `Gh` and `gh_path()` is `gh` unless set.
pub struct FakeTokenSource {
    clock: SharedClock,
    settings: Mutex<Settings>,
    reads: AtomicUsize,
}

impl FakeTokenSource {
    pub fn new(clock: SharedClock) -> Arc<FakeTokenSource> {
        Arc::new(FakeTokenSource {
            clock,
            settings: Mutex::new(Settings {
                read: TokenRead::Token(Token::new(MARKER_TOKEN.to_string())),
                delay: None,
                kind: TokenSourceKind::Gh,
                gh_path: "gh".to_string(),
            }),
            reads: AtomicUsize::new(0),
        })
    }

    fn settings(&self) -> std::sync::MutexGuard<'_, Settings> {
        // Taken and released inside one function; a poisoned lock still holds its value.
        self.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What every read gives from now on.
    pub fn set(&self, read: TokenRead) {
        self.settings().read = read;
    }

    /// Each read takes `delay` of the clock.
    pub fn set_delay(&self, delay: Duration) {
        self.settings().delay = Some(delay);
    }

    pub fn set_kind(&self, kind: TokenSourceKind) {
        self.settings().kind = kind;
    }

    pub fn set_gh_path(&self, gh_path: &str) {
        self.settings().gh_path = gh_path.to_string();
    }

    /// How many reads have begun.
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl TokenSource for FakeTokenSource {
    async fn read(&self) -> TokenRead {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let delay = self.settings().delay;
        if let Some(delay) = delay {
            // On the clock, and holding no `Busy` (clock.md): a slow read isn't work in flight.
            crate::clock::sleep(&self.clock, delay).await;
        }
        // What is set when the read ends.
        self.settings().read.clone()
    }

    fn kind(&self) -> TokenSourceKind {
        self.settings().kind
    }

    fn gh_path(&self) -> String {
        self.settings().gh_path.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, Mono, Time};
    use futures::FutureExt;
    use futures::future::BoxFuture;
    use std::sync::atomic::AtomicBool;

    /// A clock whose timers fire when `fire` is set.
    struct Manual {
        fire: AtomicBool,
    }

    impl Clock for Manual {
        fn wall(&self) -> Time {
            Time::from_unix_millis(0)
        }
        fn mono(&self) -> Mono {
            Mono::ZERO
        }
        fn sleep_until(&self, _at: Mono) -> BoxFuture<'static, ()> {
            if self.fire.load(Ordering::SeqCst) {
                futures::future::ready(()).boxed()
            } else {
                futures::future::pending().boxed()
            }
        }
    }

    #[tokio::test]
    async fn fake_token_source() {
        let manual = Arc::new(Manual {
            fire: AtomicBool::new(false),
        });
        let source = FakeTokenSource::new(manual.clone());
        assert_eq!(source.kind(), TokenSourceKind::Gh);
        assert_eq!(source.gh_path(), "gh");
        match source.read().await {
            TokenRead::Token(t) => assert_eq!(t.expose(), MARKER_TOKEN),
            other => panic!("{other:?}"),
        }
        source.set(TokenRead::NoLogin);
        assert!(matches!(source.read().await, TokenRead::NoLogin));
        assert_eq!(source.reads(), 2);
        source.set_kind(TokenSourceKind::GhTokenVar);
        source.set_gh_path("/x/gh");
        assert_eq!(source.kind(), TokenSourceKind::GhTokenVar);
        assert_eq!(source.gh_path(), "/x/gh");

        // A delay waits on the clock's timer.
        source.set_delay(Duration::from_secs(5));
        let mut slow = Box::pin(source.read());
        assert!((&mut slow).now_or_never().is_none());
        assert_eq!(source.reads(), 3);
        drop(slow);
        manual.fire.store(true, Ordering::SeqCst);
        assert!(matches!(source.read().await, TokenRead::NoLogin));
        assert_eq!(source.reads(), 4);
    }
}
