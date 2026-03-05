// ABOUTME: Meta Messenger Platform channel adapter module
// ABOUTME: Combines MessengerTransport (x-hub-signature-256) with MessengerRenderer (Graph API)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Graph API message renderer for Messenger
pub mod renderer;
/// HMAC-SHA256 signature verification and webhook parsing for Messenger
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

use self::renderer::MessengerRenderer;
use self::transport::MessengerTransport;

/// Messenger channel adapter combining transport and renderer
pub struct MessengerChannel {
    /// Wire protocol adapter for Messenger Platform
    transport: MessengerTransport,
    /// Graph API message renderer
    renderer: MessengerRenderer,
}

impl MessengerChannel {
    /// Create a new Messenger channel adapter
    #[must_use]
    pub fn new(app_secret: String) -> Self {
        Self {
            transport: MessengerTransport::new(app_secret),
            renderer: MessengerRenderer,
        }
    }
}

/// Messenger channel metadata descriptor
pub struct MessengerDescriptor;

impl ChannelDescriptor for MessengerDescriptor {
    fn name(&self) -> &'static str {
        "messenger"
    }
    fn display_name(&self) -> &'static str {
        "Messenger"
    }
    fn channel_type(&self) -> ChannelType {
        ChannelType::Messenger
    }
    fn webhook_path(&self) -> &'static str {
        "/api/messaging/webhook/messenger"
    }
    fn supports_media(&self) -> bool {
        true
    }
    fn max_message_length(&self) -> usize {
        2000
    }
    fn signature_header(&self) -> &'static str {
        "x-hub-signature-256"
    }
}

#[async_trait]
impl MessagingChannel for MessengerChannel {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Messenger
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
