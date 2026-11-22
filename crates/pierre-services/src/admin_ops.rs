// ABOUTME: Admin operations business logic extracted from web_admin route handlers
// ABOUTME: Provides user lifecycle, token management, settings, and analytics services
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::pricing::{cost_for_record, costs_of_usage_groups};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_core::models::{AdminRosterGrant, MonthlyLimitOverride, User, UserStatus};
use pierre_database::database::repositories::UserMcpTokenRepository;
use pierre_database::database::CreateUserMcpTokenRequest;
use pierre_database::repositories::analytics::next_utc_month_start;
use pierre_database::repositories::LlmUsageScope;
use pierre_database::RepositoryRegistry;
use pierre_middleware::admin_guard::require_admin;
use pierre_middleware::mask_email;
use pierre_runtime_context::DataContext;
use serde::Serialize;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Who acts on an account, and the rank rule an admin account is held to.
mod caller;
pub use caller::*;

/// What an account held on its admin standing, withdrawn when it loses it.
mod credentials;
pub use credentials::*;

/// Plan, tier, roster and override administration.
mod overrides;
pub use overrides::*;

/// The decision of a pending sign-up by a sign-up rule or the Slack room.
mod sign_up;
pub use sign_up::{
    activate_pending_sign_up, decide_sign_up, ApprovedSignUp, PendingActivation, SignUpApproval,
    SignUpDecision,
};

// =========================================================================
// Result types returned by service functions
// =========================================================================

/// A user's monthly request budget as the admin views report it
#[derive(Debug)]
pub struct UserRateLimits {
    /// Target user ID
    pub user_id: String,
    /// Effective tier label
    pub tier: String,
    /// Monthly request cap (None means unlimited)
    pub monthly_limit: Option<u32>,
    /// Requests made this month
    pub monthly_used: u32,
    /// Monthly quota remaining (None means unlimited)
    pub monthly_remaining: Option<u32>,
    /// Wall-clock time when the monthly counter resets
    pub monthly_reset: Timestamp,
    /// True when a per-user override is in effect (tier default ignored).
    pub override_active: bool,
    /// Admin note attached to the override, if any.
    pub override_note: Option<String>,
}

impl UserRateLimits {
    /// The body both admin surfaces answer `GET …/users/{id}/rate-limit`
    /// with, so the two cannot drift apart.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "user_id": self.user_id,
            "tier": self.tier,
            "rate_limits": {
                "monthly": {
                    "limit": self.monthly_limit,
                    "used": self.monthly_used,
                    "remaining": self.monthly_remaining,
                },
            },
            "reset_times": {
                "monthly_reset": self.monthly_reset.to_rfc3339(),
            },
            "override_active": self.override_active,
            "override_note": self.override_note,
        })
    }
}

/// A single tool usage entry with computed percentage
#[derive(Debug, Serialize)]
pub struct ToolUsageEntry {
    /// Tool name as recorded in usage logs
    pub tool_name: String,
    /// Number of invocations in the queried window
    pub call_count: u64,
    /// Share of total invocations (0.0–100.0)
    pub percentage: f64,
}

/// Aggregated user activity over a time period
#[derive(Debug)]
pub struct UserActivityResult {
    /// Target user ID
    pub user_id: String,
    /// Length of the aggregation window, in days
    pub period_days: i64,
    /// Requests the user's API keys made in the window, on every endpoint. Chat
    /// and token-authenticated traffic is not in this ledger.
    pub total_requests: u64,
    /// Per-tool usage breakdown, ordered by frequency
    pub top_tools: Vec<ToolUsageEntry>,
}

/// An agent installed by a user (admin User Details panel).
#[derive(Debug, Serialize)]
pub struct InstalledAgentSummary {
    /// Installed agent ID
    pub agent_id: String,
    /// Agent display title
    pub title: String,
    /// Agent category (e.g., "endurance", "strength")
    pub category: String,
    /// True if this is the user's default agent
    pub is_default: bool,
}

/// A coaching group a user belongs to (admin User Details panel).
#[derive(Debug, Serialize)]
pub struct UserGroupSummary {
    /// Group ID
    pub group_id: String,
    /// Group display name
    pub name: String,
    /// Caller's role within the group (member, admin, etc.)
    pub role: String,
    /// Agent assigned to the group
    pub agent_id: String,
    /// Member count at query time
    pub member_count: i64,
}

