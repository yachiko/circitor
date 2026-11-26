//! The commands (T27): parse, name the PR, reach the daemon, send, render, exit (CLI-1 to CLI-10).
//!
//! `run` reads nothing but its `CliEnv` and prints nothing: everything goes into the `Output` (CLI-13).

use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::api::{
    ApiError, ChangesBody, Envelope, ErrorBody, ErrorCode, ListBody, RecordBody, RefreshBody,
    Request, StatusBody,
};
use crate::cli::args::{self, Command};
use crate::cli::conn::{self, ConnError, Link, Reply};
use crate::cli::guide::GUIDE;
use crate::cli::naming::{self, BranchMatch, PrArg};
use crate::cli::render::{self, RenderCtx};
use crate::cli::{Branch, CliEnv, Interrupt, Output, exit};
use crate::config::{self, Env, Paths, Platform};
use crate::model::{PrName, PrState, Record};
use crate::poller::PollOutcome;

mod git;
#[cfg(test)]
mod stub;
#[cfg(test)]
mod tests;
mod wait;

pub use git::RealGit;

/// CLI-3: `changes` and `wait` print at most this many events (API-8's `limit`).
const EVENT_LIMIT: u32 = 50;
/// CLI-7: any request but a wait.
const REQUEST_LIMIT: Duration = Duration::from_secs(90);
/// CLI-2: the forms a PR can be named in.
const FORMS: &str = "use owner/repo#12, the PR's URL, or its number";

impl CliEnv {
    /// The process's own clock, directory, environment, platform, version, spawner, `git` and signals.
    pub fn real() -> CliEnv {
        use chrono::{Offset, TimeZone};
        let clock = crate::clock::SystemClock::new();
        let now = crate::clock::Clock::wall(&*clock).inner().naive_utc();
        // CLI-3: the local offset, from the clock's own wall time.
        let tz = chrono::Local.offset_from_utc_datetime(&now).fix();
        let env = Env::from_process();
        CliEnv {
            clock,
            tz,
            cwd: std::env::current_dir().unwrap_or_default(),
            request_limit: request_limit(&env),
            env,
            platform: Platform::current(),
            version: crate::version::version(),
            spawner: std::sync::Arc::new(conn::RealSpawner),
            git: std::sync::Arc::new(RealGit),
            interrupt: Interrupt::never(),
        }
    }
}

/// CLI-7's 90 s; TST-1: a test build lets `CIRCITOR_TEST_CLI_LIMIT` (seconds) replace it.
fn request_limit(env: &Env) -> Duration {
    #[cfg(feature = "test-endpoint")]
    if let Some(d) = env
        .get("CIRCITOR_TEST_CLI_LIMIT")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
    {
        return Duration::from_secs_f64(d);
    }
    let _ = env;
    REQUEST_LIMIT
}

/// `cli::run`'s body.
pub async fn run(args: &[String], env: &CliEnv) -> Output {
    let mut interrupt = env.interrupt.clone();
    // CLI-6, CLI-5: `SIGINT` and `SIGTERM` end the command at once, with nothing printed.
    tokio::select! {
        biased;
        code = interrupt.wait() => Output { code, ..Output::default() },
        out = command(args, env) => out,
    }
}

