// ABOUTME: Unit tests for messaging transport adapter signature verification and webhook parsing
// ABOUTME: Validates HMAC-SHA256 (WhatsApp, Messenger, Slack), Ed25519 (Discord), and secret token (Telegram)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args,
    clippy::redundant_closure_for_method_calls
)]

use hmac::{Hmac, Mac};
use http::HeaderMap;
use pierre_core::models::messaging::{ChannelType, MessageContent};
use pierre_messaging::transport::TransportAdapter;
use sha2::Sha256;

// ── Helper: compute HMAC-SHA256 hex digest ──

fn hmac_sha256_hex(secret: &str, data: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

// ═══════════════════════════════════════════════════════════════════════════════
// WhatsApp Transport Tests (HMAC-SHA256)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod whatsapp {
    use super::*;
    use pierre_messaging::channels::whatsapp::transport::WhatsAppTransport;

    const SECRET: &str = "twilio-test-secret";

    fn make_transport() -> WhatsAppTransport {
        WhatsAppTransport::new(SECRET.to_owned())
    }

    fn signed_headers(body: &[u8]) -> HeaderMap {
        let sig = hmac_sha256_hex(SECRET, body);
        let mut headers = HeaderMap::new();
        headers.insert("x-twilio-signature", sig.parse().unwrap());
        headers
    }

    #[test]
    fn test_valid_signature_passes() {
        let transport = make_transport();
        let body = b"test body";
        let headers = signed_headers(body);
        assert!(transport.verify_signature(&headers, body).is_ok());
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let transport = make_transport();
        let body = b"test body";
        let mut headers = HeaderMap::new();
        headers.insert("x-twilio-signature", "badhex000".parse().unwrap());
        let err = transport.verify_signature(&headers, body).unwrap_err();
        assert!(
            err.to_string().contains("signature mismatch"),
            "Expected signature mismatch, got: {err}"
        );
    }

    #[test]
    fn test_missing_header_rejected() {
        let transport = make_transport();
        let headers = HeaderMap::new();
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(
            err.to_string().contains("missing x-twilio-signature"),
            "Expected missing header, got: {err}"
        );
    }

    #[test]
    fn test_wrong_body_rejected() {
        let transport = make_transport();
        let body = b"original body";
        let headers = signed_headers(body);
        // Verify against different body
        let err = transport
            .verify_signature(&headers, b"tampered body")
            .unwrap_err();
        assert!(err.to_string().contains("signature mismatch"));
    }

    #[tokio::test]
    async fn test_parse_text_message() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "From": "whatsapp:+1234567890",
            "Body": "Hello Pierre",
            "MessageSid": "SM12345"
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let headers = HeaderMap::new();

        let messages = transport.parse_inbound(&headers, &body).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].channel_type, ChannelType::WhatsApp);
        assert_eq!(messages[0].sender_id, "whatsapp:+1234567890");
        assert_eq!(messages[0].channel_message_id, "SM12345");
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "Hello Pierre"),
            other => panic!("Expected Text, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_media_message() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "From": "whatsapp:+1234567890",
            "Body": "Check this out",
            "MessageSid": "SM12345",
            "MediaUrl0": "https://example.com/photo.jpg",
            "MediaContentType0": "image/jpeg"
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let headers = HeaderMap::new();

        let messages = transport.parse_inbound(&headers, &body).await.unwrap();
        assert_eq!(messages.len(), 1);
        match &messages[0].content {
            MessageContent::Media {
                url,
                mime_type,
                caption,
            } => {
                assert_eq!(url, "https://example.com/photo.jpg");
                assert_eq!(mime_type, "image/jpeg");
                assert_eq!(caption.as_deref(), Some("Check this out"));
            }
            other => panic!("Expected Media, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_invalid_json() {
        let transport = make_transport();
        let result = transport
            .parse_inbound(&HeaderMap::new(), b"not json")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid JSON"));
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Messenger Transport Tests (HMAC-SHA256 with sha256= prefix)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod messenger {
    use super::*;
    use pierre_messaging::channels::messenger::transport::MessengerTransport;

    const APP_SECRET: &str = "fb-app-secret";

    fn make_transport() -> MessengerTransport {
        MessengerTransport::new(APP_SECRET.to_owned())
    }

    fn signed_headers(body: &[u8]) -> HeaderMap {
        let sig = format!("sha256={}", hmac_sha256_hex(APP_SECRET, body));
        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", sig.parse().unwrap());
        headers
    }

    #[test]
    fn test_valid_signature_passes() {
        let transport = make_transport();
        let body = b"test body";
        let headers = signed_headers(body);
        assert!(transport.verify_signature(&headers, body).is_ok());
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let transport = make_transport();
        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", "sha256=badvalue".parse().unwrap());
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err.to_string().contains("signature mismatch"));
    }

    #[test]
    fn test_missing_sha256_prefix_rejected() {
        let transport = make_transport();
        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", "noprefix".parse().unwrap());
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err.to_string().contains("sha256="));
    }

    #[test]
    fn test_missing_header_rejected() {
        let transport = make_transport();
        let err = transport
            .verify_signature(&HeaderMap::new(), b"body")
            .unwrap_err();
        assert!(err.to_string().contains("missing x-hub-signature-256"));
    }

    #[tokio::test]
    async fn test_parse_single_text_message() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "entry": [{
                "messaging": [{
                    "sender": { "id": "USER123" },
                    "message": {
                        "mid": "mid.123",
                        "text": "Hello from Messenger"
                    }
                }]
            }]
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].channel_type, ChannelType::Messenger);
        assert_eq!(messages[0].sender_id, "USER123");
        assert_eq!(messages[0].channel_message_id, "mid.123");
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "Hello from Messenger"),
            other => panic!("Expected Text, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_batch_messages() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "entry": [{
                "messaging": [
                    {
                        "sender": { "id": "USER_A" },
                        "message": { "mid": "mid.a", "text": "First" }
                    },
                    {
                        "sender": { "id": "USER_B" },
                        "message": { "mid": "mid.b", "text": "Second" }
                    }
                ]
            }]
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn test_parse_location_attachment() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "entry": [{
                "messaging": [{
                    "sender": { "id": "USER123" },
                    "message": {
                        "mid": "mid.loc",
                        "attachments": [{
                            "type": "location",
                            "payload": {
                                "coordinates": {
                                    "lat": 48.8566,
                                    "long": 2.3522
                                }
                            }
                        }]
                    }
                }]
            }]
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        match &messages[0].content {
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                assert!((latitude - 48.8566).abs() < 0.001);
                assert!((longitude - 2.3522).abs() < 0.001);
            }
            other => panic!("Expected Location, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_empty_entry() {
        let transport = make_transport();
        let payload = serde_json::json!({ "entry": [] });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn test_parse_no_entry_field() {
        let transport = make_transport();
        let payload = serde_json::json!({ "other": "data" });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();
        assert!(messages.is_empty());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Discord Transport Tests (Ed25519)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod discord {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use pierre_messaging::channels::discord::transport::DiscordTransport;

    fn make_keypair() -> (String, SigningKey) {
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.as_bytes());
        (public_key_hex, signing_key)
    }

    fn make_transport_and_key() -> (DiscordTransport, SigningKey) {
        let (pk_hex, sk) = make_keypair();
        (DiscordTransport::new(pk_hex, "test-app-id".to_owned()), sk)
    }

    fn sign_discord(sk: &SigningKey, timestamp: &str, body: &[u8]) -> String {
        let mut message = Vec::with_capacity(timestamp.len() + body.len());
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);
        let sig = sk.sign(&message);
        hex::encode(sig.to_bytes())
    }

    #[test]
    fn test_valid_ed25519_signature_passes() {
        let (transport, sk) = make_transport_and_key();
        let timestamp = "1234567890";
        let body = b"discord body";
        let sig_hex = sign_discord(&sk, timestamp, body);

        let mut headers = HeaderMap::new();
        headers.insert("x-signature-ed25519", sig_hex.parse().unwrap());
        headers.insert("x-signature-timestamp", timestamp.parse().unwrap());

        assert!(transport.verify_signature(&headers, body).is_ok());
    }

    #[test]
    fn test_invalid_ed25519_signature_rejected() {
        let (transport, _sk) = make_transport_and_key();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-signature-ed25519",
            hex::encode([0u8; 64]).parse().unwrap(),
        );
        headers.insert("x-signature-timestamp", "12345".parse().unwrap());
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err
            .to_string()
            .contains("Ed25519 signature verification failed"));
    }

    #[test]
    fn test_missing_signature_header_rejected() {
        let (transport, _sk) = make_transport_and_key();
        let mut headers = HeaderMap::new();
        headers.insert("x-signature-timestamp", "12345".parse().unwrap());
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err.to_string().contains("missing x-signature-ed25519"));
    }

    #[test]
    fn test_missing_timestamp_header_rejected() {
        let (transport, _sk) = make_transport_and_key();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-signature-ed25519",
            hex::encode([0u8; 64]).parse().unwrap(),
        );
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err.to_string().contains("missing x-signature-timestamp"));
    }

    #[test]
    fn test_tampered_body_rejected() {
        let (transport, sk) = make_transport_and_key();
        let timestamp = "1234567890";
        let body = b"original body";
        let sig_hex = sign_discord(&sk, timestamp, body);

        let mut headers = HeaderMap::new();
        headers.insert("x-signature-ed25519", sig_hex.parse().unwrap());
        headers.insert("x-signature-timestamp", timestamp.parse().unwrap());

        let err = transport
            .verify_signature(&headers, b"tampered body")
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("Ed25519 signature verification failed"));
    }

    #[tokio::test]
    async fn test_parse_ping_interaction() {
        let (transport, _sk) = make_transport_and_key();
        let payload = serde_json::json!({ "type": 1 });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();
        assert!(messages.is_empty(), "PING should return no messages");
    }

    #[tokio::test]
    async fn test_parse_application_command() {
        let (transport, _sk) = make_transport_and_key();
        let payload = serde_json::json!({
            "type": 2,
            "id": "interaction-123",
            "channel_id": "channel-456",
            "member": {
                "user": {
                    "id": "user-789",
                    "username": "testuser"
                }
            },
            "data": {
                "options": [{
                    "value": "ask Pierre about running"
                }]
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].channel_type, ChannelType::Discord);
        assert_eq!(messages[0].sender_id, "user-789");
        assert_eq!(messages[0].sender_name.as_deref(), Some("testuser"));
        assert_eq!(messages[0].conversation_id.as_deref(), Some("channel-456"));
        assert_eq!(messages[0].channel_message_id, "interaction-123");
        match &messages[0].content {
            MessageContent::Text { body } => {
                assert_eq!(body, "ask Pierre about running");
            }
            other => panic!("Expected Text, got: {other:?}"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Slack Transport Tests (HMAC-SHA256 v0 with replay protection)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod slack {
    use super::*;
    use pierre_messaging::channels::slack::transport::SlackTransport;

    const SIGNING_SECRET: &str = "slack-signing-secret";

    fn make_transport() -> SlackTransport {
        SlackTransport::new(SIGNING_SECRET.to_owned())
    }

    fn current_timestamp() -> String {
        chrono::Utc::now().timestamp().to_string()
    }

    fn compute_slack_signature(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let hash = hmac_sha256_hex(secret, basestring.as_bytes());
        format!("v0={hash}")
    }

    fn signed_headers(timestamp: &str, body: &str) -> HeaderMap {
        let sig = compute_slack_signature(SIGNING_SECRET, timestamp, body);
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
        headers.insert("x-slack-signature", sig.parse().unwrap());
        headers
    }

    #[test]
    fn test_valid_signature_passes() {
        let transport = make_transport();
        let ts = current_timestamp();
        let body = "test body";
        let headers = signed_headers(&ts, body);
        assert!(transport
            .verify_signature(&headers, body.as_bytes())
            .is_ok());
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let transport = make_transport();
        let ts = current_timestamp();
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", ts.parse().unwrap());
        headers.insert("x-slack-signature", "v0=bad".parse().unwrap());
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err.to_string().contains("signature mismatch"));
    }

    #[test]
    fn test_replay_protection_rejects_old_timestamp() {
        let transport = make_transport();
        // Use a timestamp from 10 minutes ago (> 5 min max)
        let old_ts = (chrono::Utc::now().timestamp() - 600).to_string();
        let body = "test body";
        let headers = signed_headers(&old_ts, body);
        let err = transport
            .verify_signature(&headers, body.as_bytes())
            .unwrap_err();
        assert!(
            err.to_string().contains("old") || err.to_string().contains("Replay"),
            "Expected replay detection, got: {err}"
        );
    }

    #[test]
    fn test_missing_timestamp_header_rejected() {
        let transport = make_transport();
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-signature", "v0=something".parse().unwrap());
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err
            .to_string()
            .contains("missing x-slack-request-timestamp"));
    }

    #[test]
    fn test_missing_signature_header_rejected() {
        let transport = make_transport();
        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", "123".parse().unwrap());
        let err = transport.verify_signature(&headers, b"body").unwrap_err();
        assert!(err.to_string().contains("missing x-slack-signature"));
    }

    #[tokio::test]
    async fn test_parse_message_event() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "event": {
                "type": "message",
                "user": "U1234",
                "text": "Hello from Slack",
                "channel": "C5678",
                "ts": "1609459200.000100"
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].channel_type, ChannelType::Slack);
        assert_eq!(messages[0].sender_id, "U1234");
        assert_eq!(messages[0].conversation_id.as_deref(), Some("C5678"));
        assert_eq!(messages[0].channel_message_id, "1609459200.000100");
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "Hello from Slack"),
            other => panic!("Expected Text, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_url_verification_returns_empty() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "type": "url_verification",
            "challenge": "challenge_token"
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn test_parse_bot_message_filtered() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "event": {
                "type": "message",
                "bot_id": "B1234",
                "text": "I am a bot",
                "channel": "C5678",
                "ts": "1609459200.000200"
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();
        assert!(messages.is_empty(), "Bot messages should be filtered");
    }

    #[tokio::test]
    async fn test_parse_non_message_event_filtered() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "event": {
                "type": "channel_join",
                "user": "U1234"
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();
        assert!(messages.is_empty());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Telegram Transport Tests (Secret Token Header)
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod telegram {
    use super::*;
    use pierre_messaging::channels::telegram::transport::TelegramTransport;

    const WEBHOOK_SECRET: &str = "telegram-webhook-secret";

    fn make_transport() -> TelegramTransport {
        TelegramTransport::new(WEBHOOK_SECRET.to_owned())
    }

    fn secret_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-telegram-bot-api-secret-token",
            WEBHOOK_SECRET.parse().unwrap(),
        );
        headers
    }

    #[test]
    fn test_valid_secret_token_passes() {
        let transport = make_transport();
        let headers = secret_headers();
        assert!(transport.verify_signature(&headers, b"ignored").is_ok());
    }

    #[test]
    fn test_invalid_secret_token_rejected() {
        let transport = make_transport();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-telegram-bot-api-secret-token",
            "wrong-secret".parse().unwrap(),
        );
        let err = transport
            .verify_signature(&headers, b"ignored")
            .unwrap_err();
        assert!(err.to_string().contains("token mismatch"));
    }

    #[test]
    fn test_missing_header_rejected() {
        let transport = make_transport();
        let err = transport
            .verify_signature(&HeaderMap::new(), b"body")
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("missing X-Telegram-Bot-Api-Secret-Token"));
    }

    #[tokio::test]
    async fn test_parse_text_message() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "update_id": 12345,
            "message": {
                "message_id": 42,
                "from": {
                    "id": 100,
                    "first_name": "Alice"
                },
                "chat": {
                    "id": 200
                },
                "text": "Hello Telegram"
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].channel_type, ChannelType::Telegram);
        assert_eq!(messages[0].sender_id, "100");
        assert_eq!(messages[0].sender_name.as_deref(), Some("Alice"));
        assert_eq!(messages[0].conversation_id.as_deref(), Some("200"));
        assert_eq!(messages[0].channel_message_id, "42");
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "Hello Telegram"),
            other => panic!("Expected Text, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_location_message() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "update_id": 12345,
            "message": {
                "message_id": 43,
                "from": { "id": 100 },
                "chat": { "id": 200 },
                "location": {
                    "latitude": 45.5017,
                    "longitude": -73.5673
                }
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        match &messages[0].content {
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                assert!((latitude - 45.5017).abs() < 0.001);
                assert!((longitude + 73.5673).abs() < 0.001);
            }
            other => panic!("Expected Location, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_photo_message() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "update_id": 12345,
            "message": {
                "message_id": 44,
                "from": { "id": 100 },
                "chat": { "id": 200 },
                "photo": [
                    { "file_id": "small_id", "width": 90 },
                    { "file_id": "large_id", "width": 800 }
                ],
                "caption": "My photo"
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body)
            .await
            .unwrap();

        assert_eq!(messages.len(), 1);
        match &messages[0].content {
            MessageContent::Media {
                url,
                mime_type,
                caption,
            } => {
                assert_eq!(url, "large_id");
                assert_eq!(mime_type, "image/jpeg");
                assert_eq!(caption.as_deref(), Some("My photo"));
            }
            other => panic!("Expected Media, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_missing_message_field() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "update_id": 12345,
            "edited_message": { "text": "edited" }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let result = transport.parse_inbound(&HeaderMap::new(), &body).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("message"));
    }

    #[tokio::test]
    async fn test_parse_missing_chat_id() {
        let transport = make_transport();
        let payload = serde_json::json!({
            "update_id": 12345,
            "message": {
                "message_id": 42,
                "from": { "id": 100 },
                "text": "No chat"
            }
        });
        let body = serde_json::to_vec(&payload).unwrap();
        let result = transport.parse_inbound(&HeaderMap::new(), &body).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("chat.id"));
    }
}
