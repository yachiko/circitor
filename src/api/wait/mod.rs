//! The `wait` route and its waiters (API-9 to API-14, U6; T23).
//!
//! U6: the one route that holds a request open. The handler runs API-9's arrival up to the waiter's
//! registration and returns the head at once, with `circitor-cursor`; the body is a stream that yields
//! once, when the wait ends. **The body owns the waiter:** its guards, its `InFlight` and its future, so
//! dropping it (the client went away) undoes everything in `Drop` (API-14).

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::get;

use crate::api::condition::{Condition, Progress, evaluate};
use crate::api::types::{
    ApiError, Ask, CURSOR_HEADER, ErrorCode, Outcome, Reason, Until, WaitBody,
};
use crate::api::{
    AppState, InFlight, Params, bad_request, envelope, error, pr_from_path, present, refusal,
    window,
};
use crate::clock::{Mono, SharedClock, SleepWatch};
use crate::model::{NodeId, PrName};
use crate::poller::PollerHandle;
use crate::state::store::{Class, Named, View, Waiting};
use crate::state::{Cursor, MAX_EVENTS, MAX_WAITERS};

/// API-12: the default `timeout`, in seconds. (proposal)
const DEFAULT_TIMEOUT_SECS: u32 = 100;
/// API-12: the longest `timeout`, in seconds. (proposal)
const MAX_TIMEOUT_SECS: u32 = 3_600;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route("/v1/prs/{owner}/{repo}/{number}/wait", get(wait))
}

/// A `wait`'s parameters, checked (API-1, API-10, API-12).
struct Asked {
    pr: PrName,
    cond: Condition,
    timeout: u32,
    limit: Option<u32>,
}

/// API-1, API-10, API-12: the parameters. `Condition::check` is the one reading of API-10's rules.
fn parse(owner: &str, repo: &str, number: &str, query: Option<&str>) -> Result<Asked, ApiError> {
    let pr = pr_from_path(owner, repo, number)?;
    let mut params = Params::parse(query)?;
    let word = params
        .take("until")
        .ok_or_else(|| bad_request("parameter `until` is required"))?;
    let until = Until::parse(&word).ok_or_else(|| {
        bad_request("parameter `until` must be `merged`, `checks-done` or `changed`")
    })?;
    let mut ask = Ask::new(until);
    ask.sha = params.take("sha");
    ask.rerun_since = params.take("rerun_since");
    ask.fail_fast = params.flag("fail_fast")?;
    ask.kind = params.take("kind");
    ask.since = params.take("since");
    // API-12: 0 to 3 600, default 100; API-8: `limit` 1 to the history's length. Never clamped.
    let timeout = params
        .number("timeout", 0, MAX_TIMEOUT_SECS)?
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    let limit = params.number("limit", 1, MAX_EVENTS as u32)?;
    params.finish()?;
    let cond = Condition::check(ask)?;
    Ok(Asked {
        pr,
        cond,
        timeout,
        limit,
    })
}

fn limit_error(message: &str) -> ApiError {
    ApiError {
        code: ErrorCode::Limit,
        message: message.to_string(),
        until: None,
    }
}

/// API-12: the deadline runs from the request's arrival, on the monotonic clock plus the sleep noticed
/// since (POL-22, POL-26): never the wall clock.
struct Deadline {
    clock: SharedClock,
    sleep: Arc<SleepWatch>,
    arrived: Mono,
    slept_at_arrival: Duration,
    timeout: Duration,
    watch: tokio::sync::watch::Receiver<Duration>,
}

impl Deadline {
    fn new(state: &AppState, arrived: Mono, slept_at_arrival: Duration, secs: u32) -> Deadline {
        Deadline {
            clock: state.clock.clone(),
            sleep: state.sleep.clone(),
            arrived,
            slept_at_arrival,
            timeout: Duration::from_secs(u64::from(secs)),
            watch: state.sleep.subscribe(),
        }
    }

    /// API-12: `mono().since(arrived)` plus the sleep noticed since arrival.
    fn used(&self) -> Duration {
        self.clock.mono().since(self.arrived)
            + self.sleep.total().saturating_sub(self.slept_at_arrival)
    }

    fn passed(&self) -> bool {
        self.used() >= self.timeout
    }

    /// Completes once the deadline has passed; re-checked whenever `SleepWatch` changes (API-12).
    /// Cancel-safe: the timer is set again from the absolute time each call.
    async fn reached(&mut self) {
        loop {
            self.watch.borrow_and_update();
            let used = self.used();
            if used >= self.timeout {
                return;
            }
            let mut timer = self
                .clock
                .sleep_until(self.clock.mono().plus(self.timeout - used));
            tokio::select! {
                _ = &mut timer => {}
                changed = self.watch.changed() => {
                    // The watch's sender lives as long as `SleepWatch`, which `self` holds: this can't
                    // fail. Were it to, the timer alone still ends the wait.
                    if changed.is_err() {
                        timer.await;
                    }
                }
            }
        }
    }
}

