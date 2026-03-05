// ABOUTME: Channel descriptor trait providing metadata about each messaging platform
// ABOUTME: Static channel info: name, webhook path, capabilities, signature header
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::messaging::ChannelType;

/// Static metadata about a messaging channel
///
/// Provides compile-time channel information used for routing,
/// capability discovery, and documentation.
pub trait ChannelDescriptor {
    /// Internal channel name (lowercase, e.g., "slack")
    fn name(&self) -> &'static str;

    /// Human-readable display name (e.g., "Slack")
    fn display_name(&self) -> &'static str;

    /// Channel type enum variant
    fn channel_type(&self) -> ChannelType;

    /// Webhook URL path segment (e.g., "/api/messaging/webhook/slack")
    fn webhook_path(&self) -> &'static str;

    /// Whether this channel supports media attachments
    fn supports_media(&self) -> bool;

    /// Maximum message length in characters
    fn max_message_length(&self) -> usize;

    /// HTTP header name containing the webhook signature
    fn signature_header(&self) -> &'static str;
}
