// ABOUTME: Tier 6 text guardrails stage — disclaimer prepending, blocked-topic rejection, length caps
// ABOUTME: Provides apply_text_guardrails — post-LLM sanitizer for assistant reply text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 6 text guardrails.
//!
//! Post-processes the assistant reply content:
//!
//! - Prepends a safety disclaimer when a guardrail trigger is detected in
//!   the reply.
//! - Trims replies that exceed the admin's configured length ceiling to its
//!   leading characters rather than a canned placeholder.
//! - Rejects replies that reference blocked topics.
//!
//! Reads the active rules from
//! [`pierre_contremaitre::harness_config_registry::HarnessConfigRegistry`], so the
//! disclaimer text, blocked-topic list, length cap, and trigger keywords
//! reflect whatever the admin most recently saved via
//! `PUT /admin/settings/harness`.

use std::sync::Arc;

use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_GUARDRAIL_BLOCKED_TOPIC,
};
use pierre_contremaitre::text_guardrails::{GuardrailOutcome, GuardrailRejection};

/// Apply the live admin-configured text guardrails to an assistant reply.
///
/// Returns the (possibly disclaimer-prepended) reply, or a graceful
/// fallback string from the `messaging_strings_registry` when guardrails
/// reject the response.
///
/// One ceiling is in force here: the admin's `max_response_chars` guardrail,
/// which is a policy about how much a coach may say. The surface's transport
/// ceiling is *not* applied here — an over-limit reply is split into ordered
/// messages at the egress ([`pierre_core::chunking::chunk_reply`]), so the
/// athlete reads the whole answer instead of a prefix that stops mid-thought.
///
/// `locale` is the BCP-47 short code resolved once at the ingress boundary.
pub fn apply_text_guardrails(
    harness_config_registry: &Arc<HarnessConfigRegistry>,
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    reply: &str,
    locale: &str,
) -> String {
    let rules = harness_config_registry.current_guardrails();
    match rules.apply(reply, locale) {
        GuardrailOutcome::Allowed(text) => text,
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { length, cap }) => {
            // Surface the full content the model produced (everything received)
            // so over-cap responses are inspectable, then return the leading
            // `cap` characters rather than discarding it to a canned
            // placeholder — the user always sees the real content prefix.
            tracing::warn!(
                length,
                cap,
                full_reply = %reply,
                "guardrails: response over cap; returning first chunk"
            );
            reply.chars().take(cap).collect()
        }
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            tracing::warn!(topic, "guardrails: blocked topic in coach response");
            messaging_strings_registry.get(KEY_GUARDRAIL_BLOCKED_TOPIC, locale)
        }
    }
}
