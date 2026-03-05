// ABOUTME: Slack Block Kit response renderer formatting OutgoingMessage to blocks
// ABOUTME: Supports mrkdwn sections, action blocks with buttons, and image blocks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{MessageContent, OutgoingMessage};
use serde_json::{json, Value};

use crate::renderer::ResponseRenderer;

/// Slack Block Kit message renderer
///
/// Formats messages using Slack's Block Kit framework for rich layouts:
/// - Text: `section` block with `mrkdwn`
/// - Card: `section` with fields + `actions` block with buttons
/// - Media: `image` block
pub struct SlackRenderer;

impl ResponseRenderer for SlackRenderer {
    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        let channel = &msg.recipient_id;

        match &msg.content {
            MessageContent::Text { body } => Ok(json!({
                "channel": channel,
                "blocks": [{
                    "type": "section",
                    "text": {
                        "type": "mrkdwn",
                        "text": body
                    }
                }]
            })),
            MessageContent::Media { url, caption, .. } => Ok(json!({
                "channel": channel,
                "blocks": [{
                    "type": "image",
                    "image_url": url,
                    "alt_text": caption.as_deref().unwrap_or("Image")
                }]
            })),
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                let map_text = format!("Location: {latitude}, {longitude}");
                Ok(json!({
                    "channel": channel,
                    "blocks": [{
                        "type": "section",
                        "text": {
                            "type": "mrkdwn",
                            "text": map_text
                        }
                    }]
                }))
            }
            MessageContent::Card {
                title,
                body,
                actions,
            } => {
                let mut blocks = vec![
                    json!({
                        "type": "header",
                        "text": { "type": "plain_text", "text": title }
                    }),
                    json!({
                        "type": "section",
                        "text": { "type": "mrkdwn", "text": body }
                    }),
                ];

                if !actions.is_empty() {
                    let elements: Vec<Value> = actions
                        .iter()
                        .map(|action| {
                            let mut btn = json!({
                                "type": "button",
                                "text": { "type": "plain_text", "text": action.label }
                            });
                            if action.action_type == "url" {
                                btn["url"] = json!(action.value);
                            } else {
                                btn["action_id"] = json!(action.value);
                            }
                            btn
                        })
                        .collect();

                    blocks.push(json!({
                        "type": "actions",
                        "elements": elements
                    }));
                }

                Ok(json!({
                    "channel": channel,
                    "blocks": blocks
                }))
            }
        }
    }

    fn max_message_length(&self) -> usize {
        40000
    }

    fn supports_media(&self) -> bool {
        true
    }

    fn supports_cards(&self) -> bool {
        true
    }
}
