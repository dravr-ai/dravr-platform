// ABOUTME: Tier 5.5 claim verification stage — runs the bullshit detector pipeline over the assistant reply
// ABOUTME: Extracted from services/chat_orchestration.rs::apply_claim_verification (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tier 5.5 claim verification.
//!
//! Runs the `pierre_evals` heuristic pipeline over the assistant reply to
//! detect unsupported claims, persists verdicts for the admin dashboard,
//! and reacts per the coach's `VerificationConfig` (parsed from YAML
//! frontmatter on the coach's system prompt). Pure-Rust in Phase A, no
//! measurable latency.
//!
//! Gated behind `#[cfg(feature = "tools-verification")]` because the
//! evidence corpus and `pierre_evals` dependency are not built in every
//! workspace configuration.

use std::sync::Arc;

use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::{ExtractedClaim, VerdictOutcome, VerificationConfig, VerificationFallback};
use pierre_memory::claims::ClaimStatus;

use crate::contremaitre::messaging_strings::{
    format_template, DEFAULT_LOCALE, KEY_VERIFICATION_BLOCK_FALLBACK, KEY_VERIFICATION_WARN_SUFFIX,
};
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::services::claim_verification::{resolve_corpus, verify_reply_with_config_and_corpus};

/// Localizes the verification warn / block-fallback strings.
///
/// The Tier 5.5 banner is appended verbatim to the LLM's reply, so the
/// banner's language must match the reply's language — otherwise an
/// English session ends with a French postscript (or vice versa).
///
/// Resolution order (returns first match):
/// 1. **Reply text via whatlang** — long replies (≥ a few sentences)
///    detect reliably even for casual conversational tone, which is the
///    case the per-turn locale heuristic in `messaging_ingress` misses
///    (a 4-word user question can't be detected, but the 200-word reply
///    can).
/// 2. **Caller-supplied `locale`** — usually the per-turn locale that
///    upstream code attempted to resolve from the user's input or
///    `users.locale`. Honored verbatim when set.
/// 3. **`DEFAULT_LOCALE`** — last resort.
pub(crate) fn resolve_banner_locale(reply: &str, locale: Option<&str>) -> String {
    if let Some(info) = whatlang::detect(reply) {
        if info.is_reliable() {
            let detected = match info.lang() {
                whatlang::Lang::Fra => Some("fr"),
                whatlang::Lang::Eng => Some("en"),
                whatlang::Lang::Spa => Some("es"),
                whatlang::Lang::Deu => Some("de"),
                whatlang::Lang::Por => Some("pt"),
                _ => None,
            };
            if let Some(code) = detected {
                return code.to_owned();
            }
        }
    }
    locale.unwrap_or(DEFAULT_LOCALE).to_owned()
}

/// Inputs to [`apply_claim_verification`].
///
/// Bundles the Tier 5.5 turn context so the function signature stays under
/// clippy's `too_many_arguments` ceiling. All fields are borrowed from
/// caller-owned state — no ownership transfer, no cloning.
pub struct ClaimVerificationParams<'a> {
    /// Server resources providing the registry, corpus, and verdict repo.
    pub resources: &'a Arc<ServerResources>,
    /// Assistant reply text to scan.
    pub reply: &'a str,
    /// Parsed verification config (from the coach's prompt frontmatter).
    pub config: &'a VerificationConfig,
    /// Resolved locale for Warn/Block fallback strings, `None` → default.
    pub locale: Option<&'a str>,
}

/// Result of running Tier 5.5 verification on an assistant reply.
///
/// Callers must persist `pending_verdicts` after the assistant message is
/// durable; each verdict carries the claim and outcome fields needed to build
/// an [`InsertClaimVerdictParams`] linked to the stored message. Emitting the
/// verdict rows before the message exists would leave them with a `None`
/// `message_id` and orphan them if the message write failed.
pub struct ClaimVerificationOutcome {
    /// Finalized reply text after fallback handling.
    pub content: String,
    /// Raw verdicts produced by the detector, kept together with their claims
    /// so the caller can persist them with the assistant `message_id`.
    pub pending_verdicts: Vec<(ExtractedClaim, VerdictOutcome)>,
}

