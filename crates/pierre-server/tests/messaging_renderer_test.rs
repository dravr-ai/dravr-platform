// ABOUTME: Unit tests for messaging response renderers (WhatsApp, Messenger, Discord, Slack, Telegram)
// ABOUTME: Validates platform-specific payload formatting for all message content types
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

use pierre_core::models::messaging::{CardAction, ChannelType, MessageContent, OutgoingMessage};
use pierre_messaging::renderer::ResponseRenderer;
use uuid::Uuid;

// ── Helper: create test OutgoingMessage ──

fn text_message(channel: ChannelType, text: &str) -> OutgoingMessage {
    OutgoingMessage {
        channel_type: channel,
        recipient_id: "recipient-123".to_owned(),
        content: MessageContent::Text {
            body: text.to_owned(),
        },
        correlation_id: Uuid::nil(),
        reply_to: None,
        thread_id: None,
    }
}

fn media_message(channel: ChannelType) -> OutgoingMessage {
    OutgoingMessage {
        channel_type: channel,
        recipient_id: "recipient-123".to_owned(),
        content: MessageContent::Media {
            url: "https://example.com/photo.jpg".to_owned(),
            mime_type: "image/jpeg".to_owned(),
            caption: Some("A photo".to_owned()),
        },
        correlation_id: Uuid::nil(),
        reply_to: None,
        thread_id: None,
    }
}

fn location_message(channel: ChannelType) -> OutgoingMessage {
    OutgoingMessage {
        channel_type: channel,
        recipient_id: "recipient-123".to_owned(),
        content: MessageContent::Location {
            latitude: 48.8566,
            longitude: 2.3522,
        },
        correlation_id: Uuid::nil(),
        reply_to: None,
        thread_id: None,
    }
}

