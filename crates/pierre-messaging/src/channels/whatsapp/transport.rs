// ABOUTME: WhatsApp Business API transport via Twilio with HMAC-SHA256 signature verification
// ABOUTME: Verifies x-twilio-signature header using webhook secret, parses Twilio webhook payloads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, Mac};
use http::HeaderMap;
use pierre_core::errors::messaging::{MessagingError, MessagingResult};
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage, MessageContent,
};
use serde_json::Value;
use sha2::Sha256;
use uuid::Uuid;

use crate::transport::TransportAdapter;

/// `WhatsApp` Business API transport adapter via Twilio
///
/// Verification: HMAC-SHA256 of the request body using the Twilio webhook secret.
/// The signature is sent in the `x-twilio-signature` header as hex.
pub struct WhatsAppTransport {
    /// HTTP client for outbound Twilio API calls
    client: reqwest::Client,
    /// Twilio webhook signing secret
    webhook_secret: String,
}

impl WhatsAppTransport {
    /// Create a transport with the given Twilio webhook secret
    #[must_use]
    pub fn new(webhook_secret: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            webhook_secret,
        }
    }
}

#[async_trait]
impl TransportAdapter for WhatsAppTransport {
    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        let signature = headers
            .get("x-twilio-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| MessagingError::SignatureVerificationFailed {
                channel: "whatsapp".to_owned(),
                reason: "missing x-twilio-signature header".to_owned(),
            })?;

        // Compute HMAC-SHA256 over the body
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.webhook_secret.as_bytes()).map_err(|e| {
                MessagingError::SignatureVerificationFailed {
                    channel: "whatsapp".to_owned(),
                    reason: format!("HMAC key error: {e}"),
                }
            })?;
        mac.update(body);
        let expected = hex::encode(mac.finalize().into_bytes());

        // Constant-time comparison
        let equal: bool =
            subtle::ConstantTimeEq::ct_eq(signature.as_bytes(), expected.as_bytes()).into();
        if equal {
            Ok(())
        } else {
            Err(MessagingError::SignatureVerificationFailed {
                channel: "whatsapp".to_owned(),
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
                channel: "whatsapp".to_owned(),
                reason: format!("invalid JSON: {e}"),
            })?;

        // Twilio WhatsApp webhook format
        let from = payload
            .get("From")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let body_text = payload.get("Body").and_then(Value::as_str).unwrap_or("");
        let message_sid = payload
            .get("MessageSid")
            .and_then(Value::as_str)
            .unwrap_or("");

        let content = payload
            .get("MediaUrl0")
            .and_then(Value::as_str)
            .map_or_else(
                || MessageContent::Text {
                    body: body_text.to_owned(),
                },
                |media_url| {
                    let mime_type = payload
                        .get("MediaContentType0")
                        .and_then(Value::as_str)
                        .unwrap_or("application/octet-stream");
                    MessageContent::Media {
                        url: media_url.to_owned(),
                        mime_type: mime_type.to_owned(),
                        caption: if body_text.is_empty() {
                            None
                        } else {
                            Some(body_text.to_owned())
                        },
                    }
                },
            );

        let incoming = IncomingMessage {
            channel_type: ChannelType::WhatsApp,
            sender_id: from.to_owned(),
            sender_name: None,
            content,
            conversation_id: None,
            channel_message_id: message_sid.to_owned(),
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
        let account_sid =
            config
                .account_id
                .as_deref()
                .ok_or_else(|| MessagingError::ChannelNotConfigured {
                    channel: "whatsapp".to_owned(),
                })?;
        let auth_token =
            config
                .api_secret
                .as_deref()
                .ok_or_else(|| MessagingError::ChannelNotConfigured {
                    channel: "whatsapp".to_owned(),
                })?;

        let url = format!("https://api.twilio.com/2010-04-01/Accounts/{account_sid}/Messages.json");

        // Twilio expects form-encoded params
        let to = payload.get("To").and_then(Value::as_str).unwrap_or("");
        let from = payload.get("From").and_then(Value::as_str).unwrap_or("");
        let body_text = payload.get("Body").and_then(Value::as_str).unwrap_or("");

        let response = self
            .client
            .post(&url)
            .basic_auth(account_sid, Some(auth_token))
            .form(&[("To", to), ("From", from), ("Body", body_text)])
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

        let channel_message_id = result.get("sid").and_then(Value::as_str).map(str::to_owned);

        Ok(DeliveryReceipt {
            message_id: Uuid::new_v4().to_string(),
            channel_message_id,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
        })
    }
}
