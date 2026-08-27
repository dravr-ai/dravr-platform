// ABOUTME: Admin operations business logic extracted from web_admin route handlers
// ABOUTME: Provides user lifecycle, token management, settings, and analytics services
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Duration, Utc};
use pierre_auth::rate_limiting::UnifiedRateLimitCalculator;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_core::models::{Tenant, TenantPlan, User, UserStatus, UserTier};
use pierre_database::database::repositories::UserMcpTokenRepository;
use pierre_database::database::CreateUserMcpTokenRequest;
use pierre_database::repositories::{UserRateLimitOverride, UserTierOverride, UserToolOverride};
use pierre_database::RepositoryRegistry;
use pierre_llm::pricing::cost_for_record;
use pierre_runtime_context::DataContext;
use serde::Serialize;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

// =========================================================================
// Result types returned by service functions
// =========================================================================

/// Outcome of a user approval operation
pub struct ApproveUserResult {
    /// Updated user ID
    pub user_id: String,
    /// Updated user email
    pub email: String,
    /// New user status
    pub user_status: String,
}

/// Outcome of a user suspension operation
pub struct SuspendUserResult {
    /// Updated user ID
    pub user_id: String,
    /// Updated user email
    pub email: String,
    /// New user status
    pub user_status: String,
}

/// Outcome of a password reset token issuance
pub struct PasswordResetResult {
    /// The raw token to deliver to the user (shown only once)
    pub reset_token: String,
    /// Token lifetime in seconds
    pub expires_in_seconds: u64,
    /// Email address of the target user
    pub user_email: String,
}

/// Rate limit information for a single user
pub struct UserRateLimits {
    /// Target user ID
    pub user_id: String,
    /// Effective tier label
    pub tier: String,
    /// Daily request cap (None means unlimited)
    pub daily_limit: Option<u32>,
    /// Requests made today
    pub daily_used: u32,
    /// Daily quota remaining (None means unlimited)
    pub daily_remaining: Option<u32>,
    /// Monthly request cap (None means unlimited)
    pub monthly_limit: Option<u32>,
    /// Requests made this month
    pub monthly_used: u32,
    /// Monthly quota remaining (None means unlimited)
    pub monthly_remaining: Option<u32>,
    /// Wall-clock time when the daily counter resets
    pub daily_reset: DateTime<Utc>,
    /// Wall-clock time when the monthly counter resets
    pub monthly_reset: DateTime<Utc>,
    /// True when a per-user override is in effect (tier default ignored).
    pub override_active: bool,
    /// Operator note attached to the override, if any.
    pub override_note: Option<String>,
}

/// A single tool usage entry with computed percentage
#[derive(Serialize)]
pub struct ToolUsageEntry {
    /// Tool name as recorded in usage logs
    pub tool_name: String,
    /// Number of invocations in the queried window
    pub call_count: u64,
    /// Share of total invocations (0.0–100.0)
    pub percentage: f64,
}

/// Aggregated user activity over a time period
pub struct UserActivityResult {
    /// Target user ID
    pub user_id: String,
    /// Length of the aggregation window, in days
    pub period_days: i64,
    /// Total requests across all tools in the window
    pub total_requests: u64,
    /// Per-tool usage breakdown, ordered by frequency
    pub top_tools: Vec<ToolUsageEntry>,
}

/// A coach installed by a user (admin User Details panel).
#[derive(Serialize)]
pub struct InstalledCoachSummary {
    /// Installed coach ID
    pub coach_id: String,
    /// Coach display title
    pub title: String,
    /// Coach category (e.g., "endurance", "strength")
    pub category: String,
    /// True if this is the user's default coach
    pub is_default: bool,
}

/// A coaching group a user belongs to (admin User Details panel).
#[derive(Serialize)]
pub struct UserGroupSummary {
    /// Group ID
    pub group_id: String,
    /// Group display name
    pub name: String,
    /// Caller's role within the group (member, admin, etc.)
    pub role: String,
    /// Coach assigned to the group
    pub coach_id: String,
    /// Member count at query time
    pub member_count: i64,
}

/// Per-user enrichments rendered by the admin User Details drawer:
/// coaching persona, default coach, installed coaches, joined groups.
#[derive(Serialize)]
pub struct UserAdminProfile {
    /// Target user ID
    pub user_id: String,
    /// Configured coaching persona label
    pub coaching_persona: String,
    /// Coaches the user has installed from the Coach Store
    pub installed_coaches: Vec<InstalledCoachSummary>,
    /// Coaching groups the user belongs to
    pub joined_groups: Vec<UserGroupSummary>,
}

/// A single LLM call record for the admin activity feed
#[derive(Serialize)]
pub struct LlmCallEntry {
    /// Call record ID
    pub id: String,
    /// LLM provider name (e.g., "openai", "gemini")
    pub provider: String,
    /// Model identifier
    pub model: String,
    /// Combined prompt + completion tokens
    pub total_tokens: i64,
    /// Estimated USD cost for this call
    pub cost_usd: f64,
    /// Call category (chat, tool, embedding, etc.)
    pub call_type: String,
    /// End-to-end execution time in milliseconds
    pub execution_time_ms: Option<i64>,
    /// RFC3339 timestamp when the call was recorded
    pub created_at: String,
}

