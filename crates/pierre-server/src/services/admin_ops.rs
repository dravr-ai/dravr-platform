// ABOUTME: Admin operations business logic extracted from web_admin route handlers
// ABOUTME: Provides user lifecycle, token management, settings, and analytics services
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::config::social::SocialInsightsConfig;
use crate::errors::{AppError, ErrorCode};
use crate::llm::pricing::calculate_cost;
use crate::mcp::resources::ServerResources;
use crate::models::UserStatus;
use chrono::{DateTime, Duration, Utc};
use pierre_auth::rate_limiting::UnifiedRateLimitCalculator;
use pierre_core::models::TenantId;
use pierre_database::database::repositories::UserMcpTokenRepository;
use pierre_database::database::CreateUserMcpTokenRequest;
use serde::Serialize;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

// =========================================================================
// Result types returned by service functions
// =========================================================================

/// Outcome of a user approval operation
pub(crate) struct ApproveUserResult {
    /// Updated user ID
    pub(crate) user_id: String,
    /// Updated user email
    pub(crate) email: String,
    /// New user status
    pub(crate) user_status: String,
}

/// Outcome of a user suspension operation
pub(crate) struct SuspendUserResult {
    /// Updated user ID
    pub(crate) user_id: String,
    /// Updated user email
    pub(crate) email: String,
    /// New user status
    pub(crate) user_status: String,
}

/// Outcome of a password reset token issuance
pub(crate) struct PasswordResetResult {
    /// The raw token to deliver to the user (shown only once)
    pub(crate) reset_token: String,
    /// Token lifetime in seconds
    pub(crate) expires_in_seconds: u64,
    /// Email address of the target user
    pub(crate) user_email: String,
}

/// Rate limit information for a single user
pub(crate) struct UserRateLimits {
    pub(crate) user_id: String,
    pub(crate) tier: String,
    pub(crate) daily_limit: Option<u32>,
    pub(crate) daily_used: u32,
    pub(crate) daily_remaining: Option<u32>,
    pub(crate) monthly_limit: Option<u32>,
    pub(crate) monthly_used: u32,
    pub(crate) monthly_remaining: Option<u32>,
    pub(crate) daily_reset: DateTime<Utc>,
    pub(crate) monthly_reset: DateTime<Utc>,
}

/// A single tool usage entry with computed percentage
#[derive(Serialize)]
pub(crate) struct ToolUsageEntry {
    pub(crate) tool_name: String,
    pub(crate) call_count: u64,
    pub(crate) percentage: f64,
}

/// Aggregated user activity over a time period
pub(crate) struct UserActivityResult {
    pub(crate) user_id: String,
    pub(crate) period_days: i64,
    pub(crate) total_requests: u64,
    pub(crate) top_tools: Vec<ToolUsageEntry>,
}

/// A single LLM call record for the admin activity feed
#[derive(Serialize)]
pub(crate) struct LlmCallEntry {
    pub(crate) id: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) total_tokens: i64,
    pub(crate) cost_usd: f64,
    pub(crate) call_type: String,
    pub(crate) execution_time_ms: Option<i64>,
    pub(crate) created_at: String,
}

/// A single conversation record for the admin activity feed
#[derive(Serialize)]
pub(crate) struct ConversationEntry {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) updated_at: String,
    pub(crate) user_email: String,
}

/// Summary statistics for the admin dashboard
pub(crate) struct ActivitySummary {
    pub(crate) active_conversations: i64,
    pub(crate) llm_calls_today: i64,
    pub(crate) total_tokens_today: i64,
    pub(crate) estimated_cost_today: f64,
}

/// Full recent activity payload for the admin dashboard
pub(crate) struct RecentActivityResult {
    pub(crate) recent_llm_calls: Vec<LlmCallEntry>,
    pub(crate) recent_conversations: Vec<ConversationEntry>,
    pub(crate) summary: ActivitySummary,
}

/// Resolved auto-approval settings combining env var and database state
pub(crate) struct AutoApprovalSettings {
    pub(crate) enabled: bool,
    pub(crate) auto_approve_domains: Vec<String>,
}

// =========================================================================
// Tenant scope resolution
// =========================================================================

/// Resolve the admin user's tenant scope for listing queries.
///
/// Super-admins see all tenants (returns `None`). Regular admins are scoped
/// to their `active_tenant_id` from JWT claims (returns `Some(tenant_id)`).
/// Returns an error if a non-super-admin has no active tenant in their session.
pub(crate) async fn get_admin_tenant_scope(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
) -> Result<Option<TenantId>, AppError> {
    // SECURITY: Global lookup — resolving admin's own tenant scope
    let user = resources
        .repos
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
    Ok(Some(TenantId::from(tid)))
}

