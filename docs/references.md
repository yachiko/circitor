# References

Where the `documented` facts come from. All read on 2026-11-24.

## GitHub

- GraphQL rate limits, query cost and the node limit: https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api
- REST rate limits and the secondary limits: https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api
- Conditional requests: https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api#use-conditional-requests

## The gh CLI (2.105.0 for the source links; 2.101.0 installed)

- `gh auth token`: https://github.com/cli/cli/blob/v2.105.0/pkg/cmd/auth/token/token.go
- `gh auth login` and token storage: https://cli.github.com/manual/gh_auth_login
- Environment variables: https://cli.github.com/manual/gh_help_environment
- Extensions: https://docs.github.com/en/github-cli/github-cli/creating-github-cli-extensions

## Claude Code

- Tools, timeouts, background commands: https://code.claude.com/docs/en/tools-reference.md
- Environment variables: https://code.claude.com/docs/en/env-vars.md
- Sandboxing: https://code.claude.com/docs/en/sandboxing.md
- Settings: https://code.claude.com/docs/en/settings-reference.md
- Permissions: https://code.claude.com/docs/en/permissions.md
- MCP: https://code.claude.com/docs/en/mcp.md
- Skills: https://code.claude.com/docs/en/skills.md
- Memory (`CLAUDE.md`): https://code.claude.com/docs/en/memory.md

## OpenCode

- Repository: https://github.com/anomalyco/opencode
- v2 shell tool: https://github.com/anomalyco/opencode/blob/v2.4.1/packages/core/src/tool/plugin/shell.ts and https://opencode.ai/docs/tools/
- v1 shell tool: https://github.com/anomalyco/opencode/blob/v1.19.6/packages/opencode/src/tool/shell.ts
- MCP: https://opencode.ai/docs/mcp-servers/ (v1: https://opencode.ai/v1/docs/mcp-servers/)
- Instructions and skills: https://opencode.ai/docs/instructions/ and https://opencode.ai/docs/skills/

## pi

- Repository: https://github.com/earendil-works/pi
- Bash tool: https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/tools/bash.ts
- MCP: https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/mcp.md
- Configuration and skills: https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/configuration.md and https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/skills.md
- The author's 2025 post: https://mariozechner.at/posts/2025-11-30-pi-coding-agent/

## Homebrew (7.0.16)

- `brew services`: https://docs.brew.sh/Manpage#services-subcommand
- Service files in a formula: https://docs.brew.sh/Formula-Cookbook#service-files and https://github.com/Homebrew/brew/blob/HEAD/Library/Homebrew/service.rb
- Casks: https://docs.brew.sh/Cask-Cookbook
- Tap trust: https://docs.brew.sh/Tap-Trust
- Quarantine and the trust model: https://docs.brew.sh/Homebrew-Security-and-Supply-Chain

## Release tooling

- `dist` (cargo-dist) 0.34.0: https://github.com/axodotdev/cargo-dist/releases
- Its Homebrew installer: https://axodotdev.github.io/cargo-dist/book/installers/homebrew.html

## Rust

- axum 0.8.9 listeners: https://github.com/tokio-rs/axum/blob/axum-v0.8.9/axum/src/serve/listener.rs
- axum's Unix socket example: https://github.com/tokio-rs/axum/blob/axum-v0.8.9/examples/unix-domain-socket/src/main.rs
- tokio, Unix sockets on Windows: https://github.com/tokio-rs/tokio/issues/2201
- tokio named pipes: https://docs.rs/tokio/1.54.1/tokio/net/windows/named_pipe/index.html
- Rust std, Unix sockets on Windows: https://github.com/rust-lang/rust/issues/150487
- `tray-icon` 0.26.1: https://github.com/tauri-apps/tray-icon
- `dirs`: https://codeberg.org/dirs/dirs-rs

## Operating systems

- macOS `sockaddr_un`: https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/un.h
- macOS per-user temp directory: https://github.com/apple-oss-distributions/Libc/blob/main/gen/confstr.3
- Linux `unix(7)`: https://man7.org/linux/man-pages/man7/unix.7.html
- XDG base directories: https://specifications.freedesktop.org/basedir/latest/

## Menu bar

- SwiftBar plugin format: https://github.com/swiftbar/SwiftBar#plugin-api
- xbar: https://github.com/matryer/xbar
