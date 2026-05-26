// ABOUTME: Admin diagnostics endpoints for system observability and resource measurement
// ABOUTME: Provides tool schema size estimation and tronc-canary alerting probe
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin diagnostics routes for system observability.
//!
//! Provides endpoints to inspect tool schema sizes, token budgets, and
//! other internal metrics useful for capacity planning. Lives in
//! `pierre-server` rather than `pierre-routes-admin` because the schema
//! size endpoint depends on the [`pierre_tool_runtime::registry::ToolRegistry`]
//! type, which is internal to the composition root.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_routes_admin::auth::middleware::admin_auth_middleware;
use pierre_routes_admin::auth::service::AdminAuthService;
use serde_json::json;
use tracing::error;
use uuid::Uuid;

use pierre_core::errors::AppResult;
use pierre_tool_runtime::registry::ToolRegistry;

/// Focused context for the diagnostics sub-routes.
///
/// Carries the [`ToolRegistry`] needed by
/// `/admin/diagnostics/tool-schema-size`. Diagnostic routes do not need
/// the full [`pierre_routes_admin::AdminApiContext`] surface.
#[derive(Clone)]
pub struct DiagnosticsContext {
    /// Tool registry — primary input for schema-size estimation.
    pub tool_registry: Arc<ToolRegistry>,
}

/// Mount the diagnostics sub-routes behind the admin-token JWT middleware.
///
/// Called from `mcp::multitenant` alongside [`pierre_routes_admin::AdminRoutes::routes`]
/// so the human-facing admin token surface includes the diagnostic
/// endpoints without forcing them through the leaf crate.
pub fn routes(context: DiagnosticsContext, auth_service: AdminAuthService) -> Router {
    let context = Arc::new(context);
    Router::new()
        .route(
            "/admin/diagnostics/tool-schema-size",
            get(handle_tool_schema_size),
        )
        .route("/admin/diagnostics/tronc-canary", post(handle_tronc_canary))
        .with_state(context)
        .layer(middleware::from_fn_with_state(
            auth_service,
            admin_auth_middleware,
        ))
}

/// `GET /admin/diagnostics/tool-schema-size`.
///
/// Returns the estimated token cost of all registered MCP tool schemas,
/// broken down per tool and sorted by token cost descending.
///
/// # Errors
///
/// Returns an error when the caller's admin token lacks
/// [`AdminPermission::ViewConfiguration`].
pub async fn handle_tool_schema_size(
    State(context): State<Arc<DiagnosticsContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let estimate = context.tool_registry.total_schema_token_estimate();

    Ok((StatusCode::OK, Json(estimate)))
}

/// `POST /admin/diagnostics/tronc-canary`.
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
///
/// # Errors
///
/// Returns an error when the caller's admin token lacks
/// [`AdminPermission::ViewConfiguration`].
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
