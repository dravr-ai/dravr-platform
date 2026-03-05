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

use crate::transport::TransportAdapter;

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
        Self {
            client: reqwest::Client::new(),
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
        let payload: Value =
            serde_json::from_slice(body).map_err(|e| MessagingError::InvalidPayload {
                channel: "slack".to_owned(),
                reason: format!("invalid JSON: {e}"),
            })?;

        // Slack Events API sends events in the "event" field
        let Some(event) = payload.get("event") else {
            debug!("Slack payload without event field (may be url_verification)");
            return Ok(vec![]);
        };

        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        if event_type != "message" {
            debug!(event_type, "Ignoring non-message Slack event");
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

        let incoming = IncomingMessage {
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
        };

        Ok(vec![incoming])
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
