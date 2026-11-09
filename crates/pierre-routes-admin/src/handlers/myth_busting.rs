// ABOUTME: Admin route for the myth-busting summary
// ABOUTME: Reads claim verdicts and rolls up the claims agents keep making
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin myth-busting route.
//!
//! `GET /api/admin/myth-busting/summary` wraps
//! [`pierre_services::myth_busting::compute_summary`] for pattern review
//! (display only), over the tenants the scope rule grants the caller. The
//! summary quotes reply sentences, so reading another tenant's writes a row to
//! the audit trail.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use tracing::{error, info};

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};
use pierre_services::myth_busting::{compute_summary, DEFAULT_VERDICT_LIMIT, MAX_VERDICTS_SCANNED};

use super::admin_scope::{audit_cross_tenant_sentences, console_read_scope};
use crate::context::AdminApiContext;

/// Query parameters for the myth-busting summary endpoint.
#[derive(Debug, Deserialize)]
pub struct MythBustingQuery {
    /// What the caller asks to read; see [`pierre_core::admin::scope`].
    pub tenant_id: Option<String>,
    /// Maximum verdicts to scan, clamped to `1..=MAX_VERDICTS_SCANNED`.
    pub limit: Option<i64>,
}

/// The route a cross-tenant read of reply sentences is recorded under.
const MYTH_BUSTING_SUMMARY_ROUTE: &str = "/api/admin/myth-busting/summary";

/// Handle `GET /admin/myth-busting/summary`.
///
/// # Errors
///
/// Returns an error when the scope rule refuses the caller, the tenant id does
/// not parse, or the verdict scan fails.
pub async fn handle_get_summary(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    headers: HeaderMap,
    Query(params): Query<MythBustingQuery>,
) -> AppResult<impl IntoResponse> {
    let scope =
        console_read_scope(&context.repos, &admin_token, params.tenant_id.as_deref()).await?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_VERDICT_LIMIT)
        .clamp(1, MAX_VERDICTS_SCANNED);

    let summary = compute_summary(&context.repos.agent_repos(), scope, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to compute myth-busting summary");
            AppError::internal(format!("Failed to compute myth-busting summary: {e}"))
        })?;

    info!(
        token_id = %admin_token.token_id,
        ?scope,
        verdicts_scanned = summary.verdicts_scanned,
        flagged_total = summary.flagged_total,
        "admin fetched myth-busting summary"
    );

    audit_cross_tenant_sentences(
        &context,
        &admin_token,
        &headers,
        MYTH_BUSTING_SUMMARY_ROUTE,
        scope,
        json!({ "scope": scope, "verdicts_scanned": summary.verdicts_scanned }),
    )
    .await;

    Ok((StatusCode::OK, Json(summary)))
}
