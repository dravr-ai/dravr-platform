// ABOUTME: Tier 6 text guardrails stage — disclaimer prepending and blocked-topic rejection
// ABOUTME: Provides apply_text_guardrails (post-LLM) and answer_off_scope (the pre-LLM scope rails)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 6 text guardrails.
//!
//! Post-processes the assistant reply content:
//!
//! - Prepends a safety disclaimer when a guardrail trigger is detected in
//!   the reply.
//! - Rejects replies that reference blocked topics.
//!
//! And, before the model runs, answers the turn itself when the athlete's
//! message trips one of the guard's scope rails — see [`answer_off_scope`].
//!
//! Reads the active rules from
//! [`pierre_contremaitre::harness_config_registry::HarnessConfigRegistry`], so the
//! blocked-topic list and trigger keywords reflect whatever the admin most recently saved via
//! `PUT /admin/settings/harness`.

use std::sync::Arc;

use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_GUARDRAIL_BLOCKED_TOPIC, KEY_GUARDRAIL_DISCLAIMER,
    KEY_GUARDRAIL_MEDICAL_EMERGENCY, KEY_SCOPE_REFUSAL,
};
use pierre_contremaitre::text_guardrails::scope_rails::ScopeRail;
use pierre_contremaitre::text_guardrails::{GuardrailOutcome, GuardrailRejection};
use pierre_core::errors::AppResult;

use super::deterministic_reply::{
    deliver, DeterministicReplyInputs, PlatformReply, PLATFORM_REPLY_TRANSCRIPT_MARKER,
};
use super::group_subject::TurnStanding;
use super::onboarding::GuidedTarget;
use super::turn_extraction::{spawn_turn_extraction, ExtractedTurn};
use crate::envelope::TurnEnvelope;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;
use pierre_database::database::{ConversationRecord, MessageRecord};

/// The platform's own answer to a message that tripped a scope rail, in the
/// turn's locale. `None` when no rail fired — or when the copy is missing from
/// the registry, since an empty platform reply is worse than the model's.
fn scope_rail_reply(
    harness_config_registry: &Arc<HarnessConfigRegistry>,
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    message: &str,
    locale: &str,
) -> Option<(ScopeRail, String)> {
    let rail = harness_config_registry
        .current_guardrails()
        .classify_request(message)?;
    let key = match rail {
        ScopeRail::MedicalEmergency => KEY_GUARDRAIL_MEDICAL_EMERGENCY,
        // The same canonical sentence the prompt tells the model to emit for
        // an off-scope ask, so the athlete reads one refusal either way.
        ScopeRail::FinancialAdvice | ScopeRail::PriceLookup => KEY_SCOPE_REFUSAL,
    };
    let reply = messaging_strings_registry.get(key, locale);
    if reply.trim().is_empty() {
        tracing::error!(
            rail = rail.label(),
            key,
            locale,
            "scope rail fired but its copy is missing; the model answers instead"
        );
        return None;
    }
    Some((rail, reply))
}

/// The turn so far, as [`answer_off_scope`] reads it.
pub(crate) struct OffScopeTurn<'a> {
    /// Shared pipeline context.
    pub ctx: &'a ChatPipelineContext,
    /// The turn being answered.
    pub input: &'a TurnInput,
    /// What the surface can render, and the turn's locale.
    pub profile: &'a SurfaceProfile,
    /// Model the turn resolved to, recorded on the persisted row.
    pub active_model: &'a str,
    /// The already-persisted athlete message.
    pub user_message: &'a MessageRecord,
    /// Conversation the turn belongs to.
    pub conv: &'a ConversationRecord,
    /// Who the turn is about, and so whether the message is filed on the
    /// sender.
    pub standing: TurnStanding,
    /// The guided-walk topic the message answers, if the athlete is mid walk,
    /// so the extracted fact keeps its provenance.
    pub guided_answer: Option<GuidedTarget>,
}

