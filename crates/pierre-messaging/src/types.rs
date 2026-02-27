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

/// Connection record for a messaging provider workspace
///
/// Represents a connected workspace/team from an external provider. Each tenant
/// can have multiple connections (e.g., multiple Slack workspaces), and each
/// connection stores encrypted credentials for API access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingConnection {
    /// Unique identifier for this connection
    pub id: String,
    /// Tenant this connection belongs to
    pub tenant_id: String,
    /// Provider name (e.g., "slack", "discord")
    pub provider: String,
    /// Provider-specific workspace/team identifier
    pub team_id: String,
    /// Human-readable workspace name
    pub team_name: Option<String>,
    /// Encrypted bot token for API calls
    pub bot_token: String,
    /// Encrypted webhook signing secret for request verification
    pub signing_secret: String,
    /// User who created this connection
    pub created_by: String,
    /// When this connection was created
    pub created_at: DateTime<Utc>,
    /// When this connection was last updated
    pub updated_at: DateTime<Utc>,
}

impl MessagingConnection {
    /// Create a new messaging connection record
    #[must_use]
    pub fn new(
        tenant_id: &str,
        provider: &str,
        team_id: &str,
        team_name: Option<&str>,
        bot_token: &str,
        signing_secret: &str,
        created_by: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            tenant_id: tenant_id.to_owned(),
            provider: provider.to_owned(),
            team_id: team_id.to_owned(),
            team_name: team_name.map(ToOwned::to_owned),
            bot_token: bot_token.to_owned(),
            signing_secret: signing_secret.to_owned(),
            created_by: created_by.to_owned(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Channel binding: links an external channel to a Dravr conversation
///
/// When a channel binding is active, messages posted in the external channel
/// are forwarded to the bound Dravr conversation, and AI responses are posted
/// back to the external channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    /// Unique identifier for this binding
    pub id: String,
    /// Reference to the messaging connection
    pub messaging_connection_id: String,
    /// Tenant this binding belongs to
    pub tenant_id: String,
    /// Provider-specific channel identifier
    pub channel_id: String,
    /// Human-readable channel name
    pub channel_name: Option<String>,
    /// Dravr conversation this channel is bound to
    pub conversation_id: String,
    /// User who owns the conversation
    pub user_id: String,
    /// Whether this binding is currently active
    pub active: bool,
    /// When this binding was created
    pub created_at: DateTime<Utc>,
    /// When this binding was last updated
    pub updated_at: DateTime<Utc>,
}

impl ChannelBinding {
    /// Create a new channel binding record
    #[must_use]
    pub fn new(
        messaging_connection_id: &str,
        tenant_id: &str,
        channel_id: &str,
        channel_name: Option<&str>,
        conversation_id: &str,
        user_id: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messaging_connection_id: messaging_connection_id.to_owned(),
            tenant_id: tenant_id.to_owned(),
            channel_id: channel_id.to_owned(),
            channel_name: channel_name.map(ToOwned::to_owned),
            conversation_id: conversation_id.to_owned(),
            user_id: user_id.to_owned(),
            active: true,
            created_at: now,
            updated_at: now,
        }
    }
}
