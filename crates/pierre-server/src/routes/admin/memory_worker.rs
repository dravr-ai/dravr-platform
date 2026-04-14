// ABOUTME: Admin route exposing memory extraction worker health via user_facts aggregates
// ABOUTME: Returns tenant-scoped counts/kind breakdown so operators can spot extraction stalls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Memory extraction worker monitoring.
//!
//! The memory extraction pipeline runs as a fire-and-forget `tokio::spawn`
//! after each conversation turn, so there is no long-running worker to
//! poll. Instead this endpoint computes the pipeline's health from the
//! rows it has actually produced in `user_facts`: total counts, activity
//! windows, per-kind breakdown, and the newest update timestamp.
//!
//! When the admin tab reports `facts_last_24h == 0` but the tenant has
//! active conversations, that's the signal extraction is broken.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use pierre_memory::UserFactMetrics;

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};
use crate::models::TenantId;

use super::AdminApiContext;

/// Query parameters for the memory worker metrics endpoint.
#[derive(Debug, Deserialize)]
pub struct MemoryMetricsQuery {
    /// Tenant to compute metrics for. Admin tokens can span tenants, so
    /// the caller must pick the tenant to scope the aggregation to.
    pub tenant_id: String,
}

/// Response envelope for `GET /admin/memory/worker-metrics`.
///
/// Wraps the domain [`UserFactMetrics`] with the tenant id that was
/// scoped, so the frontend can round-trip it into the tab header without
/// a second request.
#[derive(Debug, Serialize)]
pub struct MemoryWorkerMetricsResponse {
    /// Tenant these metrics were computed against.
    pub tenant_id: String,
    /// The raw metrics snapshot.
    pub metrics: UserFactMetrics,
}

/// Handle `GET /admin/memory/worker-metrics`.
///
/// Requires [`AdminPermission::ViewConfiguration`] — the same permission
/// used by the rest of the harness observability tabs (`HarnessConfigTab`,
/// `ClaimVerdictsTab`, `MemoryPanel`).
pub(super) async fn handle_get_memory_metrics(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<MemoryMetricsQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let tenant: TenantId = params
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;

    let metrics = context
        .repos
        .memory
        .count_user_facts_metrics(tenant)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to compute memory worker metrics");
            AppError::internal(format!("Failed to compute memory worker metrics: {e}"))
        })?;

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        total = metrics.total_facts,
        last_24h = metrics.facts_last_24h,
        "admin fetched memory worker metrics"
    );

    Ok((
        StatusCode::OK,
        Json(MemoryWorkerMetricsResponse {
            tenant_id: params.tenant_id,
            metrics,
        }),
    ))
}