/// Stage 4.6: answer the turn with platform copy when the athlete's message
/// trips a scope rail, skipping the model entirely.
///
/// The rails are the input side of the Tier 6 guard (see
/// [`pierre_contremaitre::text_guardrails::scope_rails`]): acute red-flag
/// symptoms get the emergency redirect, investment advice and retail price
/// lookups get the canonical scope refusal. A model — the 3B CI model
/// especially (carnet#819) — answers these wrong often enough that the answer
/// cannot be left to it.
///
/// The athlete's message is still extracted, exactly as on the guided
/// wrap-up: chest pain reported to the coach is a fact worth keeping whoever
/// wrote the reply.
///
/// # Errors
///
/// Returns the persistence error when the platform reply cannot be written.
pub(crate) async fn answer_off_scope(turn: OffScopeTurn<'_>) -> AppResult<Option<TurnEnvelope>> {
    let OffScopeTurn {
        ctx,
        input,
        profile,
        active_model,
        user_message,
        conv,
        standing,
        guided_answer,
    } = turn;
    let Some((rail, reply)) = scope_rail_reply(
        &ctx.harness_config_registry,
        &ctx.messaging_strings_registry,
        &input.content,
        &profile.locale,
    ) else {
        return Ok(None);
    };
    tracing::info!(
        rail = rail.label(),
        tenant_id = %input.conversation_tenant_id,
        "scope rail answered the turn without the model"
    );
    // Owned copies only on the rare turn a rail fires, never on every turn.
    let inputs = DeterministicReplyInputs {
        ctx,
        input,
        profile,
        active_model: active_model.to_owned(),
        user_message: user_message.clone(),
        conv,
    };
    // A refusal or an emergency redirect offers nothing to tap: the next move
    // is the athlete's own, or a call to emergency services.
    let result = deliver(inputs, PlatformReply::text_only(reply)).await?;
    spawn_turn_extraction(ExtractedTurn {
        ctx,
        input,
        conv,
        standing,
        assistant_reply: PLATFORM_REPLY_TRANSCRIPT_MARKER,
        assistant_message_id: &result.assistant.message.id,
        answered: guided_answer,
        plan_was_saved: false,
    })
    .await;
    Ok(Some(result))
}

/// Apply the live admin-configured text guardrails to an assistant reply.
///
/// Returns the (possibly disclaimer-prepended) reply, or a graceful
/// fallback string from the `messaging_strings_registry` when guardrails
/// reject the response.
///
/// No length ceiling is in force here: a long reply is split into ordered
/// messages at the egress ([`pierre_core::chunking::chunk_reply`]), so the athlete
/// reads the whole answer instead of a prefix that stops mid-thought, and every
/// rule below sees the whole reply.
///
/// `locale` is the BCP-47 short code resolved once at the ingress boundary.
pub(crate) fn apply_text_guardrails(
    harness_config_registry: &Arc<HarnessConfigRegistry>,
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    reply: &str,
    locale: &str,
) -> String {
    let rules = harness_config_registry.current_guardrails();
    match rules.apply(reply, locale) {
        GuardrailOutcome::Allowed(text) => text,
        GuardrailOutcome::NeedsDisclaimer {
            locale: disclaimer_locale,
        } => with_disclaimer(messaging_strings_registry, reply, &disclaimer_locale),
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            tracing::warn!(topic, "guardrails: blocked topic in coach response");
            messaging_strings_registry.get(KEY_GUARDRAIL_BLOCKED_TOPIC, locale)
        }
    }
}

