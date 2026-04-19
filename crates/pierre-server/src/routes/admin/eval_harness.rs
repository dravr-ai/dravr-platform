// ABOUTME: Admin routes exposing the pierre-evals fixture browser and Tier 5.5 calibration stats
// ABOUTME: Read-only — lists fixtures, per-case summaries, and verdict drift over a time window
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin eval harness routes.
//!
//! Wraps [`crate::services::eval_harness::browse_fixtures`] and the
//! calibration aggregation in axum handlers gated behind
//! [`AdminPermission::ViewConfiguration`]. Only compiled when the
//! `tools-verification` feature is enabled because `pierre-evals` is an
//! optional dependency.

use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;
use tracing::{error, info};

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};
use crate::models::TenantId;
use crate::services::eval_harness::{
    browse_fixtures, delete_fixture, read_fixture_text, write_fixture, FixtureSummary,
};

use super::AdminApiContext;

/// Handle `GET /admin/evals/fixtures`.
pub(super) async fn handle_list_fixtures(
    State(_context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let response = spawn_blocking(browse_fixtures)
        .await
        .map_err(|e| {
            error!(error = %e, "eval fixture scan task panicked");
            AppError::internal(format!("Eval fixture scan task failed: {e}"))
        })?
        .map_err(|e| {
            error!(error = %e, "failed to browse eval fixtures");
            e
        })?;

    info!(
        service = %admin_token.service_name,
        fixture_count = response.fixture_count,
        case_total = response.case_total,
        "admin fetched eval fixture browser"
    );

    Ok((StatusCode::OK, Json(response)))
}

/// Query parameters for `GET /admin/evals/verdict-stats`.
#[derive(Debug, Deserialize)]
pub struct VerdictStatsQuery {
    /// Tenant to aggregate over (required — admin tokens can span tenants).
    pub tenant_id: String,
    /// Number of days to scan back from now, clamped to `1..=365` by
    /// the repository layer. Defaults to 30 when omitted.
    pub window_days: Option<i64>,
}

/// Handle `GET /admin/evals/verdict-stats`.
///
/// Returns per-day and total verdict counts for the Tier 5.5 claim
/// verification pipeline so the admin "Calibration" panel can show
/// pass rate and daily drift.
pub(super) async fn handle_verdict_stats(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<VerdictStatsQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_tenant_access(&params.tenant_id)?;

    let tenant: TenantId = params
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;
    let window_days = params.window_days.unwrap_or(30);

    let stats = context
        .repos
        .claim_verdicts
        .aggregate_verdict_stats(tenant, window_days)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to aggregate verdict stats");
            AppError::internal(format!("Failed to aggregate verdict stats: {e}"))
        })?;

    info!(
        service = %admin_token.service_name,
        tenant_id = %tenant,
        window_days = stats.window_days,
        days_with_data = stats.daily.len(),
        "admin fetched verdict calibration stats"
    );

    Ok((StatusCode::OK, Json(stats)))
}

/// Response envelope for fixture read/write endpoints.
#[derive(Debug, Serialize)]
pub struct FixtureContentResponse {
    /// Fixture stem (no `.jsonl` extension).
    pub name: String,
    /// Raw JSONL contents of the fixture.
    pub body: String,
}

/// Request body for `PUT /admin/evals/fixtures/{name}`.
#[derive(Debug, Deserialize)]
pub struct UpdateFixtureRequest {
    /// New raw JSONL contents to write to disk.
    pub body: String,
}

/// Handle `GET /admin/evals/fixtures/{name}`.
pub(super) async fn handle_get_fixture(
    State(_context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    AxumPath(name): AxumPath<String>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let name_for_blocking = name.clone();
    let body = spawn_blocking(move || read_fixture_text(&name_for_blocking))
        .await
        .map_err(|e| {
            error!(error = %e, "fixture read task panicked");
            AppError::internal(format!("Fixture read task failed: {e}"))
        })??;

    info!(
        service = %admin_token.service_name,
        fixture = %name,
        "admin read eval fixture"
    );

    Ok((StatusCode::OK, Json(FixtureContentResponse { name, body })))
}

/// Handle `PUT /admin/evals/fixtures/{name}`.
pub(super) async fn handle_put_fixture(
    State(_context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    AxumPath(name): AxumPath<String>,
    Json(request): Json<UpdateFixtureRequest>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    let name_for_blocking = name.clone();
    let body = request.body;
    let summary: FixtureSummary = spawn_blocking(move || write_fixture(&name_for_blocking, &body))
        .await
        .map_err(|e| {
            error!(error = %e, "fixture write task panicked");
            AppError::internal(format!("Fixture write task failed: {e}"))
        })??;

    info!(
        service = %admin_token.service_name,
        fixture = %name,
        case_count = summary.case_count,
        "admin wrote eval fixture"
    );

    Ok((StatusCode::OK, Json(summary)))
}

/// Handle `DELETE /admin/evals/fixtures/{name}`.
pub(super) async fn handle_delete_fixture(
    State(_context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    AxumPath(name): AxumPath<String>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    let name_for_blocking = name.clone();
    spawn_blocking(move || delete_fixture(&name_for_blocking))
        .await
        .map_err(|e| {
            error!(error = %e, "fixture delete task panicked");
            AppError::internal(format!("Fixture delete task failed: {e}"))
        })??;

    info!(
        service = %admin_token.service_name,
        fixture = %name,
        "admin deleted eval fixture"
    );

    Ok(StatusCode::NO_CONTENT)
}
