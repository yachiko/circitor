//! One request over the daemon's socket, written with `hyper`, for what the CLI can't send (TST-7).

use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;

/// Sends `method path_and_query` with `body` (JSON) and reads the whole answer. `Err` is a text
/// naming what failed: no daemon, or a connection dropped.
pub(super) async fn request(
    socket: &Path,
    method: &str,
    path_and_query: &str,
    body: Option<&str>,
) -> Result<(u16, String), String> {
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|e| format!("connect {}: {e}", socket.display()))?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .map_err(|e| format!("handshake: {e}"))?;
    let task = tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut builder = http::Request::builder()
        .method(method)
        .uri(path_and_query)
        .header(http::header::HOST, "circitor")
        .header(http::header::CONNECTION, "close");
    if body.is_some() {
        builder = builder.header(http::header::CONTENT_TYPE, "application/json");
    }
    let req = builder
        .body(Full::new(Bytes::from(body.unwrap_or_default().to_string())))
        .map_err(|e| format!("request: {e}"))?;
    let answer = async {
        let response = sender
            .send_request(req)
            .await
            .map_err(|e| format!("send: {e}"))?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| format!("body: {e}"))?
            .to_bytes();
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }
    .await;
    task.abort();
    answer
}
