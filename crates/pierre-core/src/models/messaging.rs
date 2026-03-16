// ABOUTME: Data types for multi-channel messaging gateway (WhatsApp, Messenger, Discord, Slack, Telegram)
// ABOUTME: Defines channel types, message content variants, delivery tracking, and retry queue entries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Supported messaging channel platforms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    /// `WhatsApp` Business Cloud API via Meta Graph API
    #[serde(rename = "whatsapp")]
    WhatsApp,
    /// Meta Messenger Platform
    Messenger,
    /// Discord Bot API
    Discord,
    /// Slack Events API
    Slack,
    /// Telegram Bot API
    Telegram,
}

impl fmt::Display for ChannelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WhatsApp => write!(f, "whatsapp"),
            Self::Messenger => write!(f, "messenger"),
            Self::Discord => write!(f, "discord"),
            Self::Slack => write!(f, "slack"),
            Self::Telegram => write!(f, "telegram"),
        }
    }
}

impl ChannelType {
    /// Determine the linking method for this channel type
    ///
    /// `Telegram` and `WhatsApp` use deep links with embedded codes.
    /// `Slack`, `Discord`, and `Messenger` use `OAuth2` authorization flows.
    #[must_use]
    pub const fn linking_method(self) -> LinkingMethod {
        match self {
            Self::Telegram | Self::WhatsApp => LinkingMethod::DeepLink,
            Self::Slack | Self::Discord | Self::Messenger => LinkingMethod::OAuth,
        }
    }
}

impl FromStr for ChannelType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "whatsapp" => Ok(Self::WhatsApp),
            "messenger" => Ok(Self::Messenger),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            "telegram" => Ok(Self::Telegram),
            other => Err(format!("unknown channel type: {other}")),
        }
    }
}

/// Message content variants for incoming and outgoing messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MessageContent {
    /// Plain text message
    Text {
        /// Message body text
        body: String,
    },
    /// Media attachment (image, video, audio, document)
    Media {
        /// Media URL or file identifier
        url: String,
        /// MIME type of the media
        mime_type: String,
        /// Optional caption for the media
        caption: Option<String>,
    },
    /// Geographic location
    Location {
        /// Latitude in decimal degrees
        latitude: f64,
        /// Longitude in decimal degrees
        longitude: f64,
    },
    /// Rich card with structured content and action buttons
    Card {
        /// Card title
        title: String,
        /// Card body text
        body: String,
        /// Interactive action buttons
        actions: Vec<CardAction>,
    },
}

/// Interactive action button within a card
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardAction {
    /// Display label for the action
    pub label: String,
    /// Action type (e.g., "url", "postback")
    pub action_type: String,
    /// Action payload or URL
    pub value: String,
}

/// Normalized incoming message from any channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMessage {
    /// Source channel platform
    pub channel_type: ChannelType,
    /// Channel-specific sender identifier
    pub sender_id: String,
    /// Human-readable sender name (if available)
    pub sender_name: Option<String>,
    /// Parsed message content
    pub content: MessageContent,
    /// Channel-specific conversation or thread identifier
    pub conversation_id: Option<String>,
    /// Channel-native message ID (used as idempotency key)
    pub channel_message_id: String,
    /// Message timestamp
    pub timestamp: DateTime<Utc>,
    /// Original webhook payload for audit logging
    pub raw_payload: Value,
    /// Correlation ID linking inbound to processing to outbound
    pub correlation_id: Uuid,
    /// Additional channel-specific metadata
    pub metadata: Value,
}

/// Outgoing message to be sent through a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// Target channel platform
    pub channel_type: ChannelType,
    /// Recipient identifier (phone number, user ID, channel ID)
    pub recipient_id: String,
    /// Message content to send
    pub content: MessageContent,
    /// Correlation ID for tracing end-to-end message flow
    pub correlation_id: Uuid,
}

/// Delivery receipt tracking outbound message lifecycle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    /// Internal message identifier
    pub message_id: String,
    /// Channel-assigned message ID (if available)
    pub channel_message_id: Option<String>,
    /// Current delivery status
    pub status: DeliveryStatus,
    /// Timestamp of the status update
    pub timestamp: DateTime<Utc>,
}

/// Delivery status progression
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryStatus {
    /// Queued for delivery
    Pending,
    /// Successfully sent to channel API
    Sent,
    /// Confirmed delivered to recipient
    Delivered,
    /// Recipient has read the message
    Read,
    /// Delivery failed (see reason in receipt)
    Failed,
    /// Exhausted all retries, moved to dead-letter queue
    Dlq,
}

