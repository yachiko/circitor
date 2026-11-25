# Platform facts

**Status:** read from documentation and source on 2026-11-24. Nothing here was run on a Mac: development is on Linux, and no Mac was at hand. **Purpose:** what Homebrew, the operating systems and the Rust libraries give circitor.

Labels are those of [GitHub API facts](02-github-api.md): **verified**, **observed**, **documented**, **assumed**. Sources are in [references](references.md). Cited from the specs as "docs/04 §3".

## 1. Machines

| Item | Value | Label |
|---|---|---|
| Development machine | Arch Linux, x86-64; `cargo` 1.98.1; `gh` 2.101.0; **no Homebrew** | verified |
| Target | macOS first, Linux later, Windows undecided | [direction](01-direction.md) |
| A Mac to test on | The owner's. Nothing macOS-specific can be checked on the development machine. | – |

## 2. Homebrew

| Item | Value | Label |
|---|---|---|
| The owner's tap | `yachiko/homebrew-tap`. It ships **casks** for the other tools (`clerk`, `fossor`, `lege`), written by each project's release workflow (GoReleaser); a GitHub App signs the commits. One formula exists, `Formula/clerk.rb`, which installs prebuilt archives for macOS and Linux. | verified (local clone) |
| **`brew services` takes formulae only** | A cask's `service` stanza is something else | documented |
| `service do` in a tap formula | Allowed; third-party taps use it | documented |
| What it generates | macOS: a LaunchAgent in `~/Library/LaunchAgents`, label `homebrew.mxcl.<formula>`, started at login. Linux: a systemd user unit in `~/.config/systemd/user`. | documented |
| Its keys | `run`, `keep_alive`, `log_path`, `error_log_path`, `environment_variables`, `working_dir`, `restart_delay` | documented |
| The service's environment | launchd starts it with a minimal `PATH`; `std_service_path_env` gives Homebrew's. `~/.homebrew/services/<formula>.env` adds variables. | documented |
| **Tap trust** | Since Homebrew 6.0.0 (2026-06-11) a third-party tap needs explicit trust. `brew install yachiko/tap/circitor` trusts that one item; a short-name install needs `brew trust` first. | documented |
| Quarantine | Homebrew quarantines **cask** downloads, so Gatekeeper checks them. No formula code path does. | documented |
| An unsigned binary from a formula | Expected to run without a Gatekeeper prompt, since nothing quarantines it. Apple Silicon is expected to need at least an ad-hoc signature. | assumed |

## 3. Release tooling

| Item | Value | Label |
|---|---|---|
| `dist` (cargo-dist) 0.34.0 | Maintained: five releases in 2026, the last on 2026-11-02 | documented |
| Its Homebrew support | Builds a **formula** that fetches the prebuilt archives and pushes it to a tap. No cask, and **no `service do` block**. | documented |
| Its targets | x86-64 and arm64 for macOS, Linux (glibc and musl) and Windows | documented |
| macOS signing or notarisation in `dist` | Not documented | documented |
| GoReleaser, which the tap's other tools use | Whether it builds Rust projects wasn't checked | assumed |

## 4. Sockets and directories

| Item | Value | Label |
|---|---|---|
| Unix socket path length | **104 bytes on macOS, 108 on Linux** (107 usable) | documented |
| Linux runtime directory | `$XDG_RUNTIME_DIR`: owned by the user, mode 0700, gone at logout or reboot, meant for sockets | documented |
| macOS runtime directory | **There is none.** The `dirs` crate returns `None`. | documented |
| macOS per-user temp directory | Mode 0700, but **files not accessed for 3 days may be removed** | documented |
| `$TMPDIR` on the owner's Mac | `/var/folders/26/<id>…`: the per-user temp directory. Its exact length wasn't given; paths of this form are about 49 bytes, which leaves room for a socket name under the 104-byte limit. | observed (the owner's report) |
| `~/Library/Application Support/<name>` | For data an application manages; survives reboots | documented |
| `~/Library/Caches` | The system may delete it | documented |
| Unix sockets on Windows | The OS has them. **tokio doesn't** on Windows (issue open since 2020). Rust's standard library has them behind an unstable feature, with stabilisation proposed in 2026-10. | documented |
| Named pipes on Windows | `tokio::net::windows::named_pipe` | documented |

## 5. Rust libraries

| Item | Value | Label |
|---|---|---|
| axum 0.8.9 | `axum::serve` takes any `Listener`; `tokio::net::UnixListener` is one on Unix. The official example removes a stale socket file before binding. | documented |
| One router on two listeners | Not built in. `Router` is `Clone`, so two `serve` calls can share one; a custom `Listener` is the other way. | documented |
| tokio 1.54.1 | Unix sockets on Unix only | documented |
| Whether tokio's timers count the time a laptop sleeps | Not checked | assumed |

## 6. The menu bar

| Item | Value | Label |
|---|---|---|
| `tray-icon` 0.26.1 with `muda` 0.21 | On macOS the icon is created on the main thread, with an event loop already running there | documented |
| Whether it needs an `.app` bundle | The documentation doesn't say. One report of an unbundled binary showing no icon was closed as an OS beta bug. | assumed |
| On Linux | GTK 3 with libappindicator, or a StatusNotifierItem backend | documented |
| SwiftBar 2.1.1 | A plugin is an executable named `<name>.<interval>.<ext>` (`circitor.30s.sh`). Lines before `---` are the menu bar title, lines after it the dropdown; `| href=` and `| bash=` make them clickable. | documented |
| xbar | Same format; its last release is from 2021 | documented |

## 7. The `gh` CLI

Token handling is in [GitHub API facts §1](02-github-api.md#1-access).

| Item | Value | Label |
|---|---|---|
| An extension in Rust | Possible: a repository named `gh-<name>`, with release assets ending in `<os>-<arch>` in Go's names (`gh-circitor-darwin-arm64`) | documented |
| What an extension can't do | Install a service, or ship anything but that one binary | documented |
| `gh pr checks --watch` | Polls every 10 s by default (`--interval`) | verified (`--help`, gh 2.101.0) |

## 8. Open items

On the owner's Mac, before a spec relies on them:

1. Install an unsigned Rust binary from a tap formula on Apple Silicon, and run it.
2. `brew services start` for a formula from the tap: the plist it writes, and the `PATH` the service sees (it must find `gh`).
3. Whether a socket in `$TMPDIR` survives three idle days.
4. Whether `gh auth token` works from a LaunchAgent, where the Keychain may ask for access.
5. `tray-icon` from an unbundled binary.

Anywhere:

6. Whether tokio's sleeps and timeouts advance across a suspend, on macOS and on Linux.
7. Whether GoReleaser can build and release a Rust binary, to keep one release tool across the tap.
