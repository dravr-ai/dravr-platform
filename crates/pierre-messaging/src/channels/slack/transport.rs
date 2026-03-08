// ABOUTME: Slack Events API transport with HMAC-SHA256 v0 signature verification
// ABOUTME: Verifies x-slack-signature header with timestamp replay protection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str;

use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, Mac};
use http::HeaderMap;
use pierre_core::errors::messaging::{MessagingError, MessagingResult};
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage, MessageContent,
    WebhookTimestampPolicy,
};
use serde_json::Value;
use sha2::Sha256;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::transport::{outbound_http_timeout, TransportAdapter};

/// Slack Events API transport adapter
///
/// Verification: HMAC-SHA256 with Slack's v0 scheme.
/// `basestring = "v0:{timestamp}:{body}"`, then `v0={hex(HMAC(signing_secret, basestring))}`.
pub struct SlackTransport {
    /// HTTP client for outbound Slack API calls
    client: reqwest::Client,
    /// Slack signing secret for webhook verification
    signing_secret: String,
}

impl SlackTransport {
    /// Create a transport with the given Slack signing secret
    #[must_use]
    pub fn new(signing_secret: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(outbound_http_timeout())
            .build()
            .unwrap_or_default();
        Self {
            client,
            signing_secret,
        }
    }
}

#[async_trait]
impl TransportAdapter for SlackTransport {
    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| MessagingError::SignatureVerificationFailed {
                channel: "slack".to_owned(),
                reason: "missing x-slack-request-timestamp header".to_owned(),
            })?;

        let signature = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| MessagingError::SignatureVerificationFailed {
                channel: "slack".to_owned(),
                reason: "missing x-slack-signature header".to_owned(),
            })?;

        // Replay protection: reject timestamps older than policy window
        let policy = WebhookTimestampPolicy::default();
        let ts: u64 =
            timestamp
                .parse()
                .map_err(|_| MessagingError::SignatureVerificationFailed {
                    channel: "slack".to_owned(),
                    reason: "invalid timestamp format".to_owned(),
                })?;
        let now = u64::try_from(Utc::now().timestamp()).unwrap_or(0);
        let age = now.saturating_sub(ts);
        if age > policy.max_age_secs {
            return Err(MessagingError::ReplayDetected {
                channel: "slack".to_owned(),
                reason: format!("timestamp {ts} is {age}s old, max {}", policy.max_age_secs),
            });
        }

        // Compute HMAC-SHA256 using Slack v0 scheme
        let body_str = str::from_utf8(body).unwrap_or("");
        let basestring = format!("v0:{timestamp}:{body_str}");
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.signing_secret.as_bytes()).map_err(|e| {
                MessagingError::SignatureVerificationFailed {
                    channel: "slack".to_owned(),
                    reason: format!("HMAC key error: {e}"),
                }
            })?;
        mac.update(basestring.as_bytes());
        let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        // Constant-time comparison
        let equal: bool =
            subtle::ConstantTimeEq::ct_eq(signature.as_bytes(), expected.as_bytes()).into();
        if equal {
            Ok(())
        } else {
            Err(MessagingError::SignatureVerificationFailed {
                channel: "slack".to_owned(),
                reason: "signature mismatch".to_owned(),
            })
        }
    }

    async fn parse_inbound(
        &self,
        _headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        // Slack interactive payloads arrive form-encoded with a `payload` key
        let payload: Value = Self::parse_slack_body(body)?;

        // Check for interactive `block_actions` payload (button taps)
        let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if payload_type == "block_actions" {
            return Ok(parse_block_actions(&payload));
        }

        // Slack Events API sends events in the "event" field
        let Some(event) = payload.get("event") else {
            debug!("Slack payload without event field (may be url_verification)");
            return Ok(vec![]);
        };

        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");

        if event_type != "message" {
            debug!(event_type, "Ignoring unhandled Slack event type");
            return Ok(vec![]);
        }

        // Skip bot messages to avoid loops
        if event.get("bot_id").is_some() {
            return Ok(vec![]);
        }

        let user_id = event
            .get("user")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let text = event.get("text").and_then(Value::as_str).unwrap_or("");
        let channel_id = event.get("channel").and_then(Value::as_str).unwrap_or("");
        let ts = event.get("ts").and_then(Value::as_str).unwrap_or("0");

        Ok(vec![IncomingMessage {
            channel_type: ChannelType::Slack,
            sender_id: user_id.to_owned(),
            sender_name: None,
            content: MessageContent::Text {
                body: text.to_owned(),
            },
            conversation_id: Some(channel_id.to_owned()),
            channel_message_id: ts.to_owned(),
            timestamp: Utc::now(),
            raw_payload: payload,
            correlation_id: Uuid::new_v4(),
            metadata: Value::Null,
        }])
    }

    async fn send_raw(
        &self,
        payload: &Value,
        config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        let token =
            config
                .api_key
                .as_deref()
                .ok_or_else(|| MessagingError::ChannelNotConfigured {
                    channel: "slack".to_owned(),
                })?;

        let response = self
            .client
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {token}"))
            .json(payload)
            .send()
            .await
            .map_err(|e| MessagingError::DeliveryFailed {
                channel: "slack".to_owned(),
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
                channel: "slack".to_owned(),
                status_code: status,
                message: body_text,
            });
        }

        let result: Value = response
            .json()
            .await
            .map_err(|e| MessagingError::InvalidPayload {
                channel: "slack".to_owned(),
                reason: format!("invalid response JSON: {e}"),
            })?;

        // Slack API returns ok: true/false in response body
        let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
        if !ok {
            let error_msg = result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            warn!(error = error_msg, "Slack API returned ok: false");
            return Err(MessagingError::ChannelApiError {
                channel: "slack".to_owned(),
                status_code: 200,
                message: error_msg.to_owned(),
            });
        }

        let channel_message_id = result.get("ts").and_then(Value::as_str).map(str::to_owned);

        Ok(DeliveryReceipt {
            message_id: Uuid::new_v4().to_string(),
            channel_message_id,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
        })
    }
}

