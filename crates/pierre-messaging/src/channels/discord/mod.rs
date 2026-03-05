// ABOUTME: Discord Bot API channel adapter module
// ABOUTME: Combines DiscordTransport (Ed25519 verification) with DiscordRenderer (embeds)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Embed-based message renderer for Discord
pub mod renderer;
/// Ed25519 signature verification and webhook parsing for Discord
pub mod transport;

use async_trait::async_trait;
use http::HeaderMap;
use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, IncomingMessage, OutgoingMessage,
};
use serde_json::Value;

use crate::channel::MessagingChannel;
use crate::descriptor::ChannelDescriptor;
use crate::renderer::ResponseRenderer;
use crate::transport::TransportAdapter;

use self::renderer::DiscordRenderer;
use self::transport::DiscordTransport;

/// Discord channel adapter combining transport and renderer
pub struct DiscordChannel {
    /// Wire protocol adapter for Discord API
    transport: DiscordTransport,
    /// Embed-based message renderer
    renderer: DiscordRenderer,
}

impl DiscordChannel {
    /// Create a new Discord channel adapter
    #[must_use]
    pub fn new(public_key_hex: String, application_id: String) -> Self {
        Self {
            transport: DiscordTransport::new(public_key_hex, application_id),
            renderer: DiscordRenderer,
        }
    }
}

/// Discord channel metadata descriptor
pub struct DiscordDescriptor;

impl ChannelDescriptor for DiscordDescriptor {
    fn name(&self) -> &'static str {
        "discord"
    }
    fn display_name(&self) -> &'static str {
        "Discord"
    }
    fn channel_type(&self) -> ChannelType {
        ChannelType::Discord
    }
    fn webhook_path(&self) -> &'static str {
        "/api/messaging/webhook/discord"
    }
    fn supports_media(&self) -> bool {
        true
    }
    fn max_message_length(&self) -> usize {
        2000
    }
    fn signature_header(&self) -> &'static str {
        "x-signature-ed25519"
    }
}

#[async_trait]
impl MessagingChannel for DiscordChannel {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Discord
    }

    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        self.transport.verify_signature(headers, body)
    }

    async fn receive(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        self.transport.parse_inbound(headers, body).await
    }

    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        self.renderer.render(msg)
    }

    async fn send(
        &self,
        msg: &OutgoingMessage,
        config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        let payload = self.render(msg)?;
        self.transport.send_raw(&payload, config).await
    }

    async fn send_raw(
        &self,
        payload: &Value,
        config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        self.transport.send_raw(payload, config).await
    }
}
