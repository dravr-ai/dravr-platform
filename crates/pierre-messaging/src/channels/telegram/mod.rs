// ABOUTME: Telegram Bot API channel adapter module
// ABOUTME: Combines TelegramTransport (secret token) with TelegramRenderer (HTML parse mode)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// HTML parse mode message renderer for Telegram
pub mod renderer;
/// Secret token verification and webhook parsing for Telegram
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

use self::renderer::TelegramRenderer;
use self::transport::TelegramTransport;

/// Telegram channel adapter combining transport and renderer
pub struct TelegramChannel {
    /// Wire protocol adapter for Telegram Bot API
    transport: TelegramTransport,
    /// HTML-based message renderer
    renderer: TelegramRenderer,
}

impl TelegramChannel {
    /// Create a new Telegram channel adapter
    #[must_use]
    pub fn new(webhook_secret: String) -> Self {
        Self {
            transport: TelegramTransport::new(webhook_secret),
            renderer: TelegramRenderer,
        }
    }
}

/// Telegram channel metadata descriptor
pub struct TelegramDescriptor;

impl ChannelDescriptor for TelegramDescriptor {
    fn name(&self) -> &'static str {
        "telegram"
    }
    fn display_name(&self) -> &'static str {
        "Telegram"
    }
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }
    fn webhook_path(&self) -> &'static str {
        "/api/messaging/webhook/telegram"
    }
    fn supports_media(&self) -> bool {
        true
    }
    fn max_message_length(&self) -> usize {
        4096
    }
    fn signature_header(&self) -> &'static str {
        "x-telegram-bot-api-secret-token"
    }
}

#[async_trait]
impl MessagingChannel for TelegramChannel {
    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
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
