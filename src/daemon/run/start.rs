//! `start`: STA-11's order, OPS-6's takeover, then the wiring.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Notify, watch};

use super::life::{Life, Signals, idle_after, idle_loop, minute_loop};
use super::serve::{Conns, accept_loop};
use super::{Running, StartError, Tasks, lock};
use crate::api::{self, AppState, DaemonInfo, PathsInfo, Request};
use crate::clock::{Mono, SharedClock, SleepWatch, sleep, timeout};
use crate::config::ensure_private_dir;
use crate::daemon::instance::{
    Acquire, Holder, LockGuard, PATIENCE, RETRY_EVERY, SocketFile, TAKEOVER_PATIENCE, acquire,
    bind, clear_socket,
};
use crate::daemon::{Deps, Mode, StopHandle};
use crate::log::Logger;
use crate::notify::{Notifier, NotifyDeps};
use crate::poller::{Engine, EngineDeps};
use crate::state::{FileContent, Loaded, Saver, SaverDeps, Store, StoreDeps, load};
use crate::status::Warnings;
use crate::transport::{Probe, SocketProbe, connect};

/// How long a shutdown request to a spawned holder may take (OPS-6): OPS-4's 2 s for an answer.
const ASK_LIMIT: Duration = Duration::from_secs(2);

/// API-5's `mode` word for a spawned daemon, as `StatusLite` carries it.
const SPAWNED: &str = "spawned";

fn mode_word(mode: Mode) -> &'static str {
    match mode {
        Mode::Spawned => "spawned",
        Mode::Foreground => "foreground",
    }
}

/// OPS-6, API-17: asks a spawned daemon to shut down and reads its whole answer. What it answers
/// doesn't matter: the lock (or the socket) says whether it went.
async fn ask_to_stop(clock: &SharedClock, socket: &Path) {
    let ask = async {
        let head = connect(socket)
            .await
            .ok()?
            .send(&Request::Shutdown)
            .await
            .ok()?;
        head.body().await.ok()
    };
    let _ = timeout(clock, ASK_LIMIT, ask).await;
}

/// STA-11, a spawned daemon with the lock busy: 0 if the holder answers, an error otherwise.
async fn busy_spawned(
    probe: &dyn SocketProbe,
    holder: Option<Holder>,
    lock_file: &Path,
) -> StartError {
    let Some(holder) = holder else {
        // Open question of T28: no holder line counts as no path, an error.
        return StartError::NoHolder(lock_file.to_path_buf());
    };
    match probe.probe(&holder.socket).await {
        Probe::Answers(_) => StartError::AlreadyRunning(holder.socket),
        // OPS-4: `Denied` counts as a daemon that doesn't answer.
        Probe::NoDaemon | Probe::Denied(_) | Probe::NotAnswering(_) => {
            StartError::HolderNotAnswering(holder.socket)
        }
    }
}

/// OPS-6: a foreground daemon takes over from a spawned holder, again each time the holder answers
/// `spawned`, until it holds the lock or 15 s have passed since `began`.
async fn take_over(
    deps: &Deps,
    mut holder: Option<Holder>,
    began: Mono,
) -> Result<LockGuard, StartError> {
    let lock_file = &deps.paths.lock_file;
    let deadline = began.plus(TAKEOVER_PATIENCE);
    loop {
        let Some(h) = holder else {
            return Err(StartError::NoHolder(lock_file.clone()));
        };
        match deps.probe.probe(&h.socket).await {
            Probe::Answers(s) if s.mode == SPAWNED => {}
            // OPS-6: a foreground holder is left alone, at once.
            Probe::Answers(_) => return Err(StartError::ForegroundHolder(h.socket)),
            Probe::NoDaemon | Probe::Denied(_) | Probe::NotAnswering(_) => {
                return Err(StartError::HolderNotAnswering(h.socket));
            }
        }
        ask_to_stop(&deps.clock, &h.socket).await;
        let left = deadline.since(deps.clock.mono());
        if left.is_zero() {
            return Err(StartError::TakeoverTimedOut(h.socket));
        }
        match acquire(lock_file, &deps.clock, left.min(PATIENCE)).await {
            Ok(Acquire::Held(guard)) => return Ok(guard),
            Ok(Acquire::Busy(next)) => {
                if deps.clock.mono() >= deadline {
                    // OPS-6: never 0 for want of the lock; 1, naming the holder's socket.
                    let socket = next.map_or(h.socket, |n| n.socket);
                    return Err(StartError::TakeoverTimedOut(socket));
                }
                holder = next;
            }
            Err(source) => {
                return Err(StartError::LockOpen {
                    path: lock_file.clone(),
                    source,
                });
            }
        }
    }
}

