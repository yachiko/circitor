//! The answers' shapes: GitHub's headers on every answer, and each fault's body (TST-3, POL-25,
//! docs/02 §2 and §5). Every body that carries text holds `MARKER_GITHUB_TEXT` (TST-11).

use std::convert::Infallible;

use bytes::Bytes;
use http::{HeaderValue, Response, StatusCode};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;
use serde_json::{Value, json};

use crate::clock::Time;
use crate::testkit::MARKER_GITHUB_TEXT;

use super::fault::{Fault, LimitStatus};

/// Every answer's body.
pub(super) type Body = UnsyncBoxBody<Bytes, std::io::Error>;

/// docs/02 §2: the GraphQL limit, `X-RateLimit-Limit: 5000`.
const LIMIT: u32 = 5000;

/// What the headers say of the rate, and GitHub's clock for `Date`.
pub(super) struct Headers {
    /// `Date`, when the test hasn't left it out.
    pub date: Option<Time>,
    pub remaining: u32,
    /// `x-ratelimit-reset`, when there's one.
    pub reset: Option<Time>,
}

fn full(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into())
        .map_err(|never: Infallible| match never {})
        .boxed_unsync()
}

/// A response with GitHub's headers (TST-3: `Date` and `x-ratelimit-*`, MOD-6, POL-25).
fn respond(status: StatusCode, h: &Headers, content_type: &str, body: Body) -> Response<Body> {
    let mut r = Response::new(body);
    *r.status_mut() = status;
    let hs = r.headers_mut();
    if let Some(date) = h.date {
        let text = date.inner().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        if let Ok(v) = HeaderValue::from_str(&text) {
            hs.insert(http::header::DATE, v);
        }
    }
    hs.insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("text/plain")),
    );
    hs.insert("x-ratelimit-limit", HeaderValue::from(LIMIT));
    hs.insert("x-ratelimit-remaining", HeaderValue::from(h.remaining));
    hs.insert(
        "x-ratelimit-used",
        HeaderValue::from(LIMIT.saturating_sub(h.remaining)),
    );
    if let Some(reset) = h.reset {
        let secs = reset.unix_millis().div_euclid(1000);
        hs.insert("x-ratelimit-reset", HeaderValue::from(secs));
    }
    hs.insert("x-ratelimit-resource", HeaderValue::from_static("graphql"));
    r
}

/// A 200 with a JSON body.
pub(super) fn json_ok(h: &Headers, body: &Value) -> Response<Body> {
    json_status(StatusCode::OK, h, body)
}

fn json_status(status: StatusCode, h: &Headers, body: &Value) -> Response<Body> {
    respond(
        status,
        h,
        "application/json; charset=utf-8",
        full(body.to_string()),
    )
}

/// A plain text answer without GitHub's headers: a redirect's target, a path that isn't `/graphql`.
pub(super) fn plain(status: StatusCode, text: &str) -> Response<Body> {
    let mut r = Response::new(full(text.to_owned()));
    *r.status_mut() = status;
    r.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain"),
    );
    r
}

/// A request the fake refuses to answer (a broken rule): GitHub's shape for a query it can't run.
pub(super) fn refused(h: &Headers, why: &str) -> Response<Body> {
    json_ok(
        h,
        &json!({ "errors": [{ "message": format!("{why} ({MARKER_GITHUB_TEXT})") }] }),
    )
}

/// The 401 docs/02 §5 measured: a REST-style body, with no `data` or `errors`.
pub(super) fn unauthorized(h: &Headers) -> Response<Body> {
    json_status(
        StatusCode::UNAUTHORIZED,
        h,
        &json!({ "message": format!("Bad credentials ({MARKER_GITHUB_TEXT})"), "status": "401" }),
    )
}

/// The rate-limit messages (POL-25, assumed: docs/02 §6 item 6).
const EXHAUSTED: &str = "API rate limit already exceeded for user ID 1.";
const SECONDARY: &str =
    "You have exceeded a secondary rate limit. Please wait a few minutes before you try again.";