/// Per-user enrichments rendered by the admin User Details drawer:
/// coaching persona, default agent, installed agents, joined groups.
#[derive(Debug, Serialize)]
pub struct UserAdminProfile {
    /// Target user ID
    pub user_id: String,
    /// Configured coaching persona label
    pub coaching_persona: String,
    /// Whether the user may become a coaching group's human coach
    pub manages_roster: bool,
    /// Who granted it as an admin, and when; `None` when no admin did.
    /// A `TrainingPeaks` disconnect leaves an admin grant in place.
    pub manages_roster_admin_grant: Option<AdminRosterGrant>,
    /// Coaches the user has installed from the Agent Store
    pub installed_agents: Vec<InstalledAgentSummary>,
    /// Coaching groups the user belongs to
    pub joined_groups: Vec<UserGroupSummary>,
}

/// A single LLM call record for the admin activity feed
#[derive(Debug, Serialize)]
pub struct LlmCallEntry {
    /// Call record ID
    pub id: String,
    /// LLM provider name (e.g., "openai", "gemini")
    pub provider: String,
    /// Model identifier
    pub model: String,
    /// Combined prompt + completion tokens
    pub total_tokens: i64,
    /// Prompt tokens served from the provider's context cache
    pub cached_tokens: i64,
    /// Prompt tokens this call wrote into the provider's context cache
    pub cached_write_tokens: i64,
    /// The part of `cached_write_tokens` written to a 1-hour cache entry
    pub cached_write_1h_tokens: i64,
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
#[derive(Debug, Serialize)]
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
#[derive(Debug)]
pub struct ActivitySummary {
    /// Conversations active in the last 15 minutes
    pub active_conversations: i64,
    /// LLM calls recorded since midnight UTC today
    pub llm_calls_today: i64,
    /// Total tokens consumed since midnight UTC today
    pub total_tokens_today: i64,
    /// USD cost of those calls, summed at current prices
    pub cost_today_usd: f64,
}

/// How many rows each recent-activity list carries. The two lists fill up
/// independently, so they can span different periods.
pub const RECENT_ACTIVITY_LIMIT: i64 = 20;

/// Full recent activity payload for the admin dashboard
#[derive(Debug)]
pub struct RecentActivityResult {
    /// Most recent LLM call records, at most [`RECENT_ACTIVITY_LIMIT`]
    pub recent_llm_calls: Vec<LlmCallEntry>,
    /// Most recent conversations, at most [`RECENT_ACTIVITY_LIMIT`]
    pub recent_conversations: Vec<ConversationEntry>,
    /// Aggregated counts for the day
    pub summary: ActivitySummary,
}

// =========================================================================
// Tenant scope resolution
// =========================================================================

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

/// Assign an approved user to the tenant the approving admin is working in.
///
/// `tenant` is the approver's scope: the active tenant of a console session, or
/// the tenant an admin token is bound to. With no scope (a platform-wide token)
/// the user's tenant is left as it is.
///
/// # Errors
///
/// Returns `Internal` if the underlying `users.update_tenant_id` repository call fails.
pub async fn assign_user_to_admin_tenant(
    repos: &RepositoryRegistry,
    tenant: Option<TenantId>,
    target_user_id: Uuid,
) -> Result<(), AppError> {
    if let Some(tenant_id) = tenant {
        // Update user's tenant_id in users table (kept in sync with tenant_users junction)
        repos
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

// =========================================================================
// User approval and suspension
// =========================================================================

/// A user's status after [`transition_user_status`], and what the move took
/// away.
#[derive(Debug)]
pub struct StatusTransition {
    /// The account as stored after the transition.
    pub user: User,
    /// The admin credentials withdrawn because the account left the active
    /// status; empty on activation, and for an account that held none. The
    /// caller records it ([`record_withdrawn_admin_credentials`]).
    pub withdrawn: WithdrawnAdminCredentials,
    /// The status the account had before the transition, as the write read
    /// it.
    pub previous: UserStatus,
}

/// Shared user-status state transition: global lookup, already-in-target-state
/// guard, and status update.
///
/// Used by [`approve_user`] for an admin's approval, the admin suspend
/// handler and `pierre-cli`'s local user commands: an admin's decision,
/// which may bring a suspended account back. A sign-up rule's activation
/// is not one and does not come through here ([`activate_pending_sign_up`]).
/// The last active admin cannot be
/// suspended: `UserRepository::update_status` refuses it. Nor does an admin
/// take itself out of `Active`, as it does not demote itself
/// ([`demote_user_from_admin`]): it would be signed out with its device
/// logins withdrawn, and only another admin could bring it back.
///
/// An account moved out of `Active` is no longer an active admin, whatever
/// its role: the device-login tokens it approved and the impersonations it
/// has open are withdrawn by the repository in the transaction that changes
/// the status, so the two cannot come apart.
///
/// `acting` is the account and who changes its status
/// ([`AccountCaller::acting_on`], which holds an admin account to a
/// platform caller). The transition is written against
/// [`AccountCaller::recorded_as`]. A caller that is an admin's own session
/// ([`AccountCaller::acting_admin`]) is refused taking that admin out of
/// `Active`; a service token and `pierre-cli` on the host act as no admin
/// and have no self to refuse. The Slack sign-up room does not come
/// through here: its taps decide a pending account only
/// ([`decide_sign_up`], which [`approve_user`] takes for it).
///
/// # Errors
///
/// Returns `InvalidInput` if the caller's own admin is the target and
/// `target_status` is not `Active`, if the user is already in
/// `target_status` or is the last active admin being moved out of `Active`,
/// `NotFound` if the user does not exist, or the repository's error if a
/// call fails.
pub async fn transition_user_status(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
    target_status: UserStatus,
) -> Result<StatusTransition, AppError> {
    let target_user_id = acting.id();
    if acting.caller().acting_admin() == Some(target_user_id) {
        let refusal = match target_status {
            UserStatus::Active => None,
            UserStatus::Suspended => Some("Cannot suspend yourself"),
            UserStatus::Pending => Some("Cannot set yourself back to pending"),
        };
        if let Some(message) = refusal {
            return Err(AppError::invalid_input(message));
        }
    }

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

    // The repository's own error is returned as it is: its refusal to suspend
    // the last active admin is the caller's to read, not a server fault.
    let write = repos
        .users
        .update_status(target_user_id, target_status, acting.caller().recorded_as())
        .await
        .inspect_err(|e| error!(error = %e, "Failed to update user status in database"))?;
    Ok(StatusTransition {
        user: write.user,
        withdrawn: write.withdrawn,
        previous: user.user_status,
    })
}

/// Approve an account: make it active, give it a tenant, and create its
/// default MCP token.
///
/// The one approval of every surface (carnet#1589): the admin API and its
/// console mount, `pierre-cli user approve`, and the Slack sign-up room.
///
/// `acting` is the account and who approves it
/// ([`AccountCaller::acting_on`]). The status write is the caller's:
///
/// - an admin's decision ([`transition_user_status`]) makes active a pending
///   account or a suspended one, which it reinstates, and refuses one
///   already active;
/// - a surface that decides sign-ups only ([`AccountCaller::decides_sign_ups_only`],
///   the Slack room) writes only while the account is still pending
///   ([`decide_sign_up`]): a sign-up decided meanwhile, here or elsewhere,
///   is [`SignUpApproval::LostAtTheWrite`] and nothing is written.
///
/// Then the tenant: the account joins `tenant`, the approver's (the active
/// tenant of a console session, or the one an admin token is bound to),
/// when there is one; an account that still belongs to no tenant is given
/// one of its own. An approval that cannot give the account its tenant
/// must not leave it active with none and nobody told, so it is put back
/// to the status it had ([`SignUpApproval::PutBack`]), where the next
/// approval finishes it. Last, the account's default MCP token, which a
/// failure only logs.
///
/// # Errors
///
/// Returns `InvalidInput` when an admin approves an account already active
/// or its own, `NotFound` for an account that does not exist, the
/// repository's error when the approving write fails, and what failed after
/// it when the account could not be put back.
pub async fn approve_user(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
    tenant: Option<TenantId>,
) -> Result<SignUpApproval, AppError> {
    let approved = if acting.caller().decides_sign_ups_only() {
        decide_sign_up(repos, acting, SignUpDecision::Approve).await?
    } else {
        Some(transition_user_status(repos, acting, UserStatus::Active).await?)
    };
    let Some(StatusTransition { user, previous, .. }) = approved else {
        return Ok(SignUpApproval::LostAtTheWrite);
    };

    let tenant_id = match approved_account_tenant(repos, tenant, &user).await {
        Ok(tenant_id) => tenant_id,
        Err(cause) => return sign_up::put_back(repos, acting, previous, cause).await,
    };

    create_default_mcp_token_for_user(repos.user_mcp_tokens.as_ref(), user.id).await;

    Ok(SignUpApproval::Approved(Box::new(ApprovedSignUp {
        user,
        tenant_id,
    })))
}

/// The tenant an approved account is attributed to: `tenant`, the
/// approver's, which the account joins, or else the one it owns or belongs
/// to, provisioned when it has none.
async fn approved_account_tenant(
    repos: &RepositoryRegistry,
    tenant: Option<TenantId>,
    user: &User,
) -> Result<Option<TenantId>, AppError> {
    if let Some(tenant_id) = tenant {
        assign_user_to_admin_tenant(repos, Some(tenant_id), user.id).await?;
        return Ok(Some(tenant_id));
    }
    sign_up::own_tenant(repos, user).await
}

// =========================================================================
// Admin privilege management (an admin manages admins)
// =========================================================================

/// Outcome of an admin privilege change (promote or demote)
#[derive(Debug)]
pub struct AdminPrivilegeChangeResult {
    /// Updated user ID
    pub user_id: String,
    /// Updated user email
    pub email: String,
    /// New admin status
    pub is_admin: bool,
    /// User role after the change (`user` or `admin`)
    pub role: String,
}

/// Outcome of a demotion: the change, and what it took away.
#[derive(Debug)]
pub struct AdminDemotion {
    /// The account after the demotion.
    pub change: AdminPrivilegeChangeResult,
    /// The admin credentials withdrawn with the role. The caller records it
    /// ([`record_withdrawn_admin_credentials`]).
    pub withdrawn: WithdrawnAdminCredentials,
}

/// Admin user summary for the admins listing
#[derive(Debug, Serialize)]
pub struct AdminUserSummary {
    /// User ID
    pub id: String,
    /// User email
    pub email: String,
    /// Display name (optional)
    pub display_name: Option<String>,
    /// User role (always `admin`)
    pub role: String,
    /// User status (active, pending, suspended)
    pub user_status: String,
    /// Account creation timestamp (RFC3339)
    pub created_at: String,
}

/// Give `user` the admin role: the write every promotion makes.
///
/// The admin API and the console reach it through
/// [`promote_user_to_admin`], which checks the caller first; `pierre-cli
/// user promote` on the host calls it as it is, since no admin's session is
/// behind that command.
///
/// An admin is approved first, then promoted (carnet#1589). An account that
/// is not active is refused: a pending one holding the role would hold
/// every power the moment any sign-up rule let it in, without an admin ever
/// having approved it, and a suspended one is reinstated by an approval,
/// which is where the decision to bring it back belongs.
///
/// # Errors
///
/// Returns `InvalidInput` if `user` is already an admin or is not active,
/// or `Internal` when the write fails.
pub async fn grant_admin_role(repos: &RepositoryRegistry, user: &User) -> Result<User, AppError> {
    if user.role.is_admin() {
        return Err(AppError::invalid_input("User is already an admin"));
    }
    refuse_promoting_unless_active(user)?;

    Ok(repos
        .users
        .set_admin_status(user.id, true)
        .await
        .map_err(|e| AppError::internal(format!("Failed to promote user: {e}")))?
        .user)
}

/// Refuse the promotion of an account that is not active, saying what its
/// status is and what to do first.
///
/// The one statement of the rule that an admin is an approved account
/// first: [`grant_admin_role`] refuses with it, and the bootstrap seeder
/// asks it before bringing an existing account to the admin role.
///
/// # Errors
///
/// Returns `InvalidInput` if `user` is pending or suspended.
pub fn refuse_promoting_unless_active(user: &User) -> Result<(), AppError> {
    if user.user_status == UserStatus::Active {
        return Ok(());
    }
    Err(AppError::invalid_input(format!(
        "Only an active account is promoted to admin, and this one is {}. \
         Approve the account first, then promote it.",
        user.user_status
    )))
}

/// Promote a user to admin: give them the `Admin` role.
///
/// Any admin may do this; nobody else can, so a user cannot escalate. The
/// target is an active account ([`grant_admin_role`]).
///
/// # Errors
///
/// Returns `PermissionDenied` if the caller is not an admin, `InvalidInput`
/// if the target is already an admin or is not active, `NotFound` if the
/// target does not exist, or `Internal` on repository failures.
pub async fn promote_user_to_admin(
    repos: &RepositoryRegistry,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<AdminPrivilegeChangeResult, AppError> {
    require_admin(admin_user_id, &repos.users).await?;

    // SECURITY: Global lookup — admins promote across tenants
    let user = repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch target user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    let updated = grant_admin_role(repos, &user).await?;

    info!(
        admin_user_id = %admin_user_id,
        target_user_id = %target_user_id,
        target_email = %mask_email(&updated.email),
        "User promoted to admin"
    );

    Ok(AdminPrivilegeChangeResult {
        user_id: updated.id.to_string(),
        email: updated.email,
        is_admin: updated.role.is_admin(),
        role: updated.role.as_str().to_owned(),
    })
}

/// Demote an admin user: reset their role to `User`.
///
/// Any admin may demote another admin. Self-demotion is rejected to avoid
/// locking the caller out of admin actions, and the repository refuses to
/// demote the last active admin, so the platform always keeps one.
///
/// The device-login tokens the demoted admin approved and the impersonations
/// it has open are withdrawn by the repository in the transaction that
/// resets the role.
///
/// # Errors
///
/// Returns `PermissionDenied` if the caller is not an admin, `InvalidInput`
/// for self-demotion, when the target is not an admin or when it is the last
/// active admin, `NotFound` if the target does not exist, or the repository's
/// error on a failed write.
pub async fn demote_user_from_admin(
    repos: &RepositoryRegistry,
    admin_user_id: Uuid,
    target_user_id: Uuid,
) -> Result<AdminDemotion, AppError> {
    require_admin(admin_user_id, &repos.users).await?;

    if admin_user_id == target_user_id {
        return Err(AppError::invalid_input("Cannot demote yourself"));
    }

    let user = repos
        .users
        .get_global(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch target user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if !user.role.is_admin() {
        return Err(AppError::invalid_input("User is not an admin"));
    }

    let write = repos.users.set_admin_status(target_user_id, false).await?;
    let (updated, withdrawn) = (write.user, write.withdrawn);

    info!(
        admin_user_id = %admin_user_id,
        target_user_id = %target_user_id,
        target_email = %mask_email(&updated.email),
        "User demoted from admin"
    );

    Ok(AdminDemotion {
        change: AdminPrivilegeChangeResult {
            user_id: updated.id.to_string(),
            email: updated.email,
            is_admin: updated.role.is_admin(),
            role: updated.role.as_str().to_owned(),
        },
        withdrawn,
    })
}

/// List all admin users across all tenants.
///
/// Any admin can view the full admin roster, ordered by email ascending.
///
/// # Errors
///
/// Returns `PermissionDenied` if the caller is not an admin, or `Internal`
/// if the admin listing repository call fails.
pub async fn list_all_admins(
    data: &DataContext,
    admin_user_id: Uuid,
) -> Result<Vec<AdminUserSummary>, AppError> {
    require_admin(admin_user_id, &data.repos().users).await?;

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
/// is persisted. Whoever holds the token signs in as the account, so `acting`
/// holds an admin account to a platform caller
/// ([`AccountCaller::acting_on`]). The token is issued under the caller's
/// [`AccountCaller::audit_name`]: the admin's id when the caller acts as
/// one, a service name or `pierre-cli` otherwise. Shared by the admin reset
/// handler and `pierre-cli`, each of which resolves the target user
/// beforehand (the handler through [`find_user_in_admin_scope`]).
///
/// # Errors
///
/// Returns `Internal` if token persistence fails.
pub async fn issue_password_reset_token(
    repos: &RepositoryRegistry,
    acting: &ActingOn<'_>,
) -> Result<String, AppError> {
    use crate::link_token::generate_link_token;

    // Selector/verifier token: only the verifier's SHA-256 hash is stored; the plaintext
    // selector indexes the row. The full "<selector>.<verifier>" token is returned for
    // delivery to the user and never persisted.
    let generated = generate_link_token();

    repos
        .password_reset
        .store_token(
            acting.id(),
            &generated.selector,
            &generated.verifier_hash,
            &acting.caller().audit_name(),
        )
        .await
        .map_err(|e| AppError::internal(format!("Failed to create reset token: {e}")))?;

    Ok(generated.token)
}

/// Find the user an admin reads or resets a password for, within the admin's
/// scope.
///
/// `scope` is `None` for a platform-wide caller (an admin's console session,
/// or an admin token bound to no tenant) and the admin's tenant otherwise, in which case a
/// user outside that tenant answers `NotFound` exactly like a missing one.
///
/// # Errors
///
/// Returns `NotFound` if the user does not exist in scope, or `Internal` if the
/// lookup fails.
pub async fn find_user_in_admin_scope(
    repos: &RepositoryRegistry,
    scope: Option<TenantId>,
    target_user_id: Uuid,
) -> Result<User, AppError> {
    match scope {
        Some(tenant_id) => repos.users.get(target_user_id, tenant_id).await,
        None => repos.users.get_global(target_user_id).await,
    }
    .map_err(|e| AppError::internal(format!("Failed to fetch user: {e}")))?
    .ok_or_else(|| AppError::not_found("User not found"))
}

// =========================================================================
// Rate limit computation
// =========================================================================

/// Compute rate limit information for a specific user.
///
/// Reports the monthly usage, limit, remaining quota and reset time the auth
/// middleware enforces: the same statement feeds both, so the limit shown is a
/// per-user override when one is set and the tier's otherwise. Shared by both
/// the cookie (`/api/admin/...`) and admin-token (`/admin/...`) surfaces so
/// the override + reset rules live once.
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

    // The statement the auth middleware reads, so this view shows what the
    // gate applies: the month's count and the override's limit, else the
    // tier's. The override row itself is read only for its note.
    let usage = repos
        .usage
        .get_jwt_current_usage(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to count monthly usage: {e}")))?;
    let monthly_limit = usage.monthly_override.resolve(&user.tier);
    let override_note = repos
        .user_rate_limit_overrides
        .get(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch rate-limit override: {e}")))?
        .and_then(|o| o.note);

    Ok(UserRateLimits {
        user_id: target_user_id.to_string(),
        tier: user.tier.to_string(),
        monthly_limit,
        monthly_used: usage.used,
        monthly_remaining: monthly_limit.map(|l| l.saturating_sub(usage.used)),
        monthly_reset: next_utc_month_start(Timestamp::now()),
        override_active: usage.monthly_override != MonthlyLimitOverride::NotSet,
        override_note,
    })
}

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
    let now = Timestamp::now();
    let start_time = now - Elapsed::days(days);

    // The list stops at the busiest endpoints, the count does not, so a share is
    // of everything the keys did.
    let (top_tools_raw, total_requests) = tokio::try_join!(
        repos
            .usage
            .get_top_tools_analysis(target_user_id, start_time, now),
        repos
            .usage
            .count_api_key_requests(target_user_id, start_time, now),
    )?;

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
// User admin profile (installed agents, joined groups, agent style)
// =========================================================================

/// Build the per-user enrichments rendered by the admin User Details drawer.
///
/// Returns coaching persona + default agent (from the user model), the list
/// of agents the user has installed from the Agent Store, and the coaching
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
    let manages_roster_admin_grant = data
        .repos()
        .users
        .manages_roster_admin_grant(target_user_id)
        .await?;

    // Installed agents require a tenant scope — use the user's first tenant.
    // Most users belong to exactly one tenant; for multi-tenant users we show
    // installs from their primary tenant only.
    let tenant_id = data
        .repos()
        .tenants
        .first_for_user(target_user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to fetch user tenants: {e}")))?;

    // Which one is selected now comes from the membership row rather than a
    // column on the user, since selection is per-tenant.
    let selected_agent = if let Some(tenant_id) = tenant_id {
        data.repos()
            .tenants
            .get_selected_agent(tenant_id, target_user_id)
            .await
            .unwrap_or_default()
    } else {
        None
    };

    let installed_agents = if let Some(tenant_id) = tenant_id {
        let agents = data
            .repos()
            .store_listings
            .get_installed_agents(target_user_id, tenant_id)
            .await
            .unwrap_or_default();
        agents
            .into_iter()
            .map(|c| InstalledAgentSummary {
                is_default: selected_agent
                    .as_deref()
                    .is_some_and(|d| d == c.id.to_string()),
                agent_id: c.id.to_string(),
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
            agent_id: g.agent_id,
            member_count: g.member_count,
        })
        .collect();

    Ok(UserAdminProfile {
        user_id: target_user_id.to_string(),
        coaching_persona: user.coaching_persona.to_string(),
        manages_roster: user.manages_roster,
        manages_roster_admin_grant,
        installed_agents,
        joined_groups,
    })
}

// =========================================================================
// Auto-approval settings
// =========================================================================

/// Fetch recent LLM calls, conversations, and summary stats for the admin dashboard,
/// over the tenants `scope` covers.
///
/// A summary is either true or an error: a failed read never renders as zero.
///
/// # Errors
///
/// Returns an error if any repository read fails.
pub async fn fetch_recent_activity(
    data: &DataContext,
    scope: AdminScope,
) -> Result<RecentActivityResult, AppError> {
    // Time boundary for "today" stats
    let today_start = Timestamp::now().utc_date().at_midnight().and_utc()?;
    let today_str = today_start.to_rfc3339();

    // Time boundary for "active conversations" (last 15 minutes)
    let fifteen_min_ago = (Timestamp::now() - Elapsed::minutes(15)).to_rfc3339();

    let repos = data.repos();
    let tenant_text = scope.tenant_filter().map(|t| t.to_string());
    let (recent_llm, recent_convos, active_convos, usage_today) = tokio::try_join!(
        repos
            .llm_usage
            .get_recent_llm_calls_admin(tenant_text.as_deref(), RECENT_ACTIVITY_LIMIT),
        repos
            .chat
            .get_recent_conversations_admin(scope, RECENT_ACTIVITY_LIMIT),
        repos
            .chat
            .count_active_conversations_since(scope, &fifteen_min_ago),
        repos
            .llm_usage
            .get_llm_usage_aggregates(tenant_text.as_deref(), &today_str),
    )?;

    let llm_calls_today = usage_today.iter().map(|row| row.calls).sum();
    let total_tokens_today = usage_today.iter().map(|row| row.total_tokens).sum();
    let usage_scope = tenant_text
        .as_deref()
        .map_or(LlmUsageScope::AllTenants, LlmUsageScope::Tenant);
    let cost_today_usd: f64 = costs_of_usage_groups(
        repos.llm_usage.as_ref(),
        usage_scope,
        &usage_today,
        &today_str,
    )
    .await?
    .iter()
    .sum();

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
                cached_tokens: r.cached_tokens,
                cached_write_tokens: r.cached_write_tokens,
                cached_write_1h_tokens: r.cached_write_1h_tokens,
                cost_usd: cost,
                call_type: r.call_type.clone(),
                execution_time_ms: r.execution_time_ms,
                created_at: r.created_at.clone(),
            }
        })
        .collect();

    // One read for every owner: this runs on each 10-second poll.
    let owners: Vec<Uuid> = recent_convos
        .iter()
        .filter_map(|convo| convo.user_id.parse().ok())
        .collect();
    let users = data.repos().users.get_global_many(&owners).await?;
    let conversations: Vec<ConversationEntry> = recent_convos
        .iter()
        .map(|convo| ConversationEntry {
            id: convo.id.clone(),
            title: convo.title.clone(),
            updated_at: convo.updated_at.clone(),
            user_email: convo
                .user_id
                .parse()
                .ok()
                .and_then(|id: Uuid| users.get(&id))
                .map_or_default(|user| user.email.clone()),
        })
        .collect();

    Ok(RecentActivityResult {
        recent_llm_calls: llm_calls,
        recent_conversations: conversations,
        summary: ActivitySummary {
            active_conversations: active_convos,
            llm_calls_today,
            total_tokens_today,
            cost_today_usd,
        },
    })
}
