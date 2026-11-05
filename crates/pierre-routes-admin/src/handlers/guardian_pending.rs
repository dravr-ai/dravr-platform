// ABOUTME: Super admin view of tool calls the Guardian parked for an athlete's /confirm: list them and deny one
// ABOUTME: Global tokens only; a super admin can deny but never approve, and never sees the stored arguments

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Parked Guardian actions, for super admins
//!
//! Mounted twice with the same handlers, like the policy they answer to:
//! `/api/admin/settings/guardian/pending` under cookie auth and
//! `/admin/settings/guardian/pending` under admin-token auth.
//!
//! The list crosses tenants, so a token scoped to one tenant is refused. It never
//! carries `arguments`, which can hold the injected content the taint rule fired
//! on, and it leaves out the drafts `/agent create` parks in the same table.
//!
//! A super admin can only deny. Approving would run an athlete's tool call on
//! someone else's word, which is what the athlete's own `/confirm` exists to
//! prevent. A denial writes no conversation note: the note the parked turn left
//! already says that without a later result the action never ran, and an athlete
//! who answers `/confirm` afterwards gets the ordinary not-found reply.
//!
//! This is a live view, not a record. Parking a call sweeps every elapsed row
//! whatever its status, so a resolved or expired row lists only until the next
//! park. The durable record of a denial is its `audit_events` row, action
//! `guardian.pending.deny`, naming the call and never its arguments.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{PendingActionStatus, PendingActionSummary};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::audit::AuditDetail;
use crate::context::AdminApiContext;

/// List size when the request names none.
const DEFAULT_LIST_LIMIT: i64 = 50;
/// The largest list a request can ask for.
const MAX_LIST_LIMIT: i64 = 200;

/// Query of the list route.
#[derive(Debug, Default, Deserialize)]
pub struct GuardianPendingListQuery {
    /// One tenant's rows; every tenant's when absent.
    pub tenant: Option<String>,
    /// `pending`, `confirmed`, `denied` or `expired`; `pending` when absent.
    pub status: Option<String>,
    /// List size, clamped to `1..=200`; 50 when absent.
    pub limit: Option<i64>,
}

/// Body of the list route.
#[derive(Debug, Serialize)]
pub struct GuardianPendingListResponse {
    /// The rows, newest first.
    pub entries: Vec<PendingActionSummary>,
    /// The status the rows were selected by.
    pub status: &'static str,
    /// The list size applied.
    pub limit: i64,
}

/// Body of the deny route.
#[derive(Debug, Serialize)]
pub struct GuardianPendingDenyResponse {
    /// The row as the denial left it.
    pub entry: PendingActionSummary,
}

/// Handle `GET /admin/settings/guardian/pending` (and its console mount).
///
/// # Errors
///
/// Returns an error when the caller lacks `ViewConfiguration`, the token is
/// scoped to one tenant, the status is not one of the four, or the read fails.
pub async fn handle_list_guardian_pending(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<GuardianPendingListQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_global_scope()?;

    let status = query
        .status
        .as_deref()
        .map_or(Ok(PendingActionStatus::Pending), |raw| {
            PendingActionStatus::parse(raw).ok_or_else(|| {
                AppError::invalid_input(format!(
                    "status '{raw}' is not one of pending, confirmed, denied, expired"
                ))
            })
        })?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);

    let entries = context
        .repos
        .guardian_actions
        .list_pending_actions_for_super_admin(query.tenant.as_deref(), status, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list parked guardian actions");
            e
        })?;

    Ok((
        StatusCode::OK,
        Json(GuardianPendingListResponse {
            entries,
            status: status.as_str(),
            limit,
        }),
    ))
}

/// Handle `POST /admin/settings/guardian/pending/{id}/deny` (and its console mount).
///
/// # Errors
///
/// Returns an error when the caller lacks `ManageConfiguration`, the token is
/// scoped to one tenant, no such call is waiting, or the write fails.
pub async fn handle_deny_guardian_pending(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    admin_token.require_global_scope()?;

    let entry = context
        .repos
        .guardian_actions
        .deny_pending_action_as_super_admin(&id)
        .await?
        .ok_or_else(|| AppError::not_found("A parked call waiting for its athlete with that id"))?;

    let audit = AuditDetail::new(
        "guardian.pending.deny",
        format!(
            "Denied the parked call of {} in tenant {}",
            entry.tool_name, entry.tenant_id
        ),
    )
    .target(json!({
        "pending_id": entry.id,
        "tool_name": entry.tool_name,
        "tenant_id": entry.tenant_id,
    }));
    info!(
        token_id = %admin_token.token_id,
        pending_id = %entry.id,
        tool_name = %entry.tool_name,
        "super admin denied a parked guardian action"
    );

    Ok(audit.attach((StatusCode::OK, Json(GuardianPendingDenyResponse { entry })).into_response()))
}
