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
use serde_json::{json, to_value, Value};
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_core::admin::models::{AdminPermission as AdminPerm, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::pagination::{Cursor, PaginationParams};
use pierre_database::backends::shared::enums::user_tier_to_str;
use pierre_middleware::redaction::mask_email;
use pierre_services::admin_ops::{self, AccountCaller, AdminStandingLost};
use pierre_services::analytics::cache_user_email;
use pierre_services::onboarding_reset::kept_for_groups;
use pierre_services::user_removal::held_providers;

use super::api_keys::json_response;
use super::linked_channels;
use super::types::{
    AdminResponse, ApproveUserRequest, ListUsersQuery, SuspendUserRequest, UserActivityQuery,
};
use super::user_removal::{account_in_reach, parse_user_id};
use crate::context::AdminApiContext;
use dravr_tronc::dates::Timestamp;

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
    /// The account's role, `admin` or `user`: what a client decides from
    role: &'static str,
    /// Whether the account holds the admin role; `role` said another way
    is_admin: bool,
    /// When the account was approved, if it has been
    approved_at: Option<String>,
    /// Who approved it, if an admin did
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
            role: user.role.as_str(),
            is_admin: user.role.is_admin(),
            approved_at: user.approved_at.map(Timestamp::to_rfc3339),
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

/// Handle user listing.
///
/// A token scoped to one tenant lists the members of that tenant only, the
/// accounts its read by id finds ([`user_read_scope`]); an admin's console
/// session and a super-admin token list every tenant's, and a token bound
/// to no tenant is refused (403).
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
        .get_by_status_cursor(status, user_read_scope(&admin_token)?, &pagination)
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
/// `/admin/users/{user_id}` carried only `delete` until now, so an admin
/// could remove a user but never read one — `pierre-cli user get <email>` hit a
/// 405 on a path that plainly looked like a fetch. Returns more than the
/// listing's summary does (status, admin flag, the providers the user holds in
/// each tenant) because the reason to ask about ONE user is usually a field the
/// list does not show.
///
/// A token bound to one tenant reads a member of that tenant, and only what
/// of it belongs to that tenant: the providers it holds there and the
/// messaging identities linked there, the groups it owns there. The preview
/// of a whole-account reset, a route only a super-admin token takes, is left
/// out of its answer.
///
/// # Errors
///
/// Returns an invalid-input error for a malformed id, a permission error for
/// a token that reaches no tenant, a not-found error for an unknown user and
/// one outside a bound token's tenant, and a database error when the user or
/// their providers cannot be read.
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

    let user_uuid = parse_user_id(&user_id)?;

    // A scoped token reads only the users of its tenant, and one outside it
    // answers 404 like a missing one, as the password reset does: the read
    // names the account's providers and its linked messaging identities.
    let scope = user_read_scope(&admin_token)?;
    let user = admin_ops::find_user_in_admin_scope(&context.repos, scope, user_uuid)
        .await
        .inspect_err(|_| warn!("User not found in admin scope: {user_id}"))?;

    // The providers the user holds: what a delete would disconnect, and what
    // `pierre-cli user delete` previews without --yes. Across tenants for a
    // caller that reaches every tenant; a bound token's own tenant's only.
    let connected_providers: Vec<_> = held_providers(&context.repos, user_uuid)
        .await?
        .into_iter()
        .filter(|held| scope.is_none_or(|own| held.tenant_id == own))
        .collect();
    // Which of them an onboarding reset keeps, which
    // `pierre-cli user reset-onboarding` previews without --yes: this server's
    // answer, not the CLI build's.
    let kept_on_onboarding_reset: Vec<&str> = connected_providers
        .iter()
        .map(|held| held.provider.as_str())
        .filter(|provider| kept_for_groups(provider))
        .collect();
    let manages_roster_admin_grant = context
        .repos
        .users
        .manages_roster_admin_grant(user_uuid)
        .await?;
    let owned_active_groups: Vec<Value> = context
        .repos
        .account_reset
        .owned_active_groups(user_uuid)
        .await?
        .into_iter()
        .filter(|group| scope.is_none_or(|own| group.tenant_id == own))
        .map(|group| json!({ "id": group.group_id, "name": group.name }))
        .collect();

    let linked_channels = match scope {
        None => linked_channels::for_user(&context.repos, user_uuid).await?,
        Some(own) => linked_channels::for_user_in_tenant(&context.repos, own, user_uuid).await?,
    };
    info!(token_id = %admin_token.token_id, "Read user {user_id}");

    let mut data = json!({
        "id": user.id.to_string(),
        "email": user.email,
        "display_name": user.display_name,
        "tier": user_tier_to_str(&user.tier),
        "status": user_status_str(user.user_status),
        "role": user.role.as_str(),
        "is_admin": user.role.is_admin(),
        "manages_roster": user.manages_roster,
        "manages_roster_admin_grant": manages_roster_admin_grant,
        "created_at": user.created_at.to_rfc3339(),
        "last_active": user.last_active.to_rfc3339(),
        "connected_providers": connected_providers,
        "kept_on_onboarding_reset": kept_on_onboarding_reset,
        "owned_active_groups": owned_active_groups,
        "linked_channels": linked_channels,
    });
    // What would refuse a whole-account reset, which `pierre-cli user reset`
    // previews without --yes instead of promising a wipe the server refuses:
    // as it stands, and once the groups the account owns are archived
    // (`--delete-owned-groups`). The reset is a super-admin route reaching
    // every tenant, so a bound token, which cannot take it, is not shown
    // what it would find in the others.
    if scope.is_none() {
        data["reset_blockers"] = json!(
            context
                .repos
                .account_reset
                .reset_blockers(user_uuid, false)
                .await?
        );
        data["reset_blockers_with_owned_groups_deleted"] = json!(
            context
                .repos
                .account_reset
                .reset_blockers(user_uuid, true)
                .await?
        );
    }

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Retrieved {}", user.email),
            data: Some(data),
        },
        StatusCode::OK,
    ))
}

