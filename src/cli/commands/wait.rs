//! `wait` (CLI-6, CLI-7, CLI-10): one deadline for the whole command, the CLI's own limit around the
//! head and the body, the first cursor kept, one repeat when the connection drops.

use std::time::Duration;

use crate::api::{Ask, Envelope, Request, WaitBody};
use crate::cli::conn::{self, Link, Reply};
use crate::cli::render::{self, NoAnswer, WaitAsk};
use crate::cli::{CliEnv, DEFAULT_WAIT_TIMEOUT, Output};
use crate::clock::{Mono, timeout};
use crate::config::Paths;

use super::{EVENT_LIMIT, answer, conn_failure, ctx, find_bare, name_offline, prefixed, socket_of};

/// CLI-7: a wait's limit is its deadline plus this.
const PAST_DEADLINE: Duration = Duration::from_secs(10);
/// CLI-6: `--timeout 0`'s limit on the CLI's side.
const ZERO_LIMIT: Duration = Duration::from_secs(60);
/// CLI-6: a request with under this left isn't sent.
const LEAST: Duration = Duration::from_secs(1);

/// How one wait request ended.
enum Sent {
    /// The whole answer: its status and body.
    Answer(u16, Vec<u8>),
    /// The connection dropped; `after_cursor` when this request's 200 head had arrived.
    Dropped { after_cursor: bool },
    /// The CLI's own limit passed (CLI-7).
    Limit,
}

/// The command's clock: its start, its deadline, its limit.
struct Times {
    zero: bool,
    deadline: Mono,
    limit: Mono,
}

