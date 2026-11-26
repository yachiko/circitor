//! The command line (T11). CLI-1, CLI-12: `parse` never prints, never exits, never asks.

use clap::{Args, ColorChoice, Parser, Subcommand, error::ErrorKind};

use crate::api::{Ask, Until};
use crate::config::Env;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    List {
        all: bool,
        since: Option<String>,
        json: bool,
    },
    Show {
        pr: Option<String>,
        fresh: bool,
        json: bool,
    },
    /// `Ask`: `api.md`; `--fail-fast` and `--kind` are in it (D8).
    Wait {
        pr: Option<String>,
        cond: Ask,
        timeout: Option<u32>,
        json: bool,
    },
    /// NTF-11.
    Notify {
        pr: Option<String>,
        what: NotifyWhat,
        json: bool,
    },
    Changes {
        pr: Option<String>,
        since: Option<String>,
        json: bool,
    },
    Track {
        pr: String,
        json: bool,
    },
    Untrack {
        pr: String,
        json: bool,
    },
    Refresh {
        json: bool,
    },
    Status {
        json: bool,
    },
    Stop,
    Daemon {
        spawned: bool,
        check_config: bool,
    },
    Guide,
    Version,
    Help(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum NotifyWhat {
    /// `life`: `--for`, 60 to 86 400.
    Register {
        cond: Ask,
        life: Option<u32>,
    },
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub command: Command,
    pub no_spawn: bool,
}

const SEE: &str = "; see: circitor help";
const NO_SPAWN_VAR: &str = "CIRCITOR_NO_SPAWN";

#[derive(Parser)]
#[command(
    name = "circitor",
    version = "0",
    about = "Watches your GitHub pull requests for coding agents",
    color = ColorChoice::Never,
    subcommand_required = true,
    disable_help_subcommand = true
)]
struct Cli {
    /// Never start a daemon (CLI-7)
    #[arg(long, global = true)]
    no_spawn: bool,
    #[command(subcommand)]
    command: Sub,
}

#[derive(Args)]
struct CondArgs {
    /// merged, checks-done or changed
    #[arg(long)]
    until: Option<String>,
    /// The head's SHA, 7 to 40 hex characters (checks-done)
    #[arg(long)]
    sha: Option<String>,
    /// A cursor from before CI started again (checks-done)
    #[arg(long)]
    rerun_since: Option<String>,
    /// End at the first failed check (checks-done)
    #[arg(long)]
    fail_fast: bool,
    /// The kinds of event that count (changed)
    #[arg(long)]
    kind: Option<String>,
    /// A cursor: where the answer's events start
    #[arg(long)]
    since: Option<String>,
}