/// `fut`, or `None` if the deadline passes first (its future is dropped: the poller finishes the work).
/// Without a deadline (`timeout=0`, API-12) `fut` runs to its end.
async fn or_deadline<F: Future>(fut: F, deadline: &mut Option<Deadline>) -> Option<F::Output> {
    match deadline {
        None => Some(fut.await),
        Some(d) => {
            tokio::select! {
                out = fut => Some(out),
                _ = d.reached() => None,
            }
        }
    }
}

/// What a registered wait holds on its PR. API-14: dropped whenever the wait ends or its body is
/// dropped, and the PR is no longer hot for it (API-13).
struct Held {
    waiting: Option<Waiting>,
    named: Option<Named>,
    poller: Arc<dyn PollerHandle>,
}

impl Drop for Held {
    fn drop(&mut self) {
        // API-14: the waiter is removed; STA-1: the request that named the PR ends.
        self.waiting.take();
        self.named.take();
        // API-13: the hot set changed again as the waiter left.
        self.poller.nudge();
    }
}

/// The answer, built from one view (MOD-13, API-9): the outcome, the record, and the events as API-8's
/// `window` gives them for this PR, from `since` or the header's cursor; too old is judged on this view
/// (STA-9). With `reset`, `records` holds the record without `checks.items` (API-19).
fn answer(
    state: &AppState,
    view: &View,
    id: &NodeId,
    from: &Cursor,
    limit: Option<u32>,
    reset_too: bool,
    verdict: (Outcome, Option<Reason>),
) -> Vec<u8> {
    // API-9, API-8, STA-9: the window rule is T43's, run on the answer's view.
    let w = window(view, Some(from), Some(id), limit);
    // API-10: a too-old `rerun_since` puts `reset: true` in the answer.
    let reset = w.reset || reset_too;
    let tier = state.poller.status().tier;
    let pr = view.get(id);
    let record = pr.map(|p| present(state, p, tier, true));
    let records = if reset {
        pr.map(|p| present(state, p, tier, false))
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    let body = WaitBody {
        outcome: verdict.0,
        reason: verdict.1,
        record,
        events: w.events,
        records,
        more_events: w.more_events,
        reset,
    };
    envelope(state, &w.cursor, body)
}

fn json_200(body: Body, cursor: &Cursor) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    // STA-9's form is `[a-z0-9]{8}-<digits>`: always a valid header value.
    let value =
        HeaderValue::try_from(cursor.to_string()).expect("a cursor is a valid header value");
    headers.insert(CURSOR_HEADER, value);
    response
}

/// API-12: the deadline passed during the read-through. A plain 200, `timeout`, `record: null`; it
/// carries `circitor-cursor` too, set to the answer's cursor (API-9). The read-through goes on.
fn read_through_timed_out(state: &AppState, asked: &Asked, id: Option<&NodeId>) -> Response {
    let view = state.store.view();
    // A PR with no node ID known has no events of its own: an ID nothing carries keeps the window's
    // `since` and `reset` rule while matching no event.
    let none = NodeId(String::new());
    let id = id.unwrap_or(&none);
    let from = asked.cond.since.clone().unwrap_or_else(|| view.cursor());
    let w = window(&view, Some(&from), Some(id), asked.limit);
    let body = WaitBody {
        outcome: Outcome::Timeout,
        reason: None,
        record: None,
        events: w.events,
        records: Vec::new(),
        more_events: w.more_events,
        reset: w.reset,
    };
    json_200(Body::from(envelope(state, &w.cursor, body)), &w.cursor)
}