impl SlackTransport {
    /// Parse the Slack request body, handling both JSON and form-encoded interactive payloads
    fn parse_slack_body(body: &[u8]) -> MessagingResult<Value> {
        // Try direct JSON first (Events API)
        if let Ok(value) = serde_json::from_slice::<Value>(body) {
            return Ok(value);
        }

        // Try form-encoded with `payload` key (interactive payloads)
        let body_str = str::from_utf8(body).map_err(|e| MessagingError::InvalidPayload {
            channel: "slack".to_owned(),
            reason: format!("invalid UTF-8: {e}"),
        })?;

        for pair in body_str.split('&') {
            if let Some(value) = pair.strip_prefix("payload=") {
                let decoded = percent_decode(value);
                return serde_json::from_str(&decoded).map_err(|e| {
                    MessagingError::InvalidPayload {
                        channel: "slack".to_owned(),
                        reason: format!("invalid JSON in interactive payload: {e}"),
                    }
                });
            }
        }

        Err(MessagingError::InvalidPayload {
            channel: "slack".to_owned(),
            reason: "body is neither valid JSON nor form-encoded interactive payload".to_owned(),
        })
    }
}

/// Parse a Slack `block_actions` interactive payload (button taps)
fn parse_block_actions(payload: &Value) -> Vec<IncomingMessage> {
    let user_id = payload
        .pointer("/user/id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let user_name = payload
        .pointer("/user/name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let channel_id = payload
        .pointer("/channel/id")
        .and_then(Value::as_str)
        .unwrap_or("");

    let Some(actions) = payload.get("actions").and_then(Value::as_array) else {
        debug!("Slack block_actions payload without actions array");
        return vec![];
    };

    actions
        .iter()
        .filter_map(|action| {
            let action_id = action.get("action_id").and_then(Value::as_str)?;
            let action_ts = action
                .get("action_ts")
                .and_then(Value::as_str)
                .unwrap_or("0");

            Some(IncomingMessage {
                channel_type: ChannelType::Slack,
                sender_id: user_id.to_owned(),
                sender_name: user_name.clone(),
                content: MessageContent::Text {
                    body: action_id.to_owned(),
                },
                conversation_id: Some(channel_id.to_owned()),
                channel_message_id: action_ts.to_owned(),
                timestamp: Utc::now(),
                raw_payload: payload.clone(),
                correlation_id: Uuid::new_v4(),
                metadata: Value::Null,
            })
        })
        .collect()
}

/// Decode a percent-encoded (URL-encoded) string
///
/// Handles `+` → space and `%XX` → byte conversions per RFC 3986.
fn percent_decode(input: &str) -> String {
    let mut result = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                result.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                    result.push(byte);
                    i += 3;
                } else {
                    result.push(b'%');
                    i += 1;
                }
            }
            other => {
                result.push(other);
                i += 1;
            }
        }
    }

    String::from_utf8(result).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
