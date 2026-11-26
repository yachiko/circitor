//! The real `Spawner` (CLI-7, OPS-5, NTF-9).

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;

use crate::cli::notify::{SOCKET_VAR, TOKEN_VAR};
use crate::cli::{Spawned, Spawner};
use crate::config::Env;

/// Never passed to a spawned daemon: OPS-5's two token variables, and NTF-9's session.
const WITHHELD: [&str; 4] = ["GH_TOKEN", "GITHUB_TOKEN", SOCKET_VAR, TOKEN_VAR];

/// The real `Spawner` (T24): `circitor daemon --spawned`, detached.
pub struct RealSpawner;

#[async_trait::async_trait]
impl Spawner for RealSpawner {
    async fn spawn(&self, log: std::fs::File, env: &Env) -> std::io::Result<Box<dyn Spawned>> {
        let exe = std::env::current_exe()?;
        let mut cmd = daemon_command(&exe, env, &log)?;
        let child = cmd.spawn()?;
        Ok(Box::new(Child(child)))
    }
}

/// The command a daemon is started with (CLI-7). Its own function, so a test can read it.
// The crate's one `unsafe`: `pre_exec` (conventions, "Code rules").
#[allow(unsafe_code)]
pub(super) fn daemon_command(
    exe: &Path,
    env: &Env,
    log: &std::fs::File,
) -> std::io::Result<tokio::process::Command> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("daemon").arg("--spawned");
    // OPS-5: a spawned daemon uses gh's login: no `GH_TOKEN`, no `GITHUB_TOKEN`; gh's own settings
    // (`GH_CONFIG_DIR`, `XDG_CONFIG_HOME`) still come from here. NTF-9: never a session's socket or token.
    cmd.env_clear();
    cmd.envs(
        env.vars
            .iter()
            .filter(|(k, _)| !WITHHELD.contains(&k.as_str())),
    );
    // CLI-7: the root directory, stdin from nowhere, stdout and stderr to the log, so the daemon never
    // holds the agent's output open.
    cmd.current_dir("/");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(log.try_clone()?));
    cmd.stderr(Stdio::from(log.try_clone()?));
    // The child outlives the CLI: it isn't killed when the handle goes.
    cmd.kill_on_drop(false);
    // SAFETY: the closure runs in the child between `fork` and `exec`, and calls only `setsid`, which
    // is async-signal-safe; it touches no memory of the parent's. CLI-7: the daemon gets its own session.
    unsafe {
        cmd.pre_exec(|| {
            nix::unistd::setsid().map_err(std::io::Error::from)?;
            Ok(())
        });
    }
    Ok(cmd)
}

/// A started daemon.
struct Child(tokio::process::Child);

impl Spawned for Child {
    fn exited(&mut self) -> Option<i32> {
        match self.0.try_wait() {
            Ok(Some(status)) => Some(
                status
                    .code()
                    .or_else(|| status.signal().map(|s| 128 + s))
                    .unwrap_or(1),
            ),
            Ok(None) => None,
            // It can't be asked: as good as gone with an error.
            Err(_) => Some(1),
        }
    }
}
