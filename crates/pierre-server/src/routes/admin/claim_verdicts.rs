// ABOUTME: Admin routes for triaging Tier 5.5 claim verdicts from the bullshit detector
// ABOUTME: Provides list/filter/detail endpoints over the claim_verdicts table
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin claim verdict triage routes.
//!
//! Surfaces the rows written by the Tier 5.5 detector pipeline
//! (`services::claim_verification::apply_claim_verification`) so tenant
//! admins can review flagged claims, see which coach emitted them, and
//! drill into the supporting evidence.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use pierre_memory::claims::ClaimVerdict;

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};
use crate::models::TenantId;

use super::AdminApiContext;

/// Query parameters for listing claim verdicts.
#[derive(Debug, Deserialize)]
pub struct ListVerdictsQuery {
    /// Tenant to list verdicts for (required — admin tokens can span tenants).
    pub tenant_id: String,
    /// Optional filter by verdict status.
    pub status: Option<String>,
    /// Optional filter by claim category.
    pub category: Option<String>,
    /// Optional filter by the coach that emitted the claim.
    pub coach_id: Option<String>,
    /// Maximum number of rows to return, clamped to `1..=200`. Defaults to 50.
    pub limit: Option<i64>,
}

/// Query parameters for listing verdicts scoped to a conversation.
#[derive(Debug, Deserialize)]
pub struct ConversationVerdictsQuery {
    /// Tenant that owns the conversation.
    pub tenant_id: String,
}

/// Wire-format representation of a `ClaimVerdict` for the admin UI.
///
/// Flat strings everywhere so the TypeScript client can render without
/// mapping `snake_case` enums into display labels on the fly.
#[derive(Debug, Serialize)]
pub struct VerdictRow {
    pub id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub coach_id: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub claim_text: String,
    pub category: String,
    pub status: String,
    pub evidence_strength: String,
    pub confidence: f32,
    pub layer_fired: String,
    pub explanation: Option<String>,
    pub evidence_refs: Option<String>,
    pub created_at: String,
}

impl From<ClaimVerdict> for VerdictRow {
    fn from(v: ClaimVerdict) -> Self {
        Self {
            id: v.id,
            tenant_id: v.tenant_id,
            user_id: v.user_id,
            coach_id: v.coach_id,
            conversation_id: v.conversation_id,
            message_id: v.message_id,
            claim_text: v.claim_text,
            category: v.category.as_str().to_owned(),
            status: v.status.as_str().to_owned(),
            evidence_strength: v.evidence_strength.as_str().to_owned(),
            confidence: v.confidence,
            layer_fired: v.layer_fired.as_str().to_owned(),
            explanation: v.explanation,
            evidence_refs: v.evidence_refs,
            created_at: v.created_at.to_rfc3339(),
        }
    }
}

/// List response envelope used by the admin UI.
#[derive(Debug, Serialize)]
pub struct VerdictListResponse {
    pub verdicts: Vec<VerdictRow>,
    pub total: usize,
}

/// Handle `GET /admin/claim-verdicts`.
///
/// Returns the most recent verdicts for the caller's tenant with optional
/// filters by status, category, and coach. Results are clamped to 200 and
/// the server does all filtering in memory — the Phase A
/// `list_recent_verdicts` repository call returns the full recent set,
/// which is fine for the low volumes expected in Phase A. When volume
/// grows, this will move to an indexed SQL filter.
pub(super) async fn handle_list_claim_verdicts(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<ListVerdictsQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let tenant: TenantId = params
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let verdicts = context
        .repos
        .claim_verdicts
        .list_recent_verdicts(tenant, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list claim verdicts");
            AppError::internal(format!("Failed to list claim verdicts: {e}"))
        })?;

    let rows: Vec<VerdictRow> = verdicts
        .into_iter()
        .filter(|v| {
            params
                .status
                .as_deref()
                .is_none_or(|s| v.status.as_str() == s)
        })
        .filter(|v| {
            params
                .category
                .as_deref()
                .is_none_or(|c| v.category.as_str() == c)
        })
        .filter(|v| {
            params
                .coach_id
                .as_deref()
                .is_none_or(|cid| v.coach_id.as_deref() == Some(cid))
        })
        .map(Into::into)
        .collect();

    let total = rows.len();
    info!(
        service = %admin_token.service_name,
        total,
        "admin listed claim verdicts"
    );

    Ok((
        StatusCode::OK,
        Json(VerdictListResponse {
            verdicts: rows,
            total,
        }),
    ))
}

/// Handle `GET /admin/claim-verdicts/{conversation_id}`.
///
/// Returns every verdict tied to a specific conversation in chronological
/// order. Used by the admin drawer to show the full verification history
/// behind a single flagged message.
pub(super) async fn handle_list_verdicts_by_conversation(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(conversation_id): Path<String>,
    Query(params): Query<ConversationVerdictsQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let tenant: TenantId = params
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;

    let verdicts = context
        .repos
        .claim_verdicts
        .list_verdicts_for_conversation(&conversation_id, tenant)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list verdicts for conversation");
            AppError::internal(format!("Failed to list verdicts for conversation: {e}"))
        })?;

    let rows: Vec<VerdictRow> = verdicts.into_iter().map(Into::into).collect();
    let total = rows.len();

    Ok((
        StatusCode::OK,
        Json(VerdictListResponse {
            verdicts: rows,
            total,
        }),
    ))
}
