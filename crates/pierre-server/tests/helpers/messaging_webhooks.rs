// ABOUTME: Per-channel webhook payload builders and signature helpers for messaging tests
// ABOUTME: Supports Telegram, Slack, Discord, WhatsApp, Messenger with correct crypto
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use serde_json::{json, Value};
use sha2::Sha256;

/// The five supported messaging channels exercised by the command coverage matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TestChannel {
    Telegram,
    Slack,
    Discord,
    WhatsApp,
    Messenger,
}

impl TestChannel {
    /// All channels in a fixed iteration order
    pub const ALL: [Self; 5] = [
        Self::Telegram,
        Self::Slack,
        Self::Discord,
        Self::WhatsApp,
        Self::Messenger,
    ];

    /// Canonical channel string matching `ChannelType` serialization and webhook path segment.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Slack => "slack",
            Self::Discord => "discord",
            Self::WhatsApp => "whatsapp",
            Self::Messenger => "messenger",
        }
    }

    /// Full webhook path on the Axum router
    #[must_use]
    pub fn webhook_path(self) -> String {
        format!("/api/messaging/webhook/{}", self.as_str())
    }
}

/// Test secrets and Ed25519 keypair used to sign outbound test webhooks.
///
/// Generate once per test process via [`ChannelSecrets::generate`] and share
/// across all (command, channel) combos in the matrix.
pub struct ChannelSecrets {
    pub telegram_webhook_secret: String,
    pub slack_signing_secret: String,
    pub whatsapp_app_secret: String,
    pub messenger_app_secret: String,
    pub discord_signing_key: SigningKey,
    pub discord_public_key_hex: String,
}

impl ChannelSecrets {
    /// Generate a fresh set of secrets and a Discord Ed25519 keypair.
    #[must_use]
    pub fn generate() -> Self {
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        let signing_key = SigningKey::from_bytes(&key_bytes);
        let public_key_hex = hex::encode(signing_key.verifying_key().as_bytes());
        Self {
            telegram_webhook_secret: "test_telegram_webhook_secret".to_owned(),
            slack_signing_secret: "test_slack_signing_secret".to_owned(),
            whatsapp_app_secret: "test_whatsapp_app_secret".to_owned(),
            messenger_app_secret: "test_messenger_app_secret".to_owned(),
            discord_signing_key: signing_key,
            discord_public_key_hex: public_key_hex,
        }
    }
}

/// A built webhook request: path + headers + JSON body.
///
/// Callers assemble an `AxumTestRequest` by iterating the headers and using
/// `.json(&request.body_value)` so the body bytes match what was signed.
pub struct WebhookRequest {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body_value: Value,
}

type HmacSha256 = Hmac<Sha256>;

