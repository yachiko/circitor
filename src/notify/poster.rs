//! The real poster: `tokio::fs::metadata`, `tokio::net::UnixStream` (NTF-6; T46).
//!
//! OPS-16: besides `github::GitHub::send`, this is the one connection the daemon makes to anything but
//! its own socket. It writes one notice to the session socket a subscription named, and never reads.

use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::sync::Arc;

use tokio::io::AsyncWriteExt;

use super::{Looked, Posted, Poster, SessionToken, SocketId};

pub struct RealPoster;

impl RealPoster {
    pub fn new() -> Arc<RealPoster> {
        Arc::new(RealPoster)
    }
}

/// NTF-6's auth line, with its newline. Built with `serde_json`, so the token is escaped.
fn auth_line(token: &SessionToken) -> String {
    let mut line = serde_json::json!({"type": "auth", "token": token.expose()}).to_string();
    line.push('\n');
    line
}

/// NTF-6's message line, with its newline. A JSON string escapes a newline and a quote by itself.
fn message_line(text: &str) -> String {
    let mut line = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": text},
    })
    .to_string();
    line.push('\n');
    line
}

/// NTF-6: a missing socket, a refused connection or a denied permission is the session gone.
fn posted_error(e: io::Error) -> Posted {
    match e.kind() {
        io::ErrorKind::NotFound
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::PermissionDenied => Posted::SessionGone(e.to_string()),
        _ => Posted::Failed(e.to_string()),
    }
}

/// What is at a path that isn't a socket of the user's, in circitor's words.
fn what_it_is(ft: &std::fs::FileType) -> &'static str {
    if ft.is_socket() {
        "a socket of another user"
    } else if ft.is_file() {
        "a regular file"
    } else if ft.is_dir() {
        "a directory"
    } else if ft.is_fifo() {
        "a FIFO"
    } else if ft.is_char_device() || ft.is_block_device() {
        "a device"
    } else {
        "not a socket"
    }
}

#[async_trait::async_trait]
impl Poster for RealPoster {
    // NTF-4: a `stat`, never a connection.
    async fn look(&self, socket: &Path) -> Looked {
        match tokio::fs::metadata(socket).await {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Looked::Missing,
            Err(e) => Looked::Failed(e.to_string()),
            Ok(meta) => {
                let id = SocketId {
                    dev: meta.dev(),
                    ino: meta.ino(),
                };
                let ft = meta.file_type();
                // NTF-2: a socket owned by the user the daemon runs as.
                if ft.is_socket() && meta.uid() == nix::unistd::geteuid().as_raw() {
                    Looked::Socket(id)
                } else {
                    Looked::Other(id, what_it_is(&ft).to_string())
                }
            }
        }
    }

    // NTF-6, OPS-16: one connection, the two lines, nothing read. `Done` is returned in the poll that
    // writes the last byte; the close is the drop, never an awaited shutdown or flush.
    async fn post(&self, socket: &Path, token: Option<&SessionToken>, text: &str) -> Posted {
        let mut stream = match tokio::net::UnixStream::connect(socket).await {
            Ok(s) => s,
            Err(e) => return posted_error(e),
        };
        if let Some(token) = token
            && let Err(e) = stream.write_all(auth_line(token).as_bytes()).await
        {
            return posted_error(e);
        }
        if let Err(e) = stream.write_all(message_line(text).as_bytes()).await {
            return posted_error(e);
        }
        Posted::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{MARKER_SESSION_TOKEN, ShortDir};
    use tokio::io::AsyncReadExt;

    async fn received(listener: &tokio::net::UnixListener) -> String {
        let (mut conn, _) = listener.accept().await.unwrap();
        let mut buf = String::new();
        conn.read_to_string(&mut buf).await.unwrap();
        buf
    }

    // NTF-6, OPS-16: the auth line and the message line, each one JSON object and a newline.
    #[tokio::test]
    async fn real_poster_writes_the_two_lines() {
        let dir = ShortDir::new();
        let path = dir.path().join("s.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let poster = RealPoster::new();
        let text = "a \"quoted\"\nline";
        let token = SessionToken::new(MARKER_SESSION_TOKEN.to_string());

        let (posted, got) =
            tokio::join!(poster.post(&path, Some(&token), text), received(&listener));
        assert_eq!(posted, Posted::Done);
        assert!(got.ends_with('\n'));
        let lines: Vec<&str> = got.split_terminator('\n').collect();
        assert_eq!(lines.len(), 2, "{got:?}");
        let auth: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(
            auth,
            serde_json::json!({"type": "auth", "token": MARKER_SESSION_TOKEN})
        );
        let msg: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(
            msg,
            serde_json::json!({"type": "user", "message": {"role": "user", "content": text}})
        );
        // The quotes and the newline are escaped in the line.
        assert!(lines[1].contains(r#"a \"quoted\"\nline"#), "{}", lines[1]);

        // Without a token: the message line alone.
        let (posted, got) = tokio::join!(poster.post(&path, None, "plain"), received(&listener));
        assert_eq!(posted, Posted::Done);
        assert_eq!(got, message_line("plain"));
        assert_eq!(got.matches('\n').count(), 1);
        let only: serde_json::Value = serde_json::from_str(got.trim_end()).unwrap();
        assert_eq!(only["type"], "user");
        assert_eq!(only["message"]["content"], "plain");
    }

    // NTF-6: a missing path and a closed listener's file are the session gone; a datagram socket fails.
    #[tokio::test]
    async fn real_poster_session_gone_and_failed() {
        let dir = ShortDir::new();
        let poster = RealPoster::new();

        let missing = dir.path().join("none.sock");
        assert!(matches!(
            poster.post(&missing, None, "x").await,
            Posted::SessionGone(_)
        ));

        let closed = dir.path().join("closed.sock");
        drop(std::os::unix::net::UnixListener::bind(&closed).unwrap());
        assert!(closed.exists());
        assert!(matches!(
            poster.post(&closed, None, "x").await,
            Posted::SessionGone(_)
        ));

        let dgram = dir.path().join("dgram.sock");
        let _d = std::os::unix::net::UnixDatagram::bind(&dgram).unwrap();
        assert!(matches!(
            poster.post(&dgram, None, "x").await,
            Posted::Failed(_)
        ));
    }

    // NTF-4: `look` is a `stat`: a socket with its device and inode, a missing path, a regular file.
    #[tokio::test]
    async fn real_poster_look() {
        let dir = ShortDir::new();
        let poster = RealPoster::new();
        let path = dir.path().join("s.sock");
        let _listener = tokio::net::UnixListener::bind(&path).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(
            poster.look(&path).await,
            Looked::Socket(SocketId {
                dev: meta.dev(),
                ino: meta.ino()
            })
        );

        assert_eq!(poster.look(&dir.path().join("none")).await, Looked::Missing);

        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let meta = std::fs::metadata(&file).unwrap();
        assert_eq!(
            poster.look(&file).await,
            Looked::Other(
                SocketId {
                    dev: meta.dev(),
                    ino: meta.ino()
                },
                "a regular file".to_string()
            )
        );
    }
}
