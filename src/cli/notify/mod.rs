//! `circitor notify` (NTF-11; T48): registers a subscription for the Claude Code session the command
//! runs in, or drops that session's subscriptions on the PR.
//!
//! The session is the one in `env.env` (CLI-13), never the process's own environment.

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::api::{Ask, CancelBody, Envelope, ErrorBody, ErrorCode, NotifyBody, Request};
use crate::cli::args::NotifyWhat;
use crate::cli::commands::{find_bare, name_offline};
use crate::cli::conn::{self, ConnError, Link, Reply};
use crate::cli::render::{self, RenderCtx};
use crate::cli::{CliEnv, Output, exit};
use crate::config;
use crate::model::PrName;
use crate::notify::{DEFAULT_LIFE, text};

#[cfg(test)]
mod stub;
#[cfg(test)]
mod tests;

/// docs/07.
pub const SOCKET_VAR: &str = "CLAUDE_CODE_MESSAGING_SOCKET";
pub const TOKEN_VAR: &str = "CLAUDE_CODE_MESSAGING_TOKEN";

/// CLI-3's cut, as `wait` sends it.
const EVENT_LIMIT: u32 = 50;
/// NTF-10: how long `--no-spawn` waits for a daemon.
const NO_SPAWN_WAIT: Duration = Duration::from_secs(8);

/// `circitor notify`, whole (NTF-11). `commands::run` calls it for `Command::Notify` and returns what it returns.
pub async fn run(
    pr: Option<&str>,
    what: &NotifyWhat,
    json: bool,
    no_spawn: bool,
    env: &CliEnv,
) -> Output {
    // NTF-11, D9: with no socket in the environment nothing else happens, and nothing is printed to stdout.
    let Some(socket) = env.env.get(SOCKET_VAR) else {
        return failure(exit::NO_SESSION, no_session_line(pr, what, None));
    };
    let socket = socket.to_string();
    match what {
        NotifyWhat::Register { cond, life } => {
            register(env, pr, cond, *life, socket, json, no_spawn).await
        }
        NotifyWhat::Cancel => cancel(env, pr, socket, json).await,
    }
}

// ---- Registering ----

async fn register(
    env: &CliEnv,
    arg: Option<&str>,
    cond: &Ask,
    life: Option<u32>,
    socket: String,
    json: bool,
    no_spawn: bool,
) -> Output {
    let paths = match config::resolve(&env.env, env.platform) {
        Ok(p) => p,
        Err(e) => return failure(exit::NO_DAEMON, e.to_string()),
    };
    let named = match name_offline(env, arg).await {
        Ok(n) => n,
        Err(out) => return out,
    };
    let mut warn = String::new();
    let mut link = match connect(env, &paths, no_spawn, &mut warn).await {
        Ok(l) => l,
        Err(e) => return conn_failure(e, warn),
    };
    let pr = match named {
        Some(n) => n,
        None => match find_bare(env, &mut link, &mut warn).await {
            Ok(n) => n,
            Err(mut out) => {
                out.stderr = warn + &out.stderr;
                return out;
            }
        },
    };
    let req = Request::Notify {
        pr: pr.clone(),
        cond: cond.clone(),
        life,
        socket,
        // NTF-9: the token goes into this body and nowhere else.
        token: env.env.get(TOKEN_VAR).map(str::to_string),
        limit: Some(EVENT_LIMIT),
    };
    let reply = match link.request(env, &req).await {
        Ok(r) => r,
        Err(e) => return conn_failure(e, warn),
    };
    let ctx = ctx(env);
    // NTF-11: the life printed is `--for`, or the default; never worked out from `expires_at`.
    let shown_life = life.map_or(DEFAULT_LIFE, |s| Duration::from_secs(u64::from(s)));
    let what = NotifyWhat::Register {
        cond: cond.clone(),
        life,
    };
    answer(
        &reply,
        json,
        link.older_unreplaced(),
        &ctx,
        warn,
        |e: &Envelope<NotifyBody>| {
            // NTF-11: registered, already known and a check that failed are all exit 0.
            let text = if e.body.registered {
                let first = format!("registered: {}", text::watching(&pr, cond, shown_life));
                render::registered(e, &first, &pr, &ctx)
            } else {
                render::already(e, &pr, cond, life, &ctx)
            };
            (text, exit::OK)
        },
        // NTF-11: the same line as with no socket, with the PR in full.
        Some(no_session_line(None, &what, Some(&pr))),
    )
}

/// NTF-10, CLI-7: `--no-spawn` waits up to 8 s for a daemon and starts none.
async fn connect(
    env: &CliEnv,
    paths: &config::Paths,
    no_spawn: bool,
    warn: &mut String,
) -> Result<Link, ConnError> {
    if !no_spawn {
        return conn::ensure(env, paths, false, None, warn).await;
    }
    match conn::existing(env, paths, warn).await? {
        Some(link) => Ok(link),
        None => conn::reconnect(env, paths, NO_SPAWN_WAIT).await,
    }
}

