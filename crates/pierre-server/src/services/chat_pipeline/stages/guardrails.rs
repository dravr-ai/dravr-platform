// ABOUTME: Tier 6 text guardrails stage — disclaimer prepending, blocked-topic rejection, length caps
// ABOUTME: Extracted from services/chat_orchestration.rs::apply_text_guardrails (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 6 text guardrails.
//!
//! Post-processes the assistant reply content:
//!
//! - Prepends a safety disclaimer when a guardrail trigger is detected in
//!   the reply.
//! - Rejects replies that exceed the configured length ceiling with a
//!   graceful fallback message.
//! - Rejects replies that reference blocked topics.
//!
//! Uses a safe default profile; per-tenant overrides are a pending Tier 6
//! admin GUI follow-up.

use std::sync::Arc;

use crate::config::text_guardrails::{GuardrailOutcome, GuardrailRejection, TextGuardrails};
use crate::contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_GUARDRAIL_BLOCKED_TOPIC, KEY_GUARDRAIL_TOO_LONG,
};
use crate::mcp::resources::ServerContext;

/// Apply the safe-default text guardrails to an assistant reply.
///
/// Returns the (possibly disclaimer-prepended) reply, or a graceful
/// fallback string from the `messaging_strings_registry` when guardrails
/// reject the response. `locale` is the BCP-47 short code resolved upstream;
/// `None` triggers the registry's `DEFAULT_LOCALE` fallback.
pub fn apply_text_guardrails(
    resources: &Arc<ServerContext>,
    reply: &str,
    locale: Option<&str>,
) -> String {
    let locale = locale.unwrap_or(DEFAULT_LOCALE);
    let rules = TextGuardrails::safe_default();
    match rules.apply(reply) {
        GuardrailOutcome::Allowed(text) => text,
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { length, cap }) => {
            tracing::warn!(
                length,
                cap,
                "guardrails: trimming over-long response to safe fallback"
            );
            resources
                .messaging_strings_registry
                .get(KEY_GUARDRAIL_TOO_LONG, locale)
        }
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            tracing::warn!(topic, "guardrails: blocked topic in coach response");
            resources
                .messaging_strings_registry
                .get(KEY_GUARDRAIL_BLOCKED_TOPIC, locale)
        }
    }
}
