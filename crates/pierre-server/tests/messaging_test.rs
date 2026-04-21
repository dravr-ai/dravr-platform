// ABOUTME: Integration tests for messaging channel adapters (signature verification, parsing, rendering)
// ABOUTME: Covers all 5 channels: Slack, Telegram, Discord, WhatsApp, Messenger plus registry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(
    missing_docs,
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

#[cfg(feature = "client-messaging")]
mod messaging_tests {
    use hmac::{Hmac, Mac};
    use http::HeaderMap;
    use pierre_core::models::messaging::{
        CardAction, ChannelType, MessageContent, OutgoingMessage,
    };
    use pierre_messaging::channel::MessagingChannel;
    use pierre_messaging::channels::discord::{
        renderer::DiscordRenderer, transport::DiscordTransport,
    };
    use pierre_messaging::channels::messenger::{
        renderer::MessengerRenderer, transport::MessengerTransport,
    };
    use pierre_messaging::channels::slack::{renderer::SlackRenderer, transport::SlackTransport};
    use pierre_messaging::channels::telegram::{
        renderer::TelegramRenderer, transport::TelegramTransport,
    };
    use pierre_messaging::channels::whatsapp::{
        renderer::WhatsAppRenderer, transport::WhatsAppTransport,
    };
    use pierre_messaging::registry::ChannelRegistry;
    use pierre_messaging::renderer::ResponseRenderer;
    use pierre_messaging::retry::{compute_retry_update, RetryDecision};
    use pierre_messaging::transport::TransportAdapter;
    use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
    use sha2::Sha256;
    use std::sync::Arc;

    // ════════════════════════════════════════════════════════════════
    // Slack signature verification tests
    // ════════════════════════════════════════════════════════════════

