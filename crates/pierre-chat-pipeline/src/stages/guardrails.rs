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
/// which is a policy about how much an agent may say. The surface's transport
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

#[cfg(test)]
mod tests {
    //! Where the transport ceiling is — and is not — applied.
    //!
    //! canot's send path never consults `ChannelDescriptor::max_message_length`:
    //! it hands the message to the channel API, which rejects an over-length body
    //! outright. That used to be answered here, by trimming the reply to the
    //! surface's `max_reply_chars` — which turned a dropped reply into a prefix
    //! that stopped mid-thought.
    //!
    //! It is answered at the egress now, by splitting the reply into ordered
    //! messages ([`pierre_core::chunking::chunk_reply`]). For that to work the
    //! pipeline has to hand the egress the WHOLE reply, so these tests pin that
    //! the guardrails stage no longer knows the transport ceiling exists.

    use std::sync::Arc;

    use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use pierre_core::chunking::chunk_reply;

    use super::apply_text_guardrails;

    fn registries() -> (Arc<HarnessConfigRegistry>, Arc<MessagingStringsRegistry>) {
        (
            Arc::new(HarnessConfigRegistry::bootstrap()),
            Arc::new(MessagingStringsRegistry::new()),
        )
    }

    /// A reply past Discord's 2000-character message ceiling leaves the stage
    /// whole. Trimming it here would delete the tail before any chunker could
    /// deliver it.
    #[test]
    fn a_reply_past_a_transport_ceiling_leaves_the_pipeline_whole() {
        let (harness, strings) = registries();
        // 20 sentences of 100 characters: past Discord's 2000, inside the
        // default 5000-character admin guardrail, so only a transport trim could
        // explain a shortened result.
        let sentence = format!("{}. ", "a".repeat(98));
        let reply = sentence.repeat(20);

        let out = apply_text_guardrails(&harness, &strings, &reply, "en");

        assert_eq!(
            out.chars().count(),
            2000,
            "the reply must reach the egress at its full length"
        );
        assert_eq!(out, reply, "and byte-identical");
    }

    /// The same reply, once the egress has it: several messages, all inside
    /// Discord's ceiling, nothing lost.
    #[test]
    fn the_egress_splits_what_the_pipeline_handed_it_whole() {
        let (harness, strings) = registries();
        let sentence = format!("{}. ", "a".repeat(98));
        let reply = sentence.repeat(20);

        let out = apply_text_guardrails(&harness, &strings, &reply, "en");
        let messages = chunk_reply(&out, 500);

        assert_eq!(messages.len(), 4, "2000 characters at 500 per message");
        for message in &messages {
            assert!(
                message.chars().count() <= 500,
                "every message must fit Discord's ceiling, got {}",
                message.chars().count()
            );
        }
        let rejoined: String = messages
            .concat()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let original: String = reply.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(
            rejoined, original,
            "every character survives the split, in order"
        );
    }

    #[test]
    fn a_short_reply_is_untouched() {
        let (harness, strings) = registries();
        let reply = "Nice ride. Your load is trending up.";

        let out = apply_text_guardrails(&harness, &strings, reply, "en");

        assert_eq!(out, reply);
    }

    /// The admin's `max_response_chars` policy is a different ceiling and still
    /// bites: it is a rule about how much an agent may say, not about what a wire
    /// will carry.
    #[test]
    fn the_admin_length_policy_still_trims() {
        let (harness, strings) = registries();
        let cap = harness.current_guardrails().max_response_chars;
        let reply = "z".repeat(cap + 500);

        let out = apply_text_guardrails(&harness, &strings, &reply, "en");

        assert_eq!(
            out.chars().count(),
            cap,
            "the admin cap is policy and outlives the transport trim"
        );
    }
}