/// Every pending member of `tenant`, newest first: the pending list of a
/// token scoped to one tenant, read through the listing the user list pages.
async fn pending_members(ctx: &AdminApiContext, tenant: TenantId) -> AppResult<Vec<User>> {
    let mut members = Vec::new();
    let mut cursor = None;
    loop {
        let page = PaginationParams::forward(cursor, PENDING_MEMBERS_PAGE);
        let page = ctx
            .repos
            .users
            .get_by_status_cursor("pending", Some(tenant), &page)
            .await?;
        members.extend(page.items);
        match page.next_cursor {
            Some(next) if page.has_more => cursor = Some(next),
            _ => return Ok(members),
        }
    }
}

/// How many accounts [`pending_members`] reads at a time.
const PENDING_MEMBERS_PAGE: usize = 100;

/// Handle pending users listing.
///
/// A token scoped to one tenant lists the pending members of that tenant
/// only ([`user_read_scope`]). A sign-up is a member of no tenant until it
/// is approved, so such a token is listed none of the sign-ups its read by
/// id would not find either. An admin's console session and a super-admin
/// token list them all, and a token bound to no tenant is refused (403).
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

    let users = match user_read_scope(&admin_token)? {
        None => ctx.repos.users.get_by_status("pending", None).await,
        Some(tenant) => pending_members(ctx, tenant).await,
    }
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

/// The tenant a caller's reads of users are confined to, `None` for a
/// caller that reads every tenant's: what the credential reaches
/// ([`ValidatedAdminToken::reach`], the scope rule's one reading).
///
/// One rule for the read of a user by id, the two lists and the per-user
/// reads (rate limits, activity), and for which account a write finds
/// (`account_in_reach`): an admin's console session, like a super-admin
/// token, reads every user; a token scoped to one tenant reads the members
/// of that tenant (`admin_ops::find_user_in_admin_scope`, the paged
/// listing's tenant filter), and anyone else is answered as a missing
/// account is. A token bound to no tenant without the super-admin flag
/// reaches no tenant and is refused (carnet#1589).
///
/// # Errors
/// `PermissionDenied` for a token that reaches no tenant, `InvalidInput`
/// for one whose tenant is not a tenant id.
pub(super) fn user_read_scope(token: &ValidatedAdminToken) -> AppResult<Option<TenantId>> {
    Ok(token.reach()?.tenant_filter())
}

