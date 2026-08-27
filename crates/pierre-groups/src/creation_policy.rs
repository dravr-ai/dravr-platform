// ABOUTME: Who may create a coaching group — the tenant-role shortcut and the group_creation_policy config
// ABOUTME: One decision shared by the REST create route, its permissions read and the /group create command
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_database::repositories::TenantRepository;
use tracing::warn;
use uuid::Uuid;

/// The admin-config key holding a tenant's group-creation policy:
/// `"everyone"` lets any member create groups, anything else reserves it to
/// the tenant's owners and admins.
pub const GROUP_CREATION_POLICY_KEY: &str = "group_creation_policy";

/// The policy applied when the tenant has not set one — the most
/// restrictive, so an unconfigured tenant never opens creation by accident.
pub const DEFAULT_GROUP_CREATION_POLICY: &str = "admins_only";

/// Whether `user_id` holds a tenant role that creates groups whatever the
/// policy says — the tenant's owner or an admin.
///
/// A role read that fails is reported as "not an admin" so the caller falls
/// through to the policy rather than either failing the request or handing
/// out the admin shortcut on an error.
pub async fn is_tenant_group_admin(
    tenants: &dyn TenantRepository,
    user_id: Uuid,
    tenant_id: TenantId,
) -> bool {
    let role = match tenants.get_user_role(user_id, tenant_id).await {
        Ok(role) => role,
        Err(e) => {
            warn!(
                %user_id, %tenant_id, error = %e,
                "Failed to read tenant role during group-creation permission check; \
                 proceeding without admin shortcut and applying the configured policy"
            );
            None
        }
    };
    role.as_deref()
        .is_some_and(|r| r == "owner" || r == "admin")
}

/// Apply the tenant's policy to a caller the role shortcut did not admit.
///
/// # Errors
///
/// Returns [`ErrorCode::PermissionDenied`] for `admins_only` and for any
/// value that is not a policy this platform knows.
pub fn policy_permits_group_creation(policy: &str) -> AppResult<()> {
    match policy {
        "everyone" => Ok(()),
        "admins_only" => Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Group creation requires admin privileges. Contact your tenant administrator.",
        )),
        _ => Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Group creation is not enabled for your account.",
        )),
    }
}

/// Check whether `user_id` may create a group in `tenant_id`.
///
/// Tenant owners and admins always may. Everyone else is subject to the
/// tenant's [`GROUP_CREATION_POLICY_KEY`], read lazily through `policy` — the
/// caller supplies the admin-config read as a future so it is paid only for
/// callers the role shortcut does not settle. A missing policy is
/// [`DEFAULT_GROUP_CREATION_POLICY`].
///
/// # Errors
///
/// Returns [`ErrorCode::PermissionDenied`] when the policy refuses the caller.
pub async fn check_create_group_permission<F>(
    tenants: &dyn TenantRepository,
    user_id: Uuid,
    tenant_id: TenantId,
    policy: F,
) -> AppResult<()>
where
    F: Future<Output = Option<String>>,
{
    if is_tenant_group_admin(tenants, user_id, tenant_id).await {
        return Ok(());
    }
    let policy = policy
        .await
        .unwrap_or_else(|| DEFAULT_GROUP_CREATION_POLICY.to_owned());
    policy_permits_group_creation(&policy)
}