fn card_message(channel: ChannelType) -> OutgoingMessage {
    OutgoingMessage {
        channel_type: channel,
        recipient_id: "recipient-123".to_owned(),
        content: MessageContent::Card {
            title: "Training Plan".to_owned(),
            body: "Your 5K plan is ready.".to_owned(),
            actions: vec![
                CardAction {
                    label: "View Plan".to_owned(),
                    action_type: "url".to_owned(),
                    value: "https://example.com/plan".to_owned(),
                },
                CardAction {
                    label: "Start Now".to_owned(),
                    action_type: "postback".to_owned(),
                    value: "start_plan".to_owned(),
                },
            ],
        },
        correlation_id: Uuid::nil(),
        reply_to: None,
        thread_id: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// WhatsApp Renderer Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod whatsapp {
    use super::*;
    use pierre_messaging::channels::whatsapp::renderer::WhatsAppRenderer;

    fn renderer() -> WhatsAppRenderer {
        WhatsAppRenderer
    }

    #[test]
    fn test_render_text_message() {
        let msg = text_message(ChannelType::WhatsApp, "Hello runner");
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["to"], "recipient-123");
        assert_eq!(payload["type"], "text");
        assert_eq!(payload["text"]["body"], "Hello runner");
    }

    #[test]
    fn test_render_media_message() {
        let msg = media_message(ChannelType::WhatsApp);
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["to"], "recipient-123");
        assert_eq!(payload["type"], "image");
        assert_eq!(payload["image"]["link"], "https://example.com/photo.jpg");
        assert_eq!(payload["image"]["caption"], "A photo");
    }

    #[test]
    fn test_render_location_message() {
        let msg = location_message(ChannelType::WhatsApp);
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["messaging_product"], "whatsapp");
        let body = payload["text"]["body"].as_str().unwrap();
        assert!(body.contains("48.8566"));
        assert!(body.contains("2.3522"));
    }

    #[test]
    fn test_render_card_message() {
        let msg = card_message(ChannelType::WhatsApp);
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["messaging_product"], "whatsapp");
        let body = payload["text"]["body"].as_str().unwrap();
        assert!(body.contains("Training Plan"));
        assert!(body.contains("Your 5K plan is ready"));
        assert!(body.contains("View Plan"));
    }

    #[test]
    fn test_capabilities() {
        let r = renderer();
        assert_eq!(r.max_message_length(), 4096);
        assert!(r.supports_media());
        assert!(!r.supports_cards());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Messenger Renderer Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod messenger {
    use super::*;
    use pierre_messaging::channels::messenger::renderer::MessengerRenderer;

    fn renderer() -> MessengerRenderer {
        MessengerRenderer
    }

    #[test]
    fn test_render_text_message() {
        let msg = text_message(ChannelType::Messenger, "Hello from Pierre");
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["recipient"]["id"], "recipient-123");
        assert_eq!(payload["message"]["text"], "Hello from Pierre");
    }

    #[test]
    fn test_render_media_message() {
        let msg = media_message(ChannelType::Messenger);
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["recipient"]["id"], "recipient-123");
        let attachment = &payload["message"]["attachment"];
        assert_eq!(attachment["type"], "image");
        assert_eq!(
            attachment["payload"]["url"],
            "https://example.com/photo.jpg"
        );
    }

    #[test]
    fn test_render_location_message() {
        let msg = location_message(ChannelType::Messenger);
        let payload = renderer().render(&msg).unwrap();
        let text = payload["message"]["text"].as_str().unwrap();
        assert!(text.contains("48.8566"));
    }

    #[test]
    fn test_render_card_message() {
        let msg = card_message(ChannelType::Messenger);
        let payload = renderer().render(&msg).unwrap();
        let template = &payload["message"]["attachment"]["payload"];
        assert_eq!(template["template_type"], "generic");
        let element = &template["elements"][0];
        assert_eq!(element["title"], "Training Plan");
        assert_eq!(element["subtitle"], "Your 5K plan is ready.");
    }

    #[test]
    fn test_capabilities() {
        let r = renderer();
        assert_eq!(r.max_message_length(), 2000);
        assert!(r.supports_media());
        assert!(r.supports_cards());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Discord Renderer Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod discord {
    use super::*;
    use pierre_messaging::channels::discord::renderer::DiscordRenderer;

    fn renderer() -> DiscordRenderer {
        DiscordRenderer
    }

    #[test]
    fn test_render_text_message() {
        let msg = text_message(ChannelType::Discord, "Coach says run");
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["channel_id"], "recipient-123");
        assert_eq!(payload["content"], "Coach says run");
    }

    #[test]
    fn test_render_media_message() {
        let msg = media_message(ChannelType::Discord);
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["channel_id"], "recipient-123");
        let embed = &payload["embeds"][0];
        assert_eq!(embed["image"]["url"], "https://example.com/photo.jpg");
    }

    #[test]
    fn test_render_card_with_actions() {
        let msg = card_message(ChannelType::Discord);
        let payload = renderer().render(&msg).unwrap();
        let embed = &payload["embeds"][0];
        assert_eq!(embed["title"], "Training Plan");
        // Discord uses action rows with buttons
        let components = &payload["components"][0]["components"];
        assert!(components.is_array());
    }

    #[test]
    fn test_capabilities() {
        let r = renderer();
        assert_eq!(r.max_message_length(), 2000);
        assert!(r.supports_media());
        assert!(r.supports_cards());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Slack Renderer Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod slack {
    use super::*;
    use pierre_messaging::channels::slack::renderer::SlackRenderer;

    fn renderer() -> SlackRenderer {
        SlackRenderer
    }

    #[test]
    fn test_render_text_message() {
        let msg = text_message(ChannelType::Slack, "Recovery day");
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["channel"], "recipient-123");
        let blocks = payload["blocks"].as_array().unwrap();
        assert!(!blocks.is_empty());
        // Text should be in a section block
        let text_block = &blocks[0];
        assert_eq!(text_block["type"], "section");
        assert_eq!(text_block["text"]["text"], "Recovery day");
    }

    #[test]
    fn test_render_media_message() {
        let msg = media_message(ChannelType::Slack);
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["channel"], "recipient-123");
        let blocks = payload["blocks"].as_array().unwrap();
        // Should contain an image block
        let image_block = blocks
            .iter()
            .find(|b| b["type"] == "image")
            .expect("Should have image block");
        assert_eq!(image_block["image_url"], "https://example.com/photo.jpg");
    }

    #[test]
    fn test_render_card_with_actions() {
        let msg = card_message(ChannelType::Slack);
        let payload = renderer().render(&msg).unwrap();
        let blocks = payload["blocks"].as_array().unwrap();
        // Should have header, section, and actions blocks
        let has_header = blocks.iter().any(|b| b["type"] == "header");
        let has_actions = blocks.iter().any(|b| b["type"] == "actions");
        assert!(has_header, "Card should have header block");
        assert!(has_actions, "Card should have actions block");
    }

    #[test]
    fn test_capabilities() {
        let r = renderer();
        assert_eq!(r.max_message_length(), 40000);
        assert!(r.supports_media());
        assert!(r.supports_cards());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Telegram Renderer Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod telegram {
    use super::*;
    use pierre_messaging::channels::telegram::renderer::TelegramRenderer;

    fn renderer() -> TelegramRenderer {
        TelegramRenderer
    }

    #[test]
    fn test_render_text_message() {
        let msg = text_message(ChannelType::Telegram, "Interval workout");
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["chat_id"], "recipient-123");
        assert_eq!(payload["text"], "Interval workout");
        assert_eq!(payload["parse_mode"], "HTML");
    }

    #[test]
    fn test_render_location_message() {
        let msg = location_message(ChannelType::Telegram);
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["chat_id"], "recipient-123");
        let lat = payload["latitude"].as_f64().unwrap();
        let lon = payload["longitude"].as_f64().unwrap();
        assert!((lat - 48.8566).abs() < 0.001);
        assert!((lon - 2.3522).abs() < 0.001);
    }

    #[test]
    fn test_render_card_with_keyboard() {
        let msg = card_message(ChannelType::Telegram);
        let payload = renderer().render(&msg).unwrap();
        assert!(payload["text"].as_str().unwrap().contains("Training Plan"));
        let keyboard = &payload["reply_markup"]["inline_keyboard"];
        assert!(keyboard.is_array());
        assert!(!keyboard.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_capabilities() {
        let r = renderer();
        assert_eq!(r.max_message_length(), 4096);
        assert!(r.supports_media());
        assert!(r.supports_cards());
    }

    #[test]
    fn test_reply_to_adds_reply_to_message_id() {
        let msg = OutgoingMessage {
            channel_type: ChannelType::Telegram,
            recipient_id: "12345".to_owned(),
            content: MessageContent::Text {
                body: "Reply text".to_owned(),
            },
            correlation_id: Uuid::nil(),
            reply_to: Some("42".to_owned()),
            thread_id: None,
        };
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["reply_to_message_id"], 42);
    }

    #[test]
    fn test_reply_to_none_omits_reply_to_message_id() {
        let msg = text_message(ChannelType::Telegram, "No reply context");
        let payload = renderer().render(&msg).unwrap();
        assert!(payload.get("reply_to_message_id").is_none());
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Slack Reply-To Thread Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "client-messaging")]
mod slack_reply_to {
    use super::*;
    use pierre_messaging::channels::slack::renderer::SlackRenderer;
    use pierre_messaging::renderer::ResponseRenderer;

    fn renderer() -> SlackRenderer {
        SlackRenderer
    }

    #[test]
    fn test_reply_to_adds_thread_ts() {
        let msg = OutgoingMessage {
            channel_type: ChannelType::Slack,
            recipient_id: "C12345".to_owned(),
            content: MessageContent::Text {
                body: "Threaded reply".to_owned(),
            },
            correlation_id: Uuid::nil(),
            reply_to: Some("1234567890.123456".to_owned()),
            thread_id: None,
        };
        let payload = renderer().render(&msg).unwrap();
        assert_eq!(payload["thread_ts"], "1234567890.123456");
        assert_eq!(payload["channel"], "C12345");
    }

    #[test]
    fn test_reply_to_none_omits_thread_ts() {
        let msg = text_message(ChannelType::Slack, "No thread context");
        let payload = renderer().render(&msg).unwrap();
        assert!(payload.get("thread_ts").is_none());
    }
}
