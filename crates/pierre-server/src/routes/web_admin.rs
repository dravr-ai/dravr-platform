// ABOUTME: Web-facing admin routes for authenticated admin users via browser
// ABOUTME: Uses cookie-based auth (same as /api/keys) for users with is_admin=true
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Web Admin Routes
//!
//! This module provides admin endpoints accessible via browser cookie authentication.
//! Unlike `/admin/*` routes which require admin service tokens, these routes
//! accept standard user authentication for users with `is_admin: true`.

use crate::{
    admin::{models::CreateAdminTokenRequest, AdminPermission},
    config::social::SocialInsightsConfig,
    errors::AppError,
    mcp::resources::ServerResources,
    middleware::{extract_auth_from_headers, require_admin},
    services::admin_ops,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use pierre_auth::auth::AuthResult;
use pierre_core::models::TenantId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Response for pending users list
#[derive(Serialize)]
struct PendingUsersResponse {
    count: usize,
    users: Vec<UserSummary>,
}

/// Response for all users list
#[derive(Serialize)]
struct AllUsersResponse {
    users: Vec<UserSummaryFull>,
    total_count: usize,
}

/// Response for admin tokens list
#[derive(Serialize)]
struct AdminTokensResponse {
    admin_tokens: Vec<AdminTokenSummary>,
    total_count: usize,
}

/// Admin token summary for listing
#[derive(Serialize)]
struct AdminTokenSummary {
    id: String,
    service_name: String,
    service_description: Option<String>,
    is_active: bool,
    is_super_admin: bool,
    created_at: String,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    token_prefix: Option<String>,
}

/// Full user summary for listing all users
#[derive(Serialize)]
struct UserSummaryFull {
    id: String,
    email: String,
    display_name: Option<String>,
    tier: String,
    user_status: String,
    is_admin: bool,
    created_at: String,
    last_active: String,
    approved_at: Option<String>,
    approved_by: Option<String>,
}

/// User summary for listing
#[derive(Serialize)]
struct UserSummary {
    id: String,
    email: String,
    display_name: Option<String>,
    tier: String,
    created_at: String,
    last_active: String,
}

/// Request to approve a user
#[derive(Deserialize)]
struct ApproveUserRequest {
    reason: Option<String>,
}

/// Request to suspend a user
#[derive(Deserialize)]
struct SuspendUserRequest {
    reason: Option<String>,
}

/// Request to set a tool override
#[derive(Deserialize)]
struct SetToolOverrideRequest {
    tool_name: String,
    is_enabled: bool,
    reason: Option<String>,
}

/// Request to create an admin token via web admin
#[derive(Deserialize)]
struct CreateAdminTokenWebRequest {
    service_name: String,
    service_description: Option<String>,
    permissions: Option<Vec<String>>,
    is_super_admin: Option<bool>,
    expires_in_days: Option<u64>,
}

/// Response for created admin token
#[derive(Serialize)]
struct CreateAdminTokenWebResponse {
    success: bool,
    token_id: String,
    service_name: String,
    jwt_token: String,
    token_prefix: String,
    is_super_admin: bool,
    expires_at: Option<String>,
}

/// Response for user status change operations
#[derive(Serialize)]
struct UserStatusChangeResponse {
    success: bool,
    message: String,
    user: UserStatusChangeUser,
}

/// User data in status change response
#[derive(Serialize)]
struct UserStatusChangeUser {
    id: String,
    email: String,
    user_status: String,
}

/// Query parameters for user activity endpoint
#[derive(Debug, Deserialize)]
pub struct UserActivityQuery {
    /// Number of days to look back (default: 30)
    pub days: Option<u32>,
}

/// Web admin routes - accessible via browser for admin users
pub struct WebAdminRoutes;

impl WebAdminRoutes {
    /// Create all web admin routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            .route("/api/admin/pending-users", get(Self::handle_pending_users))
            .route("/api/admin/users", get(Self::handle_all_users))
            .route(
                "/api/admin/tokens",
                get(Self::handle_admin_tokens).post(Self::handle_create_admin_token),
            )
            .route(
                "/api/admin/tokens/{token_id}",
                get(Self::handle_get_admin_token),
            )
            .route(
                "/api/admin/tokens/{token_id}/revoke",
                post(Self::handle_revoke_admin_token),
            )
            .route(
                "/api/admin/approve-user/{user_id}",
                post(Self::handle_approve_user),
            )
            .route(
                "/api/admin/suspend-user/{user_id}",
                post(Self::handle_suspend_user),
            )
            .route(
                "/api/admin/users/{user_id}/reset-password",
                post(Self::handle_reset_user_password),
            )
            .route(
                "/api/admin/users/{user_id}/rate-limit",
                get(Self::handle_get_user_rate_limit),
            )
            .route(
                "/api/admin/users/{user_id}/activity",
                get(Self::handle_get_user_activity),
            )
            .route(
                "/api/admin/settings/auto-approval",
                get(Self::handle_get_auto_approval).put(Self::handle_set_auto_approval),
            )
            .route(
                "/api/admin/settings/social-insights",
                get(Self::handle_get_social_insights_config)
                    .put(Self::handle_set_social_insights_config)
                    .delete(Self::handle_reset_social_insights_config),
            )
            // Tool selection routes (web admin versions with cookie auth)
            .route(
                "/api/admin/tools/catalog",
                get(Self::handle_get_tool_catalog),
            )
            .route(
                "/api/admin/tools/catalog/{tool_name}",
                get(Self::handle_get_tool_catalog_entry),
            )
            .route(
                "/api/admin/tools/global-disabled",
                get(Self::handle_get_global_disabled_tools),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}",
                get(Self::handle_get_tenant_tools),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/override",
                post(Self::handle_set_tool_override),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/override/{tool_name}",
                delete(Self::handle_remove_tool_override),
            )
            .route(
                "/api/admin/tools/tenant/{tenant_id}/summary",
                get(Self::handle_get_tool_summary),
            )
            .route(
                "/api/admin/analytics/recent-activity",
                get(Self::handle_recent_activity),
            )
            .with_state(resources)
    }

    /// Authenticate user from authorization header or cookie, requiring admin privileges
    async fn authenticate_admin(
        headers: &HeaderMap,
        resources: &Arc<ServerResources>,
    ) -> Result<AuthResult, AppError> {
        let auth = extract_auth_from_headers(headers, resources).await?;

        // Verify admin privileges using centralized guard
        require_admin(auth.user_id, &resources.repos.users).await?;

        Ok(auth)
    }

    /// Handle pending users listing for web admin users
    async fn handle_pending_users(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        // Authenticate and verify admin status
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            "Web admin listing pending users"
        );

        // Scope listing to admin's tenant (super-admins see all tenants)
        let admin_tenant_id =
            admin_ops::get_admin_tenant_scope(&resources, auth.user_id, auth.active_tenant_id)
                .await?;

        // Fetch users with Pending status
        let users = resources
            .repos
            .users
            .get_by_status("pending", admin_tenant_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch pending users: {e}")))?;

        // Convert to summaries
        let user_summaries: Vec<UserSummary> = users
            .iter()
            .map(|user| UserSummary {
                id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                tier: user.tier.to_string(),
                created_at: user.created_at.to_rfc3339(),
                last_active: user.last_active.to_rfc3339(),
            })
            .collect();

        let count = user_summaries.len();

        info!("Retrieved {count} pending users for web admin");

        Ok((
            StatusCode::OK,
            Json(PendingUsersResponse {
                count,
                users: user_summaries,
            }),
        )
            .into_response())
    }

    /// Handle listing all users for web admin users
    async fn handle_all_users(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        // Authenticate and verify admin status
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            "Web admin listing all users"
        );

        // Scope listing to admin's tenant (super-admins see all tenants)
        let admin_tenant_id =
            admin_ops::get_admin_tenant_scope(&resources, auth.user_id, auth.active_tenant_id)
                .await?;

        // Fetch users by status and combine (no get_all_users method exists)
        let mut all_users = Vec::new();

        for status in ["active", "pending", "suspended"] {
            let users = resources
                .repos
                .users
                .get_by_status(status, admin_tenant_id)
                .await
                .map_err(|e| AppError::internal(format!("Failed to fetch {status} users: {e}")))?;
            all_users.extend(users);
        }

        let users = all_users;

        // Convert to full summaries
        let user_summaries: Vec<UserSummaryFull> = users
            .iter()
            .map(|user| UserSummaryFull {
                id: user.id.to_string(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                tier: user.tier.to_string(),
                user_status: user.user_status.to_string(),
                is_admin: user.is_admin,
                created_at: user.created_at.to_rfc3339(),
                last_active: user.last_active.to_rfc3339(),
                approved_at: user.approved_at.map(|d| d.to_rfc3339()),
                approved_by: user.approved_by.map(|id| id.to_string()),
            })
            .collect();

        let total_count = user_summaries.len();

        info!("Retrieved {total_count} users for web admin");

        Ok((
            StatusCode::OK,
            Json(AllUsersResponse {
                users: user_summaries,
                total_count,
            }),
        )
            .into_response())
    }

    /// Handle listing admin tokens for web admin users
    async fn handle_admin_tokens(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        // Authenticate and verify admin status
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            "Web admin listing admin tokens"
        );

        // Fetch admin tokens (include_inactive = false for active tokens only)
        let tokens = resources
            .repos
            .admin
            .list_tokens(false)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch admin tokens: {e}")))?;

        // Convert to summaries
        let token_summaries: Vec<AdminTokenSummary> = tokens
            .iter()
            .map(|token| AdminTokenSummary {
                id: token.id.clone(),
                service_name: token.service_name.clone(),
                service_description: token.service_description.clone(),
                is_active: token.is_active,
                is_super_admin: token.is_super_admin,
                created_at: token.created_at.to_rfc3339(),
                expires_at: token.expires_at.map(|d| d.to_rfc3339()),
                last_used_at: token.last_used_at.map(|d| d.to_rfc3339()),
                token_prefix: Some(token.token_prefix.clone()),
            })
            .collect();

        let total_count = token_summaries.len();

        info!("Retrieved {total_count} admin tokens for web admin");

        Ok((
            StatusCode::OK,
            Json(AdminTokensResponse {
                admin_tokens: token_summaries,
                total_count,
            }),
        )
            .into_response())
    }

    /// Handle approving a user via web admin (cookie auth)
    async fn handle_approve_user(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Json(request): Json<ApproveUserRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_user_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin approving user"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result = admin_ops::approve_user(
            &resources,
            auth.user_id,
            auth.active_tenant_id,
            user_uuid,
            request.reason.as_deref(),
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(UserStatusChangeResponse {
                success: true,
                message: "User approved successfully".to_owned(),
                user: UserStatusChangeUser {
                    id: result.user_id,
                    email: result.email,
                    user_status: result.user_status,
                },
            }),
        )
            .into_response())
    }

    /// Handle suspending a user via web admin (cookie auth)
    async fn handle_suspend_user(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Json(request): Json<SuspendUserRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_user_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin suspending user"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result = admin_ops::suspend_user(
            &resources,
            auth.user_id,
            auth.active_tenant_id,
            user_uuid,
            request.reason.as_deref(),
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(UserStatusChangeResponse {
                success: true,
                message: "User suspended successfully".to_owned(),
                user: UserStatusChangeUser {
                    id: result.user_id,
                    email: result.email,
                    user_status: result.user_status,
                },
            }),
        )
            .into_response())
    }

    /// Build a `CreateAdminTokenRequest` from the web request payload
    fn build_admin_token_request(request: CreateAdminTokenWebRequest) -> CreateAdminTokenRequest {
        let permissions = request.permissions.map(|perms| {
            perms
                .iter()
                .filter_map(|p| p.parse::<AdminPermission>().ok())
                .collect::<Vec<_>>()
        });

        CreateAdminTokenRequest {
            service_name: request.service_name,
            service_description: request.service_description,
            permissions,
            expires_in_days: request.expires_in_days,
            is_super_admin: request.is_super_admin.unwrap_or(false),
        }
    }

    /// Handle creating an admin token via web admin (cookie auth)
    async fn handle_create_admin_token(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Json(request): Json<CreateAdminTokenWebRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        // Only super-admins can create super-admin tokens
        if request.is_super_admin.unwrap_or(false) {
            admin_ops::require_super_admin(auth.user_id, &resources).await?;
        }

        info!(
            user_id = %auth.user_id,
            service_name = %request.service_name,
            "Web admin creating admin token"
        );

        let token_request = Self::build_admin_token_request(request);

        // Generate token using database method
        let generated_token = resources
            .repos
            .admin
            .create_token(
                &token_request,
                &resources.admin_jwt_secret,
                &resources.jwks_manager,
            )
            .await
            .map_err(|e| AppError::internal(format!("Failed to create admin token: {e}")))?;

        info!(
            token_id = %generated_token.token_id,
            "Admin token created successfully via web admin"
        );

        Ok((
            StatusCode::CREATED,
            Json(CreateAdminTokenWebResponse {
                success: true,
                token_id: generated_token.token_id,
                service_name: generated_token.service_name,
                jwt_token: generated_token.jwt_token,
                token_prefix: generated_token.token_prefix,
                is_super_admin: generated_token.is_super_admin,
                expires_at: generated_token.expires_at.map(|t| t.to_rfc3339()),
            }),
        )
            .into_response())
    }

    /// Handle getting a specific admin token via web admin (cookie auth)
    async fn handle_get_admin_token(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(token_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            token_id = %token_id,
            "Web admin getting admin token details"
        );

        let token = resources
            .repos
            .admin
            .get_token_by_id(&token_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to fetch admin token: {e}")))?
            .ok_or_else(|| AppError::not_found(format!("Admin token {token_id}")))?;

        Ok((
            StatusCode::OK,
            Json(AdminTokenSummary {
                id: token.id,
                service_name: token.service_name,
                service_description: token.service_description,
                is_active: token.is_active,
                is_super_admin: token.is_super_admin,
                created_at: token.created_at.to_rfc3339(),
                expires_at: token.expires_at.map(|d| d.to_rfc3339()),
                last_used_at: token.last_used_at.map(|d| d.to_rfc3339()),
                token_prefix: Some(token.token_prefix),
            }),
        )
            .into_response())
    }

    /// Handle revoking an admin token via web admin (cookie auth)
    async fn handle_revoke_admin_token(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(token_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            token_id = %token_id,
            "Web admin revoking admin token"
        );

        resources
            .repos
            .admin
            .deactivate_token(&token_id)
            .await
            .map_err(|e| AppError::internal(format!("Failed to revoke admin token: {e}")))?;

        info!(
            "Admin token {} revoked successfully via web admin",
            token_id
        );

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Admin token revoked successfully",
                "token_id": token_id
            })),
        )
            .into_response())
    }

    /// Handle password reset via web admin
    ///
    /// Issues a one-time reset token instead of returning a temporary password.
    /// The admin delivers the token to the user, who calls `POST /api/auth/complete-reset`
    /// with the token and their chosen new password. Token expires after 1 hour.
    async fn handle_reset_user_password(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            admin_id = %auth.user_id,
            target_user_id = %user_id,
            "Web admin issuing password reset token"
        );

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let result = admin_ops::generate_password_reset_token(
            &resources,
            auth.user_id,
            auth.active_tenant_id,
            user_uuid,
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Password reset token issued",
                "data": {
                    "reset_token": result.reset_token,
                    "expires_in_seconds": result.expires_in_seconds,
                    "user_email": result.user_email,
                    "note": "Deliver this token to the user. They must call POST /api/auth/complete-reset with the token and their new password within 1 hour."
                }
            })),
        )
            .into_response())
    }

    /// Handle getting rate limit info for a user via web admin
    async fn handle_get_user_rate_limit(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let limits = admin_ops::compute_user_rate_limits(
            &resources,
            auth.user_id,
            auth.active_tenant_id,
            user_uuid,
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Rate limit information retrieved",
                "data": {
                    "user_id": limits.user_id,
                    "tier": limits.tier,
                    "rate_limits": {
                        "daily": {
                            "limit": limits.daily_limit,
                            "used": limits.daily_used,
                            "remaining": limits.daily_remaining,
                        },
                        "monthly": {
                            "limit": limits.monthly_limit,
                            "used": limits.monthly_used,
                            "remaining": limits.monthly_remaining,
                        },
                    },
                    "reset_times": {
                        "daily_reset": limits.daily_reset.to_rfc3339(),
                        "monthly_reset": limits.monthly_reset.to_rfc3339(),
                    },
                }
            })),
        )
            .into_response())
    }

    /// Handle getting user activity via web admin
    async fn handle_get_user_activity(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(user_id): Path<String>,
        Query(params): Query<UserActivityQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        let user_uuid = Uuid::parse_str(&user_id)
            .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

        let activity = admin_ops::compute_user_activity(
            &resources,
            auth.user_id,
            auth.active_tenant_id,
            user_uuid,
            params.days,
        )
        .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "User activity retrieved",
                "data": {
                    "user_id": activity.user_id,
                    "period_days": activity.period_days,
                    "total_requests": activity.total_requests,
                    "top_tools": activity.top_tools,
                }
            })),
        )
            .into_response())
    }

    /// Handle getting auto-approval setting
    async fn handle_get_auto_approval(
        headers: HeaderMap,
        State(resources): State<Arc<ServerResources>>,
    ) -> Result<impl IntoResponse, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let settings = admin_ops::get_auto_approval_settings(&resources).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Auto-approval setting retrieved",
                "data": {
                    "enabled": settings.enabled,
                    "auto_approve_domains": settings.auto_approve_domains,
                    "description": "When enabled, all new registrations are auto-approved. \
                        When disabled, only emails from auto_approve_domains are auto-approved."
                }
            })),
        )
            .into_response())
    }

    /// Handle setting auto-approval
    async fn handle_set_auto_approval(
        headers: HeaderMap,
        State(resources): State<Arc<ServerResources>>,
        Json(request): Json<serde_json::Value>,
    ) -> Result<impl IntoResponse, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        let enabled = request
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| AppError::invalid_input("Missing or invalid 'enabled' field"))?;

        info!(
            user_id = %auth.user_id,
            enabled = enabled,
            "Setting auto-approval"
        );

        admin_ops::set_auto_approval(&resources, enabled).await?;

        info!(
            user_id = %auth.user_id,
            enabled = enabled,
            "Auto-approval setting updated"
        );

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Auto-approval has been {}", if enabled { "enabled" } else { "disabled" }),
                "data": {
                    "enabled": enabled,
                    "description": "When enabled, new user registrations are automatically approved without admin intervention"
                }
            })),
        )
            .into_response())
    }

    // =========================================================================
    // Social Insights Configuration Routes (web admin versions with cookie auth)
    // =========================================================================

    /// GET `/api/admin/settings/social-insights` - Get social insights configuration
    async fn handle_get_social_insights_config(
        headers: HeaderMap,
        State(resources): State<Arc<ServerResources>>,
    ) -> Result<impl IntoResponse, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let config = admin_ops::get_social_insights_config(&resources).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Social insights configuration retrieved",
                "data": config
            })),
        )
            .into_response())
    }

    /// PUT `/api/admin/settings/social-insights` - Update social insights configuration
    async fn handle_set_social_insights_config(
        headers: HeaderMap,
        State(resources): State<Arc<ServerResources>>,
        Json(config): Json<SocialInsightsConfig>,
    ) -> Result<impl IntoResponse, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            "Updating social insights configuration"
        );

        admin_ops::set_social_insights_config(&resources, &config).await?;

        info!(
            user_id = %auth.user_id,
            "Social insights configuration updated"
        );

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Social insights configuration updated",
                "data": config
            })),
        )
            .into_response())
    }

    /// DELETE `/api/admin/settings/social-insights` - Reset social insights to defaults
    async fn handle_reset_social_insights_config(
        headers: HeaderMap,
        State(resources): State<Arc<ServerResources>>,
    ) -> Result<impl IntoResponse, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;

        info!(
            user_id = %auth.user_id,
            "Resetting social insights configuration to defaults"
        );

        let default_config = admin_ops::reset_social_insights_config(&resources).await?;

        info!(
            user_id = %auth.user_id,
            "Social insights configuration reset to defaults"
        );

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": "Social insights configuration reset to defaults",
                "data": default_config
            })),
        )
            .into_response())
    }

    // =========================================================================
    // Tool Selection Routes (web admin versions with cookie auth)
    // =========================================================================

    /// GET `/api/admin/tools/catalog` - List all tools in catalog
    async fn handle_get_tool_catalog(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let catalog = resources.tool_selection.get_catalog().await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Retrieved {} tools from catalog", catalog.len()),
                "data": catalog
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tools/catalog/{tool_name}` - Get single tool details
    async fn handle_get_tool_catalog_entry(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(tool_name): Path<String>,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let catalog = resources.tool_selection.get_catalog().await?;
        let entry = catalog
            .into_iter()
            .find(|e| e.tool_name == tool_name)
            .ok_or_else(|| AppError::not_found(format!("Tool '{tool_name}'")))?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Retrieved tool '{tool_name}'"),
                "data": entry
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tools/global-disabled` - List globally disabled tools
    async fn handle_get_global_disabled_tools(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let disabled_tools = resources.tool_selection.get_globally_disabled_tools();
        let count = disabled_tools.len();

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": if count == 0 {
                    "No tools are globally disabled".to_owned()
                } else {
                    format!("{count} tool(s) globally disabled via PIERRE_DISABLED_TOOLS")
                },
                "data": {
                    "disabled_tools": disabled_tools,
                    "count": count
                }
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/tools/tenant/{tenant_id}` - Get effective tools for tenant
    async fn handle_get_tenant_tools(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(tenant_id): Path<TenantId>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources, auth.user_id, tenant_id).await?;

        let tools = resources
            .tool_selection
            .get_effective_tools(tenant_id)
            .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Retrieved {} effective tools for tenant {tenant_id}", tools.len()),
                "data": tools
            })),
        )
            .into_response())
    }

    /// POST `/api/admin/tools/tenant/{tenant_id}/override` - Set tool override
    async fn handle_set_tool_override(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(tenant_id): Path<TenantId>,
        Json(request): Json<SetToolOverrideRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources, auth.user_id, tenant_id).await?;

        info!(
            "Setting tool override: tenant={}, tool={}, enabled={}, by={}",
            tenant_id, request.tool_name, request.is_enabled, auth.user_id
        );

        let override_entry = resources
            .tool_selection
            .set_tool_override(
                tenant_id,
                &request.tool_name,
                request.is_enabled,
                auth.user_id,
                request.reason.clone(),
            )
            .await?;

        let action = if request.is_enabled {
            "enabled"
        } else {
            "disabled"
        };

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!("Tool '{}' {} for tenant {tenant_id}", request.tool_name, action),
                "data": override_entry
            })),
        )
            .into_response())
    }

    /// DELETE `/api/admin/tools/tenant/{tenant_id}/override/{tool_name}` - Remove override
    async fn handle_remove_tool_override(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path((tenant_id, tool_name)): Path<(TenantId, String)>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources, auth.user_id, tenant_id).await?;

        info!(
            "Removing tool override: tenant={}, tool={}, by={}",
            tenant_id, tool_name, auth.user_id
        );

        let deleted = resources
            .tool_selection
            .remove_tool_override(tenant_id, &tool_name)
            .await?;

        if deleted {
            Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "message": format!("Override removed for tool '{tool_name}' on tenant {tenant_id}")
                })),
            )
                .into_response())
        } else {
            Err(AppError::not_found(format!(
                "No override found for tool '{tool_name}' on tenant {tenant_id}"
            )))
        }
    }

    /// GET `/api/admin/tools/tenant/{tenant_id}/summary` - Get availability summary
    async fn handle_get_tool_summary(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(tenant_id): Path<TenantId>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate_admin(&headers, &resources).await?;
        admin_ops::verify_admin_tenant_access(&resources, auth.user_id, tenant_id).await?;

        let summary = resources
            .tool_selection
            .get_availability_summary(tenant_id)
            .await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "message": format!(
                    "Tenant {tenant_id}: {}/{} tools enabled",
                    summary.enabled_tools, summary.total_tools
                ),
                "data": summary
            })),
        )
            .into_response())
    }

    /// GET `/api/admin/analytics/recent-activity` - Real-time activity feed for admin dashboard
    ///
    /// Returns recent LLM calls, recent conversations, and summary stats for the
    /// activity tab polling endpoint.
    async fn handle_recent_activity(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
    ) -> Result<Response, AppError> {
        Self::authenticate_admin(&headers, &resources).await?;

        let activity = admin_ops::fetch_recent_activity(&resources).await?;

        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "recent_llm_calls": activity.recent_llm_calls,
                "recent_conversations": activity.recent_conversations,
                "summary": {
                    "active_conversations": activity.summary.active_conversations,
                    "llm_calls_today": activity.summary.llm_calls_today,
                    "total_tokens_today": activity.summary.total_tokens_today,
                    "estimated_cost_today": activity.summary.estimated_cost_today,
                }
            })),
        )
            .into_response())
    }
}
