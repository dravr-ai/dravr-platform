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
use pierre_core::models::{TenantId, User, UserStatus, UserTier};
use pierre_core::pagination::{Cursor, PaginationParams};
use pierre_database::backends::shared::enums::user_tier_to_str;
use pierre_middleware::redaction::mask_email;
use pierre_services::admin_ops;
use pierre_services::analytics::cache_user_email;
use pierre_services::pre_approval::{self, AllowOutcome};
use pierre_services::user_removal::held_providers;

use super::api_keys::json_response;
use super::types::{
    AdminResponse, AllowEmailRequest, ApproveUserRequest, ListUsersQuery, SuspendUserRequest,
    UserActivityQuery,
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
    /// Account status: `pending`, `active` or `suspended`
    user_status: &'static str,
    /// Whether the account holds an admin role
    is_admin: bool,
    /// When the account was approved, if it has been
    approved_at: Option<String>,
    /// Who approved it, if an operator did
    approved_by: Option<String>,
}

impl From<&User> for UserSummary {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.to_string(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            tier: user.tier.to_string(),
            created_at: user.created_at.to_rfc3339(),
            last_active: user.last_active.to_rfc3339(),
            user_status: user_status_str(user.user_status),
            is_admin: user.is_admin,
            approved_at: user.approved_at.map(|t| t.to_rfc3339()),
            approved_by: user.approved_by.map(|id| id.to_string()),
        }
    }
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

    info!("Listing users by token: {}", admin_token.token_id);

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
        // Propagated as is: an unknown status is the caller's 400, not a 500.
        .inspect_err(|e| error!(error = %e, "Failed to fetch users from database"))?;

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

    let user_summaries: Vec<UserSummary> =
        users.iter().map(|user| UserSummary::from(*user)).collect();

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
/// listing's summary does (status, admin flag, the providers the user holds in
/// each tenant) because the reason to ask about ONE user is usually a field the
/// list does not show.
///
/// # Errors
///
/// Returns an invalid-input error for a malformed id, a not-found error for an
/// unknown user, and a database error when the user or their providers cannot
/// be read.
pub async fn handle_get_user(
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
        warn!(error = %e, "Invalid user ID format");
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

    // The providers the user holds, across tenants: what a delete would
    // disconnect, and what `pierre-cli user delete` previews without --yes.
    let connected_providers = held_providers(&context.repos, user_uuid).await?;
    let manages_roster_operator_grant = context
        .repos
        .users
        .manages_roster_operator_grant(user_uuid)
        .await?;

    info!(token_id = %admin_token.token_id, "Read user {user_id}");

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
                "manages_roster": user.manages_roster,
                "manages_roster_operator_grant": manages_roster_operator_grant,
                "created_at": user.created_at.to_rfc3339(),
                "last_active": user.last_active.to_rfc3339(),
                "connected_providers": connected_providers,
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

    info!("Listing pending users by token: {}", admin_token.token_id);

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

    let user_summaries: Vec<UserSummary> = users.iter().map(UserSummary::from).collect();

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

/// The tenant an admin token acts in: a console session's active tenant, or
/// the tenant a scoped admin token is bound to. `None` for a platform-wide
/// token.
fn token_tenant(token: &ValidatedAdminToken) -> AppResult<Option<TenantId>> {
    token
        .tenant_id
        .as_deref()
        .map(|raw| {
            Uuid::parse_str(raw)
                .map(TenantId::from_uuid)
                .map_err(|e| AppError::invalid_input(format!("Invalid tenant scope: {e}")))
        })
        .transpose()
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
    } else {
        warn!(
            recipient = %mask_email(email),
            "No approval notifier wired — account-approved email not sent"
        );
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
        "Approving user {} by token: {}",
        user_id, admin_token.token_id
    );

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        warn!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    // One approval path for the CLI and the console: the operator behind the
    // token is recorded as the approver, and the user joins the tenant the
    // approver works in. Whether an approved user needs a tenant of their own
    // is decided by the user, not by an operator's request body — the Slack
    // approval path provisions one only `if !has_tenants` (registre#407).
    let updated_user = admin_ops::approve_user(
        &ctx.repos,
        admin_token.operator_user_id,
        token_tenant(&admin_token)?,
        user_uuid,
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
        "Suspending user {} by token: {}",
        user_id, admin_token.token_id
    );

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        warn!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    // Shared get/guard/update-status core (no tenant step on suspension); the
    // operator behind the token is recorded against the transition.
    let updated_user = admin_ops::transition_user_status(
        &ctx.repos,
        user_uuid,
        UserStatus::Suspended,
        admin_token.operator_user_id,
    )
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
        "Issuing password reset token for user {} by token: {}",
        user_id, admin_token.token_id
    );

    let ctx = context.as_ref();
    let user_uuid = Uuid::parse_str(&user_id).map_err(|e| {
        warn!(error = %e, "Invalid user ID format");
        AppError::invalid_input(format!("Invalid user ID format: {e}"))
    })?;

    // A super-admin reaches every user; any other admin reaches only the users
    // of the tenant it acts in (a platform-wide token names none), and a user
    // outside it answers 404 exactly like a missing one.
    let scope = if admin_token.is_super_admin {
        None
    } else {
        token_tenant(&admin_token)?
    };
    let user = admin_ops::find_user_in_admin_scope(&ctx.repos, scope, user_uuid)
        .await
        .inspect_err(|_| warn!("User not found in admin scope: {user_id}"))?;

    // Audit identity: the operator when the token names one, else the service.
    let reset_by = admin_token
        .operator_user_id
        .map_or_else(|| admin_token.service_name.clone(), |id| id.to_string());
    let raw_token = admin_ops::issue_password_reset_token(&ctx.repos, user_uuid, &reset_by).await?;

    info!(
        "Password reset token issued for user {} by token {}",
        user_uuid, admin_token.token_id
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
                "reset_by": reset_by,
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
            data: Some(limits.to_json()),
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

    // Shared implementation: writes users.tier AND the anti-clobber marker,
    // attributed to the operator behind the token when it names one.
    let note = format!("admin tier override via {}", admin_token.service_name);
    let updated = admin_ops::set_user_tier(
        &context.repos,
        user_uuid,
        new_tier.clone(),
        Some(note),
        admin_token.operator_user_id,
    )
    .await?;

    info!(
        target_user_id = %user_uuid,
        target_user_email = %mask_email(&updated.email),
        new_tier = user_tier_to_str(&new_tier),
        token_id = %admin_token.token_id,
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

/// Body for [`handle_set_user_manages_roster`].
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct SetManagesRosterRequest {
    /// Grant (`true`) or revoke (`false`) the permission to become a coaching
    /// group's human coach.
    pub manages_roster: bool,
}

/// Admin: grant or revoke a user's `manages_roster` permission.
///
/// `POST /admin/users/{user_id}/manages-roster`, and the console's
/// `/api/admin/users/{user_id}/manages-roster`. This is how a coach with no
/// `TrainingPeaks` coach account becomes able to redeem a group's coach
/// invite. A grant records the operator behind the token and the time, and a
/// `TrainingPeaks` disconnect never takes it back. Super-admin only, like the
/// tier override: the grant lets its holder coach the members of any group
/// that invites them.
pub(crate) async fn handle_set_user_manages_roster(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(user_id): Path<String>,
    Json(request): Json<SetManagesRosterRequest>,
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

    let updated = admin_ops::set_user_manages_roster(
        &context.repos,
        user_uuid,
        request.manages_roster,
        admin_token.operator_user_id,
    )
    .await?;
    let operator_grant = context
        .repos
        .users
        .manages_roster_operator_grant(user_uuid)
        .await?;

    info!(
        target_user_id = %user_uuid,
        target_user_email = %mask_email(&updated.email),
        manages_roster = request.manages_roster,
        token_id = %admin_token.token_id,
        "Admin manages_roster change applied via token surface"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!(
                "User {} {} coach a group",
                updated.email,
                if updated.manages_roster {
                    "may now"
                } else {
                    "may no longer"
                }
            ),
            data: to_value(json!({
                "user_id": user_uuid.to_string(),
                "email": updated.email,
                "manages_roster": updated.manages_roster,
                "manages_roster_operator_grant": operator_grant,
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
        token_id = %admin_token.token_id,
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

/// Deny a request whose token lacks `ManageUsers`.
pub(crate) fn deny_without_manage_users(token: &ValidatedAdminToken) -> Option<impl IntoResponse> {
    (!token.permissions.has_permission(&AdminPerm::ManageUsers)).then(|| {
        json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: ManageUsers required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        )
    })
}

/// `GET /admin/pre-approved-emails` — the standing allow-list.
pub(crate) async fn handle_list_pre_approved_emails(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }

    let entries = pre_approval::list(&context.repos).await?;

    info!(
        count = entries.len(),
        token_id = %admin_token.token_id,
        "Pre-approved emails listed"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} pre-approved email(s)", entries.len()),
            data: to_value(json!({
                "emails": entries,
                "total": entries.len(),
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `POST /admin/pre-approved-emails` — pre-approve one address.
///
/// The address needs no account: that is what a standing allow is for. When an
/// account does exist and is pending, the allow approves it now, and the user
/// is notified through the same announcement path as `POST /admin/approve-user`.
pub(crate) async fn handle_allow_email(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<AllowEmailRequest>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }

    let ctx = context.as_ref();
    // The operator behind the token — the approving super-admin of a device
    // login, the admin of a console session. A service token names no person,
    // so `allowed_by` stays NULL rather than attributing its allow to some
    // arbitrary admin account.
    let allowed_by = admin_token.operator_user_id;
    let result = pre_approval::allow(
        &ctx.repos,
        &request.email,
        allowed_by,
        request.note.as_deref(),
    )
    .await?;

    if let Some(approved) = result.approved_user.as_ref() {
        announce_approval(
            ctx,
            &approved.id.to_string(),
            approved.id,
            &approved.email,
            approved.display_name.as_deref(),
            &admin_token.service_name,
        )
        .await;
    }

    let invited = send_invite_if_eligible(ctx, &result, request.send_invite).await;
    let recipient = mask_email(&result.email);

    info!(
        %recipient,
        outcome = ?result.outcome,
        send_invite = request.send_invite,
        invited,
        token_id = %admin_token.token_id,
        "Pre-approval allow recorded"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: result.message(),
            data: to_value(json!({
                "email": result.email,
                "outcome": result.outcome,
                "approved_user_id": result.approved_user.as_ref().map(|u| u.id.to_string()),
                "invited": invited,
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// Email the sign-up link when it was asked for and the address has no
/// account, logging which way it went. Returns whether the invite was handed
/// to the notifier.
///
/// Only an address with no account is invited. A pending one was just
/// approved by the allow and gets that announcement instead; an active or
/// suspended account already exists, so a "create your account" link would be
/// wrong.
async fn send_invite_if_eligible(
    ctx: &AdminApiContext,
    result: &pre_approval::AllowResult,
    send_invite: bool,
) -> bool {
    let invited = send_invite
        && matches!(
            result.outcome,
            AllowOutcome::Recorded | AllowOutcome::AlreadyAllowed
        );
    let recipient = mask_email(&result.email);
    if invited {
        if let Some(notifier) = ctx.approval_notifier.as_ref() {
            notifier.notify_user_invited(&result.email).await;
        } else {
            warn!(%recipient, "No approval notifier wired — invitation email not sent");
        }
    } else if send_invite {
        info!(
            %recipient,
            outcome = ?result.outcome,
            "Invitation requested but not sent: the address already has an account"
        );
    }
    invited
}

/// `DELETE /admin/pre-approved-emails/{email}` — drop a standing allow.
///
/// An account that already registered against the address keeps whatever
/// status it holds; removing the allow only stops a *future* registration from
/// skipping the queue.
pub(crate) async fn handle_disallow_email(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(email): Path<String>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }

    let result = pre_approval::disallow(&context.repos, &email).await?;

    info!(
        removed = result.removed,
        token_id = %admin_token.token_id,
        "Pre-approval removal processed"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: result.message(),
            data: to_value(json!({
                "email": result.email,
                "removed": result.removed,
                "account_status": result.account_status,
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