#[derive(Subcommand)]
enum Sub {
    /// The tracked PRs
    List {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// One PR, with its checks
    Show {
        pr: Option<String>,
        #[arg(long)]
        fresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Block until a condition holds
    Wait {
        pr: Option<String>,
        #[command(flatten)]
        cond: CondArgs,
        /// Seconds, 0 to 3600 (default 100)
        #[arg(long)]
        timeout: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// Ask to be told once, in this Claude Code session
    Notify {
        pr: Option<String>,
        #[command(flatten)]
        cond: CondArgs,
        /// The subscription's life in seconds, 60 to 86400
        #[arg(long = "for")]
        life: Option<u32>,
        /// Drop this session's subscriptions on the PR
        #[arg(long)]
        cancel: bool,
        #[arg(long)]
        json: bool,
    },
    /// What happened since a cursor
    Changes {
        pr: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Add a PR by hand
    Track {
        pr: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Remove a PR by hand
    Untrack {
        pr: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Poll now
    Refresh {
        #[arg(long)]
        json: bool,
    },
    /// The daemon's health
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Stop the daemon
    Stop,
    /// Run the daemon in the foreground
    Daemon {
        #[arg(long)]
        spawned: bool,
        #[arg(long)]
        check_config: bool,
    },
    /// Print the instructions for agents
    Guide,
    /// Print the version
    Version,
    /// Print this help
    Help,
}

/// CLI-5: a usage error is one line, then where to look.
fn usage(what: impl std::fmt::Display) -> String {
    let what = what.to_string();
    let line = what.lines().next().unwrap_or("").trim();
    format!("{line}{SEE}")
}

/// One line from clap's error: its first, without the `error: ` prefix.
fn clap_line(e: &clap::Error) -> String {
    let text = e.render().to_string();
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("wrong usage");
    let first = first.strip_prefix("error: ").unwrap_or(first);
    let first: String = first.chars().filter(|c| !c.is_control()).collect();
    format!("{first}{SEE}")
}

fn help_text() -> String {
    use clap::CommandFactory;
    Cli::command().render_help().to_string()
}

fn hex_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// API-10's checks that belong to the command line. Cursors and `kind`'s words are the daemon's.
fn ask(c: CondArgs) -> Result<Ask, String> {
    let word = c
        .until
        .ok_or_else(|| usage("--until is required: merged, checks-done or changed"))?;
    let until = Until::parse(&word).ok_or_else(|| {
        usage(format!(
            "--until {word:?} isn't one of merged, checks-done, changed"
        ))
    })?;
    if let Some(sha) = &c.sha
        && !hex_sha(sha)
    {
        return Err(usage("--sha is 7 to 40 hex characters"));
    }
    if until != Until::ChecksDone {
        for (set, flag) in [
            (c.sha.is_some(), "--sha"),
            (c.rerun_since.is_some(), "--rerun-since"),
            (c.fail_fast, "--fail-fast"),
        ] {
            if set {
                return Err(usage(format!("{flag} goes with --until checks-done only")));
            }
        }
    }
    if until != Until::Changed && c.kind.is_some() {
        return Err(usage("--kind goes with --until changed only"));
    }
    let mut a = Ask::new(until);
    a.sha = c.sha;
    a.rerun_since = c.rerun_since;
    a.fail_fast = c.fail_fast;
    a.kind = c.kind;
    a.since = c.since;
    Ok(a)
}

fn command(sub: Sub) -> Result<Command, String> {
    Ok(match sub {
        Sub::List { all, since, json } => Command::List { all, since, json },
        Sub::Show { pr, fresh, json } => Command::Show { pr, fresh, json },
        Sub::Wait {
            pr,
            cond,
            timeout,
            json,
        } => {
            let cond = ask(cond)?;
            // API-12
            if timeout.is_some_and(|t| t > 3600) {
                return Err(usage("--timeout is 0 to 3600 seconds"));
            }
            Command::Wait {
                pr,
                cond,
                timeout,
                json,
            }
        }
        Sub::Notify {
            pr,
            cond,
            life,
            cancel,
            json,
        } => {
            // NTF-11
            if cancel {
                let others = cond.until.is_some()
                    || cond.sha.is_some()
                    || cond.rerun_since.is_some()
                    || cond.fail_fast
                    || cond.kind.is_some()
                    || cond.since.is_some()
                    || life.is_some();
                if others {
                    return Err(usage("--cancel takes no condition and no --for"));
                }
                Command::Notify {
                    pr,
                    what: NotifyWhat::Cancel,
                    json,
                }
            } else {
                if cond.until.is_none() {
                    return Err(usage("notify needs --until, or --cancel"));
                }
                let cond = ask(cond)?;
                // NTF-2
                if life.is_some_and(|l| !(60..=86_400).contains(&l)) {
                    return Err(usage("--for is 60 to 86400 seconds"));
                }
                Command::Notify {
                    pr,
                    what: NotifyWhat::Register { cond, life },
                    json,
                }
            }
        }
        Sub::Changes { pr, since, json } => Command::Changes { pr, since, json },
        Sub::Track { pr, json } => Command::Track {
            pr: pr.ok_or_else(|| usage("track needs a PR: owner/repo#12, its URL or a number"))?,
            json,
        },
        Sub::Untrack { pr, json } => Command::Untrack {
            pr: pr
                .ok_or_else(|| usage("untrack needs a PR: owner/repo#12, its URL or a number"))?,
            json,
        },
        Sub::Refresh { json } => Command::Refresh { json },
        Sub::Status { json } => Command::Status { json },
        Sub::Stop => Command::Stop,
        Sub::Daemon {
            spawned,
            check_config,
        } => {
            if spawned && check_config {
                return Err(usage("daemon takes --spawned or --check-config, not both"));
            }
            Command::Daemon {
                spawned,
                check_config,
            }
        }
        Sub::Guide => Command::Guide,
        Sub::Version => Command::Version,
        Sub::Help => Command::Help(help_text()),
    })
}

/// `Err` is a usage error: its text for stderr, exit code 2 (CLI-5).
pub fn parse(args: &[String], env: &Env) -> Result<Parsed, String> {
    // CLI-1, CLI-12
    let argv = std::iter::once("circitor".to_string()).chain(args.iter().cloned());
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp => Ok(Parsed {
                    command: Command::Help(e.render().to_string()),
                    no_spawn: false,
                }),
                ErrorKind::DisplayVersion => Ok(Parsed {
                    command: Command::Version,
                    no_spawn: false,
                }),
                _ => Err(clap_line(&e)),
            };
        }
    };
    let command = command(cli.command)?;
    // CLI-7: `CIRCITOR_NO_SPAWN` set to exactly `1`.
    let no_spawn = cli.no_spawn || env.get(NO_SPAWN_VAR) == Some("1");
    Ok(Parsed { command, no_spawn })
}

#[cfg(test)]
mod tests;
