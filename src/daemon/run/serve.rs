//! Serving the socket: the accept loop and one task per connection (API-1, API-14, OPS-7).

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UnixStream;
use tokio::task::AbortHandle;

use super::lock;
use crate::clock::{Activity, Busy, SharedClock, sleep};
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

/// A connection's stream, with its answer counted as work in flight while it waits for room.
///
/// TST-4, T62: a write that the socket can't take yet parks the connection's task until the client
/// reads. Nothing else is busy then: the client waits on its read, and a large answer (200 PRs) on
/// macOS's small Unix-socket buffer let the test clock settle half-way through it. Reading isn't
/// counted: a connection waiting for a request, or a waiter's answer, isn't work in flight.
struct Writing {
    stream: UnixStream,
    activity: Activity,
    busy: Option<Busy>,
}

impl Writing {
    fn track<T>(&mut self, polled: Poll<T>) -> Poll<T> {
        match polled {
            Poll::Pending => {
                if self.busy.is_none() {
                    self.busy = Some(self.activity.begin());
                }
            }
            Poll::Ready(_) => self.busy = None,
        }
        polled
    }
}

impl AsyncRead for Writing {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for Writing {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write(cx, buf);
        self.track(polled)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write_vectored(cx, bufs);
        self.track(polled)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.stream).poll_flush(cx);
        self.track(polled)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.stream).poll_shutdown(cx);
        self.track(polled)
    }
}

impl Conns {
    pub(super) fn open(&self) -> usize {
        lock(&self.inner).open.len()
    }

    /// One task for one connection: `api::router` behind hyper-util's builder, HTTP/1 only.
    fn serve(self: &Arc<Self>, stream: UnixStream, router: axum::Router, activity: Activity) {
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
            let io = Writing {
                stream,
                activity,
                busy: None,
            };
            let _ = builder
                .serve_connection(TokioIo::new(io), TowerToHyperService::new(router))
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
            Ok((stream, _)) => conns.serve(stream, router.clone(), activity.clone()),
            Err(_) => {
                // Out of descriptors and its like: try again shortly rather than spin.
                drop(busy);
                sleep(&clock, RETRY_EVERY).await;
            }
        }
    }
}
