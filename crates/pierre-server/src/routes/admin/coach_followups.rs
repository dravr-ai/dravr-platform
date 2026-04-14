// ABOUTME: Admin routes for the tenant-wide coach followup triage queue
// ABOUTME: List pending followups and cancel stale promises from the admin UI
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin coach followup triage routes.
//!
//! Coach personas promise future check-ins ("I'll ask about your Achilles
//! tomorrow") via the followup tool. The harness stores those promises in
//! `coach_followups` and injects the pending ones into the next system
//! prompt. This admin surface lets tenant admins review the pending queue
//! across all users and coaches, and cancel stale promises that should
//! never reach the coach.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use pierre_memory::CoachFollowup;

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};
use crate::models::TenantId;

use super::AdminApiContext;

/// Query parameters for listing pending followups.
#[derive(Debug, Deserialize)]
pub struct ListFollowupsQuery {
    /// Tenant to list followups for (required — admin tokens can span tenants).
    pub tenant_id: String,
    /// Maximum number of rows to return, clamped to `1..=200`. Defaults to 100.
    pub limit: Option<i64>,
}

/// Query parameters for the cancel endpoint.
#[derive(Debug, Deserialize)]
pub struct CancelFollowupQuery {
    /// Tenant that owns the followup being cancelled.
    pub tenant_id: String,
}

/// Wire-format representation of a pending followup row.
///
/// Flattens the domain [`CoachFollowup`] enum to a stable string for the
/// status column so the `TypeScript` client can render without mapping.
#[derive(Debug, Serialize)]
pub struct FollowupRow {
    pub id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub coach_id: String,
    pub conversation_id: Option<String>,
    pub content: String,
    pub due_at: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub delivered_at: Option<String>,
}

impl From<CoachFollowup> for FollowupRow {
    fn from(f: CoachFollowup) -> Self {
        Self {
            id: f.id,
            tenant_id: f.tenant_id,
            user_id: f.user_id,
            coach_id: f.coach_id,
            conversation_id: f.conversation_id,
            content: f.content,
            due_at: f.due_at.map(|d| d.to_rfc3339()),
            status: f.status.as_str().to_owned(),
            created_at: f.created_at.to_rfc3339(),
            updated_at: f.updated_at.to_rfc3339(),
            delivered_at: f.delivered_at.map(|d| d.to_rfc3339()),
        }
    }
}

/// Response envelope for `GET /admin/coach-followups/pending`.
#[derive(Debug, Serialize)]
pub struct FollowupListResponse {
    pub followups: Vec<FollowupRow>,
    pub total: usize,
}

/// Response envelope for `POST /admin/coach-followups/{id}/cancel`.
#[derive(Debug, Serialize)]
pub struct CancelFollowupResponse {
    /// `true` when the row transitioned `pending -> cancelled`, `false` if
    /// it was already delivered/cancelled (idempotent — the post-condition
    /// is "this followup will not be injected into a coach prompt").
    pub cancelled: bool,
}

fn parse_tenant(raw: &str) -> AppResult<TenantId> {
    raw.parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {raw}")))
}

/// Handle `GET /admin/coach-followups/pending`.
pub(super) async fn handle_list_pending_followups(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<ListFollowupsQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let tenant = parse_tenant(&params.tenant_id)?;
    let limit = params.limit.unwrap_or(100).clamp(1, 200);

    let followups = context
        .repos
        .memory
        .list_pending_followups_for_tenant(tenant, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list pending followups for tenant");
            AppError::internal(format!("Failed to list pending followups: {e}"))
        })?;

    let rows: Vec<FollowupRow> = followups.into_iter().map(Into::into).collect();
    let total = rows.len();

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        total,
        "admin listed pending followups"
    );

    Ok((
        StatusCode::OK,
        Json(FollowupListResponse {
            followups: rows,
            total,
        }),
    ))
}

/// Handle `POST /admin/coach-followups/{followup_id}/cancel`.
///
/// Transitions the followup from `pending` to `cancelled` so the harness
/// will not inject it into the next coach system prompt. Requires
/// [`AdminPermission::ManageConfiguration`] — this is a write action that
/// can visibly affect coach behavior, so it sits behind a stronger
/// permission than the read endpoint.
pub(super) async fn handle_cancel_followup(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(followup_id): Path<String>,
    Query(params): Query<CancelFollowupQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    let tenant = parse_tenant(&params.tenant_id)?;

    let cancelled = context
        .repos
        .memory
        .cancel_followup(&followup_id, tenant)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to cancel followup");
            AppError::internal(format!("Failed to cancel followup: {e}"))
        })?;

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        followup_id = %followup_id,
        cancelled,
        "admin cancelled followup"
    );

    Ok((StatusCode::OK, Json(CancelFollowupResponse { cancelled })))
}
