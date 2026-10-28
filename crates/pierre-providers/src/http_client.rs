// ABOUTME: Re-exports the shared HTTP client from pierre-core for provider API calls, plus a capped body reader
// ABOUTME: read_capped reads a provider file download no further than its cap, declared length or not
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use pierre_core::http_client::api_client as shared_client;
pub use pierre_core::http_client::initialize_api_client as initialize_shared_client;
pub use pierre_core::http_client::{SharedHttpClient, SharedHttpError, SharedRequestBuilder};

/// A response body read whole, or `None` when it is larger than `cap` bytes.
///
/// A declared length past the cap is refused before any of the body is read;
/// a body that does not declare one is read a chunk at a time and dropped
/// once it passes the cap, read no further than that.
///
/// Every provider file download (a Garmin push activity file, a Wahoo workout
/// file) reads its body through here, so a body without a `Content-Length`
/// cannot grow past the cap in memory before it is checked.
///
/// # Errors
///
/// The transport error that ended the body.
pub async fn read_capped(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<Option<Vec<u8>>, reqwest::Error> {
    if response
        .content_length()
        .is_some_and(|declared| usize::try_from(declared).map_or(true, |len| len > cap))
    {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > cap {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serve `body` once per connection on a loopback port, declaring its
    /// length or streaming it chunked with none, and hand back its URL.
    async fn serve(body: Vec<u8>, declare_length: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    // The request is read up to its blank line and ignored.
                    let mut request = Vec::new();
                    let mut byte = [0_u8; 1];
                    while !request.ends_with(b"\r\n\r\n") {
                        match socket.read(&mut byte).await {
                            Ok(1) => request.push(byte[0]),
                            _ => break,
                        }
                    }
                    let mut response = Vec::new();
                    if declare_length {
                        response.extend_from_slice(
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
                                .as_bytes(),
                        );
                        response.extend_from_slice(&body);
                    } else {
                        response.extend_from_slice(
                            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
                        );
                        for chunk in body.chunks(512) {
                            response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                            response.extend_from_slice(chunk);
                            response.extend_from_slice(b"\r\n");
                        }
                        response.extend_from_slice(b"0\r\n\r\n");
                    }
                    socket.write_all(&response).await.ok();
                    socket.shutdown().await.ok();
                });
            }
        });
        format!("http://{addr}/file")
    }

    /// A file past the cap is refused, whether it declares its length or
    /// streams without one; one inside it is read whole.
    #[tokio::test]
    async fn an_oversized_file_is_refused_with_or_without_a_declared_length() {
        let file = vec![7_u8; 4_096];
        let client = reqwest::Client::new();
        for declare_length in [true, false] {
            let url = serve(file.clone(), declare_length).await;
            let fetch = || async { client.get(&url).send().await.expect("response") };
            assert_eq!(
                read_capped(fetch().await, 1_024).await.expect("read"),
                None,
                "a body past the cap is refused (declared length: {declare_length})"
            );
            assert_eq!(
                read_capped(fetch().await, 8_192).await.expect("read"),
                Some(file.clone()),
                "a body inside the cap is read whole (declared length: {declare_length})"
            );
        }
    }
}
