// ABOUTME: Telegram Bot API response renderer using HTML parse mode
// ABOUTME: Formats OutgoingMessage into sendMessage payloads with inline keyboards
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use html_escape::encode_text;
use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{MessageContent, OutgoingMessage};
use serde_json::{json, Value};

use crate::renderer::ResponseRenderer;

/// Telegram Bot API message renderer
///
/// Uses HTML parse mode for rich text and inline keyboard markup for actions.
pub struct TelegramRenderer;

impl ResponseRenderer for TelegramRenderer {
    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        let chat_id = &msg.recipient_id;

        let mut payload = match &msg.content {
            MessageContent::Text { body } => json!({
                "chat_id": chat_id,
                "text": encode_text(body).as_ref(),
                "parse_mode": "HTML"
            }),
            MessageContent::Media { url, caption, .. } => json!({
                "chat_id": chat_id,
                "photo": url,
                "caption": caption.as_deref().unwrap_or("")
            }),
            MessageContent::Location {
                latitude,
                longitude,
            } => json!({
                "chat_id": chat_id,
                "latitude": latitude,
                "longitude": longitude
            }),
            MessageContent::Card {
                title,
                body,
                actions,
            } => {
                let text = format!("<b>{}</b>\n\n{}", encode_text(title), encode_text(body));
                let mut card_payload = json!({
                    "chat_id": chat_id,
                    "text": text,
                    "parse_mode": "HTML"
                });

                if !actions.is_empty() {
                    let keyboard: Vec<Vec<Value>> = actions
                        .iter()
                        .map(|action| {
                            let btn = if action.action_type == "url" {
                                json!({ "text": action.label, "url": action.value })
                            } else {
                                json!({ "text": action.label, "callback_data": action.value })
                            };
                            vec![btn]
                        })
                        .collect();

                    card_payload["reply_markup"] = json!({
                        "inline_keyboard": keyboard
                    });
                }

                card_payload
            }
        };

        // Add reply_to_message_id for threaded reply context
        if let Some(ref reply_to) = msg.reply_to {
            if let Ok(msg_id) = reply_to.parse::<i64>() {
                payload["reply_to_message_id"] = json!(msg_id);
            }
        }

        Ok(payload)
    }

    fn max_message_length(&self) -> usize {
        4096
    }

    fn supports_media(&self) -> bool {
        true
    }

    fn supports_cards(&self) -> bool {
        true
    }
}
