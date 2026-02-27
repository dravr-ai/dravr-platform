// ABOUTME: Slack API client implementing the MessagingProvider trait
// ABOUTME: Sends messages via chat.postMessage and lists channels via conversations.list
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::HeaderMap;
use reqwest::Client;
use tracing::{debug, warn};

use crate::errors::{AppError, AppResult};
use crate::provider::MessagingProvider;
use crate::types::{ChannelInfo, OutgoingMessage, SendMessageResponse};

use super::signature::verify_slack_signature;
use super::types::{SlackConversationsListResponse, SlackPostMessageResponse};

/// Slack Web API base URL
const SLACK_API_BASE: &str = "https://slack.com/api";

/// Slack messaging provider using the Bot Token API
///
/// Implements the [`MessagingProvider`] trait for Slack workspaces. Uses
/// a Bot User OAuth Token (`xoxb-...`) for API authentication and the
/// app signing secret for webhook verification.
pub struct SlackProvider {
    /// HTTP client for Slack API calls
    http_client: Client,
    /// Bot User OAuth Token for API authentication
    bot_token: String,
    /// Signing secret for webhook request verification
    signing_secret: String,
}

impl SlackProvider {
    /// Create a new Slack provider with the given credentials
    ///
    /// # Arguments
    ///
    /// * `bot_token` - Slack Bot User OAuth Token (starts with `xoxb-`)
    /// * `signing_secret` - Slack app signing secret for webhook verification
    #[must_use]
    pub fn new(bot_token: String, signing_secret: String) -> Self {
        Self {
            http_client: Client::new(),
            bot_token,
            signing_secret,
        }
    }

    /// Create a new Slack provider with a custom HTTP client
    ///
    /// Useful for testing or when a shared client pool is preferred.
    #[must_use]
    pub fn with_client(http_client: Client, bot_token: String, signing_secret: String) -> Self {
        Self {
            http_client,
            bot_token,
            signing_secret,
        }
    }
}

#[async_trait::async_trait]
impl MessagingProvider for SlackProvider {
    fn name(&self) -> &'static str {
        "slack"
    }

    fn display_name(&self) -> &'static str {
        "Slack"
    }

    async fn send_message(&self, message: &OutgoingMessage) -> AppResult<SendMessageResponse> {
        let url = format!("{SLACK_API_BASE}/chat.postMessage");

        let mut body = serde_json::json!({
            "channel": message.channel_id,
            "text": message.text,
        });

        if let Some(ref thread_ts) = message.thread_id {
            body["thread_ts"] = serde_json::Value::String(thread_ts.clone());
        }

        debug!(channel = %message.channel_id, "Sending message to Slack channel");

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::external_service("Slack", format!("HTTP request failed: {e}"))
            })?;

        let status = response.status();
        let response_body: SlackPostMessageResponse = response.json().await.map_err(|e| {
            AppError::external_service("Slack", format!("Failed to parse response: {e}"))
        })?;

        if !response_body.ok {
            let error = response_body.error.unwrap_or_default();
            warn!(
                slack_error = %error,
                http_status = %status,
                "Slack chat.postMessage failed"
            );
            return Err(AppError::external_service(
                "Slack",
                format!("chat.postMessage failed: {error}"),
            ));
        }

        let ts = response_body.ts.unwrap_or_default();
        Ok(SendMessageResponse {
            message_id: ts.clone(),
            timestamp: Some(ts),
        })
    }

    async fn list_channels(&self) -> AppResult<Vec<ChannelInfo>> {
        let url = format!("{SLACK_API_BASE}/conversations.list");

        let response = self
            .http_client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .query(&[
                ("types", "public_channel,private_channel"),
                ("exclude_archived", "true"),
                ("limit", "200"),
            ])
            .send()
            .await
            .map_err(|e| {
                AppError::external_service("Slack", format!("HTTP request failed: {e}"))
            })?;

        let response_body: SlackConversationsListResponse = response.json().await.map_err(|e| {
            AppError::external_service("Slack", format!("Failed to parse response: {e}"))
        })?;

        if !response_body.ok {
            let error = response_body.error.unwrap_or_default();
            return Err(AppError::external_service(
                "Slack",
                format!("conversations.list failed: {error}"),
            ));
        }

        let channels = response_body
            .channels
            .unwrap_or_default()
            .into_iter()
            .map(|ch| ChannelInfo {
                id: ch.id,
                name: ch.name.unwrap_or_default(),
                is_private: ch.is_private.unwrap_or(false),
                member_count: ch.num_members,
                topic: ch.topic.and_then(|t| t.value),
            })
            .collect();

        Ok(channels)
    }

    fn verify_request(&self, headers: &HeaderMap, body: &[u8]) -> AppResult<bool> {
        verify_slack_signature(&self.signing_secret, headers, body)
    }
}
