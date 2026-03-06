// ABOUTME: WhatsApp response renderer using Meta Cloud API message format
// ABOUTME: Formats OutgoingMessage into WhatsApp Cloud API JSON payloads for delivery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{MessageContent, OutgoingMessage};
use serde_json::{json, Value};
use std::fmt::Write;

use crate::renderer::ResponseRenderer;

/// `WhatsApp` Cloud API renderer
///
/// Formats messages as Meta `WhatsApp` Business Cloud API payloads:
/// - Text: `{"messaging_product":"whatsapp","to":"phone","type":"text","text":{"body":"..."}}`
/// - Media: `{"messaging_product":"whatsapp","to":"phone","type":"image","image":{"link":"url"}}`
/// - Location/Card: rendered as text body
pub struct WhatsAppRenderer;

impl ResponseRenderer for WhatsAppRenderer {
    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        let to = &msg.recipient_id;

        match &msg.content {
            MessageContent::Text { body } => Ok(json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": "text",
                "text": { "body": body }
            })),
            MessageContent::Media {
                url,
                mime_type,
                caption,
            } => {
                let media_type = infer_media_type(mime_type);
                let mut media_obj = json!({ "link": url });
                if let Some(cap) = caption {
                    media_obj["caption"] = json!(cap);
                }
                Ok(json!({
                    "messaging_product": "whatsapp",
                    "to": to,
                    "type": media_type,
                    media_type: media_obj
                }))
            }
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                let body_text = format!("Location: {latitude}, {longitude}");
                Ok(json!({
                    "messaging_product": "whatsapp",
                    "to": to,
                    "type": "text",
                    "text": { "body": body_text }
                }))
            }
            MessageContent::Card {
                title,
                body,
                actions,
            } => {
                let mut text = format!("*{title}*\n\n{body}");
                for action in actions {
                    let _ = write!(text, "\n- {}: {}", action.label, action.value);
                }
                Ok(json!({
                    "messaging_product": "whatsapp",
                    "to": to,
                    "type": "text",
                    "text": { "body": text }
                }))
            }
        }
    }

    fn max_message_length(&self) -> usize {
        4096
    }

    fn supports_media(&self) -> bool {
        true
    }

    fn supports_cards(&self) -> bool {
        false
    }
}

/// Map a MIME type to a `WhatsApp` Cloud API media type
fn infer_media_type(mime_type: &str) -> &'static str {
    if mime_type.starts_with("image/") {
        "image"
    } else if mime_type.starts_with("video/") {
        "video"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else {
        "document"
    }
}
