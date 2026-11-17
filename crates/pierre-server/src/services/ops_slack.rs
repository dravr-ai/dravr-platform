// ABOUTME: The Slack client the server's own Web API calls go through: ops card redraws, modals, the feedback screenshot
// ABOUTME: Addressed to Slack's own host unless the context's adapter factory names another origin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where the server's own Slack Web API calls are addressed.
//!
//! The ops card redraw, the feedback reply modal and the feedback screenshot
//! upload are sent with the deployment's bot token through dravr-tronc's
//! `SlackClient`, which calls `https://slack.com` unless it is told another
//! origin. None of them travels through a channel adapter, so an offline
//! adapter factory that only captured sends would leave them, fixture token
//! and all, on the real host (carnet#1615).
//!
//! They are addressed where the context's adapter factory says the
//! platform's own requests to Slack go
//! ([`ChannelAdapterFactory::destination_api_base`]): `None` from the
//! production factory, which is Slack's own host; an origin of its own from
//! a test context's. This module is the one place the server builds a
//! `SlackClient`.
//!
//! [`ChannelAdapterFactory::destination_api_base`]: pierre_services::channel_adapters::ChannelAdapterFactory::destination_api_base

use dravr_tronc::notifications::{SlackClient, SlackConfig};
#[cfg(feature = "client-messaging")]
use pierre_core::models::messaging::ChannelType;

#[cfg(feature = "client-messaging")]
use crate::mcp::resources::ServerContext;

/// The origin `resources`' adapter factory names for the server's own
/// Slack calls: `None` in a deployment, which is Slack's own host.
#[cfg(feature = "client-messaging")]
pub(crate) fn api_origin(resources: &ServerContext) -> Option<String> {
    resources
        .mcp
        .channel_adapters
        .destination_api_base(ChannelType::Slack)
}

/// A Slack client for `config` that calls the Web API at `api_origin`, a
/// scheme and authority, or at Slack's own host when there is none: the
/// client `SlackClient::new` builds, whose `https://slack.com` is
/// dravr-tronc's to define.
pub(crate) fn client(config: &SlackConfig, api_origin: Option<&str>) -> SlackClient {
    api_origin.map_or_else(
        || SlackClient::new(config),
        |origin| SlackClient::with_api_origin(config, origin),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use dravr_tronc::notifications::SlackResult;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// A loopback server that answers every request as Slack answers a
    /// post it took, and keeps each request's first line.
    async fn stand_in() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&asked);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut head = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(read) if read > 0 => head.extend_from_slice(&chunk[..read]),
                        _ => break,
                    }
                }
                let line = String::from_utf8_lossy(&head)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                record.lock().unwrap().push(line);
                let body = r#"{"ok":true}"#;
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                             content-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .ok();
            }
        });
        (origin, asked)
    }

    fn config() -> SlackConfig {
        SlackConfig {
            bot_token: "xoxb-ops-slack-test".to_owned(),
            error_channel: String::new(),
            signing_secret: None,
        }
    }

    /// A named origin is where the client posts, on Slack's own path. The
    /// unnamed case is `SlackClient::new` itself, whose host dravr-tronc
    /// pins in its own tests; asking it here would call Slack.
    #[tokio::test]
    async fn a_named_origin_is_where_the_client_posts() {
        let (origin, asked) = stand_in().await;
        let posted = client(&config(), Some(&origin))
            .post_message_await("C0OPS", &json!([]))
            .await;
        assert!(
            matches!(posted, SlackResult::Ok),
            "the stand-in answered the post: {posted:?}"
        );
        assert_eq!(
            *asked.lock().unwrap(),
            ["POST /api/chat.postMessage HTTP/1.1"]
        );
    }
}
