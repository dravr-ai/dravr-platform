// ABOUTME: Centralized error-to-channel-reply sanitizer — the single funnel for turning AppError into text
// ABOUTME: Extension trait on AppError so every messaging site shares one sanitize-and-correlate code path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Channel error replies
//!
//! REST responses go through `impl IntoResponse for AppError`, which is a
//! single, type-enforced choke point that calls [`AppError::sanitized_message`]
//! before sending any error to the client. Messaging channels (Telegram,
//! Slack, Discord, `WhatsApp`, Messenger) and the MCP tool layer don't have
//! that shape: a reply is a `String` side-effect, not a function return value,
//! so nothing at the type level stops a handler from writing `format!("DB:
//! {e}")` and leaking schema detail.
//!
//! This module is the replacement for that missing type-enforcement. Every
//! code path that turns an [`AppError`] into a user-visible channel message
//! **must** call [`ChannelErrorReply::to_channel_reply`]. A grep gate in
//! `scripts/ci/architectural-validation.sh` forbids raw `format!("…{e}")`
//! interpolation into `MessageContent::Text { body: … }` so new error sites
//! can't reintroduce the leak.
//!
//! ## Behavior
//!
//! - [`ErrorCode::InvalidInput`] / `MissingRequiredField` / `InvalidFormat`
//!   / `ValueOutOfRange` / `RateLimitExceeded` / `QuotaExceeded` / auth-invalid
//!   JWT messages: passed through as the user message (the full rationale is
//!   already safe to display; see [`AppError::sanitized_message`]).
//! - Everything else (database / internal / external-service / auth): replaced
//!   with the locale-specific `messaging.error.generic` template from the
//!   strings registry, preserving the user-facing tone per channel locale.
//! - A short correlation id (first 8 hex of a `Uuid::new_v4()`) is emitted in
//!   both the log entry and the user-visible message so on-call can grep the
//!   server log for the full error chain without needing conversation IDs.

use pierre_core::errors::AppError;
use tracing::warn;
use uuid::Uuid;

use crate::contremaitre::messaging_strings::{format_template, KEY_ERROR_GENERIC};
use crate::mcp::resources::ServerResources;

/// Extension trait for turning an [`AppError`] into a channel-safe reply.
///
/// Implemented on `AppError` so every messaging callsite writes
/// `e.to_channel_reply(...)` — the grep gate forbids any other way of
/// interpolating an error into a channel message.
pub trait ChannelErrorReply {
    /// Build a channel-safe reply body plus the correlation id the log
    /// entry carried.
    ///
    /// The body is what goes into `MessageContent::Text { body }`; the id
    /// is returned separately so callers can include it in analytics or
    /// cross-reference with upstream logs if needed.
    fn to_channel_reply(
        &self,
        resources: &ServerResources,
        locale: &str,
        scope: &'static str,
    ) -> (String, Uuid);
}

impl ChannelErrorReply for AppError {
    fn to_channel_reply(
        &self,
        resources: &ServerResources,
        locale: &str,
        scope: &'static str,
    ) -> (String, Uuid) {
        let correlation_id = Uuid::new_v4();
        warn!(
            scope = scope,
            error = %self,
            correlation_id = %correlation_id,
            "channel reply surfaced an error"
        );

        let sanitized = self.sanitized_message();
        // Fall back to the locale-specific generic template when the
        // sanitized message equals the code's description — that means the
        // raw error is unsafe to expose, so substitute the translated
        // template with the correlation id baked in.
        let body = if sanitized == self.code.description() {
            let template = resources
                .messaging_strings_registry
                .get(KEY_ERROR_GENERIC, locale);
            let short_id = correlation_id.to_string()[..8].to_owned();
            format_template(&template, &[&short_id])
        } else {
            // Safe-to-expose category (validation / quota / rate-limit):
            // surface the handler's message but still append the short id
            // so user reports can be traced.
            let short_id = correlation_id.to_string()[..8].to_owned();
            format!("{sanitized} (ref: {short_id})")
        };

        (body, correlation_id)
    }
}
