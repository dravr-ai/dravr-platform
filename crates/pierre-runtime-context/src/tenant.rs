// ABOUTME: Canonical tenant resolution — single helper that EVERY route + service calls before touching tenant-scoped data
// ABOUTME: Replaces 10+ ad-hoc `TenantId::from(user_id)` fallbacks; verifies membership when active_tenant_id is claimed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Canonical per-request tenant resolution.
//!
//! Every route handler and tenant-scoped service must pass the authenticated
//! [`AuthResult`] through [`resolve_tenant`] before reading or writing
//! tenant-scoped data; there is exactly one tenant-resolution policy in
//! the codebase, and it lives here.
//!
//! ## Why this exists
//!
//! Before this helper, ~10 route handlers each carried their own copy of
//! `map_or_else(|| TenantId::from(user_id), |t| t.id)`. That pattern has
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
//!   Never `TenantId::from(user_id)`.
//! - **No tenant creation.** Onboarding flows that *make* a tenant must
//!   call the create path explicitly; they then have a `TenantId`
//!   already and don't need to resolve.
//! - **No caching.** Every call re-reads `tenants.list_for_user`. The
//!   list is small (typically 1–3 rows) and freshness matters when the
//!   user has just been added to or removed from a tenant.

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;

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
    let tenants = ctx.repos().tenants.list_for_user(auth.user_id).await?;

    if let Some(claimed) = auth.active_tenant_id {
        if tenants.iter().any(|t| t.id.as_uuid() == claimed) {
            return Ok(Some(TenantId::from(claimed)));
        }
        return Err(AppError::auth_invalid(
            "active_tenant_id claim does not match any tenant the user belongs to",
        ));
    }

    if let Some(first) = tenants.first() {
        return Ok(Some(first.id));
    }

    match mode {
        TenantMode::Optional => Ok(None),
        TenantMode::Required => Err(AppError::auth_invalid("no tenant associated with user")),
    }
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
