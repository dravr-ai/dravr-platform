// ABOUTME: Canonical tenant resolution — the single rule EVERY route + service applies before touching tenant-scoped data
// ABOUTME: Never fabricates a tenant from the user uuid; verifies membership when active_tenant_id is claimed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Canonical per-request tenant resolution.
//!
//! Every route handler and tenant-scoped service passes the authenticated
//! [`AuthResult`] through [`resolve_tenant`] before reading or writing
//! tenant-scoped data; that is how a request *selects* which tenant it acts
//! for, and the policy for selecting one lives here and nowhere else.
//!
//! There is one rule and two doors onto it, which differ only in what the
//! caller holds:
//!
//! - [`resolve_tenant`] — a caller holding an `Arc` of a [`MiddlewareCtx`]
//!   (every router generic over a runtime-context trait, and the server's own
//!   routes).
//! - [`resolve_tenant_in`] — a caller holding the tenants repository and no
//!   runtime context (the auth routes' focused context, a service).
//!
//! No route reads `AuthResult.active_tenant_id` to decide which tenant it
//! acts for, and none takes the head of `tenants.first_for_user` for that: a
//! claim is a request, and only a membership row answers it. What still reads
//! the claim directly is what carries it rather than acts on it — minting a
//! token, reporting which tenant a session names, recording the request's
//! identity for logs.
//!
//! ## Why this exists
//!
//! Before this helper, ~10 route handlers each carried their own copy of
//! `map_or_else(|| TenantId::from_uuid(user_id), |t| t.id)`. That pattern has
//! two problems:
//!
//! 1. **Security.** The fallback fabricates a tenant id out of the user
//!    uuid. A query gated by `tenant_id = $1` would then return rows
//!    tagged with the user uuid — silent cross-tenant access if the
//!    database invariant `tenant_id == user_id` ever broke. The fallback
//!    masked the real "user has no tenant" case behind a forged id.
//! 2. **Trust gap.** Callers that trusted `AuthResult.active_tenant_id`
//!    never verified that the user actually belonged to the claimed
//!    tenant. A stale or forged JWT claim could request cross-tenant
//!    access; the parsing layer in `pierre-auth` only validates the
//!    JWT signature, not membership.
//!
//! [`resolve_tenant`] closes both: it never fabricates an id, and it
//! verifies membership whenever `active_tenant_id` is present.
//!
//! ## The other half of the rule
//!
//! The MCP and A2A tool-execution paths are handed a tenant that is already
//! named — by an argument, a header, or an `active_tenant_id` claim — so they
//! build a `pierre_auth::tenant::TenantContext` rather than select an id.
//! `pierre_mcp_transport::tenant_isolation` is where that happens, and it
//! applies the same membership rule: a `(user, tenant)` pair with no
//! `tenant_users` row is refused, never resolved to a default role. Selecting
//! an id and naming one are different jobs; trusting an unverified claim is not
//! a difference between them.
//!
//! ## Modes
//!
//! See [`TenantMode`]. Pick based on the route's threat model:
//!
//! - [`TenantMode::Required`] — handler refuses if no tenant resolves.
//!   This is the default; use it everywhere a write touches tenant-scoped
//!   data.
//! - [`TenantMode::Optional`] — read-only diagnostics or routes that
//!   serve users who haven't been assigned a tenant yet (the user might
//!   be mid-onboarding). The handler MUST gracefully handle `None`.
//!
//! ## What this helper does NOT do
//!
//! - **No user-id fallback.** If the user has no tenants AND no
//!   `active_tenant_id`, the result is either an error
//!   ([`TenantMode::Required`]) or `None` ([`TenantMode::Optional`]).
//!   Never `TenantId::from_uuid(user_id)`.
//! - **No tenant creation.** Onboarding flows that *make* a tenant must
//!   call the create path explicitly; they then have a `TenantId`
//!   already and don't need to resolve.
//! - **No caching.** Every call re-reads `tenants.list_for_user`. The
//!   list is small (typically 1–3 rows) and freshness matters when the
//!   user has just been added to or removed from a tenant.

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_core::admin::scope::{AdminScope, ScopeCaller};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::TenantRepository;
use uuid::Uuid;

use crate::MiddlewareCtx;

/// Tenant-resolution semantics. Pick based on the route's threat model;
/// see module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantMode {
    /// Handler refuses with a 401-class error if no tenant resolves.
    /// Use for writes and reads of tenant-scoped data.
    Required,
    /// Resolves to `None` when no tenant is associated with the user.
    /// Caller MUST handle the `None` branch explicitly.
    Optional,
}

