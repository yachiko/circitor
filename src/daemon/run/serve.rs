//! Serving the socket: the accept loop and one task per connection (API-1, API-14, OPS-7).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use tokio::net::UnixStream;
use tokio::task::AbortHandle;

use super::lock;
use crate::clock::{Activity, SharedClock, sleep};
use crate::daemon::instance::{Bound, RETRY_EVERY};

#[derive(Default)]
struct Inner {
    next: u64,
    open: HashMap<u64, AbortHandle>,
    /// OPS-7: once closed, nothing new is served.
    closed: bool,
}

/// The open connections, kept so a stop can close them without an answer (OPS-7, API-14).
#[derive(Default)]
pub(super) struct Conns {
    inner: Mutex<Inner>,
}

/// Takes its connection out of `Conns` when the connection's task ends, however it ends.
struct Gone {
    conns: Arc<Conns>,
    id: u64,
}

impl Drop for Gone {
    fn drop(&mut self) {
        lock(&self.conns.inner).open.remove(&self.id);
    }
}

impl Conns {
    pub(super) fn open(&self) -> usize {
        lock(&self.inner).open.len()
    }

    /// One task for one connection: `api::router` behind hyper-util's builder, HTTP/1 only.
    fn serve(self: &Arc<Self>, stream: UnixStream, router: axum::Router) {
        // The lock is held while the task is spawned and entered: its `Gone` waits for it, so the entry
        // is always in before it can be taken out.
        let mut inner = lock(&self.inner);
        if inner.closed {
            return;
        }
        let id = inner.next;
        inner.next += 1;
        let gone = Gone {
            conns: self.clone(),
            id,
        };
        let task = tokio::spawn(async move {
            let _gone = gone;
            // API-14: no half-close. A client that closes its end ends the connection, which drops the
            // handler's future or its body: that's where a waiter is removed (T23).
            let builder = auto::Builder::new(TokioExecutor::new()).http1_only();
            let _ = builder
                .serve_connection(TokioIo::new(stream), TowerToHyperService::new(router))
                .await;
        });
        inner.open.insert(id, task.abort_handle());
    }

    /// OPS-7, API-14: every open connection, a waiter's or another, is closed without an answer, and
    /// nothing new is served.
    ///
    /// API-17: an answer already handed to the socket isn't cut. `shutdown`'s body asks for the stop when
    /// hyper drops it, in the same poll that then flushes the answer; an abort takes effect only at the
    /// task's next poll, and the stop gets here only after the engine has stopped.
    pub(super) fn abort_all(&self) {
        let open = {
            let mut inner = lock(&self.inner);
            inner.closed = true;
            std::mem::take(&mut inner.open)
        };
        for handle in open.into_values() {
            handle.abort();
        }
    }
}

/// The accept loop. A stop ends it by aborting its task (OPS-7: stop accepting).
pub(super) async fn accept_loop(
    bound: Arc<Bound>,
    router: axum::Router,
    conns: Arc<Conns>,
    activity: Activity,
    clock: SharedClock,
) {
    loop {
        let accepted = bound.listener.accept().await;
        // TST-4: work from waking until it parks again.
        let busy = activity.begin();
        match accepted {
            Ok((stream, _)) => conns.serve(stream, router.clone()),
            Err(_) => {
                // Out of descriptors and its like: try again shortly rather than spin.
                drop(busy);
                sleep(&clock, RETRY_EVERY).await;
            }
        }
    }
}
