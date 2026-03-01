// ABOUTME: Shared types for messaging providers including messages, channels, and connections
// ABOUTME: Provider-agnostic data structures used across Slack, Discord, and Teams integrations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Incoming message received from an external messaging provider
///
/// Normalized representation of a message received via webhook, regardless
/// of which provider sent it. The bridge service uses this to route messages
/// to the appropriate Dravr conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMessage {
    /// Provider-specific channel identifier (e.g., Slack channel ID)
    pub channel_id: String,
    /// Provider-specific user identifier who sent the message
    pub sender_id: String,
    /// Display name of the sender (if available)
    pub sender_name: Option<String>,
    /// Message text content
    pub text: String,
    /// Provider-specific message timestamp/identifier
    pub message_id: String,
    /// Provider-specific thread identifier (for threaded conversations)
    pub thread_id: Option<String>,
    /// Provider-specific team/workspace identifier
    pub team_id: String,
    /// When the message was sent
    pub timestamp: DateTime<Utc>,
}

/// Outgoing message to send to an external messaging provider
///
/// Provider implementations translate this into their native API format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// Target channel identifier
    pub channel_id: String,
    /// Message text content (Markdown supported by most providers)
    pub text: String,
    /// Thread identifier to reply in a thread (provider-specific)
    pub thread_id: Option<String>,
}

impl OutgoingMessage {
    /// Create a top-level text message for a channel
    #[must_use]
    pub fn text(channel_id: &str, text: &str) -> Self {
        Self {
            channel_id: channel_id.to_owned(),
            text: text.to_owned(),
            thread_id: None,
        }
    }

    /// Create a threaded reply message
    #[must_use]
    pub fn reply(channel_id: &str, thread_id: &str, text: &str) -> Self {
        Self {
            channel_id: channel_id.to_owned(),
            text: text.to_owned(),
            thread_id: Some(thread_id.to_owned()),
        }
    }
}

/// Response after sending a message to an external provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResponse {
    /// Provider-specific message identifier for the sent message
    pub message_id: String,
    /// Provider-specific timestamp of the sent message
    pub timestamp: Option<String>,
}

/// Channel information from an external messaging provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelInfo {
    /// Provider-specific channel identifier
    pub id: String,
    /// Channel display name
    pub name: String,
    /// Whether this is a private channel/group
    pub is_private: bool,
    /// Number of members in the channel (if available)
    pub member_count: Option<u32>,
    /// Channel topic/purpose (if set)
    pub topic: Option<String>,
}
