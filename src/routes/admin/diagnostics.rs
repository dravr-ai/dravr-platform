// ABOUTME: Admin diagnostics endpoints for system observability and resource measurement
// ABOUTME: Provides tool schema size estimation and other diagnostic data for capacity planning
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin diagnostics routes for system observability.
//!
//! Provides endpoints to inspect tool schema sizes, token budgets,
//! and other internal metrics useful for capacity planning.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};

use crate::{
    admin::models::{AdminPermission, ValidatedAdminToken},
    errors::{AppError, AppResult},
};

use super::AdminApiContext;

/// GET /admin/diagnostics/tool-schema-size
///
/// Returns the estimated token cost of all registered MCP tool schemas,
/// broken down per tool and sorted by token cost descending.
pub async fn handle_tool_schema_size(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let registry = context
        .tool_registry
        .as_ref()
        .ok_or_else(|| AppError::internal("Tool registry not available in this context"))?;

    let estimate = registry.total_schema_token_estimate();

    Ok((StatusCode::OK, Json(estimate)))
}