/// Verify an admin user belongs to the target tenant.
///
/// Super-admin users can access any tenant. Regular admins are restricted
/// to tenants they belong to via the `tenant_users` junction table.
pub(crate) async fn verify_admin_tenant_access(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    target_tenant_id: TenantId,
) -> Result<(), AppError> {
    // SECURITY: Global lookup — verifying admin's own tenant access
    let user = resources
        .repos
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
    let admin_tenants = resources
        .repos
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
// User lifecycle helpers
// =========================================================================

/// Auto-create a default MCP token for a newly activated user.
/// This is a non-fatal operation - failure is logged but does not propagate.
pub(crate) async fn create_default_mcp_token_for_user(
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
pub(crate) async fn assign_user_to_admin_tenant(
    resources: &Arc<ServerResources>,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
) -> Result<(), AppError> {
    if let Some(tid) = active_tenant_id {
        let tenant_id = TenantId::from(tid);
        // Update user's tenant_id in users table (kept in sync with tenant_users junction)
        resources
            .repos
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
pub(crate) async fn require_super_admin(
    user_id: Uuid,
    resources: &Arc<ServerResources>,
) -> Result<(), AppError> {
    // SECURITY: Global lookup — checking admin's own super-admin role
    let user = resources
        .repos
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

/// Approve a pending user: validate status, transition to Active, assign tenant,
/// and auto-create a default MCP token.
///
/// Returns an error describing the failure if the user is already active or the
/// database operation fails.
pub(crate) async fn approve_user(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
    reason: Option<&str>,
) -> Result<ApproveUserResult, AppError> {
    // Tenant-scoped lookup: admin can only approve users in their own tenant
    let admin_tenant = get_admin_tenant_scope(resources, admin_user_id, active_tenant_id).await?;
    let user = if let Some(tid) = admin_tenant {
        resources.repos.users.get(target_user_id, tid).await
    } else {
        resources.repos.users.get_global(target_user_id).await
    }
    .map_err(|e| {
        error!(error = %e, "Failed to fetch user from database");
        AppError::internal(format!("Failed to fetch user: {e}"))
    })?
    .ok_or_else(|| {
        warn!("User not found: {}", target_user_id);
        AppError::not_found("User not found")
    })?;

    if user.user_status == UserStatus::Active {
        return Err(AppError::invalid_input("User is already approved"));
    }

    // Use the admin user's UUID as the approver for proper audit trail
    let updated_user = resources
        .repos
        .users
        .update_status(target_user_id, UserStatus::Active, Some(admin_user_id))
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update user status in database");
            AppError::internal(format!("Failed to approve user: {e}"))
        })?;

    // Assign approved user to admin's tenant for multi-tenant isolation
    assign_user_to_admin_tenant(resources, active_tenant_id, target_user_id).await?;

    // Auto-create a default MCP token for the newly approved user
    create_default_mcp_token_for_user(resources.repos.user_mcp_tokens.as_ref(), target_user_id)
        .await;

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
/// Returns an error if the user is already suspended or the database operation fails.
pub(crate) async fn suspend_user(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
    reason: Option<&str>,
) -> Result<SuspendUserResult, AppError> {
    // Tenant-scoped lookup: admin can only suspend users in their own tenant
    let admin_tenant = get_admin_tenant_scope(resources, admin_user_id, active_tenant_id).await?;
    let user = if let Some(tid) = admin_tenant {
        resources.repos.users.get(target_user_id, tid).await
    } else {
        resources.repos.users.get_global(target_user_id).await
    }
    .map_err(|e| {
        error!(error = %e, "Failed to fetch user from database");
        AppError::internal(format!("Failed to fetch user: {e}"))
    })?
    .ok_or_else(|| {
        warn!("User not found: {}", target_user_id);
        AppError::not_found("User not found")
    })?;

    if user.user_status == UserStatus::Suspended {
        return Err(AppError::invalid_input("User is already suspended"));
    }

    // Use the admin user's UUID for audit trail (Note: approved_by is used for both approve/suspend)
    let updated_user = resources
        .repos
        .users
        .update_status(target_user_id, UserStatus::Suspended, Some(admin_user_id))
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update user status in database");
            AppError::internal(format!("Failed to suspend user: {e}"))
        })?;

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
pub(crate) struct AdminPrivilegeChangeResult {
    /// Updated user ID
    pub(crate) user_id: String,
    /// Updated user email
    pub(crate) email: String,
    /// New admin status
    pub(crate) is_admin: bool,
    /// User role after the change (user, admin, `super_admin`)
    pub(crate) role: String,
}

/// Admin user summary for the admins listing
#[derive(Serialize)]
pub(crate) struct AdminUserSummary {
    /// User ID
    pub(crate) id: String,
    /// User email
    pub(crate) email: String,
    /// Display name (optional)
    pub(crate) display_name: Option<String>,
    /// User role (admin or `super_admin`)
    pub(crate) role: String,
    /// User status (active, pending, suspended)
    pub(crate) user_status: String,
    /// Account creation timestamp (RFC3339)
    pub(crate) created_at: String,
}

/// Promote a user to admin: set `is_admin = true` and role to Admin (preserving `SuperAdmin`).
///
/// Only super-admins can perform this operation to prevent privilege escalation.
/// Returns an error if the caller is not super-admin, the target user is not found,
/// or the target is already an admin.
pub(crate) async fn promote_user_to_admin(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<AdminPrivilegeChangeResult, AppError> {
    require_super_admin(admin_user_id, resources).await?;

    // SECURITY: Global lookup — super-admins promote across tenants
    let user = resources
        .repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch target user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if user.is_admin {
        return Err(AppError::invalid_input("User is already an admin"));
    }

    let updated = resources
        .repos
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
pub(crate) async fn demote_user_from_admin(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<AdminPrivilegeChangeResult, AppError> {
    require_super_admin(admin_user_id, resources).await?;

    if admin_user_id == target_user_id {
        return Err(AppError::invalid_input("Cannot demote yourself"));
    }

    let user = resources
        .repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch target user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if !user.is_admin {
        return Err(AppError::invalid_input("User is not an admin"));
    }

    let updated = resources
        .repos
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
pub(crate) async fn list_all_admins(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
) -> Result<Vec<AdminUserSummary>, AppError> {
    require_super_admin(admin_user_id, resources).await?;

    let admins = resources
        .repos
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

/// Generate a cryptographically random password reset token and store its hash.
///
/// The raw token is returned to be delivered to the user. Only the SHA-256 hash
/// is stored in the database. The token expires after 1 hour.
pub(crate) async fn generate_password_reset_token(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
) -> Result<PasswordResetResult, AppError> {
    use rand::distributions::Alphanumeric;
    use rand::Rng;
    use sha2::{Digest, Sha256};

    // Tenant-scoped lookup: admin can only reset passwords for users in their tenant
    let admin_tenant = get_admin_tenant_scope(resources, admin_user_id, active_tenant_id).await?;
    let user = if let Some(tid) = admin_tenant {
        resources.repos.users.get(target_user_id, tid).await
    } else {
        resources.repos.users.get_global(target_user_id).await
    }
    .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
    .ok_or_else(|| AppError::not_found("User not found"))?;

    // Generate a cryptographically random reset token (48 chars alphanumeric)
    let raw_token: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();

    // Store only the SHA-256 hash of the token in the database
    let token_hash = format!("{:x}", Sha256::digest(raw_token.as_bytes()));

    let admin_id_str = admin_user_id.to_string();
    resources
        .repos
        .password_reset
        .store_token(target_user_id, &token_hash, &admin_id_str)
        .await
        .map_err(|e| AppError::internal(format!("Failed to create reset token: {e}")))?;

    info!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        "Password reset token issued via web admin"
    );

    Ok(PasswordResetResult {
        reset_token: raw_token,
        expires_in_seconds: 3600,
        user_email: user.email,
    })
}

// =========================================================================
// Rate limit computation
// =========================================================================

/// Compute rate limit information for a specific user.
///
/// Calculates daily and monthly usage, limits, remaining quota, and reset times
/// based on the user's tier.
pub(crate) async fn compute_user_rate_limits(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
) -> Result<UserRateLimits, AppError> {
    debug!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        "Fetching user rate limit"
    );

    // Tenant-scoped lookup: admin can only view metrics for users in their tenant
    let admin_tenant = get_admin_tenant_scope(resources, admin_user_id, active_tenant_id).await?;
    let user = if let Some(tid) = admin_tenant {
        resources.repos.users.get(target_user_id, tid).await
    } else {
        resources.repos.users.get_global(target_user_id).await
    }
    .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
    .ok_or_else(|| AppError::not_found("User not found"))?;

    // Get current monthly usage
    let monthly_used = resources
        .repos
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
    let daily_used = resources
        .repos
        .usage
        .get_top_tools_analysis(target_user_id, today_start, now)
        .await
        .map(|tools| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            tools.iter().map(|t| t.request_count as u32).sum::<u32>()
        })
        .unwrap_or(0);

    // Calculate limits based on tier
    let monthly_limit = user.tier.monthly_limit();
    let daily_limit = monthly_limit.map(|m| m / 30);

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
    })
}

// =========================================================================
// User activity aggregation
// =========================================================================

/// Aggregate tool usage activity for a specific user over a given time period.
pub(crate) async fn compute_user_activity(
    resources: &Arc<ServerResources>,
    admin_user_id: Uuid,
    active_tenant_id: Option<Uuid>,
    target_user_id: Uuid,
    days: Option<u32>,
) -> Result<UserActivityResult, AppError> {
    debug!(
        admin_id = %admin_user_id,
        target_user_id = %target_user_id,
        "Fetching user activity"
    );

    // Tenant-scoped lookup: admin can only view activity for users in their tenant
    let admin_tenant = get_admin_tenant_scope(resources, admin_user_id, active_tenant_id).await?;
    if let Some(tid) = admin_tenant {
        resources.repos.users.get(target_user_id, tid).await
    } else {
        resources.repos.users.get_global(target_user_id).await
    }
    .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
    .ok_or_else(|| AppError::not_found("User not found"))?;

    // Get time range for activity using days parameter (default 30)
    let days = i64::from(days.unwrap_or(30).clamp(1, 365));
    let now = Utc::now();
    let start_time = now - Duration::days(days);

    // Get top tools usage
    let top_tools_raw = resources
        .repos
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
// Auto-approval settings
// =========================================================================

/// Retrieve the effective auto-approval setting.
///
/// Precedence: env var (if set) > database > default.
pub(crate) async fn get_auto_approval_settings(
    resources: &Arc<ServerResources>,
) -> Result<AutoApprovalSettings, AppError> {
    let enabled = if resources.config.app_behavior.auto_approve_users_from_env {
        resources.config.app_behavior.auto_approve_users
    } else {
        match resources.database.is_auto_approval_enabled().await {
            Ok(Some(db_setting)) => db_setting,
            Ok(None) => resources.config.app_behavior.auto_approve_users,
            Err(e) => {
                error!(error = %e, "Failed to get auto-approval setting");
                return Err(AppError::internal(format!(
                    "Failed to get auto-approval setting: {e}"
                )));
            }
        }
    };

    Ok(AutoApprovalSettings {
        enabled,
        auto_approve_domains: resources.config.app_behavior.auto_approve_domains.clone(),
    })
}

/// Persist a new auto-approval setting to the database.
pub(crate) async fn set_auto_approval(
    resources: &Arc<ServerResources>,
    enabled: bool,
) -> Result<(), AppError> {
    resources
        .database
        .set_auto_approval_enabled(enabled)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to set auto-approval setting");
            AppError::internal(format!("Failed to set auto-approval setting: {e}"))
        })
}

// =========================================================================
// Social insights settings
// =========================================================================

/// Retrieve the current social insights configuration.
pub(crate) async fn get_social_insights_config(
    resources: &Arc<ServerResources>,
) -> Result<SocialInsightsConfig, AppError> {
    resources
        .database
        .get_social_insights_config()
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to get social insights config");
            AppError::internal(format!("Failed to get social insights config: {e}"))
        })
        .map(Option::unwrap_or_default)
}