/// Outbound queue entry for retry tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundQueueEntry {
    /// Queue entry identifier
    pub id: String,
    /// Reference to the original message
    pub message_id: String,
    /// Tenant identifier for isolation
    pub tenant_id: String,
    /// Target channel
    pub channel_type: ChannelType,
    /// Serialized outbound payload
    pub payload: Value,
    /// Current queue status (pending, retrying:N, sent, dlq)
    pub status: String,
    /// Number of delivery attempts made
    pub attempt_count: i32,
    /// Scheduled time for next retry attempt
    pub next_retry_at: Option<DateTime<Utc>>,
}

/// Per-tenant channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Configuration identifier
    pub id: String,
    /// Tenant identifier for isolation
    pub tenant_id: String,
    /// Channel platform type
    pub channel_type: ChannelType,
    /// API key or access token
    pub api_key: Option<String>,
    /// API secret or auth token
    pub api_secret: Option<String>,
    /// Webhook signing secret for signature verification
    pub webhook_secret: Option<String>,
    /// Meta webhook verify token (distinct from `webhook_secret` to avoid leaking HMAC key)
    pub verify_token: Option<String>,
    /// Platform-specific account identifier
    pub account_id: Option<String>,
    /// Phone number (for `WhatsApp`/SMS)
    pub phone_number: Option<String>,
    /// Bot token (for Discord/Telegram)
    pub bot_token: Option<String>,
    /// Whether this channel configuration is active
    pub is_active: bool,
}

/// Active messaging session linking a channel user to a Pierre conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingSession {
    /// Session identifier
    pub id: String,
    /// Pierre user identifier
    pub user_id: String,
    /// Tenant identifier
    pub tenant_id: String,
    /// Channel platform
    pub channel_type: ChannelType,
    /// Channel-specific user identifier
    pub channel_user_id: String,
    /// Channel-specific conversation or thread ID
    pub channel_conversation_id: Option<String>,
    /// Pierre chat conversation identifier
    pub pierre_conversation_id: Option<String>,
    /// Timestamp of last message activity
    pub last_message_at: DateTime<Utc>,
}

/// Webhook timestamp replay protection policy
#[derive(Debug, Clone, Copy)]
pub struct WebhookTimestampPolicy {
    /// Maximum age in seconds for a webhook timestamp to be considered valid
    pub max_age_secs: u64,
}

impl Default for WebhookTimestampPolicy {
    fn default() -> Self {
        Self {
            max_age_secs: 300, // 5 minutes
        }
    }
}

/// Channel linking method: deep link (`Telegram`, `WhatsApp`) or `OAuth` (`Slack`, `Discord`, `Messenger`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkingMethod {
    /// Deep link with embedded verification code (`Telegram` `/start`, `WhatsApp` `LINK`)
    DeepLink,
    /// Standard `OAuth2` authorization code flow
    OAuth,
}

impl fmt::Display for LinkingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeepLink => write!(f, "deep_link"),
            Self::OAuth => write!(f, "oauth"),
        }
    }
}

impl FromStr for LinkingMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "deep_link" => Ok(Self::DeepLink),
            "oauth" => Ok(Self::OAuth),
            other => Err(format!("unknown linking method: {other}")),
        }
    }
}

/// Ephemeral link state for a pending channel linking request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingLinkState {
    /// State identifier
    pub id: String,
    /// Tenant identifier
    pub tenant_id: String,
    /// Pierre user requesting the link
    pub user_id: String,
    /// Target channel platform
    pub channel_type: ChannelType,
    /// Cryptographically random verification code
    pub code: String,
    /// Linking method (`deep_link` or `oauth`)
    pub method: LinkingMethod,
    /// Whether this code has been consumed
    pub used: bool,
    /// Expiration timestamp (10 minutes from creation)
    pub expires_at: DateTime<Utc>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
}

/// Permanent mapping between a Pierre user and a messaging channel identity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingChannelLink {
    /// Link identifier
    pub id: String,
    /// Tenant identifier
    pub tenant_id: String,
    /// Pierre user identifier
    pub user_id: String,
    /// Channel platform type
    pub channel_type: ChannelType,
    /// Channel-specific user identifier (phone number, platform user ID, etc.)
    pub channel_user_id: String,
    /// Human-readable display name from the platform
    pub display_name: Option<String>,
    /// Timestamp when the link was established
    pub linked_at: DateTime<Utc>,
}

/// Duration in minutes before a link verification code expires
pub const LINK_CODE_TTL_MINUTES: i64 = 10;

/// Retry delay schedule for outbound message delivery (seconds)
pub const RETRY_DELAYS_SECS: [u64; 3] = [1, 5, 30];

/// Maximum number of retry attempts before dead-lettering
pub const MAX_RETRY_ATTEMPTS: i32 = 3;
