// ABOUTME: Discord embed-based response renderer for rich message formatting
// ABOUTME: Formats OutgoingMessage into Discord embeds with optional action components
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{MessageContent, OutgoingMessage};
use serde_json::{json, Value};

use crate::renderer::ResponseRenderer;

/// Discord embed-based message renderer
///
/// Formats messages using Discord's embed objects and message components:
/// - Text: plain content field
/// - Card: embed with title, description, and action row buttons
/// - Media: embed with image field
pub struct DiscordRenderer;

impl ResponseRenderer for DiscordRenderer {
    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        match &msg.content {
            MessageContent::Text { body } => Ok(json!({
                "content": body,
                "channel_id": msg.recipient_id
            })),
            MessageContent::Media { url, caption, .. } => Ok(json!({
                "channel_id": msg.recipient_id,
                "embeds": [{
                    "image": { "url": url },
                    "description": caption.as_deref().unwrap_or("")
                }]
            })),
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                let text = format!("Location: {latitude}, {longitude}");
                Ok(json!({
                    "content": text,
                    "channel_id": msg.recipient_id
                }))
            }
            MessageContent::Card {
                title,
                body,
                actions,
            } => {
                let mut payload = json!({
                    "channel_id": msg.recipient_id,
                    "embeds": [{
                        "title": title,
                        "description": body
                    }]
                });

                if !actions.is_empty() {
                    let buttons: Vec<Value> = actions
                        .iter()
                        .map(|action| {
                            if action.action_type == "url" {
                                json!({
                                    "type": 2,
                                    "style": 5,
                                    "label": action.label,
                                    "url": action.value
                                })
                            } else {
                                json!({
                                    "type": 2,
                                    "style": 1,
                                    "label": action.label,
                                    "custom_id": action.value
                                })
                            }
                        })
                        .collect();

                    payload["components"] = json!([{
                        "type": 1,
                        "components": buttons
                    }]);
                }

                Ok(payload)
            }
        }
    }

    fn max_message_length(&self) -> usize {
        2000
    }

    fn supports_media(&self) -> bool {
        true
    }

    fn supports_cards(&self) -> bool {
        true
    }
}