fn limit_answer(status: LimitStatus, h: &Headers, message: &str) -> Response<Body> {
    let message = format!("{message} ({MARKER_GITHUB_TEXT})");
    match status {
        LimitStatus::Forbidden => json_status(
            StatusCode::FORBIDDEN,
            h,
            &json!({ "message": message, "status": "403" }),
        ),
        LimitStatus::Ok => json_ok(
            h,
            &json!({ "errors": [{ "type": "RATE_LIMITED", "message": message }] }),
        ),
    }
}

/// The answer to a fault that answers (not `Hang`, not `Drop`). `h` is what the headers would say
/// without the fault, with the moved target's URL for a redirect.
pub(super) fn fault(fault: &Fault, h: &Headers, moved_to: &str) -> Response<Body> {
    match fault {
        Fault::Status(code) => {
            let status = StatusCode::from_u16(*code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let page = format!(
                "<html><body><h1>{code}</h1><p>Something went wrong ({MARKER_GITHUB_TEXT})</p></body></html>"
            );
            respond(status, h, "text/html; charset=utf-8", full(page))
        }
        Fault::Unauthorized => unauthorized(h),
        Fault::NoData => json_ok(
            h,
            &json!({ "errors": [{ "message": format!("Something went wrong while executing your query ({MARKER_GITHUB_TEXT})") }] }),
        ),
        Fault::NotJson => respond(
            StatusCode::OK,
            h,
            "application/json; charset=utf-8",
            full(format!("<html>not JSON ({MARKER_GITHUB_TEXT})</html>")),
        ),
        Fault::CutShort => cut_short(h),
        Fault::Redirect => {
            let mut r = respond(
                StatusCode::FOUND,
                h,
                "text/html; charset=utf-8",
                full(format!(
                    "<html><body>Moved to {moved_to} ({MARKER_GITHUB_TEXT})</body></html>"
                )),
            );
            if let Ok(v) = HeaderValue::from_str(moved_to) {
                r.headers_mut().insert(http::header::LOCATION, v);
            }
            r
        }
        Fault::Exhausted { status, reset } => {
            let headers = Headers {
                date: h.date,
                remaining: 0,
                reset: if *reset { h.reset } else { None },
            };
            limit_answer(*status, &headers, EXHAUSTED)
        }
        Fault::Secondary {
            status,
            retry_after,
        } => {
            let mut r = limit_answer(*status, h, SECONDARY);
            if let Some(secs) = retry_after {
                r.headers_mut()
                    .insert(http::header::RETRY_AFTER, HeaderValue::from(*secs));
            }
            r
        }
        // Played by the server itself; never answered.
        Fault::Hang | Fault::Drop => plain(StatusCode::INTERNAL_SERVER_ERROR, "unplayed fault"),
    }
}

/// A 200 whose body stops short of its `Content-Length`: half the bytes, a flush, then the body
/// fails and the connection closes.
fn cut_short(h: &Headers) -> Response<Body> {
    let whole = json!({ "data": { "viewer": { "login": MARKER_GITHUB_TEXT } } }).to_string();
    let half = Bytes::from(whole[..whole.len() / 2].to_owned());
    let stream = futures::stream::unfold(0u8, move |step| {
        let half = half.clone();
        async move {
            match step {
                0 => Some((Ok(Frame::data(half)), 1)),
                1 => {
                    // Let the connection flush what's written before the body fails.
                    tokio::task::yield_now().await;
                    Some((
                        Err(std::io::Error::other("the body is cut short on purpose")),
                        2,
                    ))
                }
                _ => None,
            }
        }
    });
    let mut r = respond(
        StatusCode::OK,
        h,
        "application/json; charset=utf-8",
        StreamBody::new(stream).boxed_unsync(),
    );
    r.headers_mut()
        .insert(http::header::CONTENT_LENGTH, HeaderValue::from(whole.len()));
    r
}
