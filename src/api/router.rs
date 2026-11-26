//! The router, its layer and fallbacks, and the routes' shared test state (T43).

use std::sync::{Arc as StdArc, Mutex};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

#[cfg(any(test, feature = "test-endpoint"))]
use std::sync::Arc;

use crate::clock::{Activity, Busy};
#[cfg(any(test, feature = "test-endpoint"))]
use crate::clock::{SharedClock, TestClock};
#[cfg(any(test, feature = "test-endpoint"))]
use crate::poller::fake::FakePoller;

use super::respond::error_response;
use super::types::{ApiError, ErrorCode};
use super::{AppState, notify, server, wait};

/// `server::routes()` merged with `wait::routes()` and `notify::routes()`, the `unknown_route` fallback, and the layer below.
pub fn router(state: AppState) -> axum::Router {
    let version = state.info.version.clone();
    let routes = axum::Router::new()
        .merge(server::routes())
        .merge(wait::routes())
        .merge(notify::routes());
    with_fallbacks(routes, version)
        .layer(axum::middleware::from_fn_with_state(state.clone(), layer))
        .with_state(state)
}

/// API-3: anything that isn't a route of 04-api is `unknown_route`: an unknown path, and a known path
/// with another method. Called after the routes are merged: the method fallback is set on the routes present.
fn with_fallbacks<S: Clone + Send + Sync + 'static>(
    routes: axum::Router<S>,
    version: String,
) -> axum::Router<S> {
    let unknown = move |version: &str| {
        error_response(
            version,
            ApiError {
                code: ErrorCode::UnknownRoute,
                message: "no such route".to_string(),
                until: None,
            },
        )
    };
    let for_path = version.clone();
    let for_method = version;
    let unknown_path = unknown;
    routes
        .fallback(move || {
            let body = unknown_path(&for_path);
            async move { body }
        })
        .method_not_allowed_fallback(move || {
            let body = unknown(&for_method);
            async move { body }
        })
}

/// On every request: takes the work in flight (TST-4) and puts it in the request's extensions, and runs
/// POL-22's check. It never calls `poller.client_request()`: each handler does.
async fn layer(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let in_flight = InFlight::new(&state.activity);
    request.extensions_mut().insert(in_flight.clone());
    state.sleep.check();
    let response = next.run(request).await;
    drop(in_flight);
    response
}

struct InFlightInner {
    activity: Activity,
    busy: Mutex<Option<Busy>>,
}

/// Put on every request by `router`'s layer: the request's share of the work in flight (TST-4).
#[derive(Clone)]
pub struct InFlight(StdArc<InFlightInner>);

impl InFlight {
    /// Takes `activity.begin()` now; it's given back when the last clone is dropped.
    pub(crate) fn new(activity: &Activity) -> InFlight {
        InFlight(StdArc::new(InFlightInner {
            activity: activity.clone(),
            busy: Mutex::new(Some(activity.begin())),
        }))
    }

    /// The request now waits on the clock or on the poller: not work in flight.
    pub fn park(&self) {
        // A lock held for one assignment, never across an `.await`; a poisoned lock still holds its value.
        let mut busy = self.0.busy.lock().unwrap_or_else(|e| e.into_inner());
        busy.take();
    }

    /// It woke: work again, until dropped.
    pub fn resume(&self) {
        let mut busy = self.0.busy.lock().unwrap_or_else(|e| e.into_inner());
        if busy.is_none() {
            *busy = Some(self.0.activity.begin());
        }
    }
}

/// An `AppState` over a real, empty `Store`, `FakePoller`, `NoFlush` and `NoNotify`; its `activity` is
/// the test clock's, so `settle` sees a handler's work.
#[cfg(any(test, feature = "test-endpoint"))]
pub fn test_state(clock: Arc<TestClock>) -> (AppState, Arc<FakePoller>) {
    use crate::clock::{Clock, SleepWatch};
    use crate::config::Config;
    use crate::daemon::{Mode, StopHandle};
    use crate::log::{BufferSink, Level, Logger};
    use crate::notify::NoNotify;
    use crate::state::{FileContent, NoFlush, Store, StoreDeps};
    use crate::status::Warnings;
    use crate::testkit::TEST_VERSION;

    use super::DaemonInfo;
    use super::types::PathsInfo;

    let shared: SharedClock = clock.clone();
    let logger = Logger::new(shared.clone(), Arc::new(BufferSink::default()), Level::Info);
    let config = Arc::new(Config::default());
    let store = Store::new(
        StoreDeps {
            clock: shared.clone(),
            logger: logger.clone(),
            config: config.clone(),
            seed: 1,
        },
        FileContent::default(),
    );
    let poller = FakePoller::new();
    let state = AppState {
        store,
        poller: poller.clone(),
        flush: Arc::new(NoFlush),
        notify: Arc::new(NoNotify),
        clock: shared.clone(),
        sleep: SleepWatch::new(shared.clone()),
        activity: clock.activity(),
        warnings: Warnings::new(shared.clone(), logger.clone()),
        logger,
        config,
        info: Arc::new(DaemonInfo {
            version: TEST_VERSION.to_string(),
            mode: Mode::Spawned,
            started_at: clock.wall(),
            paths: PathsInfo {
                config: "/tmp/ct/config.toml".to_string(),
                state: "/tmp/ct/state.json".to_string(),
                socket: "/tmp/ct/circitor.sock".to_string(),
                log: "/tmp/ct/circitor.log".to_string(),
            },
        }),
        stop: StopHandle::new(),
    };
    (state, poller)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    // API-3: an unknown path, and a known path with another method.
    #[tokio::test]
    async fn api3_unknown_route_for_a_path_and_a_method() {
        let routes: axum::Router<()> = axum::Router::new().route("/v1/x", get(|| async { "x" }));
        let app = with_fallbacks(routes, "1.2.3+test".to_string());
        for (method, path) in [("GET", "/v1/y"), ("POST", "/v1/x")] {
            let response = app
                .clone()
                .oneshot(
                    http::Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 404, "{method} {path}");
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["version"], "1.2.3+test");
            assert_eq!(v["error"]["code"], "unknown_route");
            assert!(v["error"]["until"].is_null());
        }
        // The known route still answers.
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/v1/x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }

    #[test]
    fn in_flight_parks_and_resumes_many_times() {
        let activity = Activity::default();
        let flight = InFlight::new(&activity);
        assert_eq!(activity.in_flight(), 1);
        flight.park();
        flight.park();
        assert_eq!(activity.in_flight(), 0);
        flight.resume();
        flight.resume();
        assert_eq!(activity.in_flight(), 1);
        let clone = flight.clone();
        drop(flight);
        assert_eq!(activity.in_flight(), 1);
        drop(clone);
        assert_eq!(activity.in_flight(), 0);
    }
}