/// A single conversation record for the admin activity feed
#[derive(Serialize)]
pub struct ConversationEntry {
    /// Conversation ID
    pub id: String,
    /// Conversation title (LLM-generated or user-set)
    pub title: String,
    /// RFC3339 timestamp of the last activity
    pub updated_at: String,
    /// Email of the conversation owner (empty if not resolvable)
    pub user_email: String,
}

/// Summary statistics for the admin dashboard
pub struct ActivitySummary {
    /// Conversations active in the last 15 minutes
    pub active_conversations: i64,
    /// LLM calls recorded since midnight UTC today
    pub llm_calls_today: i64,
    /// Total tokens consumed since midnight UTC today
    pub total_tokens_today: i64,
    /// Estimated USD spend since midnight UTC today
    pub estimated_cost_today: f64,
}

/// Full recent activity payload for the admin dashboard
pub struct RecentActivityResult {
    /// Most recent LLM call records
    pub recent_llm_calls: Vec<LlmCallEntry>,
    /// Most recent conversations
    pub recent_conversations: Vec<ConversationEntry>,
    /// Aggregated counts for the day
    pub summary: ActivitySummary,
}

// =========================================================================
// Tenant scope resolution
// =========================================================================

/// Resolve the admin user's tenant scope for listing queries.
///
/// Super-admins see all tenants (returns `None`). Regular admins are scoped
/// to their `active_tenant_id` from JWT claims (returns `Some(tenant_id)`).
///
/// # Errors
///
/// Returns an error if the admin user cannot be loaded, or if a non-super-admin
/// has no active tenant in their session.
pub async fn get_admin_tenant_scope(
    data: &DataContext,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
) -> Result<Option<TenantId>, AppError> {
    // SECURITY: Global lookup — resolving admin's own tenant scope
    let user = data
        .repos()
        .users
        .get_global(admin_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch admin user: {e}")))?
        .ok_or_else(|| AppError::not_found("Admin user not found"))?;

    // Super-admins see all tenants
    if user.role.is_super_admin() {
        return Ok(None);
    }

    // Use active_tenant_id from JWT claims (admin's selected tenant)
    let tid =
        active_tenant_id.ok_or_else(|| AppError::auth_invalid("No active tenant in session"))?;
    Ok(Some(TenantId::from_uuid(tid)))
}

/// Verify an admin user belongs to the target tenant.
///
/// Super-admin users can access any tenant. Regular admins are restricted
/// to tenants they belong to via the `tenant_users` junction table.
///
/// # Errors
///
/// Returns `PermissionDenied` if the admin does not belong to `target_tenant_id`,
/// or `Internal`/`NotFound` if the admin record or tenant list cannot be loaded.
pub async fn verify_admin_tenant_access(
    data: &DataContext,
    admin_user_id: Uuid,
    target_tenant_id: TenantId,
) -> Result<(), AppError> {
    // SECURITY: Global lookup — verifying admin's own tenant access
    let user = data
        .repos()
        .users
        .get_global(admin_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch admin user: {e}")))?
        .ok_or_else(|| AppError::not_found("Admin user not found"))?;

    // Super-admins can access any tenant
    if user.role.is_super_admin() {
        return Ok(());
    }

    // Regular admins must belong to the target tenant
    let admin_tenants = data
        .repos()
        .tenants
        .list_for_user(admin_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to get admin tenants: {e}")))?;

    let belongs_to_tenant = admin_tenants.iter().any(|t| t.id == target_tenant_id);

    if belongs_to_tenant {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Admin does not belong to the target tenant",
        ))
    }
}

// =========================================================================
// Plan & tier administration (shared by pierre-cli + web admin + token API)
// =========================================================================

/// Set a user's billing/quota tier and record the admin-override marker.
///
/// Writes `users.tier` (the effective tier the running server reads per
/// request) AND upserts a `user_tier_overrides` row so a later Stripe webhook
/// cannot clobber the manual tier. Single shared implementation behind the
/// `pierre-cli user set-tier` command, the cookie `/api/admin` route, and the
/// super-admin token route.
///
/// `set_by` is the acting admin's UUID for the audit trail, or `None` for
/// service tokens that do not map to a user row.
///
/// # Errors
///
/// Returns `Internal` if the tier write or the marker upsert fails.
pub async fn set_user_tier(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tier: UserTier,
    note: Option<String>,
    set_by: Option<Uuid>,
) -> Result<User, AppError> {
    let updated = repos
        .users
        .set_tier(user_id, tier.clone())
        .await
        .map_err(|e| AppError::internal(format!("Failed to set user tier: {e}")))?;

    let now = Utc::now();
    let override_row = UserTierOverride {
        user_id,
        tier,
        note,
        set_by,
        set_at: now,
        updated_at: now,
    };
    repos
        .user_tier_overrides
        .upsert(&override_row)
        .await
        .map_err(|e| AppError::internal(format!("Failed to record tier override: {e}")))?;

    info!(
        target_user_id = %user_id,
        target_user_email = %updated.email,
        new_tier = %updated.tier,
        "Admin tier change applied"
    );
    Ok(updated)
}

/// Clear a user's tier-override marker so the Stripe webhook drives the tier
/// again. Leaves `users.tier` as-is. Returns `true` if a marker was removed.
///
/// # Errors
///
/// Returns `Internal` if the delete fails.
pub async fn clear_user_tier_override(
    repos: &RepositoryRegistry,
    user_id: Uuid,
) -> Result<bool, AppError> {
    let removed = repos
        .user_tier_overrides
        .delete(user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to clear tier override: {e}")))?;
    info!(target_user_id = %user_id, removed, "Admin tier override cleared");
    Ok(removed)
}

/// Set a tenant's plan (`starter` / `professional` / `enterprise`), which gates
/// plan-restricted tools via `tool_catalog.min_plan`.
///
/// Distinct from the Stripe billing webhook — this is the operator backdoor
/// shared by `pierre-cli tenant set-plan` and the web admin route.
///
/// # Errors
///
/// Returns `InvalidInput` for an unknown plan string, or `Internal` if the
/// write fails.
pub async fn set_tenant_plan(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    plan: &str,
) -> Result<Tenant, AppError> {
    let normalized = plan.to_ascii_lowercase();
    if TenantPlan::parse_str(&normalized).is_none() {
        return Err(AppError::invalid_input(format!(
            "Unknown plan '{plan}' — expected starter, professional, or enterprise"
        )));
    }

    let updated = repos
        .tenants
        .set_plan(tenant_id, &normalized)
        .await
        .map_err(|e| AppError::internal(format!("Failed to set tenant plan: {e}")))?;

    info!(tenant_id = %tenant_id, plan = %normalized, "Admin tenant plan change applied");
    Ok(updated)
}

/// Set a per-user tool override (force-enable or force-disable one MCP tool for
/// one user).
///
/// Validates the tool exists in the catalog so an unknown name is a clean
/// not-found rather than an opaque foreign-key error. Shared write path behind
/// `pierre-cli tool enable/disable` and the cookie
/// `/api/admin/tools/user/{id}/override` route. No tool-selection cache
/// invalidation is needed — the per-user overlay is read fresh per request.
///
/// # Errors
///
/// Returns `NotFound` if the tool is not in the catalog, or `Internal` if the
/// database write fails.
pub async fn set_user_tool_override(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tool_name: &str,
    is_enabled: bool,
    set_by: Option<Uuid>,
    reason: Option<String>,
) -> Result<UserToolOverride, AppError> {
    repos
        .tool_selection
        .get_tool_catalog_entry(tool_name)
        .await
        .map_err(|e| AppError::internal(format!("Failed to read tool catalog: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("Tool '{tool_name}'")))?;

    let now = Utc::now();
    let row = UserToolOverride {
        user_id,
        tool_name: tool_name.to_owned(),
        is_enabled,
        set_by,
        reason,
        created_at: now,
        updated_at: now,
    };
    repos
        .user_tool_overrides
        .upsert(&row)
        .await
        .map_err(|e| AppError::internal(format!("Failed to set user tool override: {e}")))?;

    info!(
        target_user_id = %user_id,
        tool_name,
        is_enabled,
        "Per-user tool override applied"
    );
    Ok(row)
}

/// Remove a per-user tool override so the tool reverts to plan/tenant/default.
/// Returns `true` if an override row was removed.
///
/// # Errors
///
/// Returns `Internal` if the delete fails.
pub async fn remove_user_tool_override(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tool_name: &str,
) -> Result<bool, AppError> {
    let removed = repos
        .user_tool_overrides
        .delete(user_id, tool_name)
        .await
        .map_err(|e| AppError::internal(format!("Failed to remove user tool override: {e}")))?;
    info!(target_user_id = %user_id, tool_name, removed, "Per-user tool override removed");
    Ok(removed)
}

/// List a user's explicit per-user tool overrides (for the CLI `tool list` and
/// the web panel's current-state display).
///
/// # Errors
///
/// Returns `Internal` if the read fails.
pub async fn list_user_tool_overrides(
    repos: &RepositoryRegistry,
    user_id: Uuid,
) -> Result<Vec<UserToolOverride>, AppError> {
    repos
        .user_tool_overrides
        .list_for_user(user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to list user tool overrides: {e}")))
}

/// Set or update a per-user rate-limit override.
///
/// Validates that limit values are positive or `None` (unlimited at that
/// dimension); rejects zero. Verifies the target user exists before
/// persisting. Shared by the cookie `PUT
/// /api/admin/users/{id}/rate-limit-override` route and
/// `pierre-cli user set-rate-limit`.
///
/// # Errors
///
/// Returns `InvalidInput` for a zero limit, `NotFound` for a missing user,
/// or `Internal` if persistence fails.
pub async fn set_user_rate_limit_override(
    repos: &RepositoryRegistry,
    target_user_id: Uuid,
    daily_limit: Option<u32>,
    monthly_limit: Option<u32>,
    note: Option<String>,
    set_by: Option<Uuid>,
) -> Result<(), AppError> {
    if matches!(daily_limit, Some(0)) || matches!(monthly_limit, Some(0)) {
        return Err(AppError::invalid_input(
            "Rate limit values must be positive integers; use null for unlimited",
        ));
    }

    // Verify the target user exists (admin views are global).
    repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    let now = Utc::now();
    let row = UserRateLimitOverride {
        user_id: target_user_id,
        daily_limit,
        monthly_limit,
        note,
        set_by,
        set_at: now,
        updated_at: now,
    };

    repos
        .user_rate_limit_overrides
        .upsert(&row)
        .await
        .map_err(|e| AppError::internal(format!("Failed to persist rate-limit override: {e}")))?;

    info!(
        set_by = ?set_by,
        target_user_id = %target_user_id,
        daily_limit = ?daily_limit,
        monthly_limit = ?monthly_limit,
        "Per-user rate-limit override set"
    );

    Ok(())
}

/// Remove a per-user rate-limit override; the user reverts to their tier
/// default. Returns `true` when an override row existed and was removed.
///
/// # Errors
///
/// Returns `Internal` if the delete fails.
pub async fn clear_user_rate_limit_override(
    repos: &RepositoryRegistry,
    target_user_id: Uuid,
) -> Result<bool, AppError> {
    let removed = repos
        .user_rate_limit_overrides
        .delete(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to clear rate-limit override: {e}")))?;

    info!(
        target_user_id = %target_user_id,
        removed,
        "Per-user rate-limit override cleared"
    );

    Ok(removed)
}

// =========================================================================
// User lifecycle helpers
// =========================================================================

/// Auto-create a default MCP token for a newly activated user.
/// This is a non-fatal operation - failure is logged but does not propagate.
pub async fn create_default_mcp_token_for_user(
    database: &dyn UserMcpTokenRepository,
    user_id: Uuid,
) {
    let token_request = CreateUserMcpTokenRequest {
        name: "Default Token".to_owned(),
        expires_in_days: None, // Never expires
    };

    match database.create_token(user_id, &token_request).await {
        Ok(token_result) => {
            info!(
                user_id = %user_id,
                token_id = %token_result.token.id,
                "Auto-created default MCP token for user"
            );
        }
        Err(e) => {
            // Log error but don't fail - user can create token manually
            warn!(
                user_id = %user_id,
                error = %e,
                "Failed to auto-create MCP token for user (non-fatal)"
            );
        }
    }
}

/// Assign a user to the admin's tenant for multi-tenant isolation.
///
/// Uses `active_tenant_id` from the admin's JWT claims to determine the target tenant.
/// If the admin has no active tenant in their session, the assignment is skipped.
///
/// # Errors
///
/// Returns `Internal` if the underlying `users.update_tenant_id` repository call fails.
pub async fn assign_user_to_admin_tenant(
    data: &DataContext,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
) -> Result<(), AppError> {
    if let Some(tid) = active_tenant_id {
        let tenant_id = TenantId::from_uuid(tid);
        // Update user's tenant_id in users table (kept in sync with tenant_users junction)
        data.repos()
            .users
            .update_tenant_id(target_user_id, tenant_id)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to assign user to admin's tenant");
                AppError::internal(format!("Failed to assign tenant: {e}"))
            })?;
        info!(
            user_id = %target_user_id,
            tenant_id = %tenant_id,
            "Assigned approved user to admin's tenant"
        );
    }
    Ok(())
}

/// Verify the authenticated user has super-admin privileges.
///
/// # Errors
///
/// Returns `PermissionDenied` if the user is not a super-admin, or `NotFound`
/// / `Internal` if the user record cannot be loaded.
pub async fn require_super_admin(user_id: Uuid, data: &DataContext) -> Result<(), AppError> {
    // SECURITY: Global lookup — checking admin's own super-admin role
    let user = data
        .repos()
        .users
        .get_global(user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to get user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if !user.role.is_super_admin() {
        warn!(
            user_id = %user_id,
            "Non-super-admin attempted privileged operation"
        );
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Super-admin privileges required to create super-admin tokens",
        ));
    }
    Ok(())
}

// =========================================================================
// User approval and suspension
// =========================================================================

/// Shared user-status state transition: global lookup, already-in-target-state
/// guard, and status update.
///
/// Used by both the session admin surface ([`approve_user`] / [`suspend_user`])
/// and the admin-token route handlers, which layer their own
/// tenant-provisioning steps on top.
///
/// `approved_by` is the audit identity recorded against the transition: the
/// session path passes the acting admin's UUID, the admin-token path passes
/// `None` (the token is not tied to a user row).
///
/// # Errors
///
/// Returns `InvalidInput` if the user is already in `target_status`, `NotFound`
/// if the user does not exist, or `Internal` if a repository call fails.
pub async fn transition_user_status(
    repos: &RepositoryRegistry,
    target_user_id: Uuid,
    target_status: UserStatus,
    approved_by: Option<Uuid>,
) -> Result<User, AppError> {
    // Admin operations use global lookup (users may not have tenant_users entries)
    let user = repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to fetch user from database");
            AppError::internal(format!("Failed to fetch user: {e}"))
        })?
        .ok_or_else(|| {
            warn!("User not found: {}", target_user_id);
            AppError::not_found("User not found")
        })?;

    if user.user_status == target_status {
        let message = match target_status {
            UserStatus::Active => "User is already approved",
            UserStatus::Suspended => "User is already suspended",
            UserStatus::Pending => "User is already pending",
        };
        return Err(AppError::invalid_input(message));
    }

    repos
        .users
        .update_status(target_user_id, target_status, approved_by)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update user status in database");
            AppError::internal(format!("Failed to update user status: {e}"))
        })
}