/// `GET /v1/prs/{owner}/{repo}/{number}/wait` (API-9).
async fn wait(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, String)>,
    RawQuery(query): RawQuery,
    Extension(flight): Extension<InFlight>,
) -> Response {
    // API-12: the deadline runs from the request's arrival.
    let arrived = state.clock.mono();
    let slept_at_arrival = state.sleep.total();
    // API-9, in order. 1: a client request (POL-13), then the parameters (API-1).
    state.poller.client_request();
    let asked = match parse(&owner, &repo, &number, query.as_deref()) {
        Ok(a) => a,
        Err(e) => return error(&state, e),
    };
    // API-12: `timeout=0` has no deadline around the read-through and the arrival refresh.
    let mut deadline = (asked.timeout > 0)
        .then(|| Deadline::new(&state, arrived, slept_at_arrival, asked.timeout));

    // 2: STA-3's limit for a PR that isn't tracked, API-14's 64. Nothing is sent.
    let tracked = state.store.lookup(&asked.pr);
    if tracked.is_none() && state.store.places_free() == 0 {
        // STA-3: no place for a read-through.
        return error(
            &state,
            limit_error("No place left: 200 pull requests are tracked."),
        );
    }
    if state.store.view().waiters >= MAX_WAITERS {
        // API-14: at most 64 waiters.
        return error(&state, limit_error("Too many waiters: 64 are held."));
    }

    // 3: the read-through, if the PR has no record and isn't loaded (POL-16, STA-8).
    let (id, loaded) = match &tracked {
        // STA-8: a loaded PR is held with no read-through and no arrival refresh.
        Some(t) if t.class == Class::Loaded => (t.id.clone(), true),
        Some(t) if t.class != Class::First => (t.id.clone(), false),
        _ => {
            flight.park();
            let read = or_deadline(state.poller.read_through(&asked.pr), &mut deadline).await;
            flight.resume();
            match read {
                Some(Ok(id)) => (id, false),
                // API-9: a refusal before the waiter is registered answers as API-3.
                Some(Err(r)) => return error(&state, refusal(r)),
                // API-12: the deadline passed during the read-through.
                None => {
                    return read_through_timed_out(&state, &asked, tracked.as_ref().map(|t| &t.id));
                }
            }
        }
    };

    // 4: the waiter is registered (API-14's 64 again, in case another came in meanwhile), the PR
    // named (STA-1) and made hot (API-13); then the head goes out with the cursor at that moment.
    let waiting = match state.store.add_waiter(&id) {
        Ok(w) => w,
        Err(_) => return error(&state, limit_error("Too many waiters: 64 are held.")),
    };
    let held = Held {
        waiting: Some(waiting),
        named: Some(state.store.name(&id)),
        poller: state.poller.clone(),
    };
    // API-13: the hot set changed.
    state.poller.nudge();
    let cursor = state.store.view().cursor();

    let held_wait = HeldWait {
        state,
        asked,
        id,
        loaded,
        from_header: cursor.clone(),
        deadline,
        flight,
        _held: held,
        arrived,
    };
    // The body completes later (API-9): one chunk, when the wait ends. It owns the whole wait.
    let stream = futures::stream::once(async move {
        Ok::<Bytes, Infallible>(Bytes::from(held_wait.run().await))
    });
    json_200(Body::from_stream(stream), &cursor)
}

/// A registered wait: everything it holds, owned by the response's body.
struct HeldWait {
    state: AppState,
    asked: Asked,
    id: NodeId,
    loaded: bool,
    /// The header's cursor: where the answer's events start without `since` (API-9).
    from_header: Cursor,
    deadline: Option<Deadline>,
    flight: InFlight,
    /// Given back when the wait ends or its body is dropped (API-14): `Held`'s `Drop` does it.
    _held: Held,
    arrived: Mono,
}

impl HeldWait {
    /// Steps 5 and 6 of API-9's arrival, then the wait itself. Always a 200 body (API-9).
    async fn run(mut self) -> Vec<u8> {
        let state = self.state.clone();
        let mut progress = Progress::new(self.arrived);
        // STA-4: a waiter evaluates after every refresh of its PR. Taken before the arrival refresh,
        // so a refresh applied during it is seen.
        let mut changed = state.store.changed();

        // 5: the arrival refresh (API-13), unless the PR is loaded without a record (STA-8).
        // `NotSent` and `Failed` count as failed: the first evaluation follows at once (API-9).
        if !self.loaded {
            self.flight.park();
            let _ = or_deadline(state.poller.refresh_pr(&self.id), &mut self.deadline).await;
            self.flight.resume();
        }

        // 6: the first evaluation, then one after every change and at the deadline, each on a fresh
        // view. A loaded PR has no record until its first refresh: `evaluate` answers `None` then,
        // and is first truly evaluated on the view that has it (STA-8).
        loop {
            // Seen before the view is taken: a change applied after this wakes the wait again.
            changed.borrow_and_update();
            let view = state.store.view();
            if let Some(verdict) = evaluate(&self.asked.cond, &view, &self.id, &mut progress) {
                return self.answer(&view, &progress, verdict);
            }
            let over = match &self.deadline {
                // API-12: `timeout=0` evaluates once and answers.
                None => true,
                Some(d) => d.passed(),
            };
            if over {
                // API-11, API-12: neither, in time.
                return self.answer(&view, &progress, (Outcome::Timeout, None));
            }
            self.flight.park();
            if let Some(d) = self.deadline.as_mut() {
                tokio::select! {
                    moved = changed.changed() => {
                        // The store holds the sender and the state holds the store: this can't fail.
                        // Were it to, the deadline alone ends the wait.
                        if moved.is_err() {
                            d.reached().await;
                        }
                    }
                    _ = d.reached() => {}
                }
            }
            self.flight.resume();
        }
    }

    fn answer(
        &self,
        view: &View,
        progress: &Progress,
        verdict: (Outcome, Option<Reason>),
    ) -> Vec<u8> {
        let from = self.asked.cond.since.as_ref().unwrap_or(&self.from_header);
        answer(
            &self.state,
            view,
            &self.id,
            from,
            self.asked.limit,
            progress.rerun_dropped,
            verdict,
        )
    }
}

#[cfg(test)]
mod tests;
