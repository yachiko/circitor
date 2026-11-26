//! The real `Git` (CLI-2): `git -C <cwd>` through `tokio::process`. Any failure of `git` is `None` or
//! `NoRepo`: the command then says which form to use.

use std::path::Path;
use std::process::Stdio;

use crate::cli::{Branch, Git};

/// The real `Git`: runs `git` with tokio::process.
pub struct RealGit;

/// Runs `git -C <cwd> <args>`. `Some(stdout)`, trimmed, when it exits 0; `None` otherwise.
async fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

#[async_trait::async_trait]
impl Git for RealGit {
    async fn origin_url(&self, cwd: &Path) -> Option<String> {
        // CLI-2: the repository is `origin`'s.
        git(cwd, &["remote", "get-url", "origin"])
            .await
            .filter(|s| !s.is_empty())
    }

    async fn push_branch(&self, cwd: &Path) -> Branch {
        if git(cwd, &["rev-parse", "--git-dir"]).await.is_none() {
            return Branch::NoRepo;
        }
        // CLI-2: a detached HEAD has no symbolic name.
        let Some(own) = git(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .await
            .filter(|s| !s.is_empty())
        else {
            return Branch::Detached;
        };
        // CLI-2: the upstream's branch name when the upstream is on `origin`, else the branch's own name.
        let remote = git(cwd, &["config", "--get", &format!("branch.{own}.remote")]).await;
        if remote.as_deref() == Some("origin")
            && let Some(merge) =
                git(cwd, &["config", "--get", &format!("branch.{own}.merge")]).await
            && let Some(name) = merge.strip_prefix("refs/heads/")
            && !name.is_empty()
        {
            return Branch::Named(name.to_string());
        }
        Branch::Named(own)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::ShortDir;

    async fn run(cwd: &Path, args: &[&str]) -> bool {
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success())
    }

    // CLI-2: the real `git`, in a repository made here: no repository, a branch, its upstream on
    // `origin`, an upstream elsewhere, a detached HEAD.
    #[tokio::test]
    async fn cli2_real_git() {
        let dir = ShortDir::new();
        let cwd = dir.path();
        if !run(cwd, &["--version"]).await {
            // No `git` on this machine: nothing to test against.
            return;
        }
        assert_eq!(RealGit.push_branch(cwd).await, Branch::NoRepo);
        assert_eq!(RealGit.origin_url(cwd).await, None);

        assert!(run(cwd, &["init", "-q"]).await);
        assert!(run(cwd, &["checkout", "-q", "-b", "feature"]).await);
        assert_eq!(RealGit.origin_url(cwd).await, None);
        assert_eq!(
            RealGit.push_branch(cwd).await,
            Branch::Named("feature".into())
        );

        let url = "git@github.com:octo/app.git";
        assert!(run(cwd, &["remote", "add", "origin", url]).await);
        assert_eq!(RealGit.origin_url(cwd).await.as_deref(), Some(url));
        assert!(run(cwd, &["config", "branch.feature.remote", "origin"]).await);
        assert!(
            run(
                cwd,
                &["config", "branch.feature.merge", "refs/heads/up-name"]
            )
            .await
        );
        assert_eq!(
            RealGit.push_branch(cwd).await,
            Branch::Named("up-name".into())
        );
        assert!(run(cwd, &["config", "branch.feature.remote", "fork"]).await);
        assert_eq!(
            RealGit.push_branch(cwd).await,
            Branch::Named("feature".into())
        );

        let id = "-c";
        assert!(
            run(
                cwd,
                &[
                    id,
                    "user.name=t",
                    id,
                    "user.email=t@example.test",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    "x"
                ]
            )
            .await
        );
        assert!(run(cwd, &["checkout", "-q", "--detach"]).await);
        assert_eq!(RealGit.push_branch(cwd).await, Branch::Detached);
    }
}
