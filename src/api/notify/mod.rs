//! The two notify routes (NTF-2, NTF-3; T47).
//!
//! `POST …/notify` registers a subscription in NTF-2's order: the parameters; the `socket` check; the
//! read-through and the arrival refresh; one evaluation on one view `V`; then an answer already known,
//! the `auth` state, or `register` (the limits, the entry, the confirmation). **Every answer is built
//! from `V`** (MOD-13). `DELETE …/notify?socket=…` cancels (NTF-3). Neither holds a subscription: the
//! notifier does (`notify.md`). Neither makes its PR hot (NTF-5): `name_quiet`, never `name`,
//! `add_waiter` or `nudge`.

use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, PathRejection};
use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use axum::routing::post;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::api::condition::{Condition, Progress, evaluate};
use crate::api::types::{ApiError, Ask, CancelBody, ErrorCode, NotifyBody, Until};
use crate::api::{
    AppState, InFlight, Params, bad_request, error, ok, ok_at, pr_from_path, present, refusal,
    window,
};
use crate::model::{NodeId, PrName};
use crate::notify::{
    DEFAULT_LIFE, Looked, MAX_LIFE, MIN_LIFE, NewSubscription, RegisterError, SessionToken,
};
use crate::poller::ProblemCode;
use crate::state::MAX_EVENTS;
use crate::state::store::Class;

#[cfg(test)]
mod tests;

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/v1/prs/{owner}/{repo}/{number}/notify",
        post(register).delete(cancel),
    )
}

/// A body key's value as given, `null` included: absent is `None`, `null` is `Some(Value::Null)`.
fn present_value<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

/// NTF-9: the token goes straight into a `SessionToken`, never into a `String` that is kept.
fn token_value<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SessionToken>, D::Error> {
    // The serde error is replaced: it names the key and never the value.
    Option::<SessionToken>::deserialize(d)
        .map_err(|_| serde::de::Error::custom("body key `token` must be a string or null"))
}

/// The body of `POST …/notify` (NTF-2), strict (API-1): an unknown key is `bad_request` naming it.
/// Each value is kept as JSON and read below, so each error names its key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBody {
    #[serde(default, deserialize_with = "present_value")]
    until: Option<Value>,
    #[serde(default, deserialize_with = "present_value")]
    sha: Option<Value>,
    #[serde(default, deserialize_with = "present_value")]
    rerun_since: Option<Value>,
    #[serde(default, deserialize_with = "present_value")]
    fail_fast: Option<Value>,
    #[serde(default, deserialize_with = "present_value")]
    kind: Option<Value>,
    #[serde(default, deserialize_with = "present_value")]
    since: Option<Value>,
    #[serde(rename = "for", default, deserialize_with = "present_value")]
    life: Option<Value>,
    #[serde(default, deserialize_with = "present_value")]
    limit: Option<Value>,
    #[serde(default, deserialize_with = "present_value")]
    socket: Option<Value>,
    #[serde(default, deserialize_with = "token_value")]
    token: Option<SessionToken>,
}

/// A registration's parameters, checked (API-1, API-10, NTF-2).
struct Asked {
    pr: PrName,
    cond: Condition,
    life: Duration,
    life_given: bool,
    limit: Option<u32>,
    socket: PathBuf,
    token: Option<SessionToken>,
}

/// serde_json's message without its ` at line … column …`: the key is what the client needs.
fn body_error(e: serde_json::Error) -> ApiError {
    if e.is_syntax() || e.is_eof() {
        return bad_request("the body is not valid JSON");
    }
    let text = e.to_string();
    let place = format!(" at line {} column {}", e.line(), e.column());
    let text = text.strip_suffix(&place).unwrap_or(&text);
    // serde's own words for a key: `unknown field`, `missing field`.
    let text = text.replace("field", "body key");
    bad_request(text)
}

fn string(key: &str, v: Option<Value>) -> Result<Option<String>, ApiError> {
    match v {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(bad_request(format!("body key `{key}` must be a string"))),
    }
}

fn number(key: &str, v: Option<Value>, min: u64, max: u64) -> Result<Option<u64>, ApiError> {
    match v {
        None => Ok(None),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(bad_request(format!(
                "body key `{key}` must be a whole number from {min} to {max}"
            ))),
        },
        Some(_) => Err(bad_request(format!(
            "body key `{key}` must be a whole number from {min} to {max}"
        ))),
    }
}

