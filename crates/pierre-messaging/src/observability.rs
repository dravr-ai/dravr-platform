// ABOUTME: Observability helpers for structured tracing spans in messaging operations
// ABOUTME: Creates consistent spans for webhook processing, send attempts, and signature checks
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::messaging::ChannelType;
use tracing::{info_span, Span};
use uuid::Uuid;

/// Create a tracing span for an inbound webhook processing pipeline
#[must_use]
pub fn webhook_received_span(channel: ChannelType, correlation_id: Uuid) -> Span {
    info_span!(
        "messaging.webhook.received",
        channel = %channel,
        correlation_id = %correlation_id,
    )
}

/// Create a tracing span for an outbound send attempt
#[must_use]
pub fn send_attempt_span(channel: ChannelType, correlation_id: Uuid, attempt: i32) -> Span {
    info_span!(
        "messaging.send.attempt",
        channel = %channel,
        correlation_id = %correlation_id,
        attempt = attempt,
    )
}

/// Create a tracing span for webhook signature verification
#[must_use]
pub fn signature_verification_span(channel: ChannelType) -> Span {
    info_span!(
        "messaging.signature.verify",
        channel = %channel,
    )
}
