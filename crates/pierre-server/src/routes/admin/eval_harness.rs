// ABOUTME: Admin route exposing the pierre-evals golden fixture browser (Sprint C16)
// ABOUTME: Read-only — lists fixtures + per-case summaries. Live eval runs are a later sprint.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin eval harness route.
//!
//! Wraps [`crate::services::eval_harness::browse_fixtures`] in an
//! axum handler gated behind [`AdminPermission::ViewConfiguration`].
//! Only compiled when the `tools-verification` feature is enabled
//! because `pierre-evals` is an optional dependency.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use tokio::task::spawn_blocking;
use tracing::{error, info};

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};
use crate::services::eval_harness::browse_fixtures;

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
