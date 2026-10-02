// ABOUTME: Who may create a coaching group, and who may be a group's human coach
// ABOUTME: One decision each, shared by the REST create route, its permissions read and the /group commands
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{TenantId, User};
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

/// Whether `user`, acting in `caller_tenant`, may be the human coach of a
/// group in `group_tenant`.
///
/// Coaching a group means reading its consenting athletes' training, so it
/// takes `manages_roster` — earned by a `TrainingPeaks` coach account or
/// granted by a super-admin, never self-served (ADR-018) — or platform
/// admin. Athlete membership is cross-tenant; coach attachment is not, so the
/// caller must act in the group's own tenant. The one rule behind redeeming a
/// coach invite (`/group join`) and creating a group as its coach
/// (`POST /api/groups`).
#[must_use]
pub fn may_coach_group(user: &User, caller_tenant: TenantId, group_tenant: TenantId) -> bool {
    (user.manages_roster || user.is_admin) && caller_tenant == group_tenant
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(manages_roster: bool, is_admin: bool) -> User {
        let mut user = User::new("coach@example.com".to_owned(), "hash".to_owned(), None);
        user.manages_roster = manages_roster;
        user.is_admin = is_admin;
        user
    }

    #[test]
    fn a_roster_manager_coaches_in_their_own_tenant() {
        let tenant = TenantId::generate();
        assert!(may_coach_group(&user(true, false), tenant, tenant));
    }

    #[test]
    fn a_platform_admin_coaches_in_their_own_tenant() {
        let tenant = TenantId::generate();
        assert!(may_coach_group(&user(false, true), tenant, tenant));
    }

    #[test]
    fn a_coach_without_the_grant_does_not() {
        let tenant = TenantId::generate();
        assert!(!may_coach_group(&user(false, false), tenant, tenant));
    }

    #[test]
    fn no_one_coaches_across_tenants() {
        assert!(!may_coach_group(
            &user(true, true),
            TenantId::generate(),
            TenantId::generate()
        ));
    }
}
