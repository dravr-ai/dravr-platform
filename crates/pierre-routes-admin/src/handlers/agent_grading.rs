// ABOUTME: Admin route exposing per-agent claim-verification content grades for review
// ABOUTME: Wraps services::agent_grading::compute_agent_grades over the scope the caller's role allows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin agent content grading route.
//!
//! Wraps [`pierre_services::agent_grading::compute_agent_grades`] in an axum
//! handler, over the tenants the scope rule grants the caller.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use tracing::{error, info};

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};
use pierre_services::agent_grading::{
    compute_agent_grades, DEFAULT_VERDICT_LIMIT, MAX_VERDICTS_SCANNED,
};

use super::admin_scope::console_read_scope;
use crate::context::AdminApiContext;

/// Query parameters for the agent grading summary endpoint.
#[derive(Debug, Deserialize)]
pub struct AgentGradingQuery {
    /// What the caller asks to read; see [`pierre_core::admin::scope`].
    pub tenant_id: Option<String>,
    /// Maximum verdicts to scan, clamped to `1..=MAX_VERDICTS_SCANNED`.
    pub limit: Option<i64>,
}

/// Handle `GET /admin/agent-grading/summary`.
///
/// # Errors
///
/// Returns an error when the scope rule refuses the caller, the tenant id does
/// not parse, or the verdict scan fails.
pub async fn handle_get_summary(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<AgentGradingQuery>,
) -> AppResult<impl IntoResponse> {
    let scope =
        console_read_scope(&context.repos, &admin_token, params.tenant_id.as_deref()).await?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_VERDICT_LIMIT)
        .clamp(1, MAX_VERDICTS_SCANNED);

    let summary = compute_agent_grades(&context.repos.agent_repos(), scope, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to compute coach grading summary");
            AppError::internal(format!("Failed to compute coach grading summary: {e}"))
        })?;

    info!(
        token_id = %admin_token.token_id,
        ?scope,
        verdicts_scanned = summary.verdicts_scanned,
        agents = summary.grades.len(),
        "admin fetched coach grading summary"
    );

    Ok((StatusCode::OK, Json(summary)))
}
