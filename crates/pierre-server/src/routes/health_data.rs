// ABOUTME: Health data persistence routes for sleep, recovery, health snapshots, and data sources
// ABOUTME: Provides REST endpoints for querying persisted health data from wearable providers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Health data persistence routes
//!
//! This module handles REST endpoints for querying stored health data including
//! sleep sessions, recovery metrics, health snapshots, and connected data sources.
//! All handlers require valid JWT authentication.

use crate::{
    errors::AppError, mcp::resources::ServerResources, middleware::extract_auth_from_headers,
    models::TenantId,
};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{Duration, Utc};
use pierre_auth::auth::AuthResult;
use serde::Deserialize;
use std::sync::Arc;

/// Query parameters for date range filtering on health data endpoints
#[derive(Deserialize, Default)]
struct DateRangeQuery {
    /// Start date in RFC 3339 format (defaults to 30 days ago)
    #[serde(default)]
    start: Option<String>,
    /// End date in RFC 3339 format (defaults to now)
    #[serde(default)]
    end: Option<String>,
}

/// Health data persistence routes
pub struct HealthDataRoutes;

impl HealthDataRoutes {
    /// Create all health data routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route("/health-data/sleep", get(Self::handle_get_sleep))
            .route("/health-data/recovery", get(Self::handle_get_recovery))
            .route(
                "/health-data/snapshots",
                get(Self::handle_get_health_snapshots),
            )
            .route("/health-data/sources", get(Self::handle_list_data_sources))
            .with_state(resources)
    }

    /// Extract and authenticate user from authorization header
    async fn authenticate(
        headers: &HeaderMap,
        resources: &Arc<ServerResources>,
    ) -> Result<AuthResult, AppError> {
        extract_auth_from_headers(headers, resources).await
    }

    /// Get tenant ID for an authenticated user
    ///
    /// Uses `active_tenant_id` from JWT claims if available, otherwise falls back
    /// to the `user_id` as a single-tenant default.
    fn get_tenant_id(auth: &AuthResult) -> TenantId {
        auth.active_tenant_id
            .map_or_else(|| TenantId::from(auth.user_id), TenantId::from)
    }

    /// Parse date range from query params, defaulting to last 30 days
    fn parse_date_range(
        params: &DateRangeQuery,
    ) -> Result<(chrono::DateTime<Utc>, chrono::DateTime<Utc>), AppError> {
        let end = match &params.end {
            Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| {
                    AppError::invalid_input(format!("Invalid end date (expected RFC 3339): {e}"))
                })?,
            None => Utc::now(),
        };

        let start = match &params.start {
            Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| {
                    AppError::invalid_input(format!("Invalid start date (expected RFC 3339): {e}"))
                })?,
            None => Utc::now() - Duration::days(30),
        };

        Ok((start, end))
    }

    /// Handle GET /health-data/sleep - query stored sleep sessions
    async fn handle_get_sleep(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(params): Query<DateRangeQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth);
        let (start, end) = Self::parse_date_range(&params)?;

        let sessions = resources
            .repos
            .sleep
            .get_sleep_sessions(auth.user_id, &tenant_id, start, end)
            .await?;

        Ok((StatusCode::OK, Json(sessions)).into_response())
    }

    /// Handle GET /health-data/recovery - query stored recovery metrics
    async fn handle_get_recovery(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(params): Query<DateRangeQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth);
        let (start, end) = Self::parse_date_range(&params)?;

        let metrics = resources
            .repos
            .recovery
            .get_recovery_metrics(auth.user_id, &tenant_id, start, end)
            .await?;

        Ok((StatusCode::OK, Json(metrics)).into_response())
    }

    /// Handle GET /health-data/snapshots - query stored health snapshots
    async fn handle_get_health_snapshots(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(params): Query<DateRangeQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth);
        let (start, end) = Self::parse_date_range(&params)?;

        let snapshots = resources
            .repos
            .health_snapshots
            .get_health_snapshots(auth.user_id, &tenant_id, start, end)
            .await?;

        Ok((StatusCode::OK, Json(snapshots)).into_response())
    }

    /// Handle GET /health-data/sources - list connected data sources
    async fn handle_list_data_sources(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(&auth);

        let sources = resources
            .repos
            .data_sources
            .list_data_sources(auth.user_id, &tenant_id)
            .await?;

        Ok((StatusCode::OK, Json(sources)).into_response())
    }
}