/// Approve a pending user: validate status, transition to Active, assign tenant,
/// and auto-create a default MCP token.
///
/// # Errors
///
/// Returns `InvalidInput` if the user is already active, `NotFound` if the user
/// does not exist, or `Internal` if any underlying repository call fails.
pub async fn approve_user(
    data: &DataContext,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
    reason: Option<&str>,
) -> Result<ApproveUserResult, AppError> {
    // Use the admin user's UUID as the approver for proper audit trail
    let updated_user = transition_user_status(
        data.repos(),
        target_user_id,
        UserStatus::Active,
        Some(admin_user_id),
    )
    .await?;

    // Assign approved user to admin's tenant for multi-tenant isolation
    assign_user_to_admin_tenant(data, active_tenant_id, target_user_id).await?;

    // Auto-create a default MCP token for the newly approved user
    create_default_mcp_token_for_user(data.repos().user_mcp_tokens.as_ref(), target_user_id).await;

    let reason_text = reason.unwrap_or("No reason provided");
    info!(
        "User {} approved successfully. Reason: {}",
        target_user_id, reason_text
    );

    Ok(ApproveUserResult {
        user_id: updated_user.id.to_string(),
        email: updated_user.email,
        user_status: updated_user.user_status.to_string(),
    })
}

/// Suspend an active user: validate status and transition to Suspended.
///
/// # Errors
///
/// Returns `InvalidInput` if the user is already suspended, `NotFound` if the user
/// does not exist, or `Internal` if the status update fails.
pub async fn suspend_user(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
    reason: Option<&str>,
) -> Result<SuspendUserResult, AppError> {
    // Use the admin user's UUID for audit trail (Note: approved_by is used for both approve/suspend)
    let updated_user = transition_user_status(
        data.repos(),
        target_user_id,
        UserStatus::Suspended,
        Some(admin_user_id),
    )
    .await?;

    let reason_text = reason.unwrap_or("No reason provided");
    info!(
        "User {} suspended successfully. Reason: {}",
        target_user_id, reason_text
    );

    Ok(SuspendUserResult {
        user_id: updated_user.id.to_string(),
        email: updated_user.email,
        user_status: updated_user.user_status.to_string(),
    })
}

