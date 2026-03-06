// ABOUTME: Response renderer trait for channel-specific message formatting
// ABOUTME: Converts OutgoingMessage content into platform-native payload structures
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::OutgoingMessage;
use serde_json::Value;

/// Formats outgoing messages into channel-specific payload structures
///
/// Each channel has a unique message format:
/// - Slack: Block Kit JSON
/// - Discord: Embeds + components
/// - Messenger: Graph API templates
/// - `WhatsApp`: Meta Cloud API
/// - Telegram: Bot API with HTML parse mode
pub trait ResponseRenderer: Send + Sync {
    /// Render an `OutgoingMessage` into the channel's native payload format
    ///
    /// # Errors
    ///
    /// Returns `MessagingError` if the content cannot be rendered for this channel.
    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value>;

    /// Maximum message length supported by this channel
    fn max_message_length(&self) -> usize;

    /// Whether this channel supports media attachments
    fn supports_media(&self) -> bool;

    /// Whether this channel supports rich card layouts
    fn supports_cards(&self) -> bool;
}
