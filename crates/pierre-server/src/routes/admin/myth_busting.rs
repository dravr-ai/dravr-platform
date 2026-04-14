// ABOUTME: Admin route exposing the Phase D myth-busting summary over Tier 5.5 verdicts
// ABOUTME: Returns top recurring claims, coaches, categories for tenant pattern review
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin myth-busting summary route.
//!
//! Wraps [`crate::services::myth_busting::compute_summary`] in an axum
//! handler. Requires [`AdminPermission::ViewConfiguration`] — the same
//! permission that gates `ClaimVerdictsTab` because both surfaces
//! display claim text and category breakdowns.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use tracing::{error, info};

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};
use crate::models::TenantId;
use crate::services::myth_busting::{compute_summary, DEFAULT_VERDICT_LIMIT, MAX_VERDICTS_SCANNED};

use super::AdminApiContext;

/// Query parameters for the myth-busting summary endpoint.
#[derive(Debug, Deserialize)]
pub struct MythBustingQuery {
    /// Tenant to compute the summary for (admin tokens span tenants).
    pub tenant_id: String,
    /// Maximum verdicts to scan, clamped to `1..=MAX_VERDICTS_SCANNED`.
    pub limit: Option<i64>,
}

/// Handle `GET /admin/myth-busting/summary`.
pub(super) async fn handle_get_summary(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<MythBustingQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let tenant: TenantId = params
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_VERDICT_LIMIT)
        .clamp(1, MAX_VERDICTS_SCANNED);

    let summary = compute_summary(&context.repos, tenant, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to compute myth-busting summary");
            AppError::internal(format!("Failed to compute myth-busting summary: {e}"))
        })?;

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        verdicts_scanned = summary.verdicts_scanned,
        flagged_total = summary.flagged_total,
        "admin fetched myth-busting summary"
    );

    Ok((StatusCode::OK, Json(summary)))
}
