// ABOUTME: Meta Messenger Graph API response renderer for message templates
// ABOUTME: Formats OutgoingMessage into Graph API message payloads with quick replies
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{MessageContent, OutgoingMessage};
use serde_json::{json, Value};

use crate::renderer::ResponseRenderer;

/// Messenger Graph API message renderer
///
/// Formats messages using Facebook's Send API format:
/// - Text: simple text message
/// - Card: generic template with buttons
/// - Media: attachment with URL
pub struct MessengerRenderer;

impl ResponseRenderer for MessengerRenderer {
    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        let recipient = json!({ "id": msg.recipient_id });

        match &msg.content {
            MessageContent::Text { body } => Ok(json!({
                "recipient": recipient,
                "message": { "text": body }
            })),
            MessageContent::Media { url, mime_type, .. } => {
                let attachment_type = if mime_type.starts_with("image") {
                    "image"
                } else if mime_type.starts_with("video") {
                    "video"
                } else if mime_type.starts_with("audio") {
                    "audio"
                } else {
                    "file"
                };
                Ok(json!({
                    "recipient": recipient,
                    "message": {
                        "attachment": {
                            "type": attachment_type,
                            "payload": { "url": url, "is_reusable": true }
                        }
                    }
                }))
            }
            MessageContent::Location {
                latitude,
                longitude,
            } => {
                let text = format!("Location: {latitude}, {longitude}");
                Ok(json!({
                    "recipient": recipient,
                    "message": { "text": text }
                }))
            }
            MessageContent::Card {
                title,
                body,
                actions,
            } => {
                let buttons: Vec<Value> = actions
                    .iter()
                    .map(|action| {
                        if action.action_type == "url" {
                            json!({
                                "type": "web_url",
                                "url": action.value,
                                "title": action.label
                            })
                        } else {
                            json!({
                                "type": "postback",
                                "title": action.label,
                                "payload": action.value
                            })
                        }
                    })
                    .collect();

                Ok(json!({
                    "recipient": recipient,
                    "message": {
                        "attachment": {
                            "type": "template",
                            "payload": {
                                "template_type": "generic",
                                "elements": [{
                                    "title": title,
                                    "subtitle": body,
                                    "buttons": buttons
                                }]
                            }
                        }
                    }
                }))
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
