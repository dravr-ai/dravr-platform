// ABOUTME: Admin user management route handlers
// ABOUTME: Handles user listing, approval, suspension, deletion, password reset, rate limits, and activity
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Serialize;
use serde_json::{json, to_value};
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_core::admin::models::{AdminPermission as AdminPerm, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{UserStatus, UserTier};
use pierre_core::pagination::{Cursor, PaginationParams};
use pierre_database::backends::shared::enums::user_tier_to_str;
use pierre_database::RepositoryRegistry;
use pierre_services::admin_ops;
use pierre_services::analytics::cache_user_email;
use pierre_services::tenant_admin as tenant_admin_service;

use super::api_keys::json_response;
use super::types::{
    AdminResponse, ApproveUserRequest, DeleteUserRequest, ListUsersQuery, SuspendUserRequest,
    TenantCreatedInfo, UserActivityQuery,
};
use crate::context::AdminApiContext;

/// User list response
#[derive(Debug, Clone, Serialize)]
pub(crate) struct UserListResponse {
    /// List of users (sanitized - no passwords)
    users: Vec<UserSummary>,
    /// Number of users in THIS page, not in the table.
    ///
    /// Named `total` since the endpoint shipped, and kept for compatibility;
    /// `has_more` is what tells a caller whether the listing is finished.
    total: usize,
    /// Cursor to pass back as `?cursor=` for the next page. `None` = last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
    /// Whether another page exists after this one.
    has_more: bool,
}

/// Sanitized user summary for listing
#[derive(Debug, Clone, Serialize)]
pub(crate) struct UserSummary {
    /// User ID
    id: String,
    /// User email
    email: String,
    /// Display name
    display_name: Option<String>,
    /// User tier
    tier: String,
    /// When user was created
    created_at: String,
    /// Last active time
    last_active: String,
}

/// Get user status string
pub(crate) const fn user_status_str(status: UserStatus) -> &'static str {
    match status {
        UserStatus::Pending => "pending",
        UserStatus::Active => "active",
        UserStatus::Suspended => "suspended",
    }
}

/// Handle user listing
pub(crate) async fn handle_list_users(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<ListUsersQuery>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!("Listing users by service: {}", admin_token.service_name);

    let ctx = context.as_ref();

    let status = params.status.as_deref().unwrap_or("active");

    // Paginate rather than reading the table. `limit` and `offset` were declared
    // on this query since the endpoint shipped and neither was ever read — the
    // handler called `get_by_status(status, None)` and returned everything, so a
    // caller asking for ten users got all of them and a growing table would have
    // been serialised whole on every call. The repository already had cursor
    // pagination; only this handler was ignoring it.
    let page_size = params.page_size();
    let pagination =
        PaginationParams::forward(params.cursor.clone().map(Cursor::from_string), page_size);

    let page = ctx
        .repos
        .users
        .get_by_status_cursor(status, &pagination)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fetch users from database");
            AppError::internal(format!("Failed to fetch users: {e}"))
        })?;

    // Tier filtering happens here rather than in SQL because the repository's
    // cursor query keys on status. Applied after the page is read, so a filtered
    // page can be shorter than `limit` — `has_more` and `next_cursor` still
    // describe the underlying listing, which is what a caller pages on.
    let tier_filter = params.tier.as_deref().map(str::to_ascii_lowercase);
    let users: Vec<_> = page
        .items
        .iter()
        .filter(|user| {
            tier_filter
                .as_ref()
                .is_none_or(|want| user.tier.to_string().eq_ignore_ascii_case(want))
        })
        .collect();

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

    let total = user_summaries.len();

    info!(
        returned = total,
        page_size,
        has_more = page.has_more,
        "Retrieved users page"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Retrieved {total} users"),
            data: to_value(UserListResponse {
                users: user_summaries,
                total,
                next_cursor: page.next_cursor.map(|c| c.as_str().to_owned()),
                has_more: page.has_more,
            })
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Read one user by id.
///
/// `/admin/users/{user_id}` carried only `delete` until now, so an operator
/// could remove a user but never read one — `pierre-cli user get <email>` hit a
/// 405 on a path that plainly looked like a fetch. Returns more than the
/// listing's summary does (status, admin flag) because the reason to
/// ask about ONE user is usually a field the list does not show.
pub(crate) async fn handle_get_user(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        error!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    let user = context
        .as_ref()
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fetch user from database");
            AppError::internal(format!("Failed to fetch user: {e}"))
        })?
        .ok_or_else(|| {
            warn!("User not found: {user_id}");
            AppError::not_found("User not found")
        })?;

    info!(service = %admin_token.service_name, "Read user {user_id}");

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Retrieved {}", user.email),
            data: Some(json!({
                "id": user.id.to_string(),
                "email": user.email,
                "display_name": user.display_name,
                "tier": user_tier_to_str(&user.tier),
                "status": user_status_str(user.user_status),
                "is_admin": user.is_admin,
                "created_at": user.created_at.to_rfc3339(),
                "last_active": user.last_active.to_rfc3339(),
            })),
        },
        StatusCode::OK,
    ))
}