    fn compute_slack_signature(secret: &str, timestamp: &str, body: &str) -> String {
        let basestring = format!("v0:{timestamp}:{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(basestring.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn fresh_timestamp() -> String {
        chrono::Utc::now().timestamp().to_string()
    }

    #[test]
    fn test_slack_verify_signature_valid() {
        let secret = "test_slack_secret_abc123";
        let transport = SlackTransport::new(secret.to_owned());
        let body = r#"{"event":{"type":"message","text":"hello"}}"#;
        let timestamp = fresh_timestamp();
        let signature = compute_slack_signature(secret, &timestamp, body);

        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
        headers.insert("x-slack-signature", signature.parse().unwrap());

        let result = transport.verify_signature(&headers, body.as_bytes());
        assert!(
            result.is_ok(),
            "Valid Slack signature should pass: {result:?}"
        );
    }

    #[test]
    fn test_slack_verify_signature_invalid() {
        let transport = SlackTransport::new("real_secret".to_owned());
        let body = r#"{"event":{"type":"message"}}"#;
        let timestamp = fresh_timestamp();
        let signature = compute_slack_signature("wrong_secret", &timestamp, body);

        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
        headers.insert("x-slack-signature", signature.parse().unwrap());

        let result = transport.verify_signature(&headers, body.as_bytes());
        assert!(result.is_err(), "Invalid Slack signature should fail");
    }

    #[test]
    fn test_slack_verify_signature_missing_header() {
        let transport = SlackTransport::new("secret".to_owned());
        let headers = HeaderMap::new();

        let result = transport.verify_signature(&headers, b"body");
        assert!(result.is_err(), "Missing headers should fail");
    }

    #[test]
    fn test_slack_verify_signature_replay_detection() {
        let secret = "slack_replay_test";
        let transport = SlackTransport::new(secret.to_owned());
        let body = "test body";
        // 10 minutes ago, beyond the 5 min window
        let old_timestamp = (chrono::Utc::now().timestamp() - 600).to_string();
        let signature = compute_slack_signature(secret, &old_timestamp, body);

        let mut headers = HeaderMap::new();
        headers.insert("x-slack-request-timestamp", old_timestamp.parse().unwrap());
        headers.insert("x-slack-signature", signature.parse().unwrap());

        let result = transport.verify_signature(&headers, body.as_bytes());
        assert!(
            result.is_err(),
            "Old timestamp should be rejected (replay protection)"
        );
    }

    #[tokio::test]
    async fn test_slack_parse_inbound_message() {
        let transport = SlackTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "event": {
                "type": "message",
                "user": "U12345",
                "text": "Hello Pierre!",
                "channel": "C67890",
                "ts": "1234567890.123456"
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let headers = HeaderMap::new();

        let messages = transport
            .parse_inbound(&headers, &body_bytes)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_id, "U12345");
        assert_eq!(messages[0].channel_message_id, "1234567890.123456");
        assert_eq!(messages[0].conversation_id.as_deref(), Some("C67890"));
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "Hello Pierre!"),
            _ => panic!("Expected text content"),
        }
    }

    #[tokio::test]
    async fn test_slack_parse_inbound_skips_bot_messages() {
        let transport = SlackTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "event": {
                "type": "message",
                "bot_id": "B12345",
                "text": "Bot reply",
                "channel": "C67890",
                "ts": "1234567890.999"
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert!(messages.is_empty(), "Bot messages should be skipped");
    }

    #[tokio::test]
    async fn test_slack_parse_inbound_url_verification() {
        let transport = SlackTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "type": "url_verification",
            "challenge": "3eZbrw1aBm2rZgRNFdxV2595E9CY3gmdALWMmHkvFXO7tYXAYM8P"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert!(
            messages.is_empty(),
            "URL verification should return no messages"
        );
    }

    // ════════════════════════════════════════════════════════════════
    // Telegram signature verification tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn test_telegram_verify_signature_valid() {
        let secret = "my_telegram_secret_token";
        let transport = TelegramTransport::new(secret.to_owned());

        let mut headers = HeaderMap::new();
        headers.insert("x-telegram-bot-api-secret-token", secret.parse().unwrap());

        let result = transport.verify_signature(&headers, b"irrelevant body");
        assert!(result.is_ok(), "Valid Telegram secret should pass");
    }

    #[test]
    fn test_telegram_verify_signature_invalid() {
        let transport = TelegramTransport::new("correct_secret".to_owned());

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-telegram-bot-api-secret-token",
            "wrong_secret".parse().unwrap(),
        );

        let result = transport.verify_signature(&headers, b"body");
        assert!(result.is_err(), "Wrong Telegram secret should fail");
    }

    #[test]
    fn test_telegram_verify_signature_missing() {
        let transport = TelegramTransport::new("secret".to_owned());
        let result = transport.verify_signature(&HeaderMap::new(), b"body");
        assert!(result.is_err(), "Missing Telegram header should fail");
    }

    #[tokio::test]
    async fn test_telegram_parse_inbound_text() {
        let transport = TelegramTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "update_id": 123_456,
            "message": {
                "message_id": 42,
                "from": {
                    "id": 98765,
                    "first_name": "Alice"
                },
                "chat": { "id": 98765 },
                "text": "Show my training stats"
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_id, "98765");
        assert_eq!(messages[0].sender_name.as_deref(), Some("Alice"));
        assert_eq!(messages[0].channel_message_id, "42");
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "Show my training stats"),
            _ => panic!("Expected text content"),
        }
    }

    #[tokio::test]
    async fn test_telegram_parse_inbound_location() {
        let transport = TelegramTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "update_id": 123_457,
            "message": {
                "message_id": 43,
                "from": { "id": 98765, "first_name": "Bob" },
                "chat": { "id": 98765 },
                "location": { "latitude": 48.8566, "longitude": 2.3522 }
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        match &messages[0].content {
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                assert!((latitude - 48.8566).abs() < 0.0001);
                assert!((longitude - 2.3522).abs() < 0.0001);
            }
            _ => panic!("Expected location content"),
        }
    }

    #[tokio::test]
    async fn test_telegram_parse_inbound_photo() {
        let transport = TelegramTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "update_id": 123_458,
            "message": {
                "message_id": 44,
                "from": { "id": 98765, "first_name": "Carol" },
                "chat": { "id": 98765 },
                "photo": [
                    { "file_id": "small_id", "width": 90, "height": 90 },
                    { "file_id": "large_photo_id", "width": 1280, "height": 960 }
                ],
                "caption": "My run route"
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        match &messages[0].content {
            MessageContent::Media {
                url,
                mime_type,
                caption,
            } => {
                assert_eq!(url, "large_photo_id", "Should pick the largest photo");
                assert_eq!(mime_type, "image/jpeg");
                assert_eq!(caption.as_deref(), Some("My run route"));
            }
            _ => panic!("Expected media content"),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // WhatsApp (Meta Cloud API) signature verification tests
    // ════════════════════════════════════════════════════════════════

    fn compute_whatsapp_signature(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn test_whatsapp_verify_signature_valid() {
        let secret = "meta_test_app_secret";
        let transport = WhatsAppTransport::new(secret.to_owned());
        let body = b"{\"entry\":[]}";
        let signature = compute_whatsapp_signature(secret, body);

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", signature.parse().unwrap());

        let result = transport.verify_signature(&headers, body);
        assert!(result.is_ok(), "Valid WhatsApp signature should pass");
    }

    #[test]
    fn test_whatsapp_verify_signature_invalid() {
        let transport = WhatsAppTransport::new("correct_secret".to_owned());
        let body = b"{\"entry\":[]}";
        let signature = compute_whatsapp_signature("wrong_secret", body);

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", signature.parse().unwrap());

        let result = transport.verify_signature(&headers, body);
        assert!(result.is_err(), "Wrong WhatsApp secret should fail");
    }

    #[test]
    fn test_whatsapp_verify_signature_missing() {
        let transport = WhatsAppTransport::new("secret".to_owned());
        let result = transport.verify_signature(&HeaderMap::new(), b"body");
        assert!(result.is_err(), "Missing x-hub-signature-256 should fail");
    }

    #[tokio::test]
    async fn test_whatsapp_parse_inbound_text() {
        let transport = WhatsAppTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "14155551234",
                            "id": "wamid.abc123",
                            "type": "text",
                            "text": { "body": "What's my heart rate zone?" }
                        }]
                    }
                }]
            }]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_id, "14155551234");
        assert_eq!(messages[0].channel_message_id, "wamid.abc123");
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "What's my heart rate zone?"),
            _ => panic!("Expected text content"),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Messenger signature verification tests
    // ════════════════════════════════════════════════════════════════

    fn compute_messenger_signature(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn test_messenger_verify_signature_valid() {
        let secret = "meta_app_secret_test";
        let transport = MessengerTransport::new(secret.to_owned());
        let body = b"test webhook payload";
        let signature = compute_messenger_signature(secret, body);

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", signature.parse().unwrap());

        let result = transport.verify_signature(&headers, body);
        assert!(result.is_ok(), "Valid Messenger signature should pass");
    }

    #[test]
    fn test_messenger_verify_signature_invalid() {
        let transport = MessengerTransport::new("correct_secret".to_owned());
        let body = b"payload";
        let signature = compute_messenger_signature("wrong_secret", body);

        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", signature.parse().unwrap());

        let result = transport.verify_signature(&headers, body);
        assert!(result.is_err(), "Wrong Messenger secret should fail");
    }

    #[test]
    fn test_messenger_verify_signature_missing_prefix() {
        let transport = MessengerTransport::new("secret".to_owned());
        let mut headers = HeaderMap::new();
        // Missing "sha256=" prefix
        headers.insert("x-hub-signature-256", "just_hex_no_prefix".parse().unwrap());

        let result = transport.verify_signature(&headers, b"body");
        assert!(result.is_err(), "Missing sha256= prefix should fail");
    }

    #[tokio::test]
    async fn test_messenger_parse_inbound_text() {
        let transport = MessengerTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "object": "page",
            "entry": [{
                "messaging": [{
                    "sender": { "id": "SENDER_123" },
                    "message": {
                        "mid": "mid.123456",
                        "text": "How far did I run this week?"
                    }
                }]
            }]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_id, "SENDER_123");
        assert_eq!(messages[0].channel_message_id, "mid.123456");
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "How far did I run this week?"),
            _ => panic!("Expected text content"),
        }
    }

    #[tokio::test]
    async fn test_messenger_parse_inbound_batch() {
        let transport = MessengerTransport::new("secret".to_owned());
        let body = serde_json::json!({
            "object": "page",
            "entry": [
                {
                    "messaging": [
                        {
                            "sender": { "id": "USER_A" },
                            "message": { "mid": "mid.001", "text": "First message" }
                        },
                        {
                            "sender": { "id": "USER_B" },
                            "message": { "mid": "mid.002", "text": "Second message" }
                        }
                    ]
                }
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert_eq!(
            messages.len(),
            2,
            "Messenger batches should produce multiple messages"
        );
        assert_eq!(messages[0].sender_id, "USER_A");
        assert_eq!(messages[1].sender_id, "USER_B");
    }

    // ════════════════════════════════════════════════════════════════
    // Discord Ed25519 signature verification tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn test_discord_verify_signature_valid() {
        use ed25519_dalek::{Signer, SigningKey};

        // Deterministic test keypair (32-byte seed)
        let seed: [u8; 32] = [1u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.as_bytes());

        let transport = DiscordTransport::new(public_key_hex, "app_id".to_owned());

        let timestamp = "1234567890";
        let body = b"test discord body";

        // Sign: timestamp + body
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);
        let signature = signing_key.sign(&message);
        let signature_hex = hex::encode(signature.to_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("x-signature-ed25519", signature_hex.parse().unwrap());
        headers.insert("x-signature-timestamp", timestamp.parse().unwrap());

        let result = transport.verify_signature(&headers, body);
        assert!(
            result.is_ok(),
            "Valid Discord Ed25519 signature should pass: {result:?}"
        );
    }

    #[test]
    fn test_discord_verify_signature_invalid() {
        use ed25519_dalek::SigningKey;

        // Two different deterministic keypairs
        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let wrong_key = SigningKey::from_bytes(&[2u8; 32]);
        let public_key_hex = hex::encode(signing_key.verifying_key().as_bytes());

        let transport = DiscordTransport::new(public_key_hex, "app_id".to_owned());

        let timestamp = "1234567890";
        let body = b"test body";

        // Sign with the WRONG key
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);
        let bad_signature = ed25519_dalek::Signer::sign(&wrong_key, &message);
        let signature_hex = hex::encode(bad_signature.to_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("x-signature-ed25519", signature_hex.parse().unwrap());
        headers.insert("x-signature-timestamp", timestamp.parse().unwrap());

        let result = transport.verify_signature(&headers, body);
        assert!(result.is_err(), "Signature from wrong key should fail");
    }

    #[test]
    fn test_discord_verify_signature_missing_headers() {
        let transport = DiscordTransport::new(
            "a".repeat(64), // 32 bytes hex-encoded
            "app_id".to_owned(),
        );

        let result = transport.verify_signature(&HeaderMap::new(), b"body");
        assert!(result.is_err(), "Missing Discord headers should fail");
    }

    #[tokio::test]
    async fn test_discord_parse_inbound_ping() {
        let transport = DiscordTransport::new("a".repeat(64), "app_id".to_owned());
        let body = serde_json::json!({ "type": 1 });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert!(
            messages.is_empty(),
            "Discord PING should return no messages"
        );
    }

    #[tokio::test]
    async fn test_discord_parse_inbound_slash_command() {
        let transport = DiscordTransport::new("a".repeat(64), "app_id".to_owned());
        let body = serde_json::json!({
            "type": 2,
            "id": "interaction_123",
            "channel_id": "C456",
            "token": "interaction_token_xyz",
            "member": {
                "user": {
                    "id": "USER_789",
                    "username": "alice"
                }
            },
            "data": {
                "name": "pierre",
                "options": [{ "value": "show my stats" }]
            }
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();

        let messages = transport
            .parse_inbound(&HeaderMap::new(), &body_bytes)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_id, "USER_789");
        assert_eq!(messages[0].sender_name.as_deref(), Some("alice"));
        assert_eq!(messages[0].conversation_id.as_deref(), Some("C456"));
        match &messages[0].content {
            MessageContent::Text { body } => assert_eq!(body, "show my stats"),
            _ => panic!("Expected text content"),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Renderer tests
    // ════════════════════════════════════════════════════════════════

    fn make_text_message(channel: ChannelType, recipient: &str, text: &str) -> OutgoingMessage {
        OutgoingMessage {
            channel_type: channel,
            recipient_id: recipient.to_owned(),
            content: MessageContent::Text {
                body: text.to_owned(),
            },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        }
    }

    fn make_card_message(channel: ChannelType, recipient: &str) -> OutgoingMessage {
        OutgoingMessage {
            channel_type: channel,
            recipient_id: recipient.to_owned(),
            content: MessageContent::Card {
                title: "Training Summary".to_owned(),
                body: "You ran 42km this week!".to_owned(),
                actions: vec![
                    CardAction {
                        label: "View Details".to_owned(),
                        action_type: "url".to_owned(),
                        value: "https://pierre.app/stats".to_owned(),
                    },
                    CardAction {
                        label: "Share".to_owned(),
                        action_type: "postback".to_owned(),
                        value: "share_weekly_stats".to_owned(),
                    },
                ],
            },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        }
    }

    fn make_media_message(channel: ChannelType, recipient: &str) -> OutgoingMessage {
        OutgoingMessage {
            channel_type: channel,
            recipient_id: recipient.to_owned(),
            content: MessageContent::Media {
                url: "https://cdn.pierre.app/chart.png".to_owned(),
                mime_type: "image/png".to_owned(),
                caption: Some("Your weekly progress".to_owned()),
            },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        }
    }

    // ── Slack Renderer ──

    #[test]
    fn test_slack_render_text() {
        let renderer = SlackRenderer;
        let msg = make_text_message(ChannelType::Slack, "C12345", "Hello from Pierre!");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["channel"], "C12345");
        assert_eq!(payload["blocks"][0]["type"], "section");
        assert_eq!(payload["blocks"][0]["text"]["type"], "mrkdwn");
        assert_eq!(payload["blocks"][0]["text"]["text"], "Hello from Pierre!");
    }

    #[test]
    fn test_slack_render_card() {
        let renderer = SlackRenderer;
        let msg = make_card_message(ChannelType::Slack, "C12345");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["channel"], "C12345");
        // Header block
        assert_eq!(payload["blocks"][0]["type"], "header");
        assert_eq!(payload["blocks"][0]["text"]["text"], "Training Summary");
        // Body section
        assert_eq!(payload["blocks"][1]["type"], "section");
        // Actions block
        assert_eq!(payload["blocks"][2]["type"], "actions");
        let elements = payload["blocks"][2]["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[0]["text"]["text"], "View Details");
        assert!(
            elements[0]["url"].is_string(),
            "URL action should have url field"
        );
        assert_eq!(elements[1]["text"]["text"], "Share");
        assert!(
            elements[1]["action_id"].is_string(),
            "Postback action should have action_id"
        );
    }

    #[test]
    fn test_slack_render_media() {
        let renderer = SlackRenderer;
        let msg = make_media_message(ChannelType::Slack, "C12345");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["blocks"][0]["type"], "image");
        assert_eq!(
            payload["blocks"][0]["image_url"],
            "https://cdn.pierre.app/chart.png"
        );
        assert_eq!(payload["blocks"][0]["alt_text"], "Your weekly progress");
    }

    #[test]
    fn test_slack_renderer_capabilities() {
        let renderer = SlackRenderer;
        assert_eq!(renderer.max_message_length(), 40000);
        assert!(renderer.supports_media());
        assert!(renderer.supports_cards());
    }

    // ── Telegram Renderer ──

    #[test]
    fn test_telegram_render_text() {
        let renderer = TelegramRenderer;
        let msg = make_text_message(ChannelType::Telegram, "98765", "Hello from Pierre!");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["chat_id"], "98765");
        assert_eq!(payload["text"], "Hello from Pierre!");
        assert_eq!(payload["parse_mode"], "HTML");
    }

    #[test]
    fn test_telegram_render_card_with_buttons() {
        let renderer = TelegramRenderer;
        let msg = make_card_message(ChannelType::Telegram, "98765");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["chat_id"], "98765");
        assert!(payload["text"]
            .as_str()
            .unwrap()
            .contains("<b>Training Summary</b>"));
        assert!(payload["text"].as_str().unwrap().contains("42km this week"));

        let keyboard = &payload["reply_markup"]["inline_keyboard"];
        let rows = keyboard.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        // URL button
        assert_eq!(rows[0][0]["text"], "View Details");
        assert!(rows[0][0]["url"].is_string());
        // Callback button
        assert_eq!(rows[1][0]["text"], "Share");
        assert!(rows[1][0]["callback_data"].is_string());
    }

    #[test]
    fn test_telegram_render_location() {
        let renderer = TelegramRenderer;
        let msg = OutgoingMessage {
            channel_type: ChannelType::Telegram,
            recipient_id: "98765".to_owned(),
            content: MessageContent::Location {
                latitude: 48.8566,
                longitude: 2.3522,
            },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        };
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["chat_id"], "98765");
        assert_eq!(payload["latitude"], 48.8566);
        assert_eq!(payload["longitude"], 2.3522);
    }

    #[test]
    fn test_telegram_renderer_capabilities() {
        let renderer = TelegramRenderer;
        assert_eq!(renderer.max_message_length(), 4096);
        assert!(renderer.supports_media());
        assert!(renderer.supports_cards());
    }

    // ── WhatsApp Renderer (Meta Cloud API) ──

    #[test]
    fn test_whatsapp_render_text() {
        let renderer = WhatsAppRenderer;
        let msg = make_text_message(ChannelType::WhatsApp, "14155551234", "Hello!");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["to"], "14155551234");
        assert_eq!(payload["type"], "text");
        assert_eq!(payload["text"]["body"], "Hello!");
    }

    #[test]
    fn test_whatsapp_render_media() {
        let renderer = WhatsAppRenderer;
        let msg = make_media_message(ChannelType::WhatsApp, "14155551234");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["to"], "14155551234");
        assert_eq!(payload["type"], "image");
        assert_eq!(payload["image"]["link"], "https://cdn.pierre.app/chart.png");
        assert_eq!(payload["image"]["caption"], "Your weekly progress");
    }

    #[test]
    fn test_whatsapp_render_card_as_text_fallback() {
        let renderer = WhatsAppRenderer;
        let msg = make_card_message(ChannelType::WhatsApp, "14155551234");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["type"], "text");
        let body = payload["text"]["body"].as_str().unwrap();
        assert!(
            body.contains("*Training Summary*"),
            "Card title should be bold"
        );
        assert!(
            body.contains("42km this week"),
            "Card body should be included"
        );
        assert!(
            body.contains("View Details"),
            "Action labels should be listed"
        );
    }

    #[test]
    fn test_whatsapp_renderer_capabilities() {
        let renderer = WhatsAppRenderer;
        assert_eq!(renderer.max_message_length(), 4096);
        assert!(renderer.supports_media());
        assert!(
            !renderer.supports_cards(),
            "WhatsApp uses text fallback for cards"
        );
    }

    // ── Messenger Renderer ──

    #[test]
    fn test_messenger_render_text() {
        let renderer = MessengerRenderer;
        let msg = make_text_message(ChannelType::Messenger, "RECIPIENT_123", "Hello!");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["recipient"]["id"], "RECIPIENT_123");
        assert_eq!(payload["message"]["text"], "Hello!");
    }

    #[test]
    fn test_messenger_render_card() {
        let renderer = MessengerRenderer;
        let msg = make_card_message(ChannelType::Messenger, "RECIPIENT_123");
        let payload = renderer.render(&msg).unwrap();

        let template = &payload["message"]["attachment"]["payload"];
        assert_eq!(template["template_type"], "generic");
        let element = &template["elements"][0];
        assert_eq!(element["title"], "Training Summary");
        assert_eq!(element["subtitle"], "You ran 42km this week!");
        let buttons = element["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0]["type"], "web_url");
        assert_eq!(buttons[1]["type"], "postback");
    }

    #[test]
    fn test_messenger_renderer_capabilities() {
        let renderer = MessengerRenderer;
        assert_eq!(renderer.max_message_length(), 2000);
        assert!(renderer.supports_media());
        assert!(renderer.supports_cards());
    }

    // ── Discord Renderer ──

    #[test]
    fn test_discord_render_text() {
        let renderer = DiscordRenderer;
        let msg = make_text_message(ChannelType::Discord, "CHANNEL_456", "Hello!");
        let payload = renderer.render(&msg).unwrap();

        assert_eq!(payload["content"], "Hello!");
        assert_eq!(payload["channel_id"], "CHANNEL_456");
    }

    #[test]
    fn test_discord_render_card_with_embed() {
        let renderer = DiscordRenderer;
        let msg = make_card_message(ChannelType::Discord, "CHANNEL_456");
        let payload = renderer.render(&msg).unwrap();

        let embed = &payload["embeds"][0];
        assert_eq!(embed["title"], "Training Summary");
        assert_eq!(embed["description"], "You ran 42km this week!");

        let components = &payload["components"][0]["components"];
        let buttons = components.as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        // URL button style = 5 (Link)
        assert_eq!(buttons[0]["style"], 5);
        assert_eq!(buttons[0]["label"], "View Details");
        // Non-URL button style = 1 (Primary)
        assert_eq!(buttons[1]["style"], 1);
        assert_eq!(buttons[1]["label"], "Share");
    }

    #[test]
    fn test_discord_renderer_capabilities() {
        let renderer = DiscordRenderer;
        assert_eq!(renderer.max_message_length(), 2000);
        assert!(renderer.supports_media());
        assert!(renderer.supports_cards());
    }

    // ════════════════════════════════════════════════════════════════
    // Registry tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn test_registry_register_and_get() {
        use pierre_messaging::channels::telegram::TelegramChannel;

        let mut registry = ChannelRegistry::new();
        let channel: Arc<dyn MessagingChannel> =
            Arc::new(TelegramChannel::new("secret".to_owned()));

        assert!(registry.is_empty());
        let registered = registry.register(channel);
        assert!(registered, "First registration should succeed");
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());

        let adapter = registry.get(&ChannelType::Telegram);
        assert!(adapter.is_some(), "Should find registered Telegram channel");
    }

    #[test]
    fn test_registry_duplicate_registration() {
        use pierre_messaging::channels::telegram::TelegramChannel;

        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(TelegramChannel::new("s1".to_owned())) as Arc<dyn MessagingChannel>);
        let duplicate = registry
            .register(Arc::new(TelegramChannel::new("s2".to_owned())) as Arc<dyn MessagingChannel>);
        assert!(!duplicate, "Duplicate registration should return false");
        assert_eq!(registry.len(), 1, "Registry should still have 1 entry");
    }

    #[test]
    fn test_registry_registered_channels() {
        use pierre_messaging::channels::slack::SlackChannel;
        use pierre_messaging::channels::telegram::TelegramChannel;

        let mut registry = ChannelRegistry::new();
        registry.register(Arc::new(SlackChannel::new("s".to_owned())) as Arc<dyn MessagingChannel>);
        registry
            .register(Arc::new(TelegramChannel::new("t".to_owned())) as Arc<dyn MessagingChannel>);

        let types = registry.registered_channels();
        assert_eq!(types.len(), 2);
        assert!(types.contains(&ChannelType::Slack));
        assert!(types.contains(&ChannelType::Telegram));
    }

    #[test]
    fn test_registry_get_nonexistent() {
        let registry = ChannelRegistry::new();
        assert!(registry.get(&ChannelType::Discord).is_none());
    }

    // ════════════════════════════════════════════════════════════════
    // Retry logic tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn test_retry_first_attempt() {
        let update = compute_retry_update(0);
        assert_eq!(update.attempt_count, 1);
        match update.decision {
            RetryDecision::Retry { status, .. } => {
                assert_eq!(status, "retrying:1");
            }
            RetryDecision::DeadLetter => {
                panic!("First failure should retry, not dead-letter");
            }
        }
    }

    #[test]
    fn test_retry_second_attempt() {
        let update = compute_retry_update(1);
        assert_eq!(update.attempt_count, 2);
        match update.decision {
            RetryDecision::Retry { status, .. } => {
                assert_eq!(status, "retrying:2");
            }
            RetryDecision::DeadLetter => {
                panic!("Second failure should retry");
            }
        }
    }

    #[test]
    fn test_retry_third_attempt() {
        let update = compute_retry_update(2);
        assert_eq!(update.attempt_count, 3);
        match update.decision {
            RetryDecision::Retry { status, .. } => {
                assert_eq!(status, "retrying:3");
            }
            RetryDecision::DeadLetter => {
                panic!("Third failure should retry");
            }
        }
    }

    #[test]
    fn test_retry_exhausted_becomes_dead_letter() {
        let update = compute_retry_update(3);
        assert_eq!(update.attempt_count, 4);
        match update.decision {
            RetryDecision::DeadLetter => {
                // Expected: after 3 attempts, go to dead-letter
            }
            RetryDecision::Retry { .. } => {
                panic!("After 3 attempts, should dead-letter");
            }
        }
    }

    // ════════════════════════════════════════════════════════════════
    // ChannelType serialization tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn test_channel_type_display() {
        assert_eq!(ChannelType::WhatsApp.to_string(), "whatsapp");
        assert_eq!(ChannelType::Messenger.to_string(), "messenger");
        assert_eq!(ChannelType::Discord.to_string(), "discord");
        assert_eq!(ChannelType::Slack.to_string(), "slack");
        assert_eq!(ChannelType::Telegram.to_string(), "telegram");
    }

    #[test]
    fn test_channel_type_from_str() {
        use std::str::FromStr;
        assert_eq!(
            ChannelType::from_str("whatsapp").unwrap(),
            ChannelType::WhatsApp
        );
        assert_eq!(
            ChannelType::from_str("messenger").unwrap(),
            ChannelType::Messenger
        );
        assert_eq!(
            ChannelType::from_str("discord").unwrap(),
            ChannelType::Discord
        );
        assert_eq!(ChannelType::from_str("slack").unwrap(), ChannelType::Slack);
        assert_eq!(
            ChannelType::from_str("telegram").unwrap(),
            ChannelType::Telegram
        );
        assert!(
            ChannelType::from_str("sms").is_err(),
            "Unknown channel should fail"
        );
    }

    #[test]
    fn test_channel_type_roundtrip_serde() {
        let original = ChannelType::Slack;
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"slack\"");
        let deserialized: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, original);
    }

    // ════════════════════════════════════════════════════════════════
    // MessagingError retryable tests
    // ════════════════════════════════════════════════════════════════

    #[test]
    fn test_messaging_error_retryable() {
        use pierre_core::errors::messaging::MessagingError;

        let delivery = MessagingError::DeliveryFailed {
            channel: "slack".to_owned(),
            reason: "timeout".to_owned(),
            retryable: true,
        };
        assert!(delivery.is_retryable());

        let rate_limit = MessagingError::RateLimitExceeded {
            channel: "slack".to_owned(),
            retry_after_secs: 60,
        };
        assert!(rate_limit.is_retryable());
        assert_eq!(rate_limit.retry_after_secs(), Some(60));

        let sig_fail = MessagingError::SignatureVerificationFailed {
            channel: "slack".to_owned(),
            reason: "mismatch".to_owned(),
        };
        assert!(!sig_fail.is_retryable());
    }
}