// ---- Cancelling ----

async fn cancel(env: &CliEnv, arg: Option<&str>, socket: String, json: bool) -> Output {
    let paths = match config::resolve(&env.env, env.platform) {
        Ok(p) => p,
        Err(e) => return failure(exit::NO_DAEMON, e.to_string()),
    };
    let named = match name_offline(env, arg).await {
        Ok(n) => n,
        Err(out) => return out,
    };
    let mut warn = String::new();
    // CLI-8, NTF-3: never starts a daemon and never waits for one.
    let mut link = match conn::existing(env, &paths, &mut warn).await {
        Ok(Some(l)) => l,
        Ok(None) => return none_cancelled(json, warn),
        Err(e) => return conn_failure(e, warn),
    };
    let pr = match named {
        Some(n) => n,
        None => match find_bare(env, &mut link, &mut warn).await {
            Ok(n) => n,
            Err(mut out) => {
                out.stderr = warn + &out.stderr;
                return out;
            }
        },
    };
    let reply = match link
        .request(env, &Request::NotifyCancel { pr, socket })
        .await
    {
        Ok(r) => r,
        Err(e) => return conn_failure(e, warn),
    };
    answer(
        &reply,
        json,
        link.older_unreplaced(),
        &ctx(env),
        warn,
        |e: &Envelope<CancelBody>| (format!("cancelled: {}\n", e.body.cancelled), exit::OK),
        None,
    )
}

/// No daemon: there's nothing to cancel, and nothing is asked.
fn none_cancelled(json: bool, warn: String) -> Output {
    Output {
        stdout: if json {
            "{\"cancelled\":0}\n".to_string()
        } else {
            "cancelled: 0\n".to_string()
        },
        stderr: warn,
        code: exit::OK,
    }
}

// ---- Answers ----

/// A reply, whole. A 200 goes through `render`; an error through CLI-5's line and code, `no_session`
/// with `on_no_session` when given. With `--json`, stdout is the body as it came either way (CLI-4).
fn answer<T: DeserializeOwned>(
    reply: &Reply,
    json: bool,
    older: bool,
    ctx: &RenderCtx,
    warn: String,
    render: impl FnOnce(&Envelope<T>) -> (String, i32),
    on_no_session: Option<String>,
) -> Output {
    let mut out = Output {
        stderr: warn,
        ..Output::default()
    };
    if reply.status == 200 {
        match serde_json::from_slice::<Envelope<T>>(&reply.body) {
            Ok(e) => {
                let (text, code) = render(&e);
                out.code = code;
                if !json {
                    out.stdout = text;
                }
            }
            Err(_) => {
                out.stderr
                    .push_str("the daemon's answer isn't one this CLI can read\n");
                out.code = exit::ERROR;
            }
        }
    } else {
        match serde_json::from_slice::<ErrorBody>(&reply.body) {
            Ok(e) => {
                let text = match (e.error.code, on_no_session) {
                    (ErrorCode::NoSession, Some(line)) => line,
                    _ => render::api_error(&e.error, older, ctx),
                };
                out.stderr.push_str(&with_newline(text));
                out.code = render::error_code(e.error.code, older);
            }
            Err(_) => {
                out.stderr.push_str(&format!(
                    "the daemon answered HTTP {} with an error this CLI can't read\n",
                    reply.status
                ));
                out.code = exit::ERROR;
            }
        }
    }
    if json {
        out.stdout = with_newline(
            String::from_utf8_lossy(&reply.body)
                .trim_end_matches('\n')
                .to_string(),
        );
    }
    out
}

/// NTF-11's line for no session: `no session here: use circitor wait <pr> <flags>[ --since <since>]`.
/// The PR is as typed (`typed`), or in full once the daemon named it (`full`). `--cancel` names no `wait`.
fn no_session_line(typed: Option<&str>, what: &NotifyWhat, full: Option<&PrName>) -> String {
    let NotifyWhat::Register { cond, .. } = what else {
        return "no session here: there is no session's notify to cancel".to_string();
    };
    let mut s = String::from("no session here: use circitor wait ");
    if let Some(pr) = full
        .map(PrName::to_string)
        .or_else(|| typed.map(str::to_string))
    {
        s.push_str(&pr);
        s.push(' ');
    }
    s.push_str(&render::flags(cond));
    if let Some(since) = &cond.since {
        s.push_str(" --since ");
        s.push_str(since);
    }
    s
}

fn failure(code: i32, text: String) -> Output {
    Output {
        stderr: with_newline(text),
        code,
        ..Output::default()
    }
}

fn conn_failure(e: ConnError, warn: String) -> Output {
    Output {
        stderr: warn + &e.stderr,
        code: e.code,
        ..Output::default()
    }
}

fn with_newline(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn ctx(env: &CliEnv) -> RenderCtx {
    RenderCtx {
        now: env.clock.wall(),
        tz: env.tz,
        cli_version: env.version.clone(),
    }
}
