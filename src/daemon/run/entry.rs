//! `circitor daemon [--spawned]`: the real `Deps`, a runtime, `start`, then the end (OPS-6, OPS-7).

use std::sync::Arc;
use std::time::Duration;

use super::start;
use crate::clock::{Activity, SharedClock, SystemClock};
use crate::config::{self, Env, Platform, resolve};
use crate::daemon::{Deps, Mode};
use crate::github::Client;
use crate::log::{LogSink, Logger, StderrSink};
use crate::notify::RealPoster;
use crate::state::RealFs;
use crate::token::gh::GhTokenSource;
use crate::transport::RealProbe;

/// OPS-9.
const LOG_VAR: &str = "CIRCITOR_LOG";
/// TST-1.
#[cfg(feature = "test-endpoint")]
const STOP_DELAY_VAR: &str = "CIRCITOR_TEST_STOP_DELAY";
/// TST-1.
#[cfg(feature = "test-endpoint")]
const VERSION_VAR: &str = "CIRCITOR_TEST_VERSION";

/// `circitor daemon [--spawned]`: real `Deps` (config, paths, `SystemClock`, `github::client`, `token::gh`,
/// stderr), a runtime, `start`, then waits for the end. Returns the exit status.
pub fn main(spawned: bool) -> i32 {
    // OPS-7: status 1 is for a crash. A panic in any task, a tokio task's included, ends the process
    // with 1; the line holds where it happened, never its message.
    let hook_logger = Logger::new(
        SystemClock::new(),
        Arc::new(StderrSink),
        crate::log::Level::Error,
    );
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map_or_else(
            || "unknown".to_string(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        hook_logger.error(format_args!("panic at={at}"));
        std::process::exit(1);
    }));
    run_main(spawned, Env::from_process(), Arc::new(StderrSink))
}

/// `main` without the process: the environment and the log sink are given, the exit status returned.
/// Builds its own runtime; called from outside any.
pub(crate) fn run_main(spawned: bool, env: Env, sink: Arc<dyn LogSink>) -> i32 {
    let clock: SharedClock = SystemClock::new();
    // OPS-9: an unknown `CIRCITOR_LOG` is `info`, with a warning line.
    let (level, unknown) = Logger::level_from_env(env.get(LOG_VAR));
    let logger = Logger::new(clock, sink.clone(), level);
    if unknown.is_some() {
        logger.warn("log level unknown level=info");
    }
    let deps = match deps_from(spawned, &env, sink, &logger) {
        Ok(deps) => deps,
        Err(code) => return code,
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            logger.error(format_args!("runtime can't start: {e}"));
            return 1;
        }
    };
    runtime.block_on(async move {
        match start(deps).await {
            Ok(mut running) => running.exited().await,
            Err(e) => {
                logger.error(&e);
                e.code()
            }
        }
    })
}

/// The real `Deps`, without starting anything. A `PathError`, a `ConfigError` or an endpoint error is
/// one `error` line and `Err(1)`, with nothing created (OPS-1, OPS-4, TST-1).
pub(crate) fn deps_from(
    spawned: bool,
    env: &Env,
    sink: Arc<dyn LogSink>,
    logger: &Logger,
) -> Result<Deps, i32> {
    let paths = resolve(env, Platform::current()).map_err(|e| {
        logger.error(&e);
        1
    })?;
    // OPS-1: the config, before any file is touched.
    let config = config::load(&paths.config_file).map_err(|e| {
        logger.error(&e);
        1
    })?;
    let version = version(env);
    // TST-1: a test build reaches the loopback only, and stops here without it.
    let client = Client::from_env(env, &version).map_err(|e| {
        logger.error(&e);
        1
    })?;
    let clock: SharedClock = SystemClock::new();
    let activity = Activity::default();
    let token_source = GhTokenSource::new(
        &config.gh_path,
        env.clone(),
        clock.clone(),
        activity.clone(),
    );
    let (level, _) = Logger::level_from_env(env.get(LOG_VAR));
    // STA-9: two starts never share an epoch: the wall time in milliseconds XOR the process ID.
    let seed = (clock.wall().unix_millis() as u64) ^ u64::from(std::process::id());
    Ok(Deps {
        clock: clock.clone(),
        activity,
        github: Arc::new(client),
        token_source,
        fs: Arc::new(RealFs),
        probe: Arc::new(RealProbe { clock }),
        poster: RealPoster::new(),
        paths,
        config,
        mode: if spawned {
            Mode::Spawned
        } else {
            Mode::Foreground
        },
        version,
        log: sink,
        level,
        seed,
        signals: true,
        stop_delay: stop_delay(env),
    })
}

/// OPS-11; TST-1: `CIRCITOR_TEST_VERSION` replaces it in a test build.
fn version(env: &Env) -> String {
    #[cfg(feature = "test-endpoint")]
    {
        crate::version::version_from(env.get(VERSION_VAR))
    }
    #[cfg(not(feature = "test-endpoint"))]
    {
        let _ = env;
        crate::version::version_from(None)
    }
}

/// TST-1: `CIRCITOR_TEST_STOP_DELAY` seconds, in a test build only.
fn stop_delay(env: &Env) -> Option<Duration> {
    #[cfg(feature = "test-endpoint")]
    {
        env.get(STOP_DELAY_VAR)
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
    }
    #[cfg(not(feature = "test-endpoint"))]
    {
        let _ = env;
        None
    }
}
