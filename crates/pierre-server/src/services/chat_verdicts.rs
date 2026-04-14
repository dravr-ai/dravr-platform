// ABOUTME: Thin service layer that maps Tier 5.5 ClaimVerdict rows into chat-facing wire shapes
// ABOUTME: Lets the chat route stay under the 1750-line route-thinness threshold
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

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use pierre_core::models::TenantId;

use crate::errors::{AppError, AppResult};
use crate::mcp::resources::ServerResources;
use crate::middleware::extract_auth_from_headers;

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
    resources: &Arc<ServerResources>,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<ChatVerdictListResponse> {
    resources
        .repos
        .chat
        .get_conversation(conversation_id, user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    let verdicts = resources
        .repos
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

/// Resolve the active tenant for the authenticated user.
///
/// Mirrors the helper inside `routes::chat::ChatRoutes::get_tenant_id`
/// without going through the route module so this handler can stay
/// outside `routes/chat.rs` (which is at the route-thinness ceiling).
async fn resolve_tenant_id(resources: &ServerResources, user_id: uuid::Uuid) -> TenantId {
    resources
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .ok()
        .and_then(|tenants| tenants.first().map(|t| t.id))
        .unwrap_or_else(|| TenantId::from(user_id))
}

/// Axum handler for `GET /api/chat/conversations/:id/verdicts`.
///
/// Authenticates the caller, resolves their active tenant, and delegates
/// to [`list_for_conversation`]. Routed from `routes::chat` so the chat
/// route module stays under the 1750-line route-thinness threshold.
///
/// # Errors
///
/// Returns the same errors as [`list_for_conversation`] plus
/// authentication failures from the middleware extractor.
pub async fn get_verdicts_handler(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&resources, auth.user_id).await;
    let response = list_for_conversation(
        &resources,
        &conversation_id,
        &auth.user_id.to_string(),
        tenant_id,
    )
    .await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}