/// `reply` under the catalogue's disclaimer in `locale`. Missing copy leaves the
/// reply as written and logs at `error!`: a blank line above a reply protects no
/// one, and the reply is still the athlete's answer.
fn with_disclaimer(
    messaging_strings_registry: &MessagingStringsRegistry,
    reply: &str,
    locale: &str,
) -> String {
    let disclaimer = messaging_strings_registry.get(KEY_GUARDRAIL_DISCLAIMER, locale);
    if disclaimer.trim().is_empty() {
        tracing::error!(
            key = KEY_GUARDRAIL_DISCLAIMER,
            locale,
            "guardrails: a disclaimer trigger fired but its copy is missing"
        );
        return reply.to_owned();
    }
    tracing::info!(locale, "guardrails: disclaimer prepended");
    format!("{disclaimer}\n\n{reply}")
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

    use pierre_contremaitre::harness_config_document::HarnessConfigDocument;
    use pierre_contremaitre::harness_config_registry::{
        HarnessConfigRegistry, HarnessConfigSource,
    };
    use pierre_contremaitre::messaging_strings::{
        MessagingStringsRegistry, KEY_GUARDRAIL_BLOCKED_TOPIC, KEY_GUARDRAIL_DISCLAIMER,
    };
    use pierre_core::chunking::chunk_reply;
    use pierre_core::models::SUPPORTED_LOCALES;

    use super::apply_text_guardrails;

    const EN_COPY: &str = "EN copy from the catalogue.";
    const FR_COPY: &str = "Copie FR du catalogue.";

    /// The registries a turn reads, with the disclaimer copy overlaid the way a
    /// contremaitre sync delivers it.
    fn registries() -> (Arc<HarnessConfigRegistry>, Arc<MessagingStringsRegistry>) {
        let strings = MessagingStringsRegistry::new();
        for (locale, copy) in [("en", EN_COPY), ("fr", FR_COPY)] {
            let bundle = format!(r#"{{"messaging":{{"guardrail":{{"disclaimer":"{copy}"}}}}}}"#);
            strings
                .apply_bundle(locale, &bundle, "test-bundle")
                .expect("the test bundle parses");
        }
        (
            Arc::new(HarnessConfigRegistry::bootstrap()),
            Arc::new(strings),
        )
    }

    /// The shipped catalogue carries the copy in every locale, so a triggered reply
    /// is never delivered bare. The key is authored in contremaitre and reaches this
    /// tree with the pin, so this needs the pin that carries it.
    #[test]
    fn the_catalogue_carries_the_disclaimer_in_every_supported_locale() {
        let strings = MessagingStringsRegistry::new();
        for locale in SUPPORTED_LOCALES {
            assert!(
                !strings
                    .get(KEY_GUARDRAIL_DISCLAIMER, locale)
                    .trim()
                    .is_empty(),
                "no disclaimer copy for {locale}"
            );
        }
    }

    #[test]
    fn a_triggered_reply_reads_the_catalogue_copy_in_its_own_language() {
        let (harness, strings) = registries();

        let english = "See a doctor about that knee.";
        assert_eq!(
            apply_text_guardrails(&harness, &strings, english, "en"),
            format!("{EN_COPY}\n\n{english}")
        );
        let french = "Si la douleur persiste, consulte un médecin.";
        assert_eq!(
            apply_text_guardrails(&harness, &strings, french, "fr"),
            format!("{FR_COPY}\n\n{french}")
        );
    }

    /// French bread stays bread, on the path athletes actually take.
    #[test]
    fn an_untriggered_reply_carries_no_disclaimer() {
        let (harness, strings) = registries();
        let reply = "Souper : pâtes, parmesan, pain, salade.";

        assert_eq!(
            apply_text_guardrails(&harness, &strings, reply, "fr"),
            reply
        );
    }

    /// The trigger list falls back to English, and the copy must follow it. The
    /// catalogue's own fallback is French, which would put a French disclaimer on a
    /// reply matched by English words.
    #[test]
    fn a_locale_with_no_triggers_gets_the_english_rule_and_the_english_copy() {
        let (harness, strings) = registries();
        let reply = "Persistent ankle pain after training.";

        assert_eq!(
            apply_text_guardrails(&harness, &strings, reply, "it"),
            format!("{EN_COPY}\n\n{reply}")
        );
    }

    #[test]
    fn missing_copy_delivers_the_reply_as_written_not_a_blank_prefix() {
        let harness = Arc::new(HarnessConfigRegistry::bootstrap());
        let strings = Arc::new(MessagingStringsRegistry::new());
        strings
            .apply_bundle(
                "en",
                r#"{"messaging":{"guardrail":{"disclaimer":"  "}}}"#,
                "test-bundle",
            )
            .expect("the test bundle parses");
        let reply = "See a doctor about that knee.";

        assert_eq!(
            apply_text_guardrails(&harness, &strings, reply, "en"),
            reply
        );
    }

    /// A reply past Discord's 2000-character message ceiling leaves the stage
    /// whole. Trimming it here would delete the tail before any chunker could
    /// deliver it.
    #[test]
    fn a_reply_past_a_transport_ceiling_leaves_the_pipeline_whole() {
        let (harness, strings) = registries();
        // 20 sentences of 100 characters: past Discord's 2000, so a transport
        // trim would shorten the result.
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

    /// A reply well past the 5,000 characters that used to be cut reaches the
    /// egress byte for byte: the policy is to deliver whole.
    #[test]
    fn the_whole_reply_is_delivered_however_long() {
        let (harness, strings) = registries();
        let reply = "z".repeat(50_000);

        let out = apply_text_guardrails(&harness, &strings, &reply, "en");

        assert_eq!(out, reply, "nothing is cut at any length");
    }

    /// The length check used to return first, so an over-cap reply skipped both of
    /// these and went out as a raw prefix with no disclaimer.
    #[test]
    fn a_long_reply_still_gets_its_medical_disclaimer() {
        let (harness, strings) = registries();
        let reply = format!("{} See a doctor about that knee.", "a".repeat(6_000));

        let out = apply_text_guardrails(&harness, &strings, &reply, "en");

        assert_eq!(
            out,
            format!("{EN_COPY}\n\n{reply}"),
            "the reply stays whole, under its disclaimer"
        );
    }

    #[test]
    fn a_long_reply_with_a_blocked_topic_is_still_refused() {
        let (harness, strings) = registries();
        let mut document = HarnessConfigDocument::default();
        document.guardrails.blocked_topics = vec!["forbidden phrase".to_owned()];
        harness.install(document, HarnessConfigSource::AdminUpdate);
        let reply = format!("{} forbidden phrase", "a".repeat(6_000));

        let out = apply_text_guardrails(&harness, &strings, &reply, "en");

        assert!(
            !out.contains("forbidden phrase"),
            "the phrase must not reach the athlete"
        );
        assert_eq!(
            out,
            strings.get(KEY_GUARDRAIL_BLOCKED_TOPIC, "en"),
            "the refusal replaces the reply"
        );
    }
}
