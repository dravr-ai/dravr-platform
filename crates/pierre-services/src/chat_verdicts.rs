// ABOUTME: Thin service layer that maps ClaimVerdict rows into chat-facing wire shapes
// ABOUTME: Pure repository-backed helper consumed by the chat route handler in pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat verdict service.
//!
//! Wraps `ClaimVerdictRepository::list_verdicts_for_conversation` with
//! ownership verification (the user must own the conversation) and
//! converts the domain `ClaimVerdict` rows into a serializable wire shape
//! that mirrors the admin route response without crossing the admin
//! permission gate.

use pierre_providers::ai_scope;
use serde::{Deserialize, Serialize};

use pierre_contremaitre::EvidenceRegistry;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::AgentRepos;
use pierre_memory::EvidenceCitation;

/// User-facing wire shape for a claim verdict.
///
/// Mirrors the admin row but is exposed via the chat route so end users
/// can render Evidence Strength chips on their own messages without
/// needing admin permissions.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChatVerdictRow {
    /// Stable verdict identifier.
    pub id: String,
    /// Conversation the verdict was emitted in.
    pub conversation_id: Option<String>,
    /// Message the verdict belongs to (chip rendering key).
    pub message_id: Option<String>,
    /// Agent that emitted the underlying claim, if known.
    pub agent_id: Option<String>,
    /// The exact claim text the detector verified.
    pub claim_text: String,
    /// `nutrition`, `supplement`, etc.
    pub category: String,
    /// `supported`, `unsupported`, `contradicted`, `rhetorical`, `unverifiable`.
    pub status: String,
    /// `strong`, `mixed`, `weak`, `none`.
    pub evidence_strength: String,
    /// Pipeline confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Which detector layer produced the verdict.
    pub layer_fired: String,
    /// User-facing rationale rendered by the detector explanation layer.
    pub explanation: Option<String>,
    /// Comma-separated DOIs / PMIDs backing the verdict, if any.
    pub evidence_refs: Option<String>,
    /// The study behind each `evidence_refs` id, in stored order, resolved
    /// against the evidence corpus when the verdict is read (carnet#801). An
    /// id the corpus no longer holds is listed with only its `id`.
    pub evidence: Vec<EvidenceCitation>,
    /// RFC3339 emission timestamp.
    pub created_at: String,
}

/// Response envelope for `GET /api/chat/conversations/:id/verdicts`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ChatVerdictListResponse {
    /// Verdicts attached to messages in this conversation, chronological order.
    pub verdicts: Vec<ChatVerdictRow>,
    /// Convenience count (matches `verdicts.len()`).
    pub total: usize,
}

/// The study behind each id of a verdict's comma-separated `evidence_refs`.
///
/// In stored order, blanks and repeats dropped. Resolved at read time rather
/// than stored on the row, so a corrected corpus entry corrects older
/// verdicts too.
fn evidence_citations(
    evidence_refs: Option<&str>,
    corpus: &EvidenceRegistry,
) -> Vec<EvidenceCitation> {
    let mut citations: Vec<EvidenceCitation> = Vec::new();
    for id in evidence_refs.unwrap_or_default().split(',').map(str::trim) {
        if id.is_empty() || citations.iter().any(|known| known.id == id) {
            continue;
        }
        citations.push(corpus.citation(id).unwrap_or_else(|| EvidenceCitation {
            id: id.to_owned(),
            url: None,
            label: None,
            title: None,
            journal: None,
            year: None,
        }));
    }
    citations
}

/// Verify the caller owns the conversation, then return all claim
/// verdicts attached to messages in that conversation, each naming the
/// studies behind it from `corpus`.
///
/// # Errors
///
/// - [`AppError::not_found`] when the conversation does not belong to
///   the user under the given tenant.
/// - Repository errors propagated from the underlying chat or
///   claim verdict repositories.
pub async fn list_for_conversation(
    repos: &AgentRepos,
    corpus: &EvidenceRegistry,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<ChatVerdictListResponse> {
    repos
        .chat
        .get_conversation(conversation_id, user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    let mut verdicts = repos
        .claim_verdicts
        .list_verdicts_for_conversation(conversation_id, tenant_id)
        .await?;
    // A verdict quotes the reply it judged: one on a reply derived from
    // first-party-only data is withheld from an external caller (carnet#769).
    ai_scope::retain_admitted(&mut verdicts, |verdict| verdict.transport_policy);

    let rows: Vec<ChatVerdictRow> = verdicts
        .into_iter()
        .map(|v| ChatVerdictRow {
            id: v.id,
            conversation_id: v.conversation_id,
            message_id: v.message_id,
            agent_id: v.agent_id,
            claim_text: v.claim_text,
            category: v.category.as_str().to_owned(),
            status: v.status.as_str().to_owned(),
            evidence_strength: v.evidence_strength.as_str().to_owned(),
            confidence: v.confidence,
            layer_fired: v.layer_fired.as_str().to_owned(),
            explanation: v.explanation,
            evidence: evidence_citations(v.evidence_refs.as_deref(), corpus),
            evidence_refs: v.evidence_refs,
            created_at: v.created_at.to_rfc3339(),
        })
        .collect();

    let total = rows.len();
    Ok(ChatVerdictListResponse {
        verdicts: rows,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_contremaitre::evidence_registry::parse_evidence_markdown;

    const NAMED: &str = r#"---
id: doi:10.1111/sms.12104
url: https://doi.org/10.1111/sms.12104
category: training_prescription
strength: strong
citation: Rønnestad and Mujika 2014 review
label: "Rønnestad & Mujika, 2014"
title: "Optimizing strength training for running and cycling endurance performance: A review"
journal: "Scand J Med Sci Sports"
year: 2014
---

Heavy strength training improves cycling and running economy.
"#;

    fn registry_with_named_study() -> EvidenceRegistry {
        let registry = EvidenceRegistry::new();
        let corpus = parse_evidence_markdown(NAMED).expect("fixture parses");
        registry.update(
            "training_prescription",
            "ronnestad-2014",
            corpus,
            "sha".to_owned(),
        );
        registry
    }

    #[test]
    fn names_each_study_in_stored_order_and_keeps_an_unknown_id() {
        let registry = registry_with_named_study();
        let citations = evidence_citations(
            Some("doi:10.1/removed, doi:10.1111/sms.12104,,doi:10.1/removed"),
            &registry,
        );
        let ids: Vec<&str> = citations.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["doi:10.1/removed", "doi:10.1111/sms.12104"]);
        assert_eq!(citations[0].label, None);
        assert_eq!(
            citations[1].label.as_deref(),
            Some("Rønnestad & Mujika, 2014")
        );
        assert_eq!(citations[1].year, Some(2014));
    }

    #[test]
    fn a_verdict_without_references_names_no_study() {
        let registry = registry_with_named_study();
        assert!(evidence_citations(None, &registry).is_empty());
        assert!(evidence_citations(Some(" , "), &registry).is_empty());
    }

    /// An empty registry (first boot, contremaitre unreachable) resolves
    /// against the compiled-in corpus, as verification does.
    #[test]
    fn an_empty_registry_falls_back_to_the_compiled_in_corpus() {
        let registry = EvidenceRegistry::new();
        let citations = evidence_citations(Some("doi:10.1111/sms.12104"), &registry);
        assert_eq!(citations.len(), 1);
        assert_eq!(
            citations[0].url.as_deref(),
            Some("https://doi.org/10.1111/sms.12104")
        );
    }
}