async fn command(args: &[String], env: &CliEnv) -> Output {
    let parsed = match args::parse(args, &env.env) {
        Ok(p) => p,
        // CLI-5: a usage error is its line and exit 2.
        Err(line) => return usage(line),
    };
    let no_spawn = parsed.no_spawn;
    // CLI-8: these resolve no path and open no socket.
    match parsed.command {
        Command::Version => return stdout(format!("circitor {}\n", env.version)),
        Command::Help(text) => return stdout(text),
        Command::Guide => return stdout(GUIDE.to_string()),
        Command::Notify { pr, what, json } => {
            // NTF-11: T48's, whole; it exits 11 before anything else when there's no session.
            return crate::cli::notify::run(pr.as_deref(), &what, json, no_spawn, env).await;
        }
        Command::Daemon {
            check_config: false,
            ..
        } => {
            return failure(
                exit::ERROR,
                "circitor daemon runs only from the binary's entry".into(),
            );
        }
        _ => {}
    }
    let paths = match config::resolve(&env.env, env.platform) {
        Ok(p) => p,
        Err(e) => return failure(exit::NO_DAEMON, e.to_string()),
    };
    match parsed.command {
        Command::Daemon { .. } => check_config(&paths).await,
        Command::List { all, since, json } => {
            let with_since = since.is_some();
            let req = Request::List {
                all,
                since,
                checks: false,
            };
            simple(
                env,
                &paths,
                no_spawn,
                &req,
                json,
                |e: &Envelope<ListBody>, ctx| (render::list(e, with_since, ctx), exit::OK),
            )
            .await
        }
        Command::Show { pr, fresh, json } => show(env, &paths, no_spawn, pr, fresh, json).await,
        Command::Changes { pr, since, json } => {
            // CLI-1: `changes` without a PR covers every PR; it never takes the branch's.
            let pr = match pr {
                None => None,
                Some(arg) => match name_given(env, &arg).await {
                    Ok(n) => Some(n),
                    Err(out) => return out,
                },
            };
            let req = Request::Changes {
                pr: pr.clone(),
                since,
                limit: Some(EVENT_LIMIT),
            };
            simple(
                env,
                &paths,
                no_spawn,
                &req,
                json,
                |e: &Envelope<ChangesBody>, ctx| (render::changes(e, pr.as_ref(), ctx), exit::OK),
            )
            .await
        }
        Command::Track { pr, json } => track(env, &paths, no_spawn, &pr, json, true).await,
        Command::Untrack { pr, json } => track(env, &paths, no_spawn, &pr, json, false).await,
        // U10: the CLI controls the daemon: refresh, status, stop.
        Command::Refresh { json } => {
            simple(
                env,
                &paths,
                no_spawn,
                &Request::Refresh,
                json,
                |e: &Envelope<RefreshBody>, ctx| {
                    // API-16: a problem in a 200 exits as the API error of that name.
                    (
                        render::refresh(e, ctx),
                        render::refresh_code(e.body.outcome),
                    )
                },
            )
            .await
        }
        Command::Status { json } => status(env, &paths, json).await,
        Command::Stop => stop(env, &paths).await,
        Command::Wait {
            pr,
            cond,
            timeout,
            json,
        } => wait::wait(env, &paths, no_spawn, pr, cond, timeout, json).await,
        Command::Version | Command::Help(_) | Command::Guide | Command::Notify { .. } => {
            unreachable!("answered above")
        }
    }
}

/// `cli::main`'s body.
pub fn main() -> i32 {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // `daemon` without `--check-config` is handed over before any runtime is built: it builds its own.
    if let Ok(p) = args::parse(&args, &Env::from_process())
        && let Command::Daemon {
            spawned,
            check_config: false,
        } = p.command
    {
        return crate::daemon::run::main(spawned);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            write_out(&Output {
                stderr: format!("can't start: {e}\n"),
                code: exit::ERROR,
                ..Output::default()
            });
            return exit::ERROR;
        }
    };
    let out = runtime.block_on(async {
        let mut env = CliEnv::real();
        let (trigger, interrupt) = Interrupt::pair();
        env.interrupt = interrupt;
        // CLI-6: `SIGINT` and `SIGTERM` end the command at once.
        for (kind, code) in [
            (tokio::signal::unix::SignalKind::interrupt(), exit::SIGINT),
            (tokio::signal::unix::SignalKind::terminate(), exit::SIGTERM),
        ] {
            if let Ok(mut signal) = tokio::signal::unix::signal(kind) {
                let trigger = trigger.clone();
                tokio::spawn(async move {
                    if signal.recv().await.is_some() {
                        trigger.fire(code);
                    }
                });
            }
        }
        run(&args, &env).await
    });
    write_out(&out);
    // The connections' tasks and the signal handlers aren't waited for.
    runtime.shutdown_background();
    out.code
}

/// Writes both streams; a closed pipe is ignored (`println!` would panic on one).
fn write_out(out: &Output) {
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(out.stdout.as_bytes());
    let _ = so.flush();
    let mut se = std::io::stderr().lock();
    let _ = se.write_all(out.stderr.as_bytes());
    let _ = se.flush();
}