// =========================================================================
// Admin privilege management (super-admin only)
// =========================================================================

/// Outcome of an admin privilege change (promote or demote)
pub struct AdminPrivilegeChangeResult {
    /// Updated user ID
    pub user_id: String,
    /// Updated user email
    pub email: String,
    /// New admin status
    pub is_admin: bool,
    /// User role after the change (user, admin, `super_admin`)
    pub role: String,
}

/// Admin user summary for the admins listing
#[derive(Serialize)]
pub struct AdminUserSummary {
    /// User ID
    pub id: String,
    /// User email
    pub email: String,
    /// Display name (optional)
    pub display_name: Option<String>,
    /// User role (admin or `super_admin`)
    pub role: String,
    /// User status (active, pending, suspended)
    pub user_status: String,
    /// Account creation timestamp (RFC3339)
    pub created_at: String,
}

/// Promote a user to admin: set `is_admin = true` and role to Admin (preserving `SuperAdmin`).
///
/// Only super-admins can perform this operation to prevent privilege escalation.
///
/// # Errors
///
/// Returns `PermissionDenied` if the caller is not a super-admin, `InvalidInput`
/// if the target is already an admin, `NotFound` if the target does not exist,
/// or `Internal` on repository failures.
pub async fn promote_user_to_admin(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<AdminPrivilegeChangeResult, AppError> {
    require_super_admin(admin_user_id, data).await?;

    // SECURITY: Global lookup — super-admins promote across tenants
    let user = data
        .repos()
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch target user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if user.is_admin {
        return Err(AppError::invalid_input("User is already an admin"));
    }

    let updated = data
        .repos()
        .users
        .set_admin_status(target_user_id, true)
        .await
        .map_err(|e| AppError::internal(format!("Failed to promote user: {e}")))?;

    info!(
        admin_user_id = %admin_user_id,
        target_user_id = %target_user_id,
        target_email = %updated.email,
        "User promoted to admin"
    );

    Ok(AdminPrivilegeChangeResult {
        user_id: updated.id.to_string(),
        email: updated.email,
        is_admin: updated.is_admin,
        role: updated.role.as_str().to_owned(),
    })
}

/// Demote an admin user: set `is_admin = false` and role to User.
///
/// Only super-admins can perform this operation. Demoting super-admins is rejected
/// at the repository layer to prevent accidental privilege loss. Self-demotion is rejected
/// to avoid locking the caller out of admin actions.
///
/// # Errors
///
/// Returns `PermissionDenied` if the caller is not a super-admin, `InvalidInput`
/// for self-demotion or when the target is not an admin, `NotFound` if the target
/// does not exist, or `Internal` on repository failures.
pub async fn demote_user_from_admin(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<AdminPrivilegeChangeResult, AppError> {
    require_super_admin(admin_user_id, data).await?;

    if admin_user_id == target_user_id {
        return Err(AppError::invalid_input("Cannot demote yourself"));
    }

    let user = data
        .repos()
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch target user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if !user.is_admin {
        return Err(AppError::invalid_input("User is not an admin"));
    }

    let updated = data
        .repos()
        .users
        .set_admin_status(target_user_id, false)
        .await
        .map_err(|e| AppError::internal(format!("Failed to demote user: {e}")))?;

    info!(
        admin_user_id = %admin_user_id,
        target_user_id = %target_user_id,
        target_email = %updated.email,
        "User demoted from admin"
    );

    Ok(AdminPrivilegeChangeResult {
        user_id: updated.id.to_string(),
        email: updated.email,
        is_admin: updated.is_admin,
        role: updated.role.as_str().to_owned(),
    })
}

/// List all admin users across all tenants.
///
/// Only super-admins can view the full admin roster. Returns admins (and super-admins)
/// ordered by email ascending.
///
/// # Errors
///
/// Returns `PermissionDenied` if the caller is not a super-admin, or `Internal`
/// if the admin listing repository call fails.
pub async fn list_all_admins(
    data: &DataContext,
    admin_user_id: Uuid,
) -> Result<Vec<AdminUserSummary>, AppError> {
    require_super_admin(admin_user_id, data).await?;

    let admins = data
        .repos()
        .users
        .list_admins()
        .await
        .map_err(|e| AppError::internal(format!("Failed to list admins: {e}")))?;

    Ok(admins
        .into_iter()
        .map(|u| AdminUserSummary {
            id: u.id.to_string(),
            email: u.email,
            display_name: u.display_name,
            role: u.role.as_str().to_owned(),
            user_status: u.user_status.to_string(),
            created_at: u.created_at.to_rfc3339(),
        })
        .collect())
}

// =========================================================================
// Password reset
// =========================================================================

/// Token validity window for admin-issued password reset tokens, in seconds.
pub const PASSWORD_RESET_TTL_SECONDS: u64 = 3600;

/// Generate a cryptographically random password reset token and store its hash.
///
/// The raw token is returned to be delivered to the user; only the SHA-256 hash
/// is persisted. `reset_by` is the audit identity recorded against the token —
/// callers pass their own (an admin UUID for the session surface, a service
/// name for the admin-token surface). Shared by both password-reset surfaces so
/// the token-generation + storage logic lives once; each caller performs its
/// own (divergent) user lookup beforehand.
///
/// # Errors
///
/// Returns `Internal` if token persistence fails.
pub async fn issue_password_reset_token(
    repos: &RepositoryRegistry,
    target_user_id: Uuid,
    reset_by: &str,
) -> Result<String, AppError> {
    use crate::link_token::generate_link_token;

    // Selector/verifier token: only the verifier's SHA-256 hash is stored; the plaintext
    // selector indexes the row. The full "<selector>.<verifier>" token is returned for
    // delivery to the user and never persisted.
    let generated = generate_link_token();

    repos
        .password_reset
        .store_token(
            target_user_id,
            &generated.selector,
            &generated.verifier_hash,
            reset_by,
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to create reset token: {e}")))?;

    Ok(generated.token)
}

/// Generate a cryptographically random password reset token and store its hash.
///
/// The raw token is returned to be delivered to the user. Only the SHA-256 hash
/// is stored in the database. The token expires after 1 hour.
///
/// # Errors
///
/// Returns `NotFound` if the target user does not exist in the admin's tenant
/// scope, or `Internal` if the user lookup or token persistence fails.
pub async fn generate_password_reset_token(
    data: &DataContext,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
) -> Result<PasswordResetResult, AppError> {
    // Tenant-scoped lookup: admin can only reset passwords for users in their tenant
    let admin_tenant = get_admin_tenant_scope(data, admin_user_id, active_tenant_id).await?;
    let user = if let Some(tid) = admin_tenant {
        data.repos().users.get(target_user_id, tid).await
    } else {
        data.repos().users.get_global(target_user_id).await
    }
    .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
    .ok_or_else(|| AppError::not_found("User not found"))?;

    let admin_id_str = admin_user_id.to_string();
    let raw_token = issue_password_reset_token(data.repos(), target_user_id, &admin_id_str).await?;

    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        "Password reset token issued via web admin"
    );

    Ok(PasswordResetResult {
        reset_token: raw_token,
        expires_in_seconds: PASSWORD_RESET_TTL_SECONDS,
        user_email: user.email,
    })
}

// =========================================================================
// Rate limit computation
// =========================================================================

/// Compute rate limit information for a specific user.
///
/// Calculates daily and monthly usage, limits, remaining quota, and reset times
/// based on the user's tier. Shared by both the cookie (`/api/admin/...`) and
/// admin-token (`/admin/...`) surfaces so the override + reset rules live once.
///
/// # Errors
///
/// Returns `NotFound` if the target user does not exist, or `Internal` if the
/// user lookup or rate-limit override fetch fails.
pub async fn compute_user_rate_limits(
    repos: &RepositoryRegistry,
    target_user_id: Uuid,
) -> Result<UserRateLimits, AppError> {
    // Admin user views (rate-limit, activity, usage) are global by design — the
    // user list at /api/admin/users is also global ("Per-tenant isolation
    // applies to data operations, not admin views"). Earlier tenant scoping
    // here caused 404s for cross-tenant lookups in the Users panel.
    let user = repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    // Get current monthly usage
    let monthly_used = repos
        .usage
        .get_jwt_current_usage(target_user_id)
        .await
        .unwrap_or(0);

    // Get daily usage from activity logs (today's requests)
    let now = Utc::now();
    let today_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map_or(now, |t| DateTime::<Utc>::from_naive_utc_and_offset(t, Utc));
    let daily_used = repos
        .usage
        .get_top_tools_analysis(target_user_id, today_start, now)
        .await
        .map_or(0, |tools| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            tools.iter().map(|t| t.request_count as u32).sum::<u32>()
        });

    // Per-user override wins over the tier default. Industry pattern: tier
    // baseline + admin-managed exemption table. None on the override row means
    // "unlimited" — same shape as UserTier::Enterprise.monthly_limit() = None.
    let override_row = repos
        .user_rate_limit_overrides
        .get(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch rate-limit override: {e}")))?;

    let (daily_limit, monthly_limit, override_active, override_note) = if let Some(o) = override_row
    {
        (o.daily_limit, o.monthly_limit, true, o.note)
    } else {
        let monthly = user.tier.monthly_limit();
        let daily = monthly.map(|m| m / 30);
        (daily, monthly, false, None)
    };

    // Calculate remaining
    let monthly_remaining = monthly_limit.map(|l| l.saturating_sub(monthly_used));
    let daily_remaining = daily_limit.map(|l| l.saturating_sub(daily_used));

    // Calculate reset times
    let daily_reset = (now + Duration::days(1))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map_or(now, |t| DateTime::<Utc>::from_naive_utc_and_offset(t, Utc));
    let monthly_reset = UnifiedRateLimitCalculator::calculate_monthly_reset();

    Ok(UserRateLimits {
        user_id: target_user_id.to_string(),
        tier: user.tier.to_string(),
        daily_limit,
        daily_used,
        daily_remaining,
        monthly_limit,
        monthly_used,
        monthly_remaining,
        daily_reset,
        monthly_reset,
        override_active,
        override_note,
    })
}

// Per-user rate-limit override CRUD lives in
// `pierre_routes_admin::handlers::admin_rate_limit_override`. The two
// helpers that used to live here (`set_user_rate_limit_override`,
// `clear_user_rate_limit_override`) were inlined into that handler
// because they only touched `repos.users` and
// `repos.user_rate_limit_overrides` — no cross-service coupling worth
// preserving.

// =========================================================================
// User activity aggregation
// =========================================================================

/// Aggregate tool usage activity for a specific user over a given time period.
///
/// Shared by both the cookie (`/api/admin/...`) and admin-token (`/admin/...`)
/// surfaces so the clamping + percentage rules live once.
///
/// # Errors
///
/// Returns `NotFound` if the target user does not exist, or `Internal` if the
/// user lookup fails.
pub async fn compute_user_activity(
    repos: &RepositoryRegistry,
    target_user_id: Uuid,
    days: Option<u32>,
) -> Result<UserActivityResult, AppError> {
    // Admin user views are global by design — matches the unscoped list at
    // /api/admin/users. See compute_user_rate_limits for rationale.
    repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    // Get time range for activity using days parameter (default 30)
    let days = i64::from(days.unwrap_or(30).clamp(1, 365));
    let now = Utc::now();
    let start_time = now - Duration::days(days);

    // Get top tools usage
    let top_tools_raw = repos
        .usage
        .get_top_tools_analysis(target_user_id, start_time, now)
        .await
        .unwrap_or_default();

    // Calculate total requests and percentages
    let total_requests: u64 = top_tools_raw.iter().map(|t| t.request_count).sum();
    let top_tools: Vec<ToolUsageEntry> = top_tools_raw
        .into_iter()
        .map(|t| {
            let percentage = if total_requests > 0 {
                #[allow(clippy::cast_precision_loss)]
                let pct = (t.request_count as f64 / total_requests as f64) * 100.0;
                pct
            } else {
                0.0
            };
            ToolUsageEntry {
                tool_name: t.tool_name,
                call_count: t.request_count,
                percentage,
            }
        })
        .collect();

    Ok(UserActivityResult {
        user_id: target_user_id.to_string(),
        period_days: days,
        total_requests,
        top_tools,
    })
}

// =========================================================================
// User admin profile (installed coaches, joined groups, coach style)
// =========================================================================

/// Build the per-user enrichments rendered by the admin User Details drawer.
///
/// Returns coaching persona + default coach (from the user model), the list
/// of coaches the user has installed from the Coach Store, and the coaching
/// groups they're a member of. Used by `/api/admin/users/{user_id}/admin-profile`.
///
/// # Errors
///
/// Returns `NotFound` if the target user does not exist, or `Internal` if the
/// user lookup or tenant resolution fails.
pub async fn compute_user_admin_profile(
    data: &DataContext,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<UserAdminProfile, AppError> {
    debug!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        "Fetching user admin profile"
    );

    // Admin views are global (see compute_user_rate_limits rationale).
    let user = data
        .repos()
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    // Installed coaches require a tenant scope — use the user's first tenant.
    // Most users belong to exactly one tenant; for multi-tenant users we show
    // installs from their primary tenant only.
    let tenants = data
        .repos()
        .tenants
        .list_for_user(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch user tenants: {e}")))?;

    // Which one is selected now comes from the membership row rather than a
    // column on the user, since selection is per-tenant.
    let selected_coach = if let Some(tenant) = tenants.first() {
        data.repos()
            .tenants
            .get_selected_coach(tenant.id, target_user_id)
            .await
            .unwrap_or_default()
    } else {
        None
    };

    let installed_coaches = if let Some(tenant) = tenants.first() {
        let coaches = data
            .repos()
            .store_listings
            .get_installed_coaches(target_user_id, tenant.id)
            .await
            .unwrap_or_default();
        coaches
            .into_iter()
            .map(|c| InstalledCoachSummary {
                is_default: selected_coach
                    .as_deref()
                    .is_some_and(|d| d == c.id.to_string()),
                coach_id: c.id.to_string(),
                title: c.title,
                category: c.category.as_str().to_owned(),
            })
            .collect()
    } else {
        Vec::new()
    };

    // Joined groups are user-keyed and don't require a tenant scope.
    let groups = data
        .repos()
        .groups
        .list_groups_for_user(target_user_id)
        .await
        .unwrap_or_default();
    let joined_groups: Vec<UserGroupSummary> = groups
        .into_iter()
        .map(|g| UserGroupSummary {
            group_id: g.id.to_string(),
            name: g.name,
            role: g.my_role.to_string(),
            coach_id: g.coach_id,
            member_count: g.member_count,
        })
        .collect();

    Ok(UserAdminProfile {
        user_id: target_user_id.to_string(),
        coaching_persona: user.coaching_persona.to_string(),
        installed_coaches,
        joined_groups,
    })
}

// =========================================================================
// Auto-approval settings
// =========================================================================

/// Fetch recent LLM calls, conversations, and summary stats for the admin dashboard.
///
/// Resolves user emails for conversations and estimates costs using the LLM pricing model.
///
/// # Errors
///
/// Currently no failure path returns an error — all repository calls have
/// per-call defaults applied (empty/zero on failure). The `Result` is preserved
/// for forward compatibility with stricter error propagation.
pub async fn fetch_recent_activity(data: &DataContext) -> Result<RecentActivityResult, AppError> {
    // Limit for recent items
    let recent_limit: i64 = 20;

    // Time boundary for "today" stats
    let today_start = Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map_or_else(Utc::now, |t| {
            DateTime::<Utc>::from_naive_utc_and_offset(t, Utc)
        });
    let today_str = today_start.to_rfc3339();

    // Time boundary for "active conversations" (last 15 minutes)
    let fifteen_min_ago = (Utc::now() - Duration::minutes(15)).to_rfc3339();

    // Fetch all data concurrently
    let (recent_llm, recent_convos, llm_calls_today, active_convos, llm_totals) = tokio::join!(
        data.repos()
            .llm_usage
            .get_recent_llm_calls_admin(recent_limit),
        data.repos()
            .chat
            .get_recent_conversations_admin(recent_limit),
        data.repos().llm_usage.count_llm_calls_since(&today_str),
        data.repos()
            .chat
            .count_active_conversations_since(&fifteen_min_ago),
        data.repos().llm_usage.sum_llm_usage_since(&today_str),
    );

    // Resolve results, defaulting to empty/zero on error
    let recent_llm = recent_llm.unwrap_or_default();
    let recent_convos = recent_convos.unwrap_or_default();
    let llm_calls_today = llm_calls_today.unwrap_or(0);
    let active_convos = active_convos.unwrap_or(0);
    let (total_calls_today, total_tokens_today) = llm_totals.unwrap_or((0, 0));

    // Build LLM calls response with cost estimation
    let llm_calls: Vec<LlmCallEntry> = recent_llm
        .iter()
        .map(|r| {
            let cost = cost_for_record(r);
            LlmCallEntry {
                id: r.id.clone(),
                provider: r.provider.clone(),
                model: r.model.clone(),
                total_tokens: r.total_tokens,
                cost_usd: cost,
                call_type: r.call_type.clone(),
                execution_time_ms: r.execution_time_ms,
                created_at: r.created_at.clone(),
            }
        })
        .collect();

    // Build conversations response — look up user emails for display
    let mut conversations: Vec<ConversationEntry> = Vec::with_capacity(recent_convos.len());
    for convo in &recent_convos {
        // Resolve user email for display (non-critical, use empty on failure)
        let user_email = if let Ok(user_uuid) = convo.user_id.parse::<Uuid>() {
            data.repos()
                .users
                .get_global(user_uuid)
                .await
                .ok()
                .flatten()
                .map(|u| u.email)
                .unwrap_or_default()
        } else {
            String::new()
        };

        conversations.push(ConversationEntry {
            id: convo.id.clone(),
            title: convo.title.clone(),
            updated_at: convo.updated_at.clone(),
            user_email,
        });
    }

    // Estimate cost for today using pricing
    let estimated_cost_today = if total_calls_today > 0 {
        // Use average cost per token from recent calls as approximation
        recent_llm.iter().map(cost_for_record).sum::<f64>() / recent_llm.len().max(1) as f64
            * total_calls_today as f64
    } else {
        0.0
    };

    Ok(RecentActivityResult {
        recent_llm_calls: llm_calls,
        recent_conversations: conversations,
        summary: ActivitySummary {
            active_conversations: active_convos,
            llm_calls_today,
            total_tokens_today,
            estimated_cost_today,
        },
    })
}
