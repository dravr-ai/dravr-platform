// ABOUTME: The one rule deciding which tenants an admin read may cover, from the caller's server-side role
// ABOUTME: A super admin reads every tenant by default or picks one; anyone else stays on their own tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which tenants an admin read covers (carnet#1440).
//!
//! The role comes from the authenticated caller, never from the request. What
//! the request carries is only a wish, the `tenant_id` query parameter:
//!
//! | `tenant_id` | Super admin | Bound to a tenant | Bound to none |
//! |---|---|---|---|
//! | absent | every tenant | that tenant | refused |
//! | `all` | every tenant | refused | refused |
//! | a tenant id | that tenant | that tenant if it is theirs, else refused | refused |

use serde::Serialize;

use super::models::ValidatedAdminToken;
use crate::errors::{AppError, AppResult, ErrorCode};
use crate::models::TenantId;

/// The `tenant_id` value asking for every tenant.
pub const ALL_TENANTS_PARAM: &str = "all";

/// The tenants one admin read covers. Serialized into the response, so a screen
/// states what the server decided rather than what the client asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdminScope {
    /// Every tenant on the platform: the explicit super-admin path.
    AllTenants,
    /// One tenant.
    Tenant {
        /// The tenant read.
        tenant_id: TenantId,
    },
}

impl AdminScope {
    /// The tenant a read filters on, `None` for every tenant: what a scoped
    /// statement binds to its `($n IS NULL OR tenant_id = $n)` filter, so one
    /// statement serves both scopes.
    #[must_use]
    pub const fn tenant_filter(self) -> Option<TenantId> {
        match self {
            Self::AllTenants => None,
            Self::Tenant { tenant_id } => Some(tenant_id),
        }
    }
}

impl From<TenantId> for AdminScope {
    fn from(tenant_id: TenantId) -> Self {
        Self::Tenant { tenant_id }
    }
}

/// Who is asking, as far as the scope rule cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeCaller {
    /// A platform super admin.
    SuperAdmin,
    /// Anyone else who acts in one tenant.
    BoundTo(TenantId),
    /// Anyone else, with no tenant to act in.
    Unbound,
}

impl ScopeCaller {
    /// Decide the scope of a read from `requested`, the raw `tenant_id`
    /// parameter. See the module table.
    ///
    /// # Errors
    /// `InvalidInput` when `requested` is neither [`ALL_TENANTS_PARAM`] nor a
    /// tenant id; `PermissionDenied` when the caller may not read what it named.
    pub fn resolve(self, requested: Option<&str>) -> AppResult<AdminScope> {
        let named = requested
            .filter(|raw| *raw != ALL_TENANTS_PARAM)
            .map(|raw| {
                TenantId::parse_str(raw)
                    .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {raw}")))
            })
            .transpose()?;
        let asks_for_all = requested == Some(ALL_TENANTS_PARAM);
        match (self, named) {
            (Self::SuperAdmin, Some(tenant_id)) => Ok(AdminScope::Tenant { tenant_id }),
            (Self::SuperAdmin, None) => Ok(AdminScope::AllTenants),
            (Self::BoundTo(_) | Self::Unbound, None) if asks_for_all => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only a super admin can read across every tenant",
            )),
            (Self::BoundTo(own), None) => Ok(AdminScope::Tenant { tenant_id: own }),
            (Self::BoundTo(own), Some(tenant_id)) if tenant_id == own => {
                Ok(AdminScope::Tenant { tenant_id })
            }
            (Self::BoundTo(own), Some(tenant_id)) => Err(other_tenant_refusal(
                &own.to_string(),
                &tenant_id.to_string(),
            )),
            (Self::Unbound, _) => Err(no_tenant_refusal()),
        }
    }
}

/// The refusal of a credential scoped to `bound` that asks for `requested`.
pub(crate) fn other_tenant_refusal(bound: &str, requested: &str) -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        format!("Token is scoped to tenant {bound}, cannot access tenant {requested}"),
    )
}

