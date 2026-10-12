// ABOUTME: The text and type label of a channel message's content, read one way by every messaging path
// ABOUTME: The inbound and outbound ledgers, the intake, the agent pick and the room transcript all read it here

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::messaging::MessageContent;

/// Extract a content type label from the message content variant
pub(super) fn content_type_label(content: &MessageContent) -> &'static str {
    match content {
        MessageContent::Text { .. } => "text",
        MessageContent::RichText { .. } => "rich_text",
        MessageContent::Media { .. } => "media",
        MessageContent::Location { .. } => "location",
        MessageContent::Card { .. } => "card",
    }
}

/// Extract the text body from the message content (if applicable)
pub(super) fn content_body_text(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::Text { body }
        | MessageContent::RichText { body }
        | MessageContent::Card { body, .. } => Some(body.clone()),
        MessageContent::Media { caption, .. } => caption.clone(),
        MessageContent::Location { .. } => None,
    }
}
