// ABOUTME: Thin service layer that maps Tier 5.5 ClaimVerdict rows into chat-facing wire shapes
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

use serde::{Deserialize, Serialize};

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::CoachRepos;

/// User-facing wire shape for a Tier 5.5 claim verdict.
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
    /// Coach that emitted the underlying claim, if known.
    pub coach_id: Option<String>,
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

/// Verify the caller owns the conversation, then return all Tier 5.5
/// verdicts attached to messages in that conversation.
///
/// # Errors
///
/// - [`AppError::not_found`] when the conversation does not belong to
///   the user under the given tenant.
/// - Repository errors propagated from the underlying chat or
///   claim verdict repositories.
pub async fn list_for_conversation(
    repos: &CoachRepos,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<ChatVerdictListResponse> {
    repos
        .chat
        .get_conversation(conversation_id, user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    let verdicts = repos
        .claim_verdicts
        .list_verdicts_for_conversation(conversation_id, tenant_id)
        .await?;

    let rows: Vec<ChatVerdictRow> = verdicts
        .into_iter()
        .map(|v| ChatVerdictRow {
            id: v.id,
            conversation_id: v.conversation_id,
            message_id: v.message_id,
            coach_id: v.coach_id,
            claim_text: v.claim_text,
            category: v.category.as_str().to_owned(),
            status: v.status.as_str().to_owned(),
            evidence_strength: v.evidence_strength.as_str().to_owned(),
            confidence: v.confidence,
            layer_fired: v.layer_fired.as_str().to_owned(),
            explanation: v.explanation,
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
