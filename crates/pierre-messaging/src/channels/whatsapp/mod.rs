// ABOUTME: WhatsApp Business Cloud API channel adapter module via Meta Graph API
// ABOUTME: Combines WhatsAppTransport (HMAC-SHA256 per Meta spec) with WhatsAppRenderer (Cloud API payloads)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Meta Cloud API response renderer for `WhatsApp`
pub mod renderer;
/// HMAC-SHA256 signature verification and webhook parsing for `WhatsApp`
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

use self::renderer::WhatsAppRenderer;
use self::transport::WhatsAppTransport;

/// `WhatsApp` channel adapter combining transport and renderer
pub struct WhatsAppChannel {
    /// Wire protocol adapter for Meta `WhatsApp` Cloud API
    transport: WhatsAppTransport,
    /// Meta Cloud API renderer
    renderer: WhatsAppRenderer,
}

impl WhatsAppChannel {
    /// Create a new `WhatsApp` channel adapter
    #[must_use]
    pub fn new(webhook_secret: String) -> Self {
        Self {
            transport: WhatsAppTransport::new(webhook_secret),
            renderer: WhatsAppRenderer,
        }
    }
}

/// `WhatsApp` channel metadata descriptor
pub struct WhatsAppDescriptor;

impl ChannelDescriptor for WhatsAppDescriptor {
    fn name(&self) -> &'static str {
        "whatsapp"
    }
    fn display_name(&self) -> &'static str {
        "WhatsApp"
    }
    fn channel_type(&self) -> ChannelType {
        ChannelType::WhatsApp
    }
    fn webhook_path(&self) -> &'static str {
        "/api/messaging/webhook/whatsapp"
    }
    fn supports_media(&self) -> bool {
        true
    }
    fn max_message_length(&self) -> usize {
        4096
    }
    fn signature_header(&self) -> &'static str {
        "x-hub-signature-256"
    }
}

#[async_trait]
impl MessagingChannel for WhatsAppChannel {
    fn channel_type(&self) -> ChannelType {
        ChannelType::WhatsApp
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
