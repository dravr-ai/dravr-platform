// ABOUTME: Slack-specific event and API response types for the Events API and Web API
// ABOUTME: Covers url_verification, event_callback with message events, and chat.postMessage responses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

/// Top-level Slack Events API payload
///
/// Slack sends different event types through the same webhook endpoint.
/// The `type` field determines which variant applies.
///
/// See: <https://api.slack.com/events-api#receiving_events>
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum SlackEventPayload {
    /// URL verification challenge sent during webhook registration
    #[serde(rename = "url_verification")]
    UrlVerification {
        /// Challenge token to echo back
        challenge: String,
        /// Verification token (deprecated, use signing secret instead)
        token: String,
    },

    /// Event callback containing an actual Slack event
    #[serde(rename = "event_callback")]
    EventCallback(Box<SlackEventCallback>),
}

/// Data payload for an event callback from the Slack Events API
#[derive(Debug, Clone, Deserialize)]
pub struct SlackEventCallback {
    /// Workspace/team identifier
    pub team_id: String,
    /// The inner event payload
    pub event: SlackEvent,
    /// Unique event identifier for deduplication
    pub event_id: String,
    /// Unix timestamp of the event
    pub event_time: u64,
}

/// Inner Slack event types
///
/// Only message-related events are handled; other event types are ignored.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum SlackEvent {
    /// A message was posted to a channel
    #[serde(rename = "message")]
    Message(SlackMessageEvent),

    /// The app was mentioned in a channel
    #[serde(rename = "app_mention")]
    AppMention(SlackMessageEvent),
}

/// Slack message event data
///
/// Contains the message content, sender, channel, and threading information.
/// Bot messages are identified by the presence of `bot_id` or `subtype` = `bot_message`.
#[derive(Debug, Clone, Deserialize)]
pub struct SlackMessageEvent {
    /// User ID who sent the message (absent for bot messages)
    pub user: Option<String>,
    /// Message text content
    pub text: Option<String>,
    /// Channel where the message was posted
    pub channel: Option<String>,
    /// Message timestamp (serves as unique ID within the channel)
    #[serde(rename = "ts")]
    pub timestamp: Option<String>,
    /// Thread parent timestamp (present if this is a threaded reply)
    pub thread_ts: Option<String>,
    /// Bot ID if the message was sent by a bot
    pub bot_id: Option<String>,
    /// Message subtype (e.g., `bot_message`, `channel_join`)
    pub subtype: Option<String>,
    /// Channel type: "channel", "group", "im", "mpim"
    pub channel_type: Option<String>,
}

impl SlackMessageEvent {
    /// Check if this message was sent by a bot
    ///
    /// Returns true if the message has a `bot_id` or subtype indicating bot origin.
    /// Used to prevent infinite loops where the bot responds to its own messages.
    #[must_use]
    pub fn is_bot_message(&self) -> bool {
        self.bot_id.is_some() || self.subtype.as_deref() == Some("bot_message")
    }
}

/// Response from the Slack `chat.postMessage` API
///
/// See: <https://api.slack.com/methods/chat.postMessage>
#[derive(Debug, Clone, Deserialize)]
pub struct SlackPostMessageResponse {
    /// Whether the API call was successful
    pub ok: bool,
    /// Error code if the call failed
    pub error: Option<String>,
    /// Channel where the message was posted
    pub channel: Option<String>,
    /// Timestamp of the posted message (serves as message ID)
    pub ts: Option<String>,
}

/// Response from the Slack `conversations.list` API
///
/// See: <https://api.slack.com/methods/conversations.list>
#[derive(Debug, Clone, Deserialize)]
pub struct SlackConversationsListResponse {
    /// Whether the API call was successful
    pub ok: bool,
    /// Error code if the call failed
    pub error: Option<String>,
    /// List of channels/conversations
    pub channels: Option<Vec<SlackChannel>>,
}

/// Slack channel metadata from the conversations API
#[derive(Debug, Clone, Deserialize)]
pub struct SlackChannel {
    /// Channel identifier
    pub id: String,
    /// Channel name (without the # prefix)
    pub name: Option<String>,
    /// Whether this is a private channel
    pub is_private: Option<bool>,
    /// Number of members
    pub num_members: Option<u32>,
    /// Channel topic
    pub topic: Option<SlackTopic>,
}

/// Slack channel topic metadata
#[derive(Debug, Clone, Deserialize)]
pub struct SlackTopic {
    /// Topic text value
    pub value: Option<String>,
}

/// Response body for Slack URL verification challenge
#[derive(Debug, Serialize)]
pub struct UrlVerificationResponse {
    /// The challenge token echoed back to Slack
    pub challenge: String,
}
