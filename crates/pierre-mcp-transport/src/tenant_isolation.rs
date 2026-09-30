// ABOUTME: Tenant isolation and multi-tenancy management for MCP server
// ABOUTME: Handles user validation, tenant context extraction, and access control
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::tenant::{TenantContext, TenantRole};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_runtime_context::McpDispatchCtx;
// Trait methods dispatched through repos.tenants / repos.users / repos.oauth_tokens
use std::sync::Arc;
use tracing::{error, warn};
use uuid::Uuid;

/// Manages tenant isolation and multi-tenancy for the MCP server
pub struct TenantIsolation {
    resources: Arc<dyn McpDispatchCtx>,
}

impl TenantIsolation {
    /// Create a new tenant isolation manager
    #[must_use]
    pub fn new(resources: Arc<dyn McpDispatchCtx>) -> Self {
        Self { resources }
    }

    /// Verify user belongs to a tenant via `tenant_users` table
    ///
    /// # Errors
    /// Returns an error if user does not belong to the tenant
    pub async fn verify_user_tenant_membership(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<()> {
        verified_tenant_role(self.resources.repos(), user_id, tenant_id).await?;
        Ok(())
    }

    /// Get user's default tenant (first tenant they belong to)
    ///
    /// # Errors
    /// Returns an error if user has no tenant memberships
    pub async fn get_user_default_tenant(&self, user_id: Uuid) -> AppResult<TenantId> {
        let tenants = self
            .resources
            .repos()
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;

        tenants
            .first()
            .map(|t| t.id)
            .ok_or_else(|| AppError::auth_invalid("User does not belong to any tenant"))
    }
}

/// The user's role in a tenant, read from the `tenant_users` junction table.
///
/// That table is the source of truth for membership: no row means the user is
/// not a member of the tenant, which is an authorization failure and not a
/// default role. Every membership lookup in this module funnels through here,
/// so the same refusal applies whether the tenant was named explicitly or
/// resolved from the user's tenant list. A membership that has been revoked
/// therefore stops granting a role as soon as the row is gone, even while a
/// token minted earlier still names the tenant.
///
/// The result is the only value fit to hand to
/// [`TenantContext::from_verified_membership`], whose contract is that the role
/// came from this exact (user, tenant) pair.
///
/// # Errors
/// Returns [`AppError::auth_invalid`] when the user has no membership row for
/// the tenant, or [`AppError::database`] when the lookup itself fails.
async fn verified_tenant_role(
    repos: &pierre_database::RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<TenantRole> {
    repos
        .tenants
        .get_user_role(user_id, tenant_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to check tenant membership: {e}")))?
        .map(|role| TenantRole::from_db_string(&role))
        .ok_or_else(|| {
            AppError::auth_invalid(format!(
                "User {user_id} does not belong to tenant {tenant_id}"
            ))
        })
}

/// Log a failed [`extract_tenant_context_internal`] at the level its cause
/// deserves.
///
/// A membership revoked or an account deleted after its token was issued is
/// the caller's state and warns; the lookup failing is the server's and
/// pages. The MCP host, the MCP tool dispatcher and A2A all read it here.
pub fn log_tenant_failure(user_id: Uuid, error: &AppError) {
    if error.is_server_fault() {
        error!(user_id = %user_id, error = %error, "Tenant context extraction failed");
    } else {
        warn!(user_id = %user_id, error = %error, "Tenant context refused");
    }
}

/// Resolve the tenant context a user acts under (internal helper)
///
/// Priority order:
/// 1. Explicit `tenant_id` parameter
/// 2. User's default tenant (from `tenant_users` table)
///
/// Either way the resolution costs a membership lookup against the
/// `tenant_users` table; a tenant the user does not belong to is refused
/// rather than resolved to a context with a default role. This is the same
/// membership rule `pierre_runtime_context::resolve_tenant` applies when it
/// selects a tenant id from an `active_tenant_id` claim — the two differ only
/// in what they are handed, not in what they trust.
///
/// Returns `Ok(None)` only when no tenant was named and the user belongs to
/// no tenant at all.
///
/// # Errors
/// Returns an error if the user is unknown, the lookup fails, or `user_id`
/// has no membership row for the resolved tenant
pub async fn extract_tenant_context_internal(
    repos: &Arc<pierre_database::RepositoryRegistry>,
    user_id: Uuid,
    tenant_id: Option<TenantId>,
) -> AppResult<Option<TenantContext>> {
    if let Some(tenant_id) = tenant_id {
        // A user paired with a named tenant must hold a membership row for it;
        // absence is refused rather than resolved to a role.
        let role = verified_tenant_role(repos, user_id, tenant_id).await?;

        let tenant_name = match repos.tenants.get_by_id(tenant_id).await {
            Ok(tenant) => tenant.name,
            _ => "Unknown Tenant".to_owned(),
        };

        return Ok(Some(TenantContext::from_verified_membership(
            tenant_id,
            tenant_name,
            user_id,
            role,
        )));
    }

    // SECURITY: Global lookup — resolving user's default tenant
    repos
        .users
        .get_global(user_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
        .ok_or_else(|| AppError::not_found("User"))?;

    // Get user's tenants from tenant_users table
    let tenants = repos
        .tenants
        .list_for_user(user_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;

    if let Some(default_tenant) = tenants.first() {
        let user_role = verified_tenant_role(repos, user_id, default_tenant.id).await?;

        return Ok(Some(TenantContext::from_verified_membership(
            default_tenant.id,
            default_tenant.name.clone(),
            user_id,
            user_role,
        )));
    }

    Ok(None)
}
