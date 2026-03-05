// ABOUTME: Message router dispatching inbound messages to Pierre chat pipeline
// ABOUTME: Handles session lookup/creation and outbound message formatting via renderers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::messaging::IncomingMessage;
use tracing::info;

/// Routes inbound messages to the Pierre chat pipeline
///
/// Responsibilities:
/// - Look up or create a messaging session for the sender
/// - Forward the message content to the Pierre conversation engine
/// - Queue outbound responses for delivery through the channel
pub struct MessageRouter;

impl MessageRouter {
    /// Create a new message router
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Route an inbound message through the Pierre chat pipeline
    ///
    /// This method:
    /// 1. Resolves the sender to a Pierre user via session lookup
    /// 2. Forwards content to the conversation engine
    /// 3. Queues any response for outbound delivery
    pub fn route_inbound(&self, message: &IncomingMessage) {
        info!(
            channel = %message.channel_type,
            sender = %message.sender_id,
            correlation_id = %message.correlation_id,
            "Routing inbound message to chat pipeline"
        );
        // Chat pipeline integration will be wired in Phase 4
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}
