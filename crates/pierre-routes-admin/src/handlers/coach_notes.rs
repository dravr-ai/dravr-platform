// ABOUTME: Admin route for the tenant-wide coach_notes compliance audit log
// ABOUTME: Exposes every note a coach persona has authored about any user in the tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin coach note audit log route.
//!
//! Coach personas authoring notes about users creates a privacy and
//! compliance surface that must be reviewable by tenant admins. This
//! endpoint returns the tenant-wide audit log newest-first, so
//! compliance reviewers can see every note in one flat stream without
//! having to enumerate (user, coach) pairs.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_memory::CoachNote;

use crate::context::AdminApiContext;

/// Default number of notes to return when the caller omits `limit`.
const DEFAULT_AUDIT_LIMIT: i64 = 100;
/// Hard cap so a misconfigured client cannot drag the database.
const MAX_AUDIT_LIMIT: i64 = 500;

/// Query parameters for the audit log endpoint.
#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    /// Tenant to list coach notes for (admin tokens can span tenants).
    pub tenant_id: String,
    /// Maximum rows to return, clamped to `1..=500`. Defaults to 100.
    pub limit: Option<i64>,
}

/// Wire-format audit row for a single coach note.
///
/// The domain [`CoachNote`] keeps scope as an enum — this struct flattens
/// it to a stable `snake_case` string so the `TypeScript` client renders
/// without an extra mapping.
#[derive(Debug, Serialize)]
pub struct CoachNoteAuditRow {
    /// Stable identifier for this note row.
    pub id: String,
    /// Tenant that owns the note.
    pub tenant_id: String,
    /// User the note is about.
    pub user_id: String,
    /// Coach that authored the note.
    pub coach_id: String,
    /// Conversation the note was derived from, when known.
    pub conversation_id: Option<String>,
    /// Note scope flattened to a stable string (e.g. `profile`, `session`).
    pub scope: String,
    /// Free-form note content recorded by the coach.
    pub content: String,
    /// RFC3339 timestamp the note was created.
    pub created_at: String,
    /// RFC3339 timestamp the note was last updated.
    pub updated_at: String,
    /// `true` when an admin has flagged the note. Memory recall queries
    /// skip these rows; the audit panel still displays them.
    pub suppressed: bool,
}

impl From<CoachNote> for CoachNoteAuditRow {
    fn from(n: CoachNote) -> Self {
        Self {
            id: n.id,
            tenant_id: n.tenant_id,
            user_id: n.user_id,
            coach_id: n.coach_id,
            conversation_id: n.conversation_id,
            scope: n.scope.as_str().to_owned(),
            content: n.content,
            created_at: n.created_at.to_rfc3339(),
            updated_at: n.updated_at.to_rfc3339(),
            suppressed: n.suppressed,
        }
    }
}

/// Response envelope for `GET /admin/coach-notes/audit`.
#[derive(Debug, Serialize)]
pub struct CoachNoteAuditResponse {
    /// Audit rows for this page.
    pub notes: Vec<CoachNoteAuditRow>,
    /// Number of rows returned in `notes`.
    pub total: usize,
}

/// Handle `GET /admin/coach-notes/audit`.
///
/// Requires [`AdminPermission::ViewAuditLogs`] — coach notes contain
/// user profile information the coach has derived from conversations,
/// so a dedicated audit permission (rather than the generic config read)
/// gates access.
pub(crate) async fn handle_list_audit(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<AuditQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewAuditLogs)?;

    let tenant = TenantId::parse_str(&params.tenant_id)
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_AUDIT_LIMIT)
        .clamp(1, MAX_AUDIT_LIMIT);

    let notes = context
        .repos
        .memory
        .list_coach_notes_for_tenant(tenant, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list coach notes for tenant audit");
            AppError::internal(format!("Failed to list coach notes: {e}"))
        })?;

    let rows: Vec<CoachNoteAuditRow> = notes.into_iter().map(Into::into).collect();
    let total = rows.len();

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        total,
        "admin listed coach note audit log"
    );

    Ok((
        StatusCode::OK,
        Json(CoachNoteAuditResponse { notes: rows, total }),
    ))
}

/// Query params for the suppress / unsuppress endpoints — same `tenant_id`
/// shape every other admin route uses.
#[derive(Debug, Deserialize)]
pub struct SuppressQuery {
    /// Tenant the note belongs to (required for tenant-scoped writes).
    pub tenant_id: String,
}

/// Wire response for suppress / unsuppress. `changed=false` is returned
/// when the note already had the requested state (idempotent), or when
/// the row could not be found.
#[derive(Debug, Serialize)]
pub struct SuppressResponse {
    /// Note that was targeted by the suppress / unsuppress call.
    pub note_id: String,
    /// Current suppression state after the call.
    pub suppressed: bool,
    /// `true` when the call transitioned the row to a new state, `false`
    /// when the requested state was already in effect (or the row was not
    /// found).
    pub changed: bool,
}

/// Handle `POST /api/admin/coach-notes/{note_id}/suppress`.
///
/// Flips `suppressed=true`. The chat pipeline's memory recall skips
/// suppressed rows so the coach won't re-inject the note into a future
/// prompt. Idempotent — re-posting on an already-suppressed note returns
/// `changed=false` without writing.
pub(crate) async fn handle_suppress_note(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(note_id): Path<String>,
    Query(params): Query<SuppressQuery>,
) -> AppResult<impl IntoResponse> {
    set_suppressed(context, admin_token, note_id, params, true).await
}

/// Handle `POST /api/admin/coach-notes/{note_id}/unsuppress`.
///
/// Inverse of [`handle_suppress_note`] — flips `suppressed=false` so the
/// note becomes recallable again. Same idempotency contract.
pub(crate) async fn handle_unsuppress_note(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(note_id): Path<String>,
    Query(params): Query<SuppressQuery>,
) -> AppResult<impl IntoResponse> {
    set_suppressed(context, admin_token, note_id, params, false).await
}

async fn set_suppressed(
    context: Arc<AdminApiContext>,
    admin_token: ValidatedAdminToken,
    note_id: String,
    params: SuppressQuery,
    suppressed: bool,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    let tenant = TenantId::parse_str(&params.tenant_id)
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;

    let changed = context
        .repos
        .memory
        .set_coach_note_suppressed(&note_id, tenant, suppressed, &admin_token.service_name)
        .await
        .map_err(|e| {
            error!(error = %e, note_id = %note_id, "failed to update coach note suppressed flag");
            AppError::internal(format!("Failed to update suppressed flag: {e}"))
        })?;

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        note_id = %note_id,
        suppressed,
        changed,
        "admin set coach note suppressed state"
    );

    Ok((
        StatusCode::OK,
        Json(SuppressResponse {
            note_id,
            suppressed,
            changed,
        }),
    ))
}
