// ABOUTME: Core MessagingProvider trait defining the contract for chat platform integrations
// ABOUTME: Provider-agnostic interface for sending messages, verifying webhooks, and listing channels
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::HeaderMap;

use crate::errors::AppResult;
use crate::types::{ChannelInfo, OutgoingMessage, SendMessageResponse};

/// Messaging provider trait for pluggable chat platform integrations
///
/// Each external messaging platform (Slack, Discord, Teams) implements this trait
/// to enable bidirectional message bridging with the Pierre AI chat system.
///
/// # Implementors
///
/// - [`crate::slack::SlackProvider`] — Slack Bot API integration
///
/// # Example
///
/// ```rust,no_run
/// use pierre_messaging::MessagingProvider;
/// use pierre_messaging::types::OutgoingMessage;
///
/// # async fn example(provider: &dyn MessagingProvider) -> pierre_core::errors::AppResult<()> {
/// let msg = OutgoingMessage::text("channel-id", "Hello from Pierre!");
/// provider.send_message(&msg).await?;
/// # Ok(())
/// # }
/// ```
#[async_trait::async_trait]
pub trait MessagingProvider: Send + Sync {
    /// Provider identifier (e.g., "slack", "discord", "teams")
    fn name(&self) -> &'static str;

    /// Display name for UI presentation (e.g., "Slack", "Discord", "Microsoft Teams")
    fn display_name(&self) -> &'static str;

    /// Send a message to an external channel
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails or the channel is inaccessible.
    async fn send_message(&self, message: &OutgoingMessage) -> AppResult<SendMessageResponse>;

    /// List channels accessible by the bot in the connected workspace
    ///
    /// # Errors
    ///
    /// Returns an error if the API call fails.
    async fn list_channels(&self) -> AppResult<Vec<ChannelInfo>>;

    /// Verify the authenticity of an incoming webhook request
    ///
    /// Each provider uses a different signature scheme (e.g., Slack uses HMAC-SHA256).
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails due to invalid signature or missing headers.
    fn verify_request(&self, headers: &HeaderMap, body: &[u8]) -> AppResult<bool>;
}
