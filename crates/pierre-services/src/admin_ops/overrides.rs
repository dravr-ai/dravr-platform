// ABOUTME: Operator overrides on a user or tenant: tier, plan, manages_roster, tool and rate-limit overrides
// ABOUTME: Shared by pierre-cli, the web admin console and the admin token API
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppError;
use pierre_core::models::{Tenant, TenantId, TenantPlan, User, UserTier};
use pierre_database::repositories::{UserRateLimitOverride, UserTierOverride, UserToolOverride};
use pierre_database::RepositoryRegistry;
use pierre_middleware::mask_email;
use tracing::info;
use uuid::Uuid;

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
        target_user_email = %mask_email(&updated.email),
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

/// Grant or revoke a user's `manages_roster` permission as an operator.
///
/// This is how a coach with no `TrainingPeaks` coach account becomes able to
/// coach a group. Single shared implementation behind the admin-token route,
/// the console route and `pierre-cli user set --manages-roster`.
///
/// A grant records `operator` (the acting admin, `None` for a service token)
/// and the time on the user row; that record is the audit trail, and the
/// `TrainingPeaks` reconciler never takes back a grant that carries one. A
/// revoke clears the grant and its record; a bound `TrainingPeaks` coach
/// account the user still holds earns the grant again on its next profile
/// read.
///
/// # Errors
///
/// Returns `NotFound` when the user does not exist, or the repository error
/// when the grant cannot be written.
pub async fn set_user_manages_roster(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    manages_roster: bool,
    operator: Option<Uuid>,
) -> Result<User, AppError> {
    repos
        .users
        .set_manages_roster_by_operator(user_id, manages_roster, operator)
        .await?;
    let updated = repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found("User not found"))?;
    info!(
        target_user_id = %user_id,
        target_user_email = %mask_email(&updated.email),
        operator_user_id = ?operator,
        manages_roster,
        "Admin manages_roster change applied"
    );
    Ok(updated)
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

/// Set or update a per-user monthly rate-limit override.
///
/// The authentication gate enforces it in place of the tier's monthly limit
/// until it is cleared. `monthly_limit` is a positive cap, or `None` for no
/// monthly ceiling; zero is rejected. Verifies the target user exists before
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
    monthly_limit: Option<u32>,
    note: Option<String>,
    set_by: Option<Uuid>,
) -> Result<(), AppError> {
    if monthly_limit == Some(0) {
        return Err(AppError::invalid_input(
            "The monthly limit must be a positive integer; use null for unlimited",
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
