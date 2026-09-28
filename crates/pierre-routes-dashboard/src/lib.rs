// ABOUTME: Dashboard route handlers for monitoring and analytics
// ABOUTME: Provides REST endpoints for usage analytics and the tool-usage breakdown
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dashboard routes for monitoring and analytics
//!
//! This module provides endpoints for viewing usage analytics and the per-tool
//! usage breakdown. All handlers require valid JWT authentication.
//!
//! The route group is generic over [`pierre_runtime_context::DashboardCtx`] (for
//! repository registry access) and [`pierre_runtime_context::MiddlewareCtx`] (for
//! the `AuthenticatedUser` extractor); the composition root in `pierre-server`
//! implements both traits on its `ServerContext`.

#![warn(missing_docs)]

/// Service layer for dashboard data and analytics operations
pub mod service;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use pierre_core::errors::AppError;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::{DashboardCtx, MiddlewareCtx};
use serde::Deserialize;
use service::DashboardRoutes as DashboardService;
use std::sync::Arc;

/// Query parameters for usage analytics
#[derive(Deserialize)]
struct UsageAnalyticsQuery {
    #[serde(default = "default_days")]
    days: u32,
}

const fn default_days() -> u32 {
    30
}

/// Query parameters for tool usage
#[derive(Deserialize)]
struct ToolUsageQuery {
    #[serde(default)]
    api_key_id: Option<String>,
    #[serde(default = "default_time_range")]
    time_range: String,
}

fn default_time_range() -> String {
    "7d".to_owned()
}

/// Dashboard routes
pub struct DashboardRoutes;

impl DashboardRoutes {
    /// Create all dashboard routes.
    ///
    /// Generic over the runtime context so the crate stays decoupled from
    /// `pierre-server`'s `ServerContext`.
    ///
    /// Routes are prefixed with /api to match frontend API conventions:
    /// - /api/dashboard/analytics - Usage analytics with configurable time range
    /// - /api/dashboard/tool-usage - Tool usage breakdown
    pub fn routes<C>() -> Router<Arc<C>>
    where
        C: DashboardCtx + MiddlewareCtx,
    {
        Router::new()
            .route("/api/dashboard/analytics", get(handle_usage_analytics::<C>))
            .route("/api/dashboard/tool-usage", get(handle_tool_usage::<C>))
    }
}

/// Handle usage analytics request
async fn handle_usage_analytics<C: DashboardCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(params): Query<UsageAnalyticsQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();

    let service = DashboardService::new(resources);
    let response = service.get_usage_analytics(auth, params.days).await?;

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle tool usage breakdown request
async fn handle_tool_usage<C: DashboardCtx + MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    auth: AuthenticatedUser,
    Query(params): Query<ToolUsageQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();

    let service = DashboardService::new(resources);
    let response = service
        .get_tool_usage_breakdown(
            auth,
            params.api_key_id.as_deref(),
            Some(params.time_range.as_str()),
        )
        .await?;

    Ok((StatusCode::OK, Json(response)).into_response())
}
