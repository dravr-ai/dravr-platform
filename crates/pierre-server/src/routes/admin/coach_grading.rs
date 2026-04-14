// ABOUTME: Admin route exposing per-coach Tier 5.5 content grades for store ranking review
// ABOUTME: Wraps services::coach_grading::compute_coach_grades behind ViewConfiguration
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin coach content grading route.
//!
//! Wraps [`crate::services::coach_grading::compute_coach_grades`] in
//! an axum handler. Requires [`AdminPermission::ViewConfiguration`]
//! since the underlying data (`claim_verdicts`) is the same source
//! the claim-verdicts triage tab uses.

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
use crate::services::coach_grading::{
    compute_coach_grades, DEFAULT_VERDICT_LIMIT, MAX_VERDICTS_SCANNED,
};

use super::AdminApiContext;

/// Query parameters for the coach grading summary endpoint.
#[derive(Debug, Deserialize)]
pub struct CoachGradingQuery {
    /// Tenant to compute grades for (admin tokens span tenants).
    pub tenant_id: String,
    /// Maximum verdicts to scan, clamped to `1..=MAX_VERDICTS_SCANNED`.
    pub limit: Option<i64>,
}

/// Handle `GET /admin/coach-grading/summary`.
pub(super) async fn handle_get_summary(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<CoachGradingQuery>,
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

    let summary = compute_coach_grades(&context.repos, tenant, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to compute coach grading summary");
            AppError::internal(format!("Failed to compute coach grading summary: {e}"))
        })?;

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        verdicts_scanned = summary.verdicts_scanned,
        coaches = summary.grades.len(),
        "admin fetched coach grading summary"
    );

    Ok((StatusCode::OK, Json(summary)))
}
