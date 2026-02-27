// ABOUTME: Messaging provider connection and channel binding record types
// ABOUTME: Database-layer models for multi-tenant messaging platform integrations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

/// Database record for a messaging provider connection (workspace-level, per-tenant)
///
/// Represents a connected workspace/server from an external messaging platform (Slack, Discord, etc.).
/// Credentials are stored encrypted at rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingConnectionRecord {
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
    /// User ID who created this connection
    pub created_by: String,
    /// When this connection was created (ISO 8601)
    pub created_at: String,
    /// When this connection was last updated (ISO 8601)
    pub updated_at: String,
}

/// Database record for a channel binding
///
/// Links an external provider channel to a Dravr conversation. When active,
/// messages in the external channel are bridged to the Dravr AI chat system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBindingRecord {
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
    /// User who owns the bound conversation
    pub user_id: String,
    /// Whether this binding is currently active
    pub active: bool,
    /// When this binding was created (ISO 8601)
    pub created_at: String,
    /// When this binding was last updated (ISO 8601)
    pub updated_at: String,
}

/// Parameters for creating a new messaging connection
#[derive(Debug, Clone)]
pub struct CreateMessagingConnectionParams<'a> {
    /// Tenant ID
    pub tenant_id: &'a str,
    /// Provider name (e.g., "slack")
    pub provider: &'a str,
    /// Provider-specific workspace/team identifier
    pub team_id: &'a str,
    /// Human-readable workspace name
    pub team_name: Option<&'a str>,
    /// Encrypted bot token
    pub bot_token: &'a str,
    /// Encrypted signing secret
    pub signing_secret: &'a str,
    /// User ID who is creating this connection
    pub created_by: &'a str,
}

/// Parameters for creating a new channel binding
#[derive(Debug, Clone)]
pub struct CreateChannelBindingParams<'a> {
    /// Messaging connection ID
    pub messaging_connection_id: &'a str,
    /// Tenant ID
    pub tenant_id: &'a str,
    /// Provider-specific channel identifier
    pub channel_id: &'a str,
    /// Human-readable channel name
    pub channel_name: Option<&'a str>,
    /// Dravr conversation ID to bind to
    pub conversation_id: &'a str,
    /// User who owns the conversation
    pub user_id: &'a str,
}