/// Handle pending users listing
pub(crate) async fn handle_pending_users(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!(
        "Listing pending users by service: {}",
        admin_token.service_name
    );

    let ctx = context.as_ref();

    let users = ctx
        .repos
        .users
        .get_by_status("pending", None)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fetch pending users from database");
            AppError::internal(format!("Failed to fetch pending users: {e}"))
        })?;

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

    info!("Retrieved {} pending users", count);

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Retrieved {count} pending users"),
            data: to_value(json!({
                "count": count,
                "users": user_summaries
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle tenant creation and linking for user approval
///
/// Delegates to `TenantAdminService` for slug validation and tenant provisioning.
async fn create_and_link_tenant(
    repos: &RepositoryRegistry,
    user_uuid: Uuid,
    user_email: &str,
    request: &ApproveUserRequest,
    display_name: Option<&str>,
) -> AppResult<Option<TenantCreatedInfo>> {
    if !request.create_default_tenant.unwrap_or(false) {
        return Ok(None);
    }

    let tenant = tenant_admin_service::provision_tenant_for_approval(
        repos,
        user_uuid,
        user_email,
        display_name,
        request.tenant_name.as_deref(),
        request.tenant_slug.as_deref(),
    )
    .await?;

    Ok(Some(TenantCreatedInfo {
        tenant_id: tenant.id.to_string(),
        name: tenant.name,
        slug: tenant.slug,
        plan: tenant.plan,
    }))
}

/// Announce an approval: raise the operator notify event, then tell the user.
///
/// The identity cache is warmed first because the notify enricher only attaches
/// `user_email` when the record carries `user_id` and that id resolves — an
/// approved account is not otherwise guaranteed to be cached, and without both
/// the Slack message loses the subject entirely.
async fn announce_approval(
    ctx: &AdminApiContext,
    user_id: &str,
    user_uuid: Uuid,
    email: &str,
    display_name: Option<&str>,
    approved_by: &str,
) {
    cache_user_email(user_id, email);
    info!(
        target: "notify",
        event = "user.approved",
        user_id = %user_id,
        approved_by = %approved_by,
        "user account approved"
    );

    // Notify the user: approval email + a message on each linked channel.
    if let Some(notifier) = ctx.approval_notifier.as_ref() {
        notifier
            .notify_user_approved(user_uuid, email, display_name)
            .await;
    }
}

/// Handle user approval workflow
pub(crate) async fn handle_approve_user(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(request): Json<ApproveUserRequest>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!(
        "Approving user {} by service: {}",
        user_id, admin_token.service_name
    );

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        error!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    // Shared get/guard/update-status core; the admin-token surface layers its
    // own tenant provisioning (named tenant from request body) below.
    let updated_user =
        admin_ops::transition_user_status(&ctx.repos, user_uuid, UserStatus::Active, None).await?;

    let tenant_created = create_and_link_tenant(
        &ctx.repos,
        user_uuid,
        &updated_user.email,
        &request,
        updated_user.display_name.as_deref(),
    )
    .await?;

    let reason = request.reason.as_deref().unwrap_or("No reason provided");
    info!("User {} approved successfully. Reason: {}", user_id, reason);

    announce_approval(
        ctx,
        &user_id,
        user_uuid,
        &updated_user.email,
        updated_user.display_name.as_deref(),
        &admin_token.service_name,
    )
    .await;

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "User approved successfully".to_owned(),
            data: to_value(json!({
                "user": {
                    "id": updated_user.id.to_string(),
                    "email": updated_user.email,
                    "user_status": user_status_str(updated_user.user_status),
                    "approved_by": updated_user.approved_by,
                    "approved_at": updated_user.approved_at.map(|t| t.to_rfc3339()),
                },
                "tenant_created": tenant_created,
                "reason": reason
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle user suspension workflow
pub(crate) async fn handle_suspend_user(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(request): Json<SuspendUserRequest>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!(
        "Suspending user {} by service: {}",
        user_id, admin_token.service_name
    );

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        error!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    // Shared get/guard/update-status core (no tenant step on suspension).
    let updated_user =
        admin_ops::transition_user_status(&ctx.repos, user_uuid, UserStatus::Suspended, None)
            .await?;

    let reason = request.reason.as_deref().unwrap_or("No reason provided");
    info!(
        "User {} suspended successfully. Reason: {}",
        user_id, reason
    );

    cache_user_email(&user_id, &updated_user.email);
    info!(
        target: "notify",
        event = "user.suspended",
        user_id = %user_id,
        suspended_by = %admin_token.service_name,
        "user account suspended"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "User suspended successfully".to_owned(),
            data: to_value(json!({
                "user": {
                    "id": updated_user.id.to_string(),
                    "email": updated_user.email,
                    "user_status": user_status_str(updated_user.user_status),
                },
                "reason": reason
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle user deletion workflow
///
/// Permanently deletes a user and all associated data (cascades via foreign keys).
/// This action cannot be undone.
pub(crate) async fn handle_delete_user(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(request): Json<DeleteUserRequest>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!(
        "Deleting user {} by service: {}",
        user_id, admin_token.service_name
    );

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        error!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    let user = ctx
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fetch user from database");
            AppError::internal(format!("Failed to fetch user: {e}"))
        })?
        .ok_or_else(|| {
            warn!("User not found: {}", user_id);
            AppError::not_found("User not found")
        })?;

    let user_email = user.email.clone();

    ctx.repos.users.delete(user_uuid).await.map_err(|e| {
        error!(error = %e, "Failed to delete user from database");
        AppError::internal(format!("Failed to delete user: {e}"))
    })?;

    let reason = request.reason.as_deref().unwrap_or("No reason provided");
    info!(
        "User {} ({}) deleted successfully. Reason: {}",
        user_id, user_email, reason
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "User deleted successfully".to_owned(),
            data: to_value(json!({
                "deleted_user": {
                    "id": user_id,
                    "email": user_email,
                },
                "reason": reason
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle password reset for a user (admin only)
///
/// Issues a one-time reset token instead of a temporary password. The admin
/// delivers the token to the user, who then calls `POST /api/auth/complete-reset`
/// with the token and their chosen new password. The token expires after 1 hour
/// and can only be used once.
pub(crate) async fn handle_reset_user_password(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    info!(
        "Issuing password reset token for user {} by service: {}",
        user_id, admin_token.service_name
    );

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        error!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    // Admin-token surface uses an unscoped global lookup (no admin tenant).
    let user = ctx
        .repos
        .users
        .get_global(user_uuid)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fetch user from database");
            AppError::internal(format!("Failed to fetch user: {e}"))
        })?
        .ok_or_else(|| {
            warn!("User not found: {}", user_id);
            AppError::not_found("User not found")
        })?;

    // Shared token-generation + storage core; audit identity is the service name.
    let raw_token =
        admin_ops::issue_password_reset_token(&ctx.repos, user_uuid, &admin_token.service_name)
            .await?;

    info!(
        "Password reset token issued for user {} by service {}",
        user.email, admin_token.service_name
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Password reset token issued".to_owned(),
            data: to_value(json!({
                "user_id": user_uuid.to_string(),
                "email": user.email,
                "reset_token": raw_token,
                "expires_in_seconds": admin_ops::PASSWORD_RESET_TTL_SECONDS,
                "reset_by": admin_token.service_name,
                "note": "Deliver this token to the user. They must call POST /api/auth/complete-reset with the token and their new password within 1 hour."
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle getting rate limit info for a user
pub(crate) async fn handle_get_user_rate_limit(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

    let limits = admin_ops::compute_user_rate_limits(&ctx.repos, user_uuid).await?;

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Rate limit information retrieved".to_owned(),
            data: to_value(json!({
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
                "override_active": limits.override_active,
                "override_note": limits.override_note,
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle getting user activity logs
pub(crate) async fn handle_get_user_activity(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Query(params): Query<UserActivityQuery>,
) -> AppResult<impl IntoResponse> {
    if !admin_token
        .permissions
        .has_permission(&AdminPerm::ManageUsers)
    {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

    let activity = admin_ops::compute_user_activity(&ctx.repos, user_uuid, params.days).await?;

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "User activity retrieved".to_owned(),
            data: to_value(json!({
                "user_id": activity.user_id,
                "period_days": activity.period_days,
                "total_requests": activity.total_requests,
                "top_tools": activity.top_tools,
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Body for [`handle_set_user_tier`].
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct SetUserTierRequest {
    /// `"starter"`, `"professional"`, or `"enterprise"`. Anything else
    /// returns 400 — silent fallback to Starter would mask typos.
    pub tier: String,
}

/// Admin: change a user's billing tier (Starter / Professional /
/// Enterprise).
///
/// Restricted to super-admin tokens because the tier write is
/// orthogonal to `ManageUsers` — Stripe webhook drives Stripe-paid
/// upgrades; this route is the human-operator backdoor for QA, comp
/// accounts, and overrides outside the Stripe loop. Auditable via
/// the existing `tracing::info` on the path.
pub(crate) async fn handle_set_user_tier(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(request): Json<SetUserTierRequest>,
) -> AppResult<impl IntoResponse> {
    if !admin_token.is_super_admin {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: super-admin token required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

    let new_tier = match request.tier.to_ascii_lowercase().as_str() {
        "starter" => UserTier::Starter,
        "professional" => UserTier::Professional,
        "enterprise" => UserTier::Enterprise,
        other => {
            return Err(AppError::invalid_input(format!(
                "Unknown tier '{other}' — expected starter, professional, or enterprise",
            )));
        }
    };

    // Shared implementation: writes users.tier AND the anti-clobber marker
    // (set_by = None — the admin token is a service token, not a user UUID).
    let note = format!("admin tier override via {}", admin_token.service_name);
    let updated = admin_ops::set_user_tier(
        &context.repos,
        user_uuid,
        new_tier.clone(),
        Some(note),
        None,
    )
    .await?;

    info!(
        target_user_id = %user_uuid,
        target_user_email = %updated.email,
        new_tier = user_tier_to_str(&new_tier),
        admin_service = %admin_token.service_name,
        "Admin tier change applied via token surface"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!(
                "User {} tier set to {}",
                updated.email,
                user_tier_to_str(&new_tier)
            ),
            data: to_value(json!({
                "user_id": user_uuid.to_string(),
                "email": updated.email,
                "tier": user_tier_to_str(&new_tier),
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Admin: clear a user's tier override so the Stripe webhook drives the
/// tier again.
///
/// `DELETE /admin/users/{user_id}/tier`. Removes the marker recorded by
/// [`handle_set_user_tier`]; the user's current `users.tier` is left as-is
/// and the next Stripe billing event re-syncs it. Super-admin only, for
/// symmetry with the set path.
pub(crate) async fn handle_clear_user_tier_override(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if !admin_token.is_super_admin {
        return Ok(json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: super-admin token required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        ));
    }

    let user_uuid = Uuid::parse_str(&user_id)
        .map_err(|e| AppError::invalid_input(format!("Invalid user ID format: {e}")))?;

    let removed = admin_ops::clear_user_tier_override(&context.repos, user_uuid).await?;

    info!(
        target_user_id = %user_uuid,
        admin_service = %admin_token.service_name,
        removed,
        "Admin tier override cleared via token surface"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: if removed {
                "Tier override cleared; billing webhook will drive the tier".to_owned()
            } else {
                "No tier override was set for this user".to_owned()
            },
            data: to_value(json!({
                "user_id": user_uuid.to_string(),
                "removed": removed,
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}
