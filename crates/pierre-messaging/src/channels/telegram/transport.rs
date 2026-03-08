// ABOUTME: Telegram Bot API transport adapter for webhook parsing and message sending
// ABOUTME: Secret token header verification with constant-time comparison, Update payload parsing
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
use tracing::{debug, warn};
use uuid::Uuid;

use crate::transport::{outbound_http_timeout, TransportAdapter};

/// Telegram Bot API transport adapter
///
/// Verification: `X-Telegram-Bot-Api-Secret-Token` header matched against
/// the configured webhook secret using constant-time comparison.
pub struct TelegramTransport {
    /// HTTP client for outbound Bot API calls
    client: reqwest::Client,
    /// Expected secret token for webhook verification
    webhook_secret: String,
}

impl TelegramTransport {
    /// Create a transport with the given webhook secret
    #[must_use]
    pub fn new(webhook_secret: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(outbound_http_timeout())
            .build()
            .unwrap_or_default();
        Self {
            client,
            webhook_secret,
        }
    }
}

#[async_trait]
impl TransportAdapter for TelegramTransport {
    fn verify_signature(&self, headers: &HeaderMap, _body: &[u8]) -> MessagingResult<()> {
        let secret_header = headers
            .get("x-telegram-bot-api-secret-token")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| MessagingError::SignatureVerificationFailed {
                channel: "telegram".to_owned(),
                reason: "missing X-Telegram-Bot-Api-Secret-Token header".to_owned(),
            })?;

        // Constant-time comparison to prevent timing attacks
        let expected = self.webhook_secret.as_bytes();
        let received = secret_header.as_bytes();
        let equal: bool = subtle::ConstantTimeEq::ct_eq(received, expected).into();

        if equal {
            Ok(())
        } else {
            Err(MessagingError::SignatureVerificationFailed {
                channel: "telegram".to_owned(),
                reason: "secret token mismatch".to_owned(),
            })
        }
    }

    async fn parse_inbound(
        &self,
        _headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        let update: Value =
            serde_json::from_slice(body).map_err(|e| MessagingError::InvalidPayload {
                channel: "telegram".to_owned(),
                reason: format!("invalid JSON: {e}"),
            })?;

        // Telegram sends one Update per webhook — check for callback_query first (button taps)
        if let Some(callback) = update.get("callback_query") {
            return Ok(parse_callback_query(callback, &update));
        }

        let Some(message) = update.get("message") else {
            debug!("Telegram update without message or callback_query field");
            return Ok(vec![]);
        };

        let chat_id = message
            .pointer("/chat/id")
            .and_then(Value::as_i64)
            .ok_or_else(|| MessagingError::InvalidPayload {
                channel: "telegram".to_owned(),
                reason: "missing chat.id".to_owned(),
            })?;

        let from_id = message
            .pointer("/from/id")
            .and_then(Value::as_i64)
            .unwrap_or(chat_id);

        let from_name = message
            .pointer("/from/first_name")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let message_id = message
            .get("message_id")
            .and_then(Value::as_i64)
            .unwrap_or(0);

        let content = message.get("text").and_then(Value::as_str).map_or_else(
            || parse_non_text_content(message),
            |text| MessageContent::Text {
                body: text.to_owned(),
            },
        );

        let incoming = IncomingMessage {
            channel_type: ChannelType::Telegram,
            sender_id: from_id.to_string(),
            sender_name: from_name,
            content,
            conversation_id: Some(chat_id.to_string()),
            channel_message_id: message_id.to_string(),
            timestamp: Utc::now(),
            raw_payload: update,
            correlation_id: Uuid::new_v4(),
            metadata: Value::Null,
        };

        Ok(vec![incoming])
    }

    async fn send_raw(
        &self,
        payload: &Value,
        config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        let bot_token =
            config
                .bot_token
                .as_deref()
                .ok_or_else(|| MessagingError::ChannelNotConfigured {
                    channel: "telegram".to_owned(),
                })?;

        let method = resolve_bot_api_method(payload);
        let url = format!("https://api.telegram.org/bot{bot_token}/{method}");

        let response = self
            .client
            .post(&url)
            .json(payload)
            .send()
            .await
            .map_err(|e| MessagingError::DeliveryFailed {
                channel: "telegram".to_owned(),
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
                channel: "telegram".to_owned(),
                status_code: status,
                message: body_text,
            });
        }

        let result: Value = response
            .json()
            .await
            .map_err(|e| MessagingError::InvalidPayload {
                channel: "telegram".to_owned(),
                reason: format!("invalid response JSON: {e}"),
            })?;

        let channel_message_id = result
            .pointer("/result/message_id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string());

        Ok(DeliveryReceipt {
            message_id: Uuid::new_v4().to_string(),
            channel_message_id,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
        })
    }
}

/// Resolve the Telegram Bot API method from the rendered payload shape
///
/// The renderer produces payloads with different keys depending on content type:
/// - `"photo"` key → `sendPhoto`
/// - `"latitude"` key → `sendLocation`
/// - default → `sendMessage` (text and card content)
fn resolve_bot_api_method(payload: &Value) -> &'static str {
    if payload.get("photo").is_some() {
        "sendPhoto"
    } else if payload.get("latitude").is_some() {
        "sendLocation"
    } else {
        "sendMessage"
    }
}

/// Parse a Telegram `callback_query` update (inline keyboard button tap)
///
/// The `callback_query` contains the button's `data` field and the original message context.
fn parse_callback_query(callback: &Value, update: &Value) -> Vec<IncomingMessage> {
    let from_id = callback
        .pointer("/from/id")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let from_name = callback
        .pointer("/from/first_name")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let chat_id = callback
        .pointer("/message/chat/id")
        .and_then(Value::as_i64)
        .unwrap_or(from_id);

    let callback_data = callback.get("data").and_then(Value::as_str).unwrap_or("");
    let callback_id = callback.get("id").and_then(Value::as_str).unwrap_or("0");

    let incoming = IncomingMessage {
        channel_type: ChannelType::Telegram,
        sender_id: from_id.to_string(),
        sender_name: from_name,
        content: MessageContent::Text {
            body: callback_data.to_owned(),
        },
        conversation_id: Some(chat_id.to_string()),
        channel_message_id: callback_id.to_owned(),
        timestamp: Utc::now(),
        raw_payload: update.clone(),
        correlation_id: Uuid::new_v4(),
        metadata: Value::Null,
    };

    vec![incoming]
}

/// Parse non-text message content (location, photo, or unsupported)
fn parse_non_text_content(message: &Value) -> MessageContent {
    if let Some(location) = message.get("location") {
        let lat = location
            .get("latitude")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let lon = location
            .get("longitude")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        return MessageContent::Location {
            latitude: lat,
            longitude: lon,
        };
    }

    if message.get("photo").is_some() {
        // Photo array — take the largest (last) entry
        let photo_url = message
            .get("photo")
            .and_then(Value::as_array)
            .and_then(|arr| arr.last())
            .and_then(|p| p.get("file_id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let caption = message
            .get("caption")
            .and_then(Value::as_str)
            .map(str::to_owned);
        return MessageContent::Media {
            url: photo_url,
            mime_type: "image/jpeg".to_owned(),
            caption,
        };
    }

    warn!("Telegram message with unsupported content type");
    MessageContent::Text {
        body: "[unsupported message type]".to_owned(),
    }
}