impl Times {
    /// What's left for `ensure` and `reconnect`: of the deadline, or of 60 s for `--timeout 0`.
    fn budget(&self, now: Mono) -> Duration {
        if self.zero {
            self.limit.since(now)
        } else {
            self.deadline.since(now)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn wait(
    env: &CliEnv,
    paths: &Paths,
    no_spawn: bool,
    pr: Option<String>,
    cond: Ask,
    given: Option<u32>,
    json: bool,
) -> Output {
    let clock = &env.clock;
    // CLI-6: one deadline for the whole command, from its start.
    let start = clock.mono();
    let secs = given.unwrap_or(DEFAULT_WAIT_TIMEOUT);
    let zero = given == Some(0);
    let deadline = start.plus(Duration::from_secs(u64::from(secs)));
    // CLI-7: the deadline plus 10 s; `--timeout 0`, 60 s.
    let limit = if zero {
        start.plus(ZERO_LIMIT)
    } else {
        deadline.plus(PAST_DEADLINE)
    };
    let times = Times {
        zero,
        deadline,
        limit,
    };

    let mut warn = String::new();
    let named = match name_offline(env, pr.as_deref()).await {
        Ok(n) => n,
        Err(out) => return out,
    };
    let mut link: Option<Link> = None;
    let pr = match named {
        Some(n) => n,
        None => {
            // CLI-6: naming counts against the deadline; `ensure`'s budget is what's left.
            let budget = times.budget(clock.mono());
            let mut l = match conn::ensure(env, paths, no_spawn, Some(budget), &mut warn).await {
                Ok(l) => l,
                // No PR is named yet: there's no `again:` to print, only the connection's lines.
                Err(e) => return conn_failure(e, warn),
            };
            let found = find_bare(env, &mut l, &mut warn).await;
            match found {
                Ok(n) => {
                    link = Some(l);
                    n
                }
                Err(out) => return prefixed(warn, out),
            }
        }
    };
    let ask = WaitAsk {
        pr: pr.clone(),
        cond: cond.clone(),
        timeout: secs,
        timeout_given: given.is_some(),
        json,
    };
    // CLI-6: the first `circitor-cursor` of the command, never a later one.
    let mut first_cursor: Option<String> = None;
    // CLI-10: one repeat for the whole command.
    let mut repeated = false;

    let ending = |kind: NoAnswer, first: &Option<String>, link: Option<&Link>, warn: &str| {
        let cursor = cond.since.as_deref().or(first.as_deref());
        let text = render::wait_no_answer(kind, &ask, cursor, socket_of(link, paths));
        let mut out = Output {
            stderr: warn.to_string(),
            code: render::no_answer_code(kind),
            ..Output::default()
        };
        // With `--json` the three endings go to stderr: stdout holds only bodies (CLI-4).
        if json {
            out.stderr.push_str(&text);
        } else {
            out.stdout = text;
        }
        out
    };

    loop {
        let l = match link.take() {
            Some(l) => l,
            None => {
                let budget = times.budget(clock.mono());
                if !zero && budget < LEAST {
                    return ending(NoAnswer::NoTimeLeft, &first_cursor, None, &warn);
                }
                match conn::ensure(env, paths, no_spawn, Some(budget), &mut warn).await {
                    Ok(l) => l,
                    // CLI-6: time running out inside `ensure` is a timeout, not exit 6.
                    Err(_) if !zero && times.budget(clock.mono()) < LEAST => {
                        return ending(NoAnswer::NoTimeLeft, &first_cursor, None, &warn);
                    }
                    Err(e) => return conn_failure(e, warn),
                }
            }
        };
        let now = clock.mono();
        let left = deadline.since(now);
        // CLI-6: a request with under 1 s left isn't sent, unless `--timeout 0` was given.
        if !zero && left < LEAST {
            return ending(NoAnswer::NoTimeLeft, &first_cursor, Some(&l), &warn);
        }
        let mut sent_cond = cond.clone();
        // CLI-10: the repeat is the `again:` command: `since` is the one given, or the first cursor.
        if sent_cond.since.is_none() {
            sent_cond.since = first_cursor.clone();
        }
        let req = Request::Wait {
            pr: pr.clone(),
            cond: sent_cond,
            // CLI-6: the whole seconds left, rounded down; `--timeout 0` always sends 0.
            timeout: if zero {
                0
            } else {
                u32::try_from(left.as_secs()).unwrap_or(u32::MAX)
            },
            limit: Some(EVENT_LIMIT),
        };
        // CLI-7: the limit around the head and the body.
        let sent = match timeout(clock, limit.since(now), once(&l, &req, &mut first_cursor)).await {
            Ok(s) => s,
            Err(_) => Sent::Limit,
        };
        match sent {
            Sent::Answer(status, body) => {
                return answered(env, &l, &ask, status, body, json, warn);
            }
            Sent::Limit => {
                return ending(NoAnswer::NotAnswering, &first_cursor, Some(&l), &warn);
            }
            Sent::Dropped { .. } if repeated => {
                // CLI-10: a second drop.
                return ending(NoAnswer::DaemonStopped, &first_cursor, Some(&l), &warn);
            }
            Sent::Dropped { after_cursor: true } => {
                repeated = true;
                // CLI-10: the cursor had arrived: start no daemon, wait up to 8 s for one to answer.
                let budget = times.budget(clock.mono());
                match conn::reconnect(env, paths, budget).await {
                    Ok(again) => link = Some(again),
                    Err(_) => {
                        return ending(NoAnswer::DaemonStopped, &first_cursor, Some(&l), &warn);
                    }
                }
            }
            Sent::Dropped {
                after_cursor: false,
            } => {
                // CLI-10: dropped before the cursor: tried once more as a new command, which may start one.
                repeated = true;
            }
        }
    }
}

/// One wait request: the head, the first cursor kept, then the body.
async fn once(link: &Link, req: &Request, first_cursor: &mut Option<String>) -> Sent {
    let Ok(head) = link.open(req).await else {
        return Sent::Dropped {
            after_cursor: false,
        };
    };
    let status = head.status();
    let had_cursor = status == 200;
    if status == 200
        && first_cursor.is_none()
        && let Some(c) = head.cursor()
    {
        *first_cursor = Some(c.to_string());
    }
    match head.body().await {
        Ok(body) => Sent::Answer(status, body),
        Err(_) => Sent::Dropped {
            after_cursor: had_cursor,
        },
    }
}

/// The wait's answer: a 200 is `render::wait` and `render::wait_code`; anything else an API error.
fn answered(
    env: &CliEnv,
    link: &Link,
    ask: &WaitAsk,
    status: u16,
    body: Vec<u8>,
    json: bool,
    warn: String,
) -> Output {
    let reply = Reply { status, body };
    answer(
        &reply,
        json,
        link.older_unreplaced(),
        &ctx(env),
        warn,
        |e: &Envelope<WaitBody>, ctx| {
            (
                render::wait(e, ask, ctx),
                render::wait_code(&e.body, ask.cond.until),
            )
        },
    )
}
