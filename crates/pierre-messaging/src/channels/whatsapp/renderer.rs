// ABOUTME: WhatsApp response renderer using Twilio Messages API format
// ABOUTME: Formats OutgoingMessage into Twilio form parameters for WhatsApp delivery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{MessageContent, OutgoingMessage};
use serde_json::{json, Value};
use std::fmt::Write;

use crate::renderer::ResponseRenderer;

/// `WhatsApp` Twilio Messages API renderer
///
/// Formats messages as Twilio API parameters:
/// - Text: `Body` field
/// - Media: `MediaUrl` field with optional `Body` caption
/// - Location/Card: formatted text body
pub struct WhatsAppRenderer;

impl ResponseRenderer for WhatsAppRenderer {
    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        let to = format!("whatsapp:{}", msg.recipient_id);

        match &msg.content {
            MessageContent::Text { body } => Ok(json!({
                "To": to,
                "Body": body
            })),
            MessageContent::Media { url, caption, .. } => {
                let mut payload = json!({
                    "To": to,
                    "MediaUrl": url
                });
                if let Some(cap) = caption {
                    payload["Body"] = json!(cap);
                }
                Ok(payload)
            }
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                let body_text = format!("Location: {latitude}, {longitude}");
                Ok(json!({
                    "To": to,
                    "Body": body_text
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
                    "To": to,
                    "Body": text
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