/// The tenant an admin token acts in: a console session's active tenant, or
/// the tenant a scoped admin token is bound to. `None` for a platform-wide
/// token.
pub(super) fn token_tenant(token: &ValidatedAdminToken) -> AppResult<Option<TenantId>> {
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

/// Announce an approval: raise the admin notify event, then tell the user.
///
/// The identity cache is warmed first because the notify enricher only attaches
/// `user_email` when the record carries `user_id` and that id resolves — an
/// approved account is not otherwise guaranteed to be cached, and without both
/// the Slack message loses the subject entirely.
pub(super) async fn announce_approval(
    ctx: &AdminApiContext,
    user_id: &str,
    user_uuid: Uuid,
    email: &str,
    display_name: Option<&str>,
    approved_by: &str,
) {
    cache_user_email(user_id, email);
    // The tenant sign-up keyed its events on, so user.approved lands on the
    // same identity instead of the shared `service` one. A failed lookup only
    // costs the attribution.
    let tenant_id = ctx
        .repos
        .tenants
        .list_for_user(user_uuid)
        .await
        .ok()
        .and_then(|tenants| Tenant::attribution_tenant(&tenants, user_uuid));
    info!(
        target: "notify",
        event = "user.approved",
        user_id = %user_id,
        tenant_id = tenant_id.map(display),
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

/// Handle user approval workflow.
///
/// The account is read as every write reads it (`account_in_reach`): one
/// outside a bound token's tenant answers 404; an admin (a suspended one,
/// reinstated) is approved only by an admin's console session or a
/// super-admin token, 403 otherwise; and a token scoped to one tenant
/// approves only a user held entirely within it, 403 otherwise, as it
/// deletes one.
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
    let caller = AccountCaller::of_token(&admin_token);
    // Approving a suspended admin hands back every power, so it takes the
    // rank suspending one does, and sets the account active in every tenant
    // it is in, so it takes the token's tenant too.
    let (acting, _) = account_in_reach(ctx, &admin_token, &caller, &user_id).await?;
    let user_uuid = acting.id();

    // One approval for every surface (the CLI, the console, the Slack room):
    // the admin behind the token is recorded as the approver, and the user
    // joins the tenant the approver works in. Whether an approved user needs
    // a tenant of their own is decided by the user, not by an admin's request
    // body: one that belongs to none is given its own (registre#407).
    let updated_user = admin_ops::approve_user(&ctx.repos, &acting, token_tenant(&admin_token)?)
        .await?
        .into_approved()?
        .user;

    let reason = request.reason.as_deref().unwrap_or("No reason provided");
    info!("User {} approved successfully. Reason: {}", user_id, reason);

    announce_approval(
        ctx,
        &user_id,
        user_uuid,
        &updated_user.email,
        updated_user.display_name.as_deref(),
        caller.name(),
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
                    "approved_at": updated_user.approved_at.map(Timestamp::to_rfc3339),
                },
                "reason": reason
            }))
            .ok(),
        },
        StatusCode::OK,
    ))
}

/// Handle user suspension workflow.
///
/// Over this API an admin is suspended only by an admin's console session or
/// a super-admin token (403 otherwise); the last active admin by no one, and
/// no admin by its own session or device login (400). An account outside a
/// bound token's tenant answers 404, and a token scoped to one tenant
/// suspends only a user held entirely within it, 403 otherwise, as it
/// deletes one (`account_in_reach`).
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
    let caller = AccountCaller::of_token(&admin_token);
    // Suspending an admin withdraws its device logins and impersonations, so
    // it takes the rank removing one does: a token without the super-admin
    // flag suspends users, never the admins above it. And it signs the
    // account out of every tenant it is in, so it takes the token's tenant.
    let (acting, _) = account_in_reach(ctx, &admin_token, &caller, &user_id).await?;
    let user_uuid = acting.id();

    // Shared get/guard/update-status core (no tenant step on suspension); the
    // admin behind the token is recorded against the transition, and is
    // the actor an admin's own suspension is refused to: a console session
    // or a device login acts as its admin, a service token as no one.
    let transition =
        admin_ops::transition_user_status(&ctx.repos, &acting, UserStatus::Suspended).await?;
    caller
        .record_withdrawn(
            &ctx.repos,
            &transition.withdrawn,
            AdminStandingLost::Deactivated,
            Some(user_uuid),
        )
        .await;
    let updated_user = transition.user;

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
        suspended_by = %caller.name(),
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
///
/// The account is read as every write reads it (`account_in_reach`): a
/// token scoped to one tenant finds a member of its tenant (404 otherwise),
/// and is handed a reset token as it suspends or deletes: only for a user
/// held entirely within that tenant (403 otherwise, nothing issued). Whoever
/// holds the reset token signs in as the whole account, in every tenant it
/// is in.
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
    // The reset token signs its holder in as the account, so a token without
    // the super-admin flag is handed none for an admin (403), and a bound
    // token none for a user held outside its tenant.
    let caller = AccountCaller::of_token(&admin_token);
    let (acting, _) = account_in_reach(ctx, &admin_token, &caller, &user_id).await?;
    let user_uuid = acting.id();
    let user = acting.account();

    // Audit identity: the admin when the token names one, else the service.
    let reset_by = caller.audit_name();
    let raw_token = admin_ops::issue_password_reset_token(&ctx.repos, &acting).await?;

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

/// Handle getting rate limit info for a user.
///
/// A token scoped to one tenant reads a member of that tenant only; anyone
/// else answers 404, as a missing account does ([`user_read_scope`]).
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

    let scope = user_read_scope(&admin_token)?;
    admin_ops::find_user_in_admin_scope(&ctx.repos, scope, user_uuid).await?;
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

/// Handle getting user activity logs.
///
/// A token scoped to one tenant reads a member of that tenant only; anyone
/// else answers 404, as a missing account does ([`user_read_scope`]).
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

    let scope = user_read_scope(&admin_token)?;
    admin_ops::find_user_in_admin_scope(&ctx.repos, scope, user_uuid).await?;
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
/// upgrades; this route is the human-admin backdoor for QA, comp
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
    // attributed to the admin behind the token when it names one.
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
/// invite. A grant records the admin behind the token and the time, and a
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
    let admin_grant = context
        .repos
        .users
        .manages_roster_admin_grant(user_uuid)
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
                "manages_roster_admin_grant": admin_grant,
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