/// OPS-4, with the lock held: a stale socket goes; one that answers is another daemon's, met as STA-11
/// and OPS-6 meet an answering holder; anything else stops the start, naming the path.
async fn clear(deps: &Deps, began: Mono) -> Result<(), StartError> {
    let path = &deps.paths.socket;
    let deadline = began.plus(TAKEOVER_PATIENCE);
    let mut asked = false;
    loop {
        let found =
            clear_socket(path, &*deps.probe)
                .await
                .map_err(|source| StartError::SocketCheck {
                    path: path.clone(),
                    source,
                })?;
        match found {
            SocketFile::Absent | SocketFile::RemovedStale => return Ok(()),
            SocketFile::Live(status) => match deps.mode {
                Mode::Spawned => return Err(StartError::AlreadyRunning(path.clone())),
                Mode::Foreground if status.mode != SPAWNED => {
                    return Err(StartError::ForegroundHolder(path.clone()));
                }
                Mode::Foreground => {
                    if deps.clock.mono() >= deadline {
                        return Err(StartError::TakeoverTimedOut(path.clone()));
                    }
                    // Asked once: a second request would cut its stop short (OPS-7).
                    if !asked {
                        ask_to_stop(&deps.clock, path).await;
                        asked = true;
                    }
                    sleep(&deps.clock, RETRY_EVERY).await;
                }
            },
            SocketFile::NotASocket => return Err(StartError::NotASocket(path.clone())),
            SocketFile::NotAnswering => return Err(StartError::SocketNotAnswering(path.clone())),
        }
    }
}