/// Resolve the per-request tenant id.
///
/// # Resolution order
///
/// 1. If `auth.active_tenant_id` is `Some(id)`: look up the user's
///    tenant list and return the claimed id only when membership is
///    verified; otherwise return [`AppError::auth_invalid`] (stale or
///    forged JWT claim).
/// 2. Else, fall back to the user's first listed tenant (single-tenant
///    accounts and tokens without an `active_tenant_id` claim).
/// 3. If the list is empty:
///    - `Required` → [`AppError::auth_invalid`] with message "no tenant
///      associated with user".
///    - `Optional` → `Ok(None)`.
///
/// # Errors
///
/// - Repository read failure on `tenants.list_for_user` propagates as
///   [`AppError`].
/// - `Required` with no resolution returns an auth error; never
///   fabricates a `TenantId` from the user uuid.
/// - `active_tenant_id` claim that doesn't match a real membership
///   returns an auth error.
pub async fn resolve_tenant<C>(
    ctx: &Arc<C>,
    auth: &AuthResult,
    mode: TenantMode,
) -> AppResult<Option<TenantId>>
where
    C: MiddlewareCtx + ?Sized,
{
    resolve_tenant_in(ctx.repos().tenants.as_ref(), auth, mode).await
}

/// [`resolve_tenant`] for a caller that holds the tenants repository and no
/// runtime context: the same rule, read from the same membership rows.
///
/// # Errors
///
/// Everything [`resolve_tenant`] returns.
pub async fn resolve_tenant_in(
    tenants: &dyn TenantRepository,
    auth: &AuthResult,
    mode: TenantMode,
) -> AppResult<Option<TenantId>> {
    let memberships: Vec<TenantId> = tenants
        .list_for_user(auth.user_id)
        .await?
        .iter()
        .map(|tenant| tenant.id)
        .collect();
    select_tenant(&memberships, auth.active_tenant_id, mode)
}

/// The selection rule itself, over the tenants the user belongs to in the
/// order the repository lists them.
fn select_tenant(
    memberships: &[TenantId],
    claimed: Option<Uuid>,
    mode: TenantMode,
) -> AppResult<Option<TenantId>> {
    if let Some(claimed) = claimed {
        if memberships.iter().any(|t| t.as_uuid() == claimed) {
            return Ok(Some(TenantId::from_uuid(claimed)));
        }
        return Err(AppError::auth_invalid(
            "active_tenant_id claim does not match any tenant the user belongs to",
        ));
    }

    if let Some(first) = memberships.first() {
        return Ok(Some(*first));
    }

    match mode {
        TenantMode::Optional => Ok(None),
        TenantMode::Required => Err(AppError::auth_invalid("no tenant associated with user")),
    }
}

/// The admin scope rule's view of a caller authenticated as a user.
///
/// The role is the one `cookie_admin_middleware` reads, `users.role`, fetched
/// here on every request so a role change applies at once and nothing in the
/// request or the token's claims can claim it. An admin reaches every tenant;
/// anyone else is bound to the tenant [`resolve_tenant`] verifies they belong
/// to.
///
/// # Errors
///
/// A repository failure, an unknown user, or a caller with no tenant.
pub async fn scope_caller<C>(ctx: &Arc<C>, auth: &AuthResult) -> AppResult<ScopeCaller>
where
    C: MiddlewareCtx + ?Sized,
{
    // SECURITY: Global lookup — the role is what decides whether a tenant applies.
    let user = ctx
        .repos()
        .users
        .get_global(auth.user_id)
        .await?
        .ok_or_else(|| AppError::auth_invalid("user no longer exists"))?;
    if user.role.is_admin() {
        return Ok(ScopeCaller::Platform);
    }
    let own = require(resolve_tenant(ctx, auth, TenantMode::Required).await?)?;
    Ok(ScopeCaller::BoundTo(own))
}