/// NTF-2's body, then API-10's checks through `Condition::check`, the one reading of them.
fn parse(
    path: Result<Path<(String, String, String)>, PathRejection>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Asked, ApiError> {
    let Path((owner, repo, number_s)) =
        path.map_err(|_| bad_request("the path isn't `{owner}/{repo}/{number}`"))?;
    let pr = pr_from_path(&owner, &repo, &number_s)?;
    let bytes = body.map_err(|_| bad_request("the body can't be read"))?;
    // NTF-2: strict, `deny_unknown_fields`, every error naming its key (API-1).
    let raw: RawBody = serde_json::from_slice(&bytes).map_err(body_error)?;
    let word =
        string("until", raw.until)?.ok_or_else(|| bad_request("body key `until` is required"))?;
    let until = Until::parse(&word).ok_or_else(|| {
        bad_request("body key `until` must be `merged`, `checks-done` or `changed`")
    })?;
    let socket = string("socket", raw.socket)?
        .ok_or_else(|| bad_request("body key `socket` is required"))?;
    // NTF-2: `bad_request` for a `socket` that isn't an absolute path; `no_session` for the rest.
    let socket = PathBuf::from(socket);
    if !socket.is_absolute() {
        return Err(bad_request("body key `socket` must be an absolute path"));
    }
    let mut ask = Ask::new(until);
    ask.sha = string("sha", raw.sha)?;
    ask.rerun_since = string("rerun_since", raw.rerun_since)?;
    // NTF-2: in a body `fail_fast` is a JSON boolean, never `1`.
    ask.fail_fast = match raw.fail_fast {
        None => false,
        Some(Value::Bool(b)) => b,
        Some(_) => return Err(bad_request("body key `fail_fast` must be true or false")),
    };
    // NTF-2: `kind` is one comma-separated string.
    ask.kind = string("kind", raw.kind)?;
    ask.since = string("since", raw.since)?;
    // NTF-2: `for`, 60 to 86 400, default 86 400. (proposal)
    let life_s = number("for", raw.life, MIN_LIFE.as_secs(), MAX_LIFE.as_secs())?;
    // API-8: `limit`, 1 to the history's length.
    let limit = number("limit", raw.limit, 1, MAX_EVENTS as u64)?.map(|n| n as u32);
    let cond = Condition::check(ask)?;
    Ok(Asked {
        pr,
        cond,
        life: life_s.map(Duration::from_secs).unwrap_or(DEFAULT_LIFE),
        life_given: life_s.is_some(),
        limit,
        socket,
        token: raw.token,
    })
}

fn coded(code: ErrorCode, message: impl Into<String>) -> ApiError {
    ApiError {
        code,
        message: message.into(),
        until: None,
    }
}

/// API-3's `no_session`: the system's error, or circitor's words for what is there; nothing from
/// the request but the word `socket`, never the path or the token.
fn no_session(why: &str) -> ApiError {
    coded(
        ErrorCode::NoSession,
        format!("The session socket can't be used: {why}."),
    )
}

/// `POST /v1/prs/{owner}/{repo}/{number}/notify` (NTF-2), in its order.
async fn register(
    State(state): State<AppState>,
    Extension(flight): Extension<InFlight>,
    path: Result<Path<(String, String, String)>, PathRejection>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    // API-11, NTF-4: `head_moved`'s 60 s run from the arrival.
    let arrived = state.clock.mono();
    // NTF-2, 1: a client request (POL-13), then the parameters (API-1).
    state.poller.client_request();
    let asked = match parse(path, body) {
        Ok(a) => a,
        Err(e) => return error(&state, e),
    };

    // NTF-2, 2: the `socket` check, a `stat`, straight after the parameters: a bad socket never
    // reaches GitHub, and gets `no_session` even when the answer would be known.
    flight.park();
    let looked = state.notify.look(&asked.socket).await;
    flight.resume();
    let socket_id = match looked {
        Looked::Socket(id) => id,
        Looked::Other(_, what) => return error(&state, no_session(&what)),
        Looked::Missing => return error(&state, no_session("nothing is at its path")),
        Looked::Failed(e) => return error(&state, no_session(&e)),
    };

    // 3: STA-3's limit for a PR that isn't tracked. Nothing is sent.
    let tracked = state.store.lookup(&asked.pr);
    if tracked.is_none() && state.store.places_free() == 0 {
        return error(
            &state,
            coded(
                ErrorCode::Limit,
                "No place left: 200 pull requests are tracked.",
            ),
        );
    }

    // 4: the read-through when the PR has no record, a loaded one included (NTF-2, STA-8).
    let id: NodeId = match tracked {
        Some(t) if t.class == Class::Open || t.class == Class::Ended => t.id,
        _ => {
            flight.park();
            let read = state.poller.read_through(&asked.pr).await;
            flight.resume();
            match read {
                Ok(id) => id,
                // API-3: a refusal answers as its error.
                Err(r) => return error(&state, refusal(r)),
            }
        }
    };

    // 5: STA-1: the request names its PR, for the 60 minutes only (NTF-2, NTF-5). Held until the
    // answer. The cursor now is where an answer's events start without `since`.
    let _named = state.store.name_quiet(&id);
    let from_arrival = state.store.view().cursor();

    // 6: the arrival refresh (API-13); `NotSent` and `Failed` count as failed, and the evaluation
    // follows at once (API-9). The one forced refresh a registration causes (NTF-5).
    flight.park();
    let _ = state.poller.refresh_pr(&id).await;
    flight.resume();

    // 7: one view, `V`, and one evaluation on it (NTF-2, API-12's `timeout=0`). Every answer is `V`'s.
    let view = state.store.view();
    let mut progress = Progress::new(arrived);
    let verdict = evaluate(&asked.cond, &view, &id, &mut progress);
    let tier = state.poller.status().tier;
    let pr_view = view.get(&id);
    let record = pr_view.map(|p| present(&state, p, tier, true));

    // 8: an answer already known: as the `wait` it stands for, `registered: false`. Nothing is kept,
    // nothing posted; an earlier same request is still dropped (NTF-2).
    if let Some((outcome, reason)) = verdict {
        let replaced = state.notify.drop_same(&id, &asked.socket, &asked.cond.ask);
        let from = asked.cond.since.as_ref().unwrap_or(&from_arrival);
        let w = window(&view, Some(from), Some(&id), asked.limit);
        // API-10: a too-old `rerun_since` puts `reset: true` in the answer.
        let reset = w.reset || progress.rerun_dropped;
        let records = reset_records(&state, pr_view, tier, reset);
        let body = NotifyBody {
            registered: false,
            replaced,
            expires_at: None,
            outcome: Some(outcome),
            reason,
            record,
            events: w.events,
            records,
            more_events: w.more_events,
            reset,
        };
        return ok_at(&state, &w.cursor, body);
    }

    // 9: no outcome in the `auth` state: nothing can be told until a person logs in (NTF-2). Before
    // the limit, which `register` checks.
    if let Some(p) = state
        .poller
        .status()
        .problem
        .filter(|p| p.code == ProblemCode::Auth)
    {
        return error(&state, coded(ErrorCode::Auth, p.message));
    }

    // 10: kept. The answer's events and cursor are `V`'s; a `since` too old on `V` is answered with
    // the reset and let go: the subscription's cursor is then `V`'s (NTF-2, NTF-8).
    let w = window(&view, asked.cond.since.as_ref(), Some(&id), asked.limit);
    let reset = w.reset || progress.rerun_dropped;
    let records = reset_records(&state, pr_view, tier, reset);
    let cursor = match &asked.cond.since {
        Some(c) if !view.too_old(c) => c.clone(),
        _ => view.cursor(),
    };
    // MOD-13: no view after `register`: a refresh applied during the confirmation is the notice's.
    drop(view);
    let new = NewSubscription {
        id,
        pr: asked.pr,
        cond: asked.cond,
        progress,
        cursor,
        life: asked.life,
        life_given: asked.life_given,
        socket: asked.socket,
        socket_id,
        token: asked.token,
    };
    flight.park();
    let registered = state.notify.register(new).await;
    match registered {
        Ok(r) => {
            flight.resume();
            let body = NotifyBody {
                registered: true,
                replaced: r.replaced,
                expires_at: Some(r.expires_at),
                outcome: None,
                reason: None,
                record,
                events: w.events,
                records,
                more_events: w.more_events,
                reset,
            };
            ok_at(&state, &w.cursor, body)
        }
        Err(RegisterError::Limit) => {
            flight.resume();
            error(
                &state,
                coded(
                    ErrorCode::Limit,
                    "Too many subscriptions: 64 are held, or 32 for this socket.",
                ),
            )
        }
        Err(RegisterError::NoSession(e)) => {
            flight.resume();
            error(&state, no_session(&e))
        }
        // OPS-7: the daemon is closing this connection without an answer. The `InFlight` stays
        // parked; the future is dropped with the connection.
        Err(RegisterError::Stopping) => std::future::pending().await,
    }
}

/// A reset's `records`: the record without `checks.items` (API-19).
fn reset_records(
    state: &AppState,
    pr: Option<&crate::state::PrView>,
    tier: crate::poller::Tier,
    reset: bool,
) -> Vec<crate::model::Record> {
    if !reset {
        return Vec::new();
    }
    pr.map(|p| present(state, p, tier, false))
        .into_iter()
        .collect()
}

/// `DELETE /v1/prs/{owner}/{repo}/{number}/notify?socket=…` (NTF-3). Never reads through, never
/// waits for the first poll (API-18), posts nothing. Idempotent.
async fn cancel(
    State(state): State<AppState>,
    path: Result<Path<(String, String, String)>, PathRejection>,
    query: RawQuery,
) -> Response {
    state.poller.client_request();
    let parsed = (|| {
        let Path((owner, repo, number)) =
            path.map_err(|_| bad_request("the path isn't `{owner}/{repo}/{number}`"))?;
        let pr = pr_from_path(&owner, &repo, &number)?;
        let mut p = Params::parse(query.0.as_deref())?;
        let socket = p
            .take("socket")
            .ok_or_else(|| bad_request("parameter `socket` is required"))?;
        p.finish()?;
        Ok::<_, ApiError>((pr, socket))
    })();
    let (pr, socket) = match parsed {
        Ok(v) => v,
        Err(e) => return error(&state, e),
    };
    // NTF-3: the node ID only if it's tracked; never a read-through.
    let tracked = state.store.lookup(&pr);
    // STA-1: the request names its PR for the 60 minutes only.
    let _named = tracked.as_ref().map(|t| state.store.name_quiet(&t.id));
    // NTF-1, NTF-3: also one registered under this name, whatever its PR is called now.
    let cancelled = state
        .notify
        .cancel(&pr, tracked.as_ref().map(|t| &t.id), FsPath::new(&socket));
    let view = state.store.view();
    ok(&state, &view, CancelBody { cancelled })
}
