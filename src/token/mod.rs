//! The GitHub token and where it comes from (`plan/contracts/github.md`). T02's and final:
//! T12 builds `gh`, T43 `fake`.

#[cfg(any(test, feature = "test-endpoint"))]
pub mod fake;
pub mod gh;

pub use gh::GhTokenSource;

/// The GitHub token. No `Display`, no `Serialize`; `Debug` prints `Token(…)` (OPS-5).
#[derive(Clone)]
pub struct Token(String);

impl Token {
    pub fn new(value: String) -> Token {
        Token(value)
    }

    /// For the `Authorization` header only.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn same_as(&self, other: &Token) -> bool {
        self.0 == other.0
    }
}

// OPS-5: the value never leaves through `Debug`.
impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(…)")
    }
}

/// "The token source in use" (OPS-5). `as_str`: `gh`, `GH_TOKEN`, `GITHUB_TOKEN` (API-5's `token_source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenSourceKind {
    Gh,
    GhTokenVar,
    GithubTokenVar,
}

impl TokenSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenSourceKind::Gh => "gh",
            TokenSourceKind::GhTokenVar => "GH_TOKEN",
            TokenSourceKind::GithubTokenVar => "GITHUB_TOKEN",
        }
    }
}

#[derive(Clone, Debug)]
pub enum TokenRead {
    Token(Token),
    /// Non-zero exit, stderr holds `no oauth token found`.
    NoLogin,
    Failed(GhFailure),
}

/// Never `gh`'s output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhFailure {
    CantRun,
    Exit(i32),
    Killed,
    Timeout,
    NoOutput,
}

#[async_trait::async_trait]
pub trait TokenSource: Send + Sync + 'static {
    /// One read of `gh auth token --hostname github.com` (OPS-5), within 10 s of the clock.
    /// Concurrent calls share one run.
    async fn read(&self) -> TokenRead;
    /// From the daemon's own environment, fixed at start.
    fn kind(&self) -> TokenSourceKind;
    /// The `gh` that was run last, or `gh_path` as configured before any run (API-5, API-3's `auth` message).
    fn gh_path(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::MARKER_TOKEN;

    // OPS-5: the token never leaves through `Debug`.
    #[test]
    fn ops5_token_debug_holds_no_value() {
        let t = Token::new(MARKER_TOKEN.to_string());
        assert_eq!(format!("{t:?}"), "Token(…)");
        let read = TokenRead::Token(t.clone());
        assert!(!format!("{read:?}").contains(MARKER_TOKEN));
        assert_eq!(t.expose(), MARKER_TOKEN);
        assert!(t.same_as(&Token::new(MARKER_TOKEN.to_string())));
        assert_eq!(
            [
                TokenSourceKind::Gh,
                TokenSourceKind::GhTokenVar,
                TokenSourceKind::GithubTokenVar
            ]
            .map(|k| k.as_str()),
            ["gh", "GH_TOKEN", "GITHUB_TOKEN"]
        );
    }
}
