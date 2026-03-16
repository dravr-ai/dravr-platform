// ABOUTME: WhatsApp Business Cloud API transport with x-hub-signature-256 HMAC-SHA256 verification
// ABOUTME: Parses webhook entry[].changes[].value.messages[] into normalized IncomingMessage structs
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

use pierre_core::http_client::api_client;

use crate::meta_signature::verify_meta_signature;
use crate::transport::TransportAdapter;

/// `WhatsApp` Business Cloud API transport adapter
///
/// Verification: HMAC-SHA256 of the request body using the app secret.
/// Header `x-hub-signature-256` contains `sha256={hex}` — identical to Messenger.
pub struct WhatsAppTransport {
    /// Shared HTTP client for outbound Graph API calls
    client: &'static reqwest::Client,
    /// Facebook/Meta app secret for webhook HMAC verification
    app_secret: String,
}

impl WhatsAppTransport {
    /// Create a transport with the given Meta app secret
    #[must_use]
    pub fn new(app_secret: String) -> Self {
        Self {
            client: api_client(),
            app_secret,
        }
    }
}

#[async_trait]
impl TransportAdapter for WhatsAppTransport {
    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        verify_meta_signature("whatsapp", &self.app_secret, headers, body)
    }

    async fn parse_inbound(
        &self,
        _headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        let payload: Value =
            serde_json::from_slice(body).map_err(|e| MessagingError::InvalidPayload {
                channel: "whatsapp".to_owned(),
                reason: format!("invalid JSON: {e}"),
            })?;

        let mut messages = Vec::new();

        // WhatsApp Cloud API: { "entry": [{ "changes": [{ "value": { "messages": [...] } }] }] }
        let entries = payload.get("entry").and_then(Value::as_array);

        let Some(entries) = entries else {
            return Ok(messages);
        };

        for entry in entries {
            let Some(changes) = entry.get("changes").and_then(Value::as_array) else {
                continue;
            };

            for change in changes {
                let Some(value) = change.get("value") else {
                    continue;
                };

                // Skip status updates (delivery receipts) — only process user messages
                let Some(msgs) = value.get("messages").and_then(Value::as_array) else {
                    continue;
                };

                for msg in msgs {
                    let sender_id = msg.get("from").and_then(Value::as_str).unwrap_or("unknown");

                    let channel_message_id = msg.get("id").and_then(Value::as_str).unwrap_or("");

                    let content = parse_whatsapp_content(msg);

                    messages.push(IncomingMessage {
                        channel_type: ChannelType::WhatsApp,
                        sender_id: sender_id.to_owned(),
                        sender_name: None,
                        content,
                        conversation_id: None,
                        channel_message_id: channel_message_id.to_owned(),
                        timestamp: Utc::now(),
                        raw_payload: msg.clone(),
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
                    channel: "whatsapp".to_owned(),
                })?;

        let phone_number_id =
            config
                .phone_number
                .as_deref()
                .ok_or_else(|| MessagingError::ChannelNotConfigured {
                    channel: "whatsapp".to_owned(),
                })?;

        let url = format!("https://graph.facebook.com/v22.0/{phone_number_id}/messages");

        let response = self
            .client
            .post(&url)
            .bearer_auth(access_token)
            .json(payload)
            .send()
            .await
            .map_err(|e| MessagingError::DeliveryFailed {
                channel: "whatsapp".to_owned(),
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
                channel: "whatsapp".to_owned(),
                status_code: status,
                message: body_text,
            });
        }

        let result: Value = response
            .json()
            .await
            .map_err(|e| MessagingError::InvalidPayload {
                channel: "whatsapp".to_owned(),
                reason: format!("invalid response JSON: {e}"),
            })?;

        // Response: { "messages": [{ "id": "wamid.xxx" }] }
        let channel_message_id = result
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|m| m.get("id"))
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

/// Parse message content from a `WhatsApp` Cloud API message object
fn parse_whatsapp_content(msg: &Value) -> MessageContent {
    let msg_type = msg.get("type").and_then(Value::as_str).unwrap_or("text");

    match msg_type {
        "text" => {
            let body = msg
                .pointer("/text/body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            MessageContent::Text { body }
        }
        "image" | "video" | "audio" | "document" => {
            let media_obj = msg.get(msg_type);
            let url = media_obj
                .and_then(|m| m.get("link").or_else(|| m.get("id")))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let mime_type = media_obj
                .and_then(|m| m.get("mime_type"))
                .and_then(Value::as_str)
                .unwrap_or(&format!("{msg_type}/*"))
                .to_owned();
            let caption = media_obj
                .and_then(|m| m.get("caption"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            MessageContent::Media {
                url,
                mime_type,
                caption,
            }
        }
        "location" => {
            let latitude = msg
                .pointer("/location/latitude")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let longitude = msg
                .pointer("/location/longitude")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            MessageContent::Location {
                latitude,
                longitude,
            }
        }
        _ => MessageContent::Text {
            body: "[unsupported message type]".to_owned(),
        },
    }
}
