// ABOUTME: Dashboard route handlers for monitoring and analytics
// ABOUTME: Provides REST endpoints for viewing system status, usage analytics, and request logs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Dashboard routes for monitoring and analytics
//!
//! This module provides endpoints for viewing usage statistics, rate limit status,
//! request logs, and other monitoring data. All handlers require valid JWT authentication.

/// Service layer for dashboard data and analytics operations
pub mod service;

use crate::{errors::AppError, mcp::resources::ServerContext, middleware::AuthenticatedUser};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
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

/// Query parameters for request logs
#[derive(Deserialize)]
struct RequestLogsQuery {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    time_range: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tool: Option<String>,
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
    /// Create all dashboard routes
    ///
    /// Routes are prefixed with /api to match frontend API conventions:
    /// - /api/dashboard/overview - Dashboard overview (status, user, admin)
    /// - /api/dashboard/analytics - Usage analytics with configurable time range
    /// - /api/dashboard/rate-limits - Rate limit status
    /// - /api/dashboard/request-logs - Request logs with filtering
    /// - /api/dashboard/request-stats - Detailed request statistics
    /// - /api/dashboard/tool-usage - Tool usage breakdown
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            // Primary dashboard endpoints matching frontend API calls
            .route(
                "/api/dashboard/overview",
                get(Self::handle_dashboard_overview),
            )
            .route(
                "/api/dashboard/analytics",
                get(Self::handle_usage_analytics),
            )
            .route("/api/dashboard/rate-limits", get(Self::handle_rate_limits))
            .route(
                "/api/dashboard/request-logs",
                get(Self::handle_request_logs),
            )
            .route(
                "/api/dashboard/request-stats",
                get(Self::handle_detailed_stats),
            )
            .route("/api/dashboard/tool-usage", get(Self::handle_tool_usage))
            // Alternative routes without /api prefix
            .route("/dashboard/status", get(Self::handle_dashboard_overview))
            .route("/dashboard/user", get(Self::handle_dashboard_overview))
            .route("/dashboard/admin", get(Self::handle_dashboard_overview))
            .route("/dashboard/detailed", get(Self::handle_detailed_stats))
            .route("/dashboard/usage", get(Self::handle_usage_analytics))
            .route("/dashboard/rate-limits", get(Self::handle_rate_limits))
            .route("/dashboard/logs", get(Self::handle_request_logs))
            .with_state(resources)
    }

    /// Handle dashboard overview request
    async fn handle_dashboard_overview(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let service = DashboardService::new(resources);
        let response = service.get_dashboard_overview(auth).await?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle detailed stats request
    async fn handle_detailed_stats(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let service = DashboardService::new(resources);
        let response = service.get_request_stats(auth, None, None).await?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle usage analytics request
    async fn handle_usage_analytics(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Query(params): Query<UsageAnalyticsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let service = DashboardService::new(resources);
        let response = service.get_usage_analytics(auth, params.days).await?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle rate limits overview request
    async fn handle_rate_limits(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let service = DashboardService::new(resources);
        let response = service.get_rate_limit_overview(auth).await?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle request logs request
    async fn handle_request_logs(
        State(resources): State<Arc<ServerContext>>,
        auth: AuthenticatedUser,
        Query(params): Query<RequestLogsQuery>,
    ) -> Result<Response, AppError> {
        let auth = auth.into_inner();

        let service = DashboardService::new(resources);
        let response = service
            .get_request_logs(
                auth,
                params.api_key.as_deref(),
                params.time_range.as_deref(),
                params.status.as_deref(),
                params.tool.as_deref(),
            )
            .await?;

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Handle tool usage breakdown request
    async fn handle_tool_usage(
        State(resources): State<Arc<ServerContext>>,
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
}
