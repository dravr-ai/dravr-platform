// ABOUTME: The one rule deciding which tenants an admin credential reaches, for its reads and its writes on users
// ABOUTME: An admin reaches every tenant or picks one; a bound token stays on its own tenant; an unbound one reaches none
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which tenants an admin read covers (carnet#1440).
//!
//! The role comes from the authenticated caller, never from the request. What
//! the request carries is only a wish, the `tenant_id` query parameter. The
//! platform column is an admin's console session or a super-admin service
//! token; the other two are a scoped service token, or a tenant owner who is
//! not an admin:
//!
//! | `tenant_id` | Platform | Bound to a tenant | Bound to none |
//! |---|---|---|---|
//! | absent | every tenant | that tenant | refused |
//! | `all` | every tenant | refused | refused |
//! | a tenant id | that tenant | that tenant if it is theirs, else refused | refused |
//!
//! It is the one rule for what a credential reaches (carnet#1589): a read
//! naming a tenant ([`ScopeCaller::resolve`],
//! [`ValidatedAdminToken::require_tenant_access`]), and an action on users,
//! which names none ([`ValidatedAdminToken::reach`]): a bound token acts on
//! the accounts held entirely within its tenant
//! ([`AdminScope::holds_entirely`]), and a token bound to no tenant without
//! the super-admin flag reaches no tenant, so no user either. A setting
//! global to the platform takes the platform column alone
//! ([`ValidatedAdminToken::require_global_scope`]).

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
    /// Every tenant on the platform: the explicit platform-wide path.
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

impl AdminScope {
    /// Whether an account held in `tenants`, every tenant it is a member of
    /// or holds a provider in, lies within this scope: anywhere, for every
    /// tenant; for one tenant, only an account held there and nowhere else.
    /// An account held in no tenant (a sign-up not yet approved) belongs to
    /// no tenant's scope.
    #[must_use]
    pub fn holds_entirely(self, tenants: impl IntoIterator<Item = TenantId>) -> bool {
        match self {
            Self::AllTenants => true,
            Self::Tenant { tenant_id } => {
                let mut tenants = tenants.into_iter().peekable();
                tenants.peek().is_some() && tenants.all(|tenant| tenant == tenant_id)
            }
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
    /// A caller that reaches every tenant: an admin, or a super-admin
    /// service token.
    Platform,
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
            (Self::Platform, Some(tenant_id)) => Ok(AdminScope::Tenant { tenant_id }),
            (Self::Platform, None) => Ok(AdminScope::AllTenants),
            (Self::BoundTo(_) | Self::Unbound, None) if asks_for_all => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "Only an admin can read across every tenant",
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
fn other_tenant_refusal(bound: &str, requested: &str) -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        format!("Token is scoped to tenant {bound}, cannot access tenant {requested}"),
    )
}

/// The refusal of a token with neither a tenant nor the super-admin flag.
fn no_tenant_refusal() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "Token has no tenant scope — super-admin required for cross-tenant access",
    )
}

impl ValidatedAdminToken {
    /// What this credential reaches when it names no tenant: every tenant
    /// for an admin's session or a super-admin token, its own tenant for a
    /// token bound to one. The scope of every action on users.
    ///
    /// # Errors
    /// `PermissionDenied` for a token bound to no tenant without the
    /// super-admin flag, which reaches none; `InvalidInput` when the tenant
    /// the credential carries is not a tenant id.
    pub fn reach(&self) -> AppResult<AdminScope> {
        self.scope_caller()?.resolve(None)
    }

    /// Verify the credential may read `requested`, a tenant id: the scope
    /// rule's read naming one tenant ([`ScopeCaller::resolve`]).
    ///
    /// # Errors
    /// `PermissionDenied` when the credential does not reach `requested`;
    /// `InvalidInput` when `requested`, or the tenant the credential carries,
    /// is not a tenant id.
    pub fn require_tenant_access(&self, requested: &str) -> AppResult<()> {
        self.scope_caller()?.resolve(Some(requested)).map(drop)
    }

    /// Verify the credential may read or change a setting global to the
    /// platform: it must reach every tenant ([`Self::reach`]), as an admin's
    /// console session or a super-admin token does, whatever tenant it
    /// carries. A token bound to one tenant reaches only that one, and a
    /// token bound to no tenant without the super-admin flag reaches none,
    /// so neither changes what applies to every tenant.
    ///
    /// # Errors
    /// `PermissionDenied` for a credential that does not reach every tenant;
    /// `InvalidInput` when the tenant the credential carries is not a tenant id.
    pub fn require_global_scope(&self) -> AppResult<()> {
        match self.scope_caller()? {
            ScopeCaller::Platform => Ok(()),
            ScopeCaller::BoundTo(_) | ScopeCaller::Unbound => Err(AppError::new(
                ErrorCode::PermissionDenied,
                "This setting applies to the whole platform; a token that does not reach every tenant cannot read or change it",
            )),
        }
    }