// ---- Outputs ----

fn line(s: impl Into<String>) -> String {
    let mut s = s.into();
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn stdout(text: String) -> Output {
    Output {
        stdout: text,
        ..Output::default()
    }
}

fn failure(code: i32, text: String) -> Output {
    Output {
        stderr: line(text),
        code,
        ..Output::default()
    }
}

/// CLI-5: exit 2, wrong usage or a PR that couldn't be named.
fn usage(text: String) -> Output {
    failure(exit::USAGE, text)
}

/// A `ConnError`: its lines and its code, after the warnings.
fn conn_failure(e: ConnError, warn: String) -> Output {
    Output {
        stdout: String::new(),
        stderr: warn + &e.stderr,
        code: e.code,
    }
}

fn ctx(env: &CliEnv) -> RenderCtx {
    RenderCtx {
        now: env.clock.wall(),
        tz: env.tz,
        cli_version: env.version.clone(),
    }
}

/// CLI-4: the body as it came, on one line.
fn body_line(body: &[u8]) -> String {
    line(String::from_utf8_lossy(body).trim_end_matches('\n'))
}

/// CLI-5: any answer that isn't a 200: its API-3 error, one line, its code. One that doesn't decode is 1.
fn error_answer(status: u16, body: &[u8], older: bool, ctx: &RenderCtx) -> (String, i32) {
    match serde_json::from_slice::<ErrorBody>(body) {
        Ok(e) => (
            render::api_error(&e.error, older, ctx),
            // CLI-9: `bad_request` and `unknown_route` from an older daemon kept are 1.
            render::error_code(e.error.code, older),
        ),
        // A newer daemon's code this CLI doesn't know is 1 (CLI-5).
        Err(_) => (
            line(format!(
                "the daemon answered HTTP {status} with an error this CLI can't read"
            )),
            exit::ERROR,
        ),
    }
}

/// CLI-5: a 200 whose body isn't what this CLI expects is an unexpected error.
fn undecodable() -> (String, i32) {
    (
        line("the daemon's answer isn't one this CLI can read"),
        exit::ERROR,
    )
}

/// An answer, whole: a 200 through `render` to stdout, anything else through `error_answer` to stderr;
/// with `--json`, stdout is the body as it came either way, and only the code is decoded for (CLI-4).
fn answer<T: DeserializeOwned>(
    reply: &Reply,
    json: bool,
    older: bool,
    ctx: &RenderCtx,
    warn: String,
    render: impl FnOnce(&Envelope<T>, &RenderCtx) -> (String, i32),
) -> Output {
    let mut out = Output {
        stderr: warn,
        ..Output::default()
    };
    if reply.status == 200 {
        match serde_json::from_slice::<Envelope<T>>(&reply.body) {
            Ok(e) => {
                let (text, code) = render(&e, ctx);
                out.code = code;
                if !json {
                    out.stdout = text;
                }
            }
            Err(_) => {
                let (text, code) = undecodable();
                out.stderr.push_str(&text);
                out.code = code;
            }
        }
    } else {
        let (text, code) = error_answer(reply.status, &reply.body, older, ctx);
        out.stderr.push_str(&text);
        out.code = code;
    }
    if json {
        out.stdout = body_line(&reply.body);
    }
    out
}

/// `list`, `changes`, `track`, `untrack`, `refresh`: `ensure`, one request, its answer.
async fn simple<T: DeserializeOwned>(
    env: &CliEnv,
    paths: &Paths,
    no_spawn: bool,
    req: &Request,
    json: bool,
    render: impl FnOnce(&Envelope<T>, &RenderCtx) -> (String, i32),
) -> Output {
    let mut warn = String::new();
    let mut link = match conn::ensure(env, paths, no_spawn, None, &mut warn).await {
        Ok(l) => l,
        Err(e) => return conn_failure(e, warn),
    };
    send(env, &mut link, req, json, warn, render).await
}

async fn send<T: DeserializeOwned>(
    env: &CliEnv,
    link: &mut Link,
    req: &Request,
    json: bool,
    warn: String,
    render: impl FnOnce(&Envelope<T>, &RenderCtx) -> (String, i32),
) -> Output {
    match link.request(env, req).await {
        Ok(reply) => answer(
            &reply,
            json,
            link.older_unreplaced(),
            &ctx(env),
            warn,
            render,
        ),
        Err(e) => conn_failure(e, warn),
    }
}

// ---- The commands that aren't one request ----

async fn show(
    env: &CliEnv,
    paths: &Paths,
    no_spawn: bool,
    pr: Option<String>,
    fresh: bool,
    json: bool,
) -> Output {
    let named = match name_offline(env, pr.as_deref()).await {
        Ok(n) => n,
        Err(out) => return out,
    };
    let mut warn = String::new();
    let mut link = match conn::ensure(env, paths, no_spawn, None, &mut warn).await {
        Ok(l) => l,
        Err(e) => return conn_failure(e, warn),
    };
    let pr = match named {
        Some(n) => n,
        None => match find_bare(env, &mut link, &mut warn).await {
            Ok(n) => n,
            Err(out) => return prefixed(warn, out),
        },
    };
    let req = Request::Show { pr, fresh };
    send(
        env,
        &mut link,
        &req,
        json,
        warn,
        |e: &Envelope<RecordBody>, ctx| (render::show(e, ctx), exit::OK),
    )
    .await
}

/// `track` and `untrack` (API-15): `render::tracked` names the PR given when the answer has no record.
async fn track(
    env: &CliEnv,
    paths: &Paths,
    no_spawn: bool,
    arg: &str,
    json: bool,
    track: bool,
) -> Output {
    let pr = match name_given(env, arg).await {
        Ok(n) => n,
        Err(out) => return out,
    };
    let req = if track {
        Request::Track { pr: pr.clone() }
    } else {
        Request::Untrack { pr: pr.clone() }
    };
    simple(
        env,
        paths,
        no_spawn,
        &req,
        json,
        |e: &Envelope<RecordBody>, ctx| (render::tracked(e, &pr, ctx), exit::OK),
    )
    .await
}

/// `warn` (and the naming note) before what `out` has on stderr.
fn prefixed(warn: String, mut out: Output) -> Output {
    out.stderr = warn + &out.stderr;
    out
}

/// CLI-8: `status` never starts or replaces a daemon; with none running it says so and exits 6.
async fn status(env: &CliEnv, paths: &Paths, json: bool) -> Output {
    let mut warn = String::new();
    let mut link = match conn::existing(env, paths, &mut warn).await {
        Ok(Some(l)) => l,
        Ok(None) => return prefixed(warn, failure(exit::NO_DAEMON, "not running".into())),
        Err(e) => return conn_failure(e, warn),
    };
    send(
        env,
        &mut link,
        &Request::Status,
        json,
        warn,
        |e: &Envelope<StatusBody>, ctx| (render::status(e, ctx), exit::OK),
    )
    .await
}

/// CLI-8: `stop` exits 0, whether a daemon was running or not.
async fn stop(env: &CliEnv, paths: &Paths) -> Output {
    let mut warn = String::new();
    match conn::stop(env, paths, &mut warn).await {
        Ok(was_running) => Output {
            stdout: render::stop(was_running),
            stderr: warn,
            code: exit::OK,
        },
        Err(e) => conn_failure(e, warn),
    }
}

/// OPS-1: `daemon --check-config`: exit 0 when the file is missing or valid, else its one line and 1.
async fn check_config(paths: &Paths) -> Output {
    let file = paths.config_file.clone();
    let checked = tokio::task::spawn_blocking(move || config::load(&file).map(|_| ())).await;
    match checked {
        Ok(Ok(())) => Output::default(),
        Ok(Err(e)) => failure(exit::ERROR, e.to_string()),
        Err(e) => failure(exit::ERROR, format!("the config check failed: {e}")),
    }
}

// ---- Naming a PR (CLI-2) ----

/// A PR argument that must name one: `track`, `untrack`, `changes PR`.
async fn name_given(env: &CliEnv, arg: &str) -> Result<PrName, Output> {
    match name_offline(env, Some(arg)).await? {
        Some(n) => Ok(n),
        None => Err(usage(format!("no PR named; {FORMS}"))),
    }
}

/// The repository of `origin` and the branch, for a bare name. `Err`: exit 2 and its line.
async fn repo_and_branch(env: &CliEnv) -> Result<(String, String, String), Output> {
    let not_here = || {
        usage(format!(
            "{} isn't a git repository with a github.com origin remote; {FORMS} (owner/repo#12 or the URL outside one)",
            env.cwd.display()
        ))
    };
    let Some((owner, repo)) = env
        .git
        .origin_url(&env.cwd)
        .await
        .as_deref()
        .and_then(naming::repo_of_origin)
    else {
        return Err(not_here());
    };
    match env.git.push_branch(&env.cwd).await {
        Branch::Named(b) => Ok((owner, repo, b)),
        // CLI-2: a detached HEAD is exit 2.
        Branch::Detached => Err(usage(format!(
            "HEAD is detached in {}: no branch to find a PR by; {FORMS}",
            env.cwd.display()
        ))),
        Branch::NoRepo => Err(not_here()),
    }
}

/// CLI-2 before any request: `Full` as it is, `Number` with `origin`'s repository. `Ok(None)` is `Bare`,
/// to be found with `find_bare`; `Err` is the whole `Output` (exit 2 and its line).
pub async fn name_offline(env: &CliEnv, arg: Option<&str>) -> Result<Option<PrName>, Output> {
    match naming::pr_arg(arg) {
        Err(line) => Err(usage(line)),
        // CLI-2: a full name or a URL is used as it is: no `git`.
        Ok(PrArg::Full(name)) => Ok(Some(name)),
        Ok(PrArg::Number(number)) => {
            // CLI-2: `12` takes its repository from `origin`, before any request.
            let origin = env.git.origin_url(&env.cwd).await;
            match origin.as_deref().and_then(naming::repo_of_origin) {
                Some((owner, repo)) => Ok(Some(PrName {
                    owner,
                    repo,
                    number,
                })),
                None => Err(usage(format!(
                    "{number}: {} has no github.com origin remote to take the repository from; use owner/repo#{number}",
                    env.cwd.display()
                ))),
            }
        }
        Ok(PrArg::Bare) => {
            // CLI-2: outside a repository, or on a detached HEAD, nothing is sent and nothing started.
            repo_and_branch(env).await?;
            Ok(None)
        }
    }
}

/// One request through `link`, decoded; an answer that isn't a 200, or doesn't decode, is the whole
/// `Output` of the command.
async fn fetch<T: DeserializeOwned>(
    env: &CliEnv,
    link: &mut Link,
    req: &Request,
) -> Result<Envelope<T>, Output> {
    let reply = link
        .request(env, req)
        .await
        .map_err(|e| conn_failure(e, String::new()))?;
    if reply.status != 200 {
        let (text, code) = error_answer(
            reply.status,
            &reply.body,
            link.older_unreplaced(),
            &ctx(env),
        );
        return Err(failure(code, text));
    }
    serde_json::from_slice(&reply.body).map_err(|_| {
        let (text, code) = undecodable();
        failure(code, text)
    })
}

/// What `find_bare` reads of a status: only `github.login` (API-5). Anything else is ignored.
#[derive(serde::Deserialize)]
struct LoginOnly {
    github: LoginGithub,
}
#[derive(serde::Deserialize)]
struct LoginGithub {
    login: Option<String>,
}

/// What the refresh answered, as far as the search cares.
enum PollAnswer {
    Done,
    Shared,
    Problem(PollOutcome, ApiError),
    Error(i32, String),
}

fn several(names: &[PrName], branch: &str) -> Output {
    let list: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    usage(format!(
        "several open PRs on branch {branch}: {}; name one of them",
        list.join(", ")
    ))
}

/// CLI-2's flow for no argument, against a daemon: the list, the login, the poll, the list again, the
/// choice. `note` gets the line for an ended match. `Err` is the whole `Output`.
// The contract's signature: the callee appends lines to the caller's text.
#[allow(clippy::ptr_arg)]
pub async fn find_bare(env: &CliEnv, link: &mut Link, note: &mut String) -> Result<PrName, Output> {
    let (owner, repo, branch) = repo_and_branch(env).await?;
    let all = Request::List {
        all: true,
        since: None,
        checks: false,
    };
    // `list` first: it waits for the first poll (API-18); then the login (API-5).
    let first: Envelope<ListBody> = fetch(env, link, &all).await?;
    let login = fetch::<LoginOnly>(env, link, &Request::Status)
        .await?
        .body
        .github
        .login;
    let login = login.as_deref();
    match naming::match_branch(&first.body.records, &owner, &repo, &branch, login, false) {
        BranchMatch::Open(n) => return Ok(n),
        BranchMatch::Several(v) => return Err(several(&v, &branch)),
        BranchMatch::Ended(_) | BranchMatch::None => {}
    }
    // CLI-2: no open match: a poll, then the list again, whatever the poll answered.
    let polled = match link.request(env, &Request::Refresh).await {
        Ok(reply) if reply.status == 200 => {
            match serde_json::from_slice::<Envelope<RefreshBody>>(&reply.body) {
                Ok(e) => match e.body.outcome {
                    PollOutcome::Polled => PollAnswer::Done,
                    PollOutcome::Shared => PollAnswer::Shared,
                    problem => {
                        let code = match problem {
                            PollOutcome::Auth => ErrorCode::Auth,
                            PollOutcome::RateLimited => ErrorCode::RateLimited,
                            _ => ErrorCode::GithubUnavailable,
                        };
                        let message = e
                            .daemon
                            .message
                            .clone()
                            .unwrap_or_else(|| format!("the poll stopped on {}", word(&code)));
                        PollAnswer::Problem(
                            problem,
                            ApiError {
                                code,
                                message,
                                until: e.daemon.until,
                            },
                        )
                    }
                },
                Err(_) => {
                    let (text, code) = undecodable();
                    PollAnswer::Error(code, text)
                }
            }
        }
        Ok(reply) => {
            let (text, code) = error_answer(
                reply.status,
                &reply.body,
                link.older_unreplaced(),
                &ctx(env),
            );
            PollAnswer::Error(code, text)
        }
        Err(e) => PollAnswer::Error(e.code, e.stderr),
    };
    let second: Envelope<ListBody> = fetch(env, link, &all).await?;
    let records = &second.body.records;
    match naming::match_branch(records, &owner, &repo, &branch, login, true) {
        BranchMatch::Open(n) => Ok(n),
        BranchMatch::Ended(n) => {
            // CLI-2: an ended match is taken, and the output says so.
            let state = records
                .iter()
                .find(|r| r.pr == n)
                .map_or("ended".to_string(), |r: &Record| ended_word(r.state));
            note.push_str(&line(format!(
                "no open PR on branch {branch}: using {n}, which is {state}"
            )));
            Ok(n)
        }
        BranchMatch::Several(v) => Err(several(&v, &branch)),
        BranchMatch::None => {
            let who = login.unwrap_or("the login in use");
            let none = format!("no tracked PR by {who} on branch {branch} of {owner}/{repo}");
            Err(match polled {
                // CLI-2: after a problem, that problem's code and line.
                PollAnswer::Problem(outcome, e) => failure(
                    render::refresh_code(outcome),
                    render::api_error(&e, false, &ctx(env)),
                ),
                PollAnswer::Error(code, text) => failure(code, text),
                // CLI-2: after `shared`, a PR opened seconds ago may not be found yet.
                PollAnswer::Shared => usage(format!(
                    "{none}; a PR opened seconds ago may not be found yet: name it by number, as in circitor show 12"
                )),
                // CLI-2: after the poll, exit 2 means GitHub was read and no PR matches.
                PollAnswer::Done => usage(format!("{none}; {FORMS}")),
            })
        }
    }
}

fn ended_word(state: PrState) -> String {
    word(&state)
}

/// A serde enum's own word (`merged`, `rate_limited`).
fn word<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The socket for CLI-6's lines when no `Link` was made.
fn socket_of<'a>(link: Option<&'a Link>, paths: &'a Paths) -> &'a Path {
    link.map_or(paths.socket.as_path(), |l| l.socket())
}
