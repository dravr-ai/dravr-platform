// ABOUTME: Admin route exposing memory extraction worker health from its run ledger, beside user_facts aggregates
// ABOUTME: Runs, failures and backlog are measured; the scope follows the caller's role, platform-wide for a super admin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Memory extraction worker monitoring.
//!
//! A turn's extraction runs in the background, recorded in a job ledger until
//! it lands. The worker's health is measured from its run ledger: how many runs
//! ended and how, and which extractions are still owed past a run's lease. The
//! `user_facts` aggregates sit beside it as a description of the store, whose
//! rows other writers also touch, and never as a signal of the worker.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::ExtractionRunStats;
use pierre_memory::UserFactMetrics;
use pierre_services::memory_extraction::EXTRACTION_JOB_LEASE;

use super::admin_scope::console_read_scope;
use crate::context::AdminApiContext;

/// Query parameters for the memory worker metrics endpoint.
#[derive(Debug, Deserialize)]
pub struct MemoryMetricsQuery {
    /// What the caller asks to read; see [`pierre_core::admin::scope`].
    pub tenant_id: Option<String>,
}

/// Response envelope for `GET /admin/memory/worker-metrics`.
#[derive(Debug, Serialize)]
pub struct MemoryWorkerMetricsResponse {
    /// The tenants these numbers cover.
    pub scope: AdminScope,
    /// What the worker measurably did.
    pub runs: ExtractionRunStats,
    /// What `user_facts` holds.
    pub metrics: UserFactMetrics,
}

/// Handle `GET /admin/memory/worker-metrics`.
///
/// # Errors
///
/// Returns an error when the scope rule refuses the caller, the tenant id does
/// not parse, or a read fails.
pub async fn handle_get_memory_metrics(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<MemoryMetricsQuery>,
) -> AppResult<impl IntoResponse> {
    let scope =
        console_read_scope(&context.repos, &admin_token, params.tenant_id.as_deref()).await?;

    let now_ms = Utc::now().timestamp_millis();
    let lease_ms = i64::try_from(EXTRACTION_JOB_LEASE.as_millis()).unwrap_or(i64::MAX);
    let owed_since_ms = now_ms.saturating_sub(lease_ms);
    let memory = &context.repos.memory;
    let jobs = &context.repos.memory_extraction_jobs;
    let reads = tokio::try_join!(
        memory.count_user_facts_metrics(scope),
        jobs.extraction_run_stats(scope, now_ms, owed_since_ms),
    );
    let (metrics, runs) = reads.map_err(|e| {
        error!(error = %e, "failed to compute memory worker metrics");
        AppError::internal(format!("Failed to compute memory worker metrics: {e}"))
    })?;

    info!(
        token_id = %admin_token.token_id,
        ?scope,
        total = metrics.total_facts,
        last_24h = metrics.facts_last_24h,
        runs_failed_24h = runs.last_24h.failed,
        backlog = runs.backlog,
        "admin fetched memory worker metrics"
    );

    Ok((
        StatusCode::OK,
        Json(MemoryWorkerMetricsResponse {
            scope,
            runs,
            metrics,
        }),
    ))
}
