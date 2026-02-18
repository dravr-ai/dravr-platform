// ABOUTME: Usage status REST endpoint for quota and usage information
// ABOUTME: Provides GET /api/usage/status with daily/weekly counters and reset times
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Usage status routes
//!
//! Provides a JWT-authenticated endpoint for users to check their current
//! usage quotas, counter values, and reset times.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;

use crate::{
    auth::AuthResult,
    database::repositories::{ChatRepository, TenantRepository},
    errors::AppError,
    mcp::resources::ServerResources,
    models::TenantId,
    security::cookies::get_cookie_value,
    services::usage_counter::{LimitCheckResult, UsageCounterService},
};

/// Usage status response containing all quota information
#[derive(Debug, Serialize)]
pub struct UsageStatusResponse {
    /// Daily counter statuses
    pub daily: DailyUsageStatus,
    /// Weekly counter statuses
    pub weekly: WeeklyUsageStatus,
    /// Resource limit statuses
    pub resources: ResourceUsageStatus,
}

/// Daily usage counters with limit info
#[derive(Debug, Serialize)]
pub struct DailyUsageStatus {
    /// Daily message quota status
    pub messages: LimitCheckResult,
    /// Daily token budget status
    pub tokens: LimitCheckResult,
    /// Daily tool calls status
    pub tool_calls: LimitCheckResult,
}

/// Weekly usage counters with limit info
#[derive(Debug, Serialize)]
pub struct WeeklyUsageStatus {
    /// Weekly message quota status
    pub messages: LimitCheckResult,
    /// Weekly token budget status
    pub tokens: LimitCheckResult,
    /// Weekly tool calls status
    pub tool_calls: LimitCheckResult,
}

/// Resource limit statuses (non-counter based)
#[derive(Debug, Serialize)]
pub struct ResourceUsageStatus {
    /// Current number of conversations
    pub conversations: i64,
    /// Maximum allowed conversations
    pub max_conversations: i64,
    /// Current number of coaches
    pub coaches: i64,
    /// Maximum allowed coaches
    pub max_coaches: i64,
}

/// Usage routes handler
pub struct UsageRoutes;

impl UsageRoutes {
    /// Create usage status routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route("/api/usage/status", get(Self::get_status))
            .with_state(resources)
    }

    /// Extract and authenticate user from authorization header or cookie
    pub(crate) async fn authenticate(
        headers: &HeaderMap,
        resources: &Arc<ServerResources>,
    ) -> Result<AuthResult, AppError> {
        let auth_value =
            if let Some(auth_header) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
                auth_header.to_owned()
            } else if let Some(token) = get_cookie_value(headers, "auth_token") {
                format!("Bearer {token}")
            } else {
                return Err(AppError::auth_invalid(
                    "Missing authorization header or cookie",
                ));
            };

        resources
            .auth_middleware
            .authenticate_request(Some(&auth_value))
            .await
            .map_err(|e| AppError::auth_invalid(format!("Authentication failed: {e}")))
    }

    /// Get user's `tenant_id` (defaults to `user_id` if no tenant)
    pub(crate) async fn get_tenant_id(
        user_id: uuid::Uuid,
        resources: &Arc<ServerResources>,
    ) -> Result<TenantId, AppError> {
        let tenants = resources.database.list_for_user(user_id).await?;
        Ok(tenants
            .first()
            .map_or_else(|| TenantId::from(user_id), |t| t.id))
    }

    /// GET /api/usage/status - Get current usage quota status
    async fn get_status(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;
        let user_id_str = auth.user_id.to_string();
        let tenant_id_str = tenant_id.to_string();

        let admin_config = resources
            .admin_config
            .as_ref()
            .ok_or_else(|| AppError::internal("Admin config service not available"))?;

        let usage_svc = UsageCounterService::new(resources.database.as_ref(), admin_config);

        // Fetch all counter statuses in parallel-friendly sequence
        let daily_messages = usage_svc
            .check_limit(&tenant_id_str, &user_id_str, "daily_messages")
            .await?;
        let daily_tokens = usage_svc
            .check_limit(&tenant_id_str, &user_id_str, "daily_tokens")
            .await?;
        let daily_tool_calls = usage_svc
            .check_limit(&tenant_id_str, &user_id_str, "daily_tool_calls")
            .await?;
        let weekly_messages = usage_svc
            .check_limit(&tenant_id_str, &user_id_str, "weekly_messages")
            .await?;
        let weekly_tokens = usage_svc
            .check_limit(&tenant_id_str, &user_id_str, "weekly_tokens")
            .await?;
        let weekly_tool_calls = usage_svc
            .check_limit(&tenant_id_str, &user_id_str, "weekly_tool_calls")
            .await?;

        // Get resource counts for conversations and coaches
        let conversation_count = resources
            .database
            .count_conversations(&user_id_str, tenant_id)
            .await
            .unwrap_or(0);

        let max_conversations = admin_config
            .get_value(
                "usage_quotas.max_active_conversations",
                Some(&tenant_id_str),
            )
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64())
            .unwrap_or(50);

        // Coach count is only available for SQLite (coaches are SQLite-only)
        let coach_count = resources
            .coaches_manager()
            .ok()
            .map(|m| async move { m.count(auth.user_id, tenant_id).await.unwrap_or(0) });
        let coach_count_val = if let Some(future) = coach_count {
            i64::from(future.await)
        } else {
            0
        };

        let max_coaches = admin_config
            .get_value("usage_quotas.max_coaches_per_user", Some(&tenant_id_str))
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64())
            .unwrap_or(20);

        let response = UsageStatusResponse {
            daily: DailyUsageStatus {
                messages: daily_messages,
                tokens: daily_tokens,
                tool_calls: daily_tool_calls,
            },
            weekly: WeeklyUsageStatus {
                messages: weekly_messages,
                tokens: weekly_tokens,
                tool_calls: weekly_tool_calls,
            },
            resources: ResourceUsageStatus {
                conversations: conversation_count,
                max_conversations,
                coaches: coach_count_val,
                max_coaches,
            },
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }
}