/// The refusal of a credential with neither a tenant nor the super-admin role.
pub(crate) fn no_tenant_refusal() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "Token has no tenant scope — super-admin required for cross-tenant access",
    )
}

impl ValidatedAdminToken {
    /// The scope rule's view of this credential.
    ///
    /// # Errors
    /// `InvalidInput` when the tenant the credential carries is not a tenant id.
    pub fn scope_caller(&self) -> AppResult<ScopeCaller> {
        if self.is_super_admin {
            return Ok(ScopeCaller::SuperAdmin);
        }
        self.tenant_id
            .as_deref()
            .map_or(Ok(ScopeCaller::Unbound), |raw| {
                TenantId::parse_str(raw)
                    .map(ScopeCaller::BoundTo)
                    .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {raw}")))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const MINE: &str = "00000000-0000-0000-0000-000000000001";
    const THEIRS: &str = "00000000-0000-0000-0000-000000000002";

    const fn mine() -> TenantId {
        TenantId::from_uuid(Uuid::from_u128(1))
    }

    const fn theirs() -> TenantId {
        TenantId::from_uuid(Uuid::from_u128(2))
    }

    fn denied(result: AppResult<AdminScope>) -> bool {
        matches!(result, Err(e) if e.code == ErrorCode::PermissionDenied)
    }

    #[test]
    fn a_super_admin_reads_everything_unless_it_picks_one_tenant() {
        let caller = ScopeCaller::SuperAdmin;
        assert_eq!(
            caller.resolve(None).ok(),
            Some(AdminScope::AllTenants),
            "bug: a super admin's default is the whole platform"
        );
        assert_eq!(
            caller.resolve(Some("all")).ok(),
            Some(AdminScope::AllTenants),
            "bug: a super admin may name every tenant"
        );
        assert_eq!(
            caller.resolve(Some(THEIRS)).ok(),
            Some(AdminScope::Tenant {
                tenant_id: theirs()
            }),
            "bug: a super admin's pick must narrow the read"
        );
    }

    #[test]
    fn a_bound_caller_stays_on_its_own_tenant() {
        let caller = ScopeCaller::BoundTo(mine());
        let own = Some(AdminScope::Tenant { tenant_id: mine() });
        assert_eq!(caller.resolve(None).ok(), own, "bug: default is own tenant");
        assert_eq!(
            caller.resolve(Some(MINE)).ok(),
            own,
            "bug: own tenant named"
        );
        assert!(
            denied(caller.resolve(Some(THEIRS))),
            "bug: another tenant must be refused"
        );
        assert!(
            denied(caller.resolve(Some("all"))),
            "bug: every tenant must be refused"
        );
    }

    #[test]
    fn an_unbound_caller_reaches_nothing() {
        for requested in [None, Some("all"), Some(THEIRS)] {
            assert!(
                denied(ScopeCaller::Unbound.resolve(requested)),
                "bug: {requested:?} must be refused without a tenant"
            );
        }
    }

    #[test]
    fn a_malformed_tenant_is_invalid_input_for_every_caller() {
        for caller in [
            ScopeCaller::SuperAdmin,
            ScopeCaller::BoundTo(mine()),
            ScopeCaller::Unbound,
        ] {
            assert!(
                matches!(caller.resolve(Some("ALL")), Err(e) if e.code == ErrorCode::InvalidInput),
                "bug: anything but `all` or an id must not read as a scope"
            );
        }
    }

    #[test]
    fn the_scope_serializes_as_the_screens_read_it() {
        assert_eq!(
            serde_json::to_value(AdminScope::AllTenants).ok(),
            Some(serde_json::json!({ "kind": "all_tenants" })),
            "bug: wire shape read by the web client"
        );
        assert_eq!(
            serde_json::to_value(AdminScope::Tenant { tenant_id: mine() }).ok(),
            Some(serde_json::json!({ "kind": "tenant", "tenant_id": MINE })),
            "bug: wire shape read by the web client"
        );
    }
}