fn hmac_hex(secret: &str, data: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

/// Build a Telegram Bot API Update payload with the given text.
///
/// Uses the secret-token header scheme (no body hashing).
#[must_use]
pub fn telegram_webhook(
    text: &str,
    msg_id: i64,
    sender_id: &str,
    secrets: &ChannelSecrets,
) -> WebhookRequest {
    let numeric_sender: i64 = sender_id.parse().unwrap_or(99);
    let body_value = json!({
        "update_id": 2000 + msg_id,
        "message": {
            "message_id": msg_id,
            "from": { "id": numeric_sender, "first_name": "CmdTest" },
            "chat": { "id": numeric_sender, "type": "private" },
            "text": text,
        }
    });
    WebhookRequest {
        path: TestChannel::Telegram.webhook_path(),
        headers: vec![(
            "x-telegram-bot-api-secret-token".to_owned(),
            secrets.telegram_webhook_secret.clone(),
        )],
        body_value,
    }
}

/// Build a Slack `event_callback` webhook payload signed with HMAC-SHA256 v0.
#[must_use]
pub fn slack_webhook(
    text: &str,
    msg_id: i64,
    sender_id: &str,
    secrets: &ChannelSecrets,
) -> WebhookRequest {
    let body_value = json!({
        "type": "event_callback",
        "event": {
            "type": "message",
            "user": sender_id,
            "text": text,
            "channel": "C_TEST",
            "ts": format!("1700000000.{msg_id:06}"),
        },
    });
    // Slack v0 signature basestring is `v0:{timestamp}:{body}` — body must
    // match what the router receives, so we use the same serializer (`to_string`)
    // that `AxumTestRequest::json` uses downstream.
    let body_str = serde_json::to_string(&body_value).expect("serialize slack body");
    let timestamp = Utc::now().timestamp().to_string();
    let basestring = format!("v0:{timestamp}:{body_str}");
    let signature = format!(
        "v0={}",
        hmac_hex(&secrets.slack_signing_secret, basestring.as_bytes())
    );

    WebhookRequest {
        path: TestChannel::Slack.webhook_path(),
        headers: vec![
            ("x-slack-request-timestamp".to_owned(), timestamp),
            ("x-slack-signature".to_owned(), signature),
        ],
        body_value,
    }
}

/// Build a Discord `APPLICATION_COMMAND` interaction signed with Ed25519.
///
/// The user's slash-command text is carried in `data.options[0].value` which
/// is what the canot Discord transport extracts as message text.
#[must_use]
pub fn discord_webhook(
    text: &str,
    msg_id: i64,
    sender_id: &str,
    secrets: &ChannelSecrets,
) -> WebhookRequest {
    let body_value = json!({
        "type": 2,
        "id": format!("interaction_{msg_id}"),
        "application_id": "test-discord-app",
        "channel_id": "channel-test",
        "member": {
            "user": { "id": sender_id, "username": "cmdtest" }
        },
        "data": {
            "name": "pierre",
            "options": [{ "name": "text", "value": text, "type": 3 }]
        }
    });
    let body_bytes = serde_json::to_vec(&body_value).expect("serialize discord body");
    let timestamp = Utc::now().timestamp().to_string();
    let mut message = Vec::with_capacity(timestamp.len() + body_bytes.len());
    message.extend_from_slice(timestamp.as_bytes());
    message.extend_from_slice(&body_bytes);
    let signature = secrets.discord_signing_key.sign(&message);
    let signature_hex = hex::encode(signature.to_bytes());

    WebhookRequest {
        path: TestChannel::Discord.webhook_path(),
        headers: vec![
            ("x-signature-ed25519".to_owned(), signature_hex),
            ("x-signature-timestamp".to_owned(), timestamp),
        ],
        body_value,
    }
}

/// Build a `WhatsApp` Business Cloud API webhook signed with Meta HMAC-SHA256.
#[must_use]
pub fn whatsapp_webhook(
    text: &str,
    msg_id: i64,
    sender_id: &str,
    phone_number_id: &str,
    secrets: &ChannelSecrets,
) -> WebhookRequest {
    let body_value = json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "test-wa-account",
            "changes": [{
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "+15551234567",
                        "phone_number_id": phone_number_id,
                    },
                    "messages": [{
                        "from": sender_id,
                        "id": format!("wamid.test_{msg_id}"),
                        "timestamp": "1700000000",
                        "type": "text",
                        "text": { "body": text }
                    }]
                },
                "field": "messages"
            }]
        }]
    });
    let body_bytes = serde_json::to_vec(&body_value).expect("serialize whatsapp body");
    let signature = format!(
        "sha256={}",
        hmac_hex(&secrets.whatsapp_app_secret, &body_bytes)
    );

    WebhookRequest {
        path: TestChannel::WhatsApp.webhook_path(),
        headers: vec![("x-hub-signature-256".to_owned(), signature)],
        body_value,
    }
}

/// Build a Messenger Page webhook signed with Meta HMAC-SHA256.
#[must_use]
pub fn messenger_webhook(
    text: &str,
    msg_id: i64,
    sender_id: &str,
    page_id: &str,
    secrets: &ChannelSecrets,
) -> WebhookRequest {
    let body_value = json!({
        "object": "page",
        "entry": [{
            "id": page_id,
            "time": 1_700_000_000,
            "messaging": [{
                "sender": { "id": sender_id },
                "recipient": { "id": page_id },
                "timestamp": 1_700_000_000,
                "message": {
                    "mid": format!("mid.test_{msg_id}"),
                    "text": text,
                }
            }]
        }]
    });
    let body_bytes = serde_json::to_vec(&body_value).expect("serialize messenger body");
    let signature = format!(
        "sha256={}",
        hmac_hex(&secrets.messenger_app_secret, &body_bytes)
    );

    WebhookRequest {
        path: TestChannel::Messenger.webhook_path(),
        headers: vec![("x-hub-signature-256".to_owned(), signature)],
        body_value,
    }
}
