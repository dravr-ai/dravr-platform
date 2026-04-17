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
use pierre_evals::{VerificationConfig, VerificationFallback};
use pierre_memory::claims::ClaimStatus;

use crate::contremaitre::messaging_strings::{
    format_template, DEFAULT_LOCALE, KEY_VERIFICATION_BLOCK_FALLBACK, KEY_VERIFICATION_WARN_SUFFIX,
};
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::services::claim_verification::{resolve_corpus, verify_reply_with_config_and_corpus};

/// Run the bullshit detector over the finalized assistant reply.
///
/// Persists verdicts and reacts to unsupported or contradicted claims per
/// the coach's [`VerificationConfig::fallback_behavior`]. Verdicts are
/// persisted on a best-effort basis — database failures are logged and
/// swallowed so dispatch never fails on an audit write.
pub async fn apply_claim_verification(
    resources: &Arc<ServerResources>,
    reply: &str,
    user_id: &str,
    conversation_id: &str,
    coach_id: Option<&str>,
    tenant_id: TenantId,
    config: &VerificationConfig,
) -> String {
    if !config.enabled {
        return reply.to_owned();
    }

    let corpus = resolve_corpus(resources);
    let verdicts = verify_reply_with_config_and_corpus(reply, config, &corpus);
    if verdicts.is_empty() {
        return reply.to_owned();
    }

    let mut problems: Vec<String> = Vec::new();
    for (claim, outcome) in &verdicts {
        if matches!(
            outcome.status,
            ClaimStatus::Unsupported | ClaimStatus::Contradicted
        ) {
            problems.push(claim.text.clone());
        }
        let params = InsertClaimVerdictParams {
            tenant_id,
            user_id,
            coach_id,
            conversation_id: Some(conversation_id),
            message_id: None,
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

    if problems.is_empty() {
        return reply.to_owned();
    }

    match config.fallback_behavior {
        VerificationFallback::Warn => {
            let suffix_template = resources
                .messaging_strings_registry
                .get(KEY_VERIFICATION_WARN_SUFFIX, DEFAULT_LOCALE);
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
                .get(KEY_VERIFICATION_BLOCK_FALLBACK, DEFAULT_LOCALE)
        }
    }
}