/// Run the bullshit detector over the finalized assistant reply.
///
/// Computes verdicts and applies the coach's
/// [`VerificationConfig::fallback_behavior`] to the reply, but defers
/// persisting verdicts. The caller is expected to write the assistant message
/// first and then invoke [`persist_pending_verdicts`] with the resulting
/// `message_id`. This keeps the audit row linked to the message that
/// produced it and avoids orphan verdicts when the message write fails.
pub async fn apply_claim_verification(
    params: ClaimVerificationParams<'_>,
) -> ClaimVerificationOutcome {
    let ClaimVerificationParams {
        resources,
        reply,
        config,
        locale,
    } = params;
    let locale = resolve_banner_locale(reply, locale);
    let locale = locale.as_str();

    if !config.enabled {
        return ClaimVerificationOutcome {
            content: reply.to_owned(),
            pending_verdicts: Vec::new(),
        };
    }

    let corpus = resolve_corpus(resources);
    let verdicts = verify_reply_with_config_and_corpus(reply, config, &corpus);
    if verdicts.is_empty() {
        return ClaimVerificationOutcome {
            content: reply.to_owned(),
            pending_verdicts: Vec::new(),
        };
    }

    let problems: Vec<&str> = verdicts
        .iter()
        .filter(|(_, outcome)| {
            matches!(
                outcome.status,
                ClaimStatus::Unsupported | ClaimStatus::Contradicted
            )
        })
        .map(|(claim, _)| claim.text.as_str())
        .collect();

    let content = if problems.is_empty() {
        reply.to_owned()
    } else {
        match config.fallback_behavior {
            VerificationFallback::Warn => {
                let suffix_template = resources
                    .messaging_strings_registry
                    .get(KEY_VERIFICATION_WARN_SUFFIX, locale);
                let count = problems.len().to_string();
                let suffix = format_template(&suffix_template, &[&count]);
                format!("{reply}\n\n---\n{suffix}")
            }
            VerificationFallback::Silent => reply.to_owned(),
            VerificationFallback::Block => {
                tracing::warn!(
                    flagged_claims = problems.len(),
                    "Tier 5.5 block fallback fired — replacing reply"
                );
                resources
                    .messaging_strings_registry
                    .get(KEY_VERIFICATION_BLOCK_FALLBACK, locale)
            }
        }
    };

    ClaimVerificationOutcome {
        content,
        pending_verdicts: verdicts,
    }
}

/// Persist the verdicts produced by [`apply_claim_verification`].
///
/// The caller invokes this after the assistant message has been stored so the
/// `message_id` can link each verdict to the reply it came from — the admin
/// UI uses that link to drill into the full verification history behind any
/// flagged message. Writes are best-effort: a single row failing is logged
/// and does not affect the user-facing turn.
pub async fn persist_pending_verdicts(
    resources: &Arc<ServerResources>,
    tenant_id: TenantId,
    user_id: &str,
    conversation_id: &str,
    coach_id: Option<&str>,
    message_id: &str,
    pending: &[(ExtractedClaim, VerdictOutcome)],
) {
    for (claim, outcome) in pending {
        let params = InsertClaimVerdictParams {
            tenant_id,
            user_id,
            coach_id,
            conversation_id: Some(conversation_id),
            message_id: Some(message_id),
            claim_text: &claim.text,
            category: claim.category,
            status: outcome.status,
            evidence_strength: outcome.evidence_strength,
            confidence: outcome.confidence,
            layer_fired: outcome.layer_fired,
            explanation: Some(&outcome.explanation),
            evidence_refs: outcome.evidence_refs.as_deref(),
        };
        if let Err(e) = resources
            .repos
            .claim_verdicts
            .insert_claim_verdict(&params)
            .await
        {
            tracing::warn!(error = %e, "failed to persist claim verdict");
        }
    }
}