    /// The scope rule's view of this credential.
    ///
    /// # Errors
    /// `InvalidInput` when the tenant the credential carries is not a tenant id.
    pub fn scope_caller(&self) -> AppResult<ScopeCaller> {
        if self.is_super_admin {
            return Ok(ScopeCaller::Platform);
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
    use crate::admin::models::AdminPermissions;
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
    fn a_platform_caller_reads_everything_unless_it_picks_one_tenant() {
        let caller = ScopeCaller::Platform;
        assert_eq!(
            caller.resolve(None).ok(),
            Some(AdminScope::AllTenants),
            "bug: a platform caller's default is the whole platform"
        );
        assert_eq!(
            caller.resolve(Some("all")).ok(),
            Some(AdminScope::AllTenants),
            "bug: a platform caller may name every tenant"
        );
        assert_eq!(
            caller.resolve(Some(THEIRS)).ok(),
            Some(AdminScope::Tenant {
                tenant_id: theirs()
            }),
            "bug: a platform caller's pick must narrow the read"
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
            ScopeCaller::Platform,
            ScopeCaller::BoundTo(mine()),
            ScopeCaller::Unbound,
        ] {
            assert!(
                matches!(caller.resolve(Some("ALL")), Err(e) if e.code == ErrorCode::InvalidInput),
                "bug: anything but `all` or an id must not read as a scope"
            );
        }
    }

    /// An account is within one tenant's scope only when every tenant it is
    /// held in is that one, and it is held in one at least; within every
    /// tenant's scope whatever it holds.
    #[test]
    fn a_tenant_holds_only_what_is_entirely_its_own() {
        let own = AdminScope::Tenant { tenant_id: mine() };
        assert!(own.holds_entirely([mine()]));
        assert!(own.holds_entirely([mine(), mine()]));
        assert!(
            !own.holds_entirely([mine(), theirs()]),
            "bug: held elsewhere too"
        );
        assert!(!own.holds_entirely([theirs()]), "bug: held elsewhere only");
        assert!(!own.holds_entirely([]), "bug: a sign-up is in no tenant");
        for tenants in [vec![], vec![mine()], vec![mine(), theirs()]] {
            assert!(AdminScope::AllTenants.holds_entirely(tenants));
        }
    }

    /// What a credential reaches naming no tenant, and the read naming one,
    /// are the module table's rows for every kind of caller.
    #[test]
    fn a_credential_reaches_what_the_table_says() {
        let token = |is_super_admin, tenant: Option<&str>| ValidatedAdminToken {
            token_id: "t".to_owned(),
            service_name: "s".to_owned(),
            permissions: AdminPermissions::super_admin(),
            is_super_admin,
            tenant_id: tenant.map(str::to_owned),
            operator_user_id: None,
            user_info: None,
        };
        assert_eq!(token(true, None).reach().ok(), Some(AdminScope::AllTenants));
        assert_eq!(
            token(true, Some(MINE)).reach().ok(),
            Some(AdminScope::AllTenants),
            "bug: a console session's active tenant does not bound it"
        );
        assert_eq!(
            token(false, Some(MINE)).reach().ok(),
            Some(AdminScope::Tenant { tenant_id: mine() })
        );
        assert!(
            matches!(token(false, None).reach(), Err(e) if e.code == ErrorCode::PermissionDenied),
            "bug: a token bound to no tenant reaches none"
        );

        assert!(token(true, None).require_tenant_access(THEIRS).is_ok());
        assert!(token(false, Some(MINE)).require_tenant_access(MINE).is_ok());
        for refused in [
            token(false, Some(MINE)).require_tenant_access(THEIRS),
            token(false, None).require_tenant_access(MINE),
        ] {
            assert!(matches!(refused, Err(e) if e.code == ErrorCode::PermissionDenied));
        }
    }

    /// A setting global to the platform is read and changed only by a
    /// credential that reaches every tenant: a token bound to one tenant is
    /// refused, and so is a token bound to none without the super-admin
    /// flag, which reaches no tenant at all.
    #[test]
    fn only_a_credential_reaching_every_tenant_touches_a_global_setting() {
        let token = |is_super_admin, tenant: Option<&str>| ValidatedAdminToken {
            token_id: "t".to_owned(),
            service_name: "s".to_owned(),
            permissions: AdminPermissions::super_admin(),
            is_super_admin,
            tenant_id: tenant.map(str::to_owned),
            operator_user_id: None,
            user_info: None,
        };
        // A console admin carries the active tenant here and is not bound by it.
        assert!(token(true, Some(MINE)).require_global_scope().is_ok());
        assert!(token(true, None).require_global_scope().is_ok());
        for (refused, why) in [
            (token(false, Some(MINE)), "bug: a token bound to one tenant"),
            (
                token(false, None),
                "bug: a token bound to no tenant reaches none",
            ),
        ] {
            assert!(
                matches!(refused.require_global_scope(), Err(e) if e.code == ErrorCode::PermissionDenied),
                "{why}"
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