/// STA-11's order: the directories, the lock, the lock file's line, a stale socket, `state.json`, the
/// socket, and only then the engine (which reads the token). The notifier is spawned after the engine (it reads
/// `PollerHandle::status`) and before the accept loop; it starts empty (NTF-9). Returns once the socket is bound and serving.
pub async fn start(deps: Deps) -> Result<Running, StartError> {
    let clock = deps.clock.clone();
    let logger = Logger::new(clock.clone(), deps.log.clone(), deps.level);
    // OPS-7: the handlers first; installing them touches no file.
    let signals = if deps.signals {
        Some(Signals::install().map_err(StartError::Signals)?)
    } else {
        None
    };
    let paths = deps.paths.clone();

    // 1. OPS-3: the private directories.
    ensure_private_dir(&paths.state_dir)?;
    if let Some(dir) = &paths.socket_dir {
        ensure_private_dir(dir)?;
    }

    // 2. STA-11: the lock, tried for 6 s; an error from opening it stops at once.
    let began = clock.mono();
    let guard = match acquire(&paths.lock_file, &clock, PATIENCE).await {
        Ok(Acquire::Held(guard)) => guard,
        // 3. STA-11: busy, spawned.
        Ok(Acquire::Busy(holder)) if deps.mode == Mode::Spawned => {
            return Err(busy_spawned(&*deps.probe, holder, &paths.lock_file).await);
        }
        // 4. OPS-6: busy, foreground.
        Ok(Acquire::Busy(holder)) => take_over(&deps, holder, began).await?,
        Err(source) => {
            return Err(StartError::LockOpen {
                path: paths.lock_file.clone(),
                source,
            });
        }
    };

    // 5. STA-11: only once the lock is held, the holder line.
    guard
        .write_holder(std::process::id(), &paths.socket)
        .map_err(|source| StartError::WriteHolder {
            path: paths.lock_file.clone(),
            source,
        })?;

    // 6. OPS-4: a stale socket.
    clear(&deps, began).await?;

    // 7. STA-8, STA-11: `state.json`, the store, the saver.
    let warnings = Warnings::new(clock.clone(), logger.clone());
    let loaded = load(&*deps.fs, &paths.state_file, &logger, &warnings);
    // STA-8: a newer file is left alone and never written.
    let writable = !matches!(loaded, Loaded::Newer);
    let content = match loaded {
        Loaded::Content(content) => content,
        Loaded::SetAside | Loaded::Newer => FileContent::default(),
    };
    let config = Arc::new(deps.config);
    let store = Store::new(
        StoreDeps {
            clock: clock.clone(),
            logger: logger.clone(),
            config: config.clone(),
            seed: deps.seed,
        },
        content,
    );
    let saver = Saver::spawn(SaverDeps {
        fs: deps.fs.clone(),
        path: paths.state_file.clone(),
        source: store.clone(),
        clock: clock.clone(),
        logger: logger.clone(),
        warnings: warnings.clone(),
        activity: deps.activity.clone(),
        writable,
    });

    // 8. OPS-4: the socket; then the engine, which reads the token (OPS-5: after the bind, and `start`
    // never waits for it); then the notifier, empty (NTF-9); then the accept loop.
    let bound = match bind(&paths.socket) {
        Ok(bound) => Arc::new(bound),
        Err(source) => {
            saver.abort();
            return Err(StartError::Bind {
                path: paths.socket.clone(),
                source,
            });
        }
    };
    let guard = Arc::new(guard);
    let sleep_watch = SleepWatch::new(clock.clone());
    let engine = Engine::spawn(EngineDeps {
        clock: clock.clone(),
        github: deps.github.clone(),
        token_source: deps.token_source.clone(),
        store: store.clone(),
        config: config.clone(),
        logger: logger.clone(),
        warnings: warnings.clone(),
        activity: deps.activity.clone(),
        sleep: sleep_watch.clone(),
    });
    let notifier = Notifier::spawn(NotifyDeps {
        store: store.clone(),
        clock: clock.clone(),
        logger: logger.clone(),
        activity: deps.activity.clone(),
        poster: deps.poster.clone(),
        poller: engine.clone(),
        sleep: sleep_watch.clone(),
        warnings: warnings.clone(),
        config: config.clone(),
    });
    let stop = StopHandle::new();
    let info = DaemonInfo {
        version: deps.version.clone(),
        mode: deps.mode,
        started_at: clock.wall(),
        paths: PathsInfo {
            config: paths.config_file.display().to_string(),
            state: paths.state_file.display().to_string(),
            socket: paths.socket.display().to_string(),
            log: paths.log_file.display().to_string(),
        },
    };
    let state = AppState {
        store: store.clone(),
        poller: engine.clone(),
        flush: saver.clone(),
        notify: notifier.clone(),
        clock: clock.clone(),
        sleep: sleep_watch,
        activity: deps.activity.clone(),
        logger: logger.clone(),
        warnings,
        config: config.clone(),
        info: Arc::new(info),
        stop: stop.clone(),
    };
    let router = api::router(state);

    let conns = Arc::new(Conns::default());
    let second = Arc::new(Notify::new());
    let tasks = Arc::new(Mutex::new(Tasks::default()));
    let (status_tx, status) = watch::channel(None);
    {
        let mut t = lock(&tasks);
        t.accept = Some(tokio::spawn(accept_loop(
            bound.clone(),
            router,
            conns.clone(),
            deps.activity.clone(),
            clock.clone(),
        )));
        if let Some(exit_after) = idle_after(deps.mode, config.exit_after) {
            t.idle = Some(tokio::spawn(idle_loop(
                clock.clone(),
                deps.activity.clone(),
                store.clone(),
                engine.clone(),
                stop.clone(),
                exit_after,
            )));
        }
        t.minute = Some(tokio::spawn(minute_loop(
            clock.clone(),
            deps.activity.clone(),
            guard.clone(),
            bound.clone(),
            stop.clone(),
        )));
        if let Some(signals) = signals {
            t.signals = Some(tokio::spawn(signals.run(
                stop.clone(),
                second.clone(),
                deps.activity.clone(),
            )));
        }
        let life = Life {
            clock: clock.clone(),
            activity: deps.activity.clone(),
            logger: logger.clone(),
            stop: stop.clone(),
            second: second.clone(),
            tasks: tasks.clone(),
            bound: Some(bound),
            lock: Some(guard),
            engine: engine.clone(),
            notifier: notifier.clone(),
            saver: saver.clone(),
            conns: conns.clone(),
            stop_delay: deps.stop_delay,
            status: status_tx,
        };
        t.life = Some(tokio::spawn(life.run()));
    }

    // 9. OPS-9: started, with the version, the mode and the paths.
    logger.info(format_args!(
        "start version={} mode={} socket={} state={} config={}",
        deps.version,
        mode_word(deps.mode),
        paths.socket.display(),
        paths.state_file.display(),
        paths.config_file.display(),
    ));

    Ok(Running {
        socket: paths.socket,
        stop,
        second,
        conns,
        notifier,
        engine,
        saver,
        status,
        tasks,
    })
}
