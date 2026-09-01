// ABOUTME: A real channel adapter with only its outbound sends captured, for route-driven tests
// ABOUTME: Keeps production inbound parsing while removing the live POST that made CI runs flaky
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

//! Offline outbound for tests that drive the real webhook route.
//!
//! A test that posts to `/api/messaging/webhook/{channel}` gets whatever
//! adapter the tenant's stored config builds — in production a live transport.
//! `group_transcript_test` seeded a fixture bot token and therefore attempted a
//! real POST to `api.telegram.org` on every turn: answered `401` in ~100 ms on a
//! developer's machine, and left hanging on a CI runner that cannot reach
//! Telegram, which on 2026-09-01 consumed the whole 10 s budget those tests wait
//! on (run 33462729445).
//!
//! Wrapping rather than replacing the adapter is the point. Signature
//! verification, payload parsing and rendering stay the production ones, so the
//! test still exercises our ingress; only the egress is removed.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use http::HeaderMap;
use pierre_core::models::messaging::{
    ChannelConfig, ChannelType, DeliveryReceipt, DeliveryStatus, IncomingMessage, OutgoingMessage,
};
use pierre_mcp_server::routes::messaging::adapter_factory::ChannelAdapterFactory;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::error::MessagingResult;
use pierre_messaging::factory::create_adapter_from_config;
use pierre_messaging::turn::ConversationTurnId;
use serde_json::Value;

/// A real channel adapter with its outbound sends captured instead of sent.
///
/// Inbound is delegated verbatim — signature verification, payload parsing and
/// rendering all run the production adapter — so a route-driven test still
/// exercises our ingress rather than a stand-in for it. Only `send`/`send_raw`
/// are intercepted, because those are the only methods that leave the process.
///
/// This exists because `group_transcript_test` seeds a fixture bot token and
/// drives the real Telegram webhook route, so every turn attempted a live POST
/// to `api.telegram.org`. A developer's machine gets a 401 in ~100 ms and the
/// turn finishes; a CI runner that cannot reach Telegram hangs instead, and on
/// 2026-09-01 that consumed the entire 10 s budget the transcript tests wait on.
/// The tests were passing or failing on a third party's reachability.
pub struct OfflineSendChannel {
    /// The production adapter every inbound method delegates to.
    inner: Arc<dyn MessagingChannel>,
    /// Shared with the factory that built this adapter, so a test holding the
    /// factory reads every send across every adapter the ingress constructed.
    sent: SendLog,
}

/// Outbound messages captured in place of delivery, newest last.
pub type SendLog = Arc<Mutex<Vec<OutgoingMessage>>>;

impl OfflineSendChannel {
    /// Wrap a real adapter, capturing its outbound sends into `sent`.
    pub fn wrapping(inner: Arc<dyn MessagingChannel>, sent: SendLog) -> Self {
        Self { inner, sent }
    }
}

#[async_trait]
impl MessagingChannel for OfflineSendChannel {
    fn channel_type(&self) -> ChannelType {
        self.inner.channel_type()
    }

    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()> {
        self.inner.verify_signature(headers, body)
    }

    async fn receive(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>> {
        self.inner.receive(headers, body).await
    }

    fn render(&self, msg: &OutgoingMessage) -> MessagingResult<Value> {
        self.inner.render(msg)
    }

    async fn send(
        &self,
        msg: &OutgoingMessage,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        self.sent.lock().unwrap().push(msg.clone());
        Ok(DeliveryReceipt {
            message_id: "offline-send".to_owned(),
            channel_message_id: None,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
            turn_id: msg.turn_id,
        })
    }

    async fn send_raw(
        &self,
        _payload: &Value,
        turn_id: ConversationTurnId,
        _config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt> {
        Ok(DeliveryReceipt {
            message_id: "offline-send".to_owned(),
            channel_message_id: None,
            status: DeliveryStatus::Sent,
            timestamp: Utc::now(),
            turn_id,
        })
    }
}

/// A [`ChannelAdapterFactory`] that builds the production adapter and wraps it
/// so its sends stay in the process.
///
/// [`ChannelAdapterFactory`]: pierre_mcp_server::routes::messaging::adapter_factory::ChannelAdapterFactory
/// Holds the shared [`SendLog`] every adapter it builds writes into, so a test
/// can still assert the turn attempted delivery — the coverage the live
/// transport used to carry implicitly, and which taking the network out would
/// otherwise remove silently.
#[derive(Default)]
pub struct OfflineSendAdapters {
    sent: SendLog,
}

impl OfflineSendAdapters {
    /// Every outbound message the ingress handed to a channel, in order.
    pub fn sends(&self) -> Vec<OutgoingMessage> {
        self.sent.lock().unwrap().clone()
    }
}

impl ChannelAdapterFactory for OfflineSendAdapters {
    fn build(
        &self,
        channel_type: ChannelType,
        config: &Value,
    ) -> Option<Arc<dyn MessagingChannel>> {
        let inner = create_adapter_from_config(channel_type, config).ok()?;
        Some(Arc::new(OfflineSendChannel::wrapping(
            inner,
            Arc::clone(&self.sent),
        )))
    }
}
