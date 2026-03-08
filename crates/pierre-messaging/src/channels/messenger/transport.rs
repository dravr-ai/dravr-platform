// ABOUTME: Meta Messenger Platform transport with x-hub-signature-256 HMAC-SHA256 verification
// ABOUTME: Parses webhook entry batches into normalized IncomingMessage structs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use http::HeaderMap;
use pierre_core::errors::messaging::{MessagingError, MessagingResult};
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage, MessageContent,
};
use serde_json::Value;
use uuid::Uuid;

use crate::meta_signature::verify_meta_signature;
use crate::transport::{outbound_http_timeout, TransportAdapter};

/// Meta Messenger Platform transport adapter
///
/// Verification: HMAC-SHA256 of the request body using the app secret.
/// Header `x-hub-signature-256` contains `sha256={hex}`.
pub struct MessengerTransport {
    /// HTTP client for outbound Graph API calls
    client: reqwest::Client,
    /// Facebook app secret for webhook HMAC verification
    app_secret: String,
}

impl MessengerTransport {
    /// Create a transport with the given Facebook app secret
    #[must_use]
    pub fn new(app_secret: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(outbound_http_timeout())
            .build()
            .unwrap_or_default();
        Self { client, app_secret }
    }
}

#[async_trait]
impl TransportAdapter for MessengerTransport {
    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        verify_meta_signature("messenger", &self.app_secret, headers, body)
    }

    async fn parse_inbound(
        &self,
        _headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        let payload: Value =
            serde_json::from_slice(body).map_err(|e| MessagingError::InvalidPayload {
                channel: "messenger".to_owned(),
                reason: format!("invalid JSON: {e}"),
            })?;

        let mut messages = Vec::new();

        // Messenger batches: { "entry": [{ "messaging": [...] }] }
        let entries = payload.get("entry").and_then(Value::as_array);

        let Some(entries) = entries else {
            return Ok(messages);
        };

        for entry in entries {
            let Some(messaging_events) = entry.get("messaging").and_then(Value::as_array) else {
                continue;
            };

            for event in messaging_events {
                let sender_id = event
                    .pointer("/sender/id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");

                // Handle regular messages
                if let Some(msg) = event.get("message") {
                    let mid = msg.get("mid").and_then(Value::as_str).unwrap_or("");
                    let content = parse_message_content(msg);

                    messages.push(IncomingMessage {
                        channel_type: ChannelType::Messenger,
                        sender_id: sender_id.to_owned(),
                        sender_name: None,
                        content,
                        conversation_id: Some(sender_id.to_owned()),
                        channel_message_id: mid.to_owned(),
                        timestamp: Utc::now(),
                        raw_payload: event.clone(),
                        correlation_id: Uuid::new_v4(),
                        metadata: Value::Null,
                    });
                    continue;
                }

                // Handle postback events (button taps from generic templates)
                if let Some(postback) = event.get("postback") {
                    let payload_text = postback
                        .get("payload")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let mid = postback.get("mid").and_then(Value::as_str).unwrap_or("");

                    messages.push(IncomingMessage {
                        channel_type: ChannelType::Messenger,
                        sender_id: sender_id.to_owned(),
                        sender_name: None,
                        content: MessageContent::Text {
                            body: payload_text.to_owned(),
                        },
                        conversation_id: Some(sender_id.to_owned()),
                        channel_message_id: mid.to_owned(),
                        timestamp: Utc::now(),
                        raw_payload: event.clone(),
                        correlation_id: Uuid::new_v4(),
                        metadata: Value::Null,
                    });
                }
            }
        }

        Ok(messages)
    }

    async fn send_raw(
        &self,
        payload: &Value,
        config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        let access_token =
            config
                .api_key
                .as_deref()
                .ok_or_else(|| MessagingError::ChannelNotConfigured {
                    channel: "messenger".to_owned(),
                })?;

        let url = "https://graph.facebook.com/v18.0/me/messages";

        let response = self
            .client
            .post(url)
            .bearer_auth(access_token)
            .json(payload)
            .send()
            .await
            .map_err(|e| MessagingError::DeliveryFailed {
                channel: "messenger".to_owned(),
                reason: format!("HTTP request failed: {e}"),
                retryable: true,
            })?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body_text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_owned());
            return Err(MessagingError::ChannelApiError {
                channel: "messenger".to_owned(),
                status_code: status,
                message: body_text,
            });
        }

        let result: Value = response
            .json()
            .await
            .map_err(|e| MessagingError::InvalidPayload {
                channel: "messenger".to_owned(),
                reason: format!("invalid response JSON: {e}"),
            })?;

        let channel_message_id = result
            .get("message_id")
            .and_then(Value::as_str)
            .map(str::to_owned);

        Ok(DeliveryReceipt {
            message_id: Uuid::new_v4().to_string(),
            channel_message_id,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
        })
    }
}

/// Parse message content from a Messenger message object
fn parse_message_content(msg: &Value) -> MessageContent {
    // Check for text first
    if let Some(text) = msg.get("text").and_then(Value::as_str) {
        return MessageContent::Text {
            body: text.to_owned(),
        };
    }

    // Check for attachments
    if let Some(attachments) = msg.get("attachments").and_then(Value::as_array) {
        if let Some(attachment) = attachments.first() {
            let att_type = attachment.get("type").and_then(Value::as_str).unwrap_or("");

            if att_type == "location" {
                let lat = attachment
                    .pointer("/payload/coordinates/lat")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let lon = attachment
                    .pointer("/payload/coordinates/long")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                return MessageContent::Location {
                    latitude: lat,
                    longitude: lon,
                };
            }

            if matches!(att_type, "image" | "video" | "audio" | "file") {
                let url = attachment
                    .pointer("/payload/url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                return MessageContent::Media {
                    url,
                    mime_type: format!("{att_type}/*"),
                    caption: None,
                };
            }
        }
    }

    MessageContent::Text {
        body: "[unsupported message type]".to_owned(),
    }
}
