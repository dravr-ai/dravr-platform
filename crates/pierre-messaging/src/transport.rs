// ABOUTME: Transport adapter trait for channel-specific webhook ingress and outbound HTTP
// ABOUTME: Handles raw wire protocol: signature verification, payload parsing, HTTP sending
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::time::Duration;

use async_trait::async_trait;
use http::HeaderMap;
use pierre_core::errors::messaging::MessagingResult;
use pierre_core::models::messaging::{ChannelConfig, DeliveryReceipt, IncomingMessage};
use serde_json::Value;

/// Default timeout in seconds for outbound HTTP requests to channel APIs
const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;

/// Resolve the outbound HTTP timeout from `MESSAGING_HTTP_TIMEOUT_SECS` env var,
/// falling back to 30 seconds if unset or unparseable
#[must_use]
pub fn outbound_http_timeout() -> Duration {
    let secs = env::var("MESSAGING_HTTP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Low-level transport adapter for channel wire protocols
///
/// Each channel platform has unique webhook formats, signature schemes,
/// and outbound API conventions. This trait abstracts those differences.
#[async_trait]
pub trait TransportAdapter: Send + Sync {
    /// Verify the webhook signature using channel-specific cryptography
    ///
    /// Implementations use constant-time comparison to prevent timing attacks:
    /// - `WhatsApp`/Messenger/Slack: HMAC-SHA256
    /// - Discord: Ed25519
    /// - Telegram: Secret token header
    ///
    /// # Errors
    ///
    /// Returns `MessagingError::SignatureVerificationFailed` on mismatch.
    fn verify_signature(&self, headers: &HeaderMap, body: &[u8]) -> MessagingResult<()>;

    /// Parse the raw webhook body into normalized `IncomingMessage` structs
    ///
    /// # Errors
    ///
    /// Returns `MessagingError::InvalidPayload` if the body cannot be parsed.
    async fn parse_inbound(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> MessagingResult<Vec<IncomingMessage>>;

    /// Send a pre-rendered payload to the channel API
    ///
    /// # Errors
    ///
    /// Returns `MessagingError::DeliveryFailed` or `MessagingError::ChannelApiError`.
    async fn send_raw(
        &self,
        payload: &Value,
        config: &ChannelConfig,
    ) -> MessagingResult<DeliveryReceipt>;
}