/// Persist updated social insights configuration.
pub(crate) async fn set_social_insights_config(
    resources: &Arc<ServerResources>,
    config: &SocialInsightsConfig,
) -> Result<(), AppError> {
    resources
        .database
        .set_social_insights_config(config)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to set social insights config");
            AppError::internal(format!("Failed to set social insights config: {e}"))
        })
}

/// Reset social insights configuration to defaults.
pub(crate) async fn reset_social_insights_config(
    resources: &Arc<ServerResources>,
) -> Result<SocialInsightsConfig, AppError> {
    resources
        .database
        .delete_social_insights_config()
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to reset social insights config");
            AppError::internal(format!("Failed to reset social insights config: {e}"))
        })?;

    Ok(SocialInsightsConfig::default())
}

// =========================================================================
// Analytics: recent activity dashboard
// =========================================================================

/// Fetch recent LLM calls, conversations, and summary stats for the admin dashboard.
///
/// Resolves user emails for conversations and estimates costs using the LLM pricing model.
pub(crate) async fn fetch_recent_activity(
    resources: &Arc<ServerResources>,
) -> Result<RecentActivityResult, AppError> {
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
        resources
            .repos
            .llm_usage
            .get_recent_llm_calls_admin(recent_limit),
        resources
            .repos
            .chat
            .get_recent_conversations_admin(recent_limit),
        resources.repos.llm_usage.count_llm_calls_since(&today_str),
        resources
            .repos
            .chat
            .count_active_conversations_since(&fifteen_min_ago),
        resources.repos.llm_usage.sum_llm_usage_since(&today_str),
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
            let cost = calculate_cost(&r.provider, &r.model, r.prompt_tokens, r.completion_tokens);
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
            resources
                .repos
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
        recent_llm
            .iter()
            .map(|r| calculate_cost(&r.provider, &r.model, r.prompt_tokens, r.completion_tokens))
            .sum::<f64>()
            / recent_llm.len().max(1) as f64
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