/// The scope of an analytics read on a route that keeps user auth, so a tenant
/// owner who is not an admin is admitted on their tenant role.
///
/// An admin is decided by `users.role` and reads every tenant. Anyone else
/// must hold an admin or owner role in the tenant they are bound to, then
/// stays on it; a plain member is refused with the error `role_refusal`
/// builds, a 403, which leaves their session alone where a 401 would sign
/// them out.
///
/// # Errors
///
/// `PermissionDenied` for a plain member, everything [`scope_caller`] returns, and
/// whatever [`ScopeCaller::resolve`] refuses in `requested`.
pub async fn analytics_read_scope<C>(
    ctx: &Arc<C>,
    auth: &AuthResult,
    requested: Option<&str>,
    role_refusal: impl FnOnce() -> AppError,
) -> AppResult<AdminScope>
where
    C: MiddlewareCtx + ?Sized,
{
    let caller = scope_caller(ctx, auth).await?;
    if let ScopeCaller::BoundTo(own) = caller {
        let tenant_role = ctx.repos().tenants.get_user_role(auth.user_id, own).await?;
        if !matches!(tenant_role.as_deref(), Some("admin" | "owner")) {
            return Err(role_refusal());
        }
    }
    caller.resolve(requested)
}

/// Convenience wrapper that errors on `Optional` resolutions that returned
/// `None`. Use this when the handler took an [`Option`] from
/// [`resolve_tenant`] and downstream code needs the id unconditionally.
///
/// # Errors
///
/// Returns [`AppError::auth_invalid`] if the inner value is `None`.
pub fn require<T>(opt: Option<T>) -> AppResult<T> {
    opt.ok_or_else(|| AppError::auth_invalid("tenant required for this operation but not resolved"))
}

/// The tenant a request acts in, by the canonical rule; a caller with no
/// tenant at all is refused.
///
/// # Errors
///
/// Whatever [`resolve_tenant`] returns under [`TenantMode::Required`].
pub async fn require_tenant<C>(ctx: &Arc<C>, auth: &AuthResult) -> AppResult<TenantId>
where
    C: MiddlewareCtx + ?Sized,
{
    require(resolve_tenant(ctx, auth, TenantMode::Required).await?)
}

/// [`require_tenant`] for a caller that holds the tenant repository and no
/// context.
///
/// # Errors
///
/// Whatever [`resolve_tenant_in`] returns under [`TenantMode::Required`].
pub async fn require_tenant_in(
    tenants: &dyn TenantRepository,
    auth: &AuthResult,
) -> AppResult<TenantId> {
    require(resolve_tenant_in(tenants, auth, TenantMode::Required).await?)
}

#[cfg(test)]
mod tests {
    use super::{select_tenant, TenantMode};
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::TenantId;
    use uuid::Uuid;

    #[test]
    fn a_claim_the_user_belongs_to_selects_that_tenant_over_the_first() {
        let first = TenantId::from_uuid(Uuid::new_v4());
        let second = Uuid::new_v4();
        let memberships = [first, TenantId::from_uuid(second)];

        let selected = select_tenant(&memberships, Some(second), TenantMode::Required);

        assert_eq!(
            selected.ok().flatten().map(|t| t.as_uuid()),
            Some(second),
            "a verified claim is the tenant the request acts for"
        );
    }

    #[test]
    fn a_claim_with_no_membership_is_refused_in_both_modes() {
        let memberships = [TenantId::from_uuid(Uuid::new_v4())];
        let stale = Uuid::new_v4();

        for mode in [TenantMode::Required, TenantMode::Optional] {
            let refused = select_tenant(&memberships, Some(stale), mode);
            assert_eq!(
                refused.err().map(|e| e.code),
                Some(ErrorCode::AuthInvalid),
                "a claim that outlived its membership never falls back to another tenant"
            );
        }
    }

    #[test]
    fn no_claim_selects_the_first_tenant() {
        let first = Uuid::new_v4();
        let memberships = [
            TenantId::from_uuid(first),
            TenantId::from_uuid(Uuid::new_v4()),
        ];

        let selected = select_tenant(&memberships, None, TenantMode::Required);

        assert_eq!(
            selected.ok().flatten().map(|t| t.as_uuid()),
            Some(first),
            "a credential without a claim acts for the account's first tenant"
        );
    }

    #[test]
    fn no_tenant_at_all_is_an_error_when_required_and_none_when_optional() {
        let required = select_tenant(&[], None, TenantMode::Required);
        assert_eq!(
            required.err().map(|e| e.code),
            Some(ErrorCode::AuthInvalid),
            "a route that needs a tenant refuses an account that has none"
        );

        let optional = select_tenant(&[], None, TenantMode::Optional);
        assert!(
            matches!(optional, Ok(None)),
            "a route that can serve an account with no tenant is told there is none"
        );
    }
}
