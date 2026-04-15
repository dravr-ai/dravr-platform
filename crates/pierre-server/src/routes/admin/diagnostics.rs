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
use serde_json::json;
use tracing::error;
use uuid::Uuid;

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

/// POST /admin/diagnostics/tronc-canary
///
/// Emits a synthetic ERROR-level tracing event tagged with a fresh correlation
/// ID. The dravr-tronc error notification layer listens for `Level::ERROR`
/// events and forwards them to Slack (`SLACK_ERROR_CHANNEL`) and email
/// (`NOTIFY_EMAIL_TO`). A scheduled workflow hits this endpoint every few
/// hours and an operator confirms the canary message lands in the channel.
/// If the canary stops arriving, the alerting pipeline is broken BEFORE the
/// next real production outage surfaces the gap.
///
/// Returns the correlation ID so the caller can grep Cloud Logging or Slack
/// to confirm the event round-tripped.
pub async fn handle_tronc_canary(
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let correlation_id = Uuid::new_v4();
    error!(
        correlation_id = %correlation_id,
        event = "tronc-canary",
        "Slack alert pipeline health check (synthetic error, no action required)"
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "emitted",
            "correlation_id": correlation_id,
            "event": "tronc-canary",
            "message": "Synthetic ERROR event emitted — confirm it lands in Slack and email alert channels",
        })),
    ))
}
