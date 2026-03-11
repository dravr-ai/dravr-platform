// ABOUTME: Discord Bot API transport with Ed25519 signature verification
// ABOUTME: Verifies x-signature-ed25519 header using ed25519-dalek, parses interaction payloads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use http::HeaderMap;
use pierre_core::errors::messaging::{MessagingError, MessagingResult};
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage, MessageContent,
};
use serde_json::Value;
use tracing::debug;
use uuid::Uuid;

use pierre_core::http_client::api_client;

use crate::transport::TransportAdapter;

/// Discord Bot API transport adapter
///
/// Verification: Ed25519 signature using the application's public key.
/// Signs `timestamp + body` and compares against `x-signature-ed25519` header.
pub struct DiscordTransport {
    /// Hex-encoded Ed25519 public key
    public_key_hex: String,
    /// Discord application ID for webhook endpoint construction
    application_id: String,
    /// Shared HTTP client for outbound Discord API calls
    client: &'static reqwest::Client,
}

impl DiscordTransport {
    /// Create a transport with the given Ed25519 public key and application ID
    #[must_use]
    pub fn new(public_key_hex: String, application_id: String) -> Self {
        Self {
            public_key_hex,
            application_id,
            client: api_client(),
        }
    }
}

#[async_trait]
impl TransportAdapter for DiscordTransport {
    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        let signature_hex = headers
            .get("x-signature-ed25519")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| MessagingError::SignatureVerificationFailed {
                channel: "discord".to_owned(),
                reason: "missing x-signature-ed25519 header".to_owned(),
            })?;

        let timestamp = headers
            .get("x-signature-timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| MessagingError::SignatureVerificationFailed {
                channel: "discord".to_owned(),
                reason: "missing x-signature-timestamp header".to_owned(),
            })?;

        // Decode the public key and signature from hex
        let pk_bytes = hex::decode(&self.public_key_hex).map_err(|e| {
            MessagingError::SignatureVerificationFailed {
                channel: "discord".to_owned(),
                reason: format!("invalid public key hex: {e}"),
            }
        })?;

        let sig_bytes = hex::decode(signature_hex).map_err(|e| {
            MessagingError::SignatureVerificationFailed {
                channel: "discord".to_owned(),
                reason: format!("invalid signature hex: {e}"),
            }
        })?;

        let pk_array: [u8; 32] =
            pk_bytes
                .try_into()
                .map_err(|_| MessagingError::SignatureVerificationFailed {
                    channel: "discord".to_owned(),
                    reason: "public key must be 32 bytes".to_owned(),
                })?;

        let verifying_key = VerifyingKey::from_bytes(&pk_array).map_err(|e| {
            MessagingError::SignatureVerificationFailed {
                channel: "discord".to_owned(),
                reason: format!("invalid Ed25519 public key: {e}"),
            }
        })?;

        let signature = Signature::from_slice(&sig_bytes).map_err(|e| {
            MessagingError::SignatureVerificationFailed {
                channel: "discord".to_owned(),
                reason: format!("invalid Ed25519 signature: {e}"),
            }
        })?;

        // Discord signs: timestamp + body
        let mut message = Vec::with_capacity(timestamp.len() + body.len());
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);

        verifying_key.verify(&message, &signature).map_err(|_| {
            MessagingError::SignatureVerificationFailed {
                channel: "discord".to_owned(),
                reason: "Ed25519 signature verification failed".to_owned(),
            }
        })
    }

    async fn parse_inbound(
        &self,
        _headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        let payload: Value =
            serde_json::from_slice(body).map_err(|e| MessagingError::InvalidPayload {
                channel: "discord".to_owned(),
                reason: format!("invalid JSON: {e}"),
            })?;

        // Discord interaction types: 1=PING, 2=APPLICATION_COMMAND, 3=MESSAGE_COMPONENT
        let interaction_type = payload.get("type").and_then(Value::as_u64).unwrap_or(0);

        // Type 1 = PING: respond with pong (handled at route level)
        if interaction_type == 1 {
            debug!("Discord PING interaction, no messages to parse");
            return Ok(vec![]);
        }

        // Extract content based on interaction type
        let content_text = if interaction_type == 3 {
            // MESSAGE_COMPONENT: button click — extract custom_id as the action payload
            payload
                .pointer("/data/custom_id")
                .and_then(Value::as_str)
                .unwrap_or("")
        } else {
            // APPLICATION_COMMAND: extract from options or resolved messages
            let data = payload.get("data");
            data.and_then(|d| d.get("options"))
                .and_then(Value::as_array)
                .and_then(|opts| opts.first())
                .and_then(|opt| opt.get("value"))
                .and_then(Value::as_str)
                .or_else(|| {
                    payload
                        .pointer("/data/resolved/messages")
                        .and_then(Value::as_object)
                        .and_then(|msgs| msgs.values().next())
                        .and_then(|msg| msg.get("content"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("")
        };

        let user_id = payload
            .pointer("/member/user/id")
            .or_else(|| payload.pointer("/user/id"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        let username = payload
            .pointer("/member/user/username")
            .or_else(|| payload.pointer("/user/username"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let channel_id = payload
            .get("channel_id")
            .and_then(Value::as_str)
            .unwrap_or("");

        let interaction_id = payload.get("id").and_then(Value::as_str).unwrap_or("0");

        let incoming = IncomingMessage {
            channel_type: ChannelType::Discord,
            sender_id: user_id.to_owned(),
            sender_name: username,
            content: MessageContent::Text {
                body: content_text.to_owned(),
            },
            conversation_id: Some(channel_id.to_owned()),
            channel_message_id: interaction_id.to_owned(),
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
        let bot_token =
            config
                .bot_token
                .as_deref()
                .ok_or_else(|| MessagingError::ChannelNotConfigured {
                    channel: "discord".to_owned(),
                })?;

        // Determine URL: interaction followup or channel message
        let interaction_token = payload.get("interaction_token").and_then(Value::as_str);

        let (url, send_payload) = interaction_token.map_or_else(
            || {
                // Channel message via REST API
                let channel_id = payload
                    .get("channel_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let url = format!("https://discord.com/api/v10/channels/{channel_id}/messages");
                (url, payload.clone())
            },
            |token| {
                let url = format!(
                    "https://discord.com/api/v10/webhooks/{}/{token}",
                    self.application_id
                );
                // Remove the interaction_token from the payload before sending
                let mut p = payload.clone();
                if let Some(obj) = p.as_object_mut() {
                    obj.remove("interaction_token");
                }
                (url, p)
            },
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {bot_token}"))
            .json(&send_payload)
            .send()
            .await
            .map_err(|e| MessagingError::DeliveryFailed {
                channel: "discord".to_owned(),
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
                channel: "discord".to_owned(),
                status_code: status,
                message: body_text,
            });
        }

        let result: Value = response
            .json()
            .await
            .map_err(|e| MessagingError::InvalidPayload {
                channel: "discord".to_owned(),
                reason: format!("invalid response JSON: {e}"),
            })?;

        let channel_message_id = result.get("id").and_then(Value::as_str).map(str::to_owned);

        Ok(DeliveryReceipt {
            message_id: Uuid::new_v4().to_string(),
            channel_message_id,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
        })
    }
}
