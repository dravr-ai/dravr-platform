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

use crate::config::text_guardrails::{GuardrailOutcome, GuardrailRejection, TextGuardrails};

/// Apply the safe-default text guardrails to an assistant reply.
///
/// Returns the (possibly disclaimer-prepended) reply, or a graceful
/// fallback string when guardrails reject the response.
pub fn apply_text_guardrails(reply: &str) -> String {
    let rules = TextGuardrails::safe_default();
    match rules.apply(reply) {
        GuardrailOutcome::Allowed(text) => text,
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { length, cap }) => {
            tracing::warn!(
                length,
                cap,
                "guardrails: trimming over-long response to safe fallback"
            );
            "I have a longer response prepared but it exceeds the configured length cap. \
             Want me to break it into a shorter summary?"
                .to_owned()
        }
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            tracing::warn!(topic, "guardrails: blocked topic in coach response");
            "I'd rather not get into that here. Let's stay focused on your training and \
             recovery. Is there something specific I can help with?"
                .to_owned()
        }
    }
}
