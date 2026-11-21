// ABOUTME: The console admin's side of the scope rule: resolves a read's tenants and lists the tenants an admin can pick
// ABOUTME: Every screen scoped by carnet#1440 reads through here; the rule itself is pierre_core::admin::scope
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which tenants a console read covers (carnet#1440).
//!
//! An admin's console session, like a super-admin token, reads the platform or
//! picks one tenant; a token scoped to a tenant stays on it. These read routes
//! check that rule instead of `ViewConfiguration`, so a scoped token that
//! lacks it is shown its own tenant rather than refused outright, while
//! harness configuration stays out of its reach.

use std::cmp::Ordering;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::{Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::repositories::RecentReadKey;

use crate::audit::{record_admin_read, AuditDetail};
use crate::context::AdminApiContext;

/// Resolve the tenants a read covers for a console or CLI admin credential.
///
/// An admin's console session and the token of a device login both carry the
/// super-admin flag, so neither is bound to a tenant: they read the platform,
/// or the one tenant they name. A token without the flag carries its binding
/// in its own stored row, the only thing that decides what it reads.
///
/// # Errors
/// What [`ScopeCaller::resolve`](pierre_core::admin::scope::ScopeCaller::resolve)
/// refuses, and `InvalidInput` when the token's own tenant is not a tenant id.
pub(crate) fn console_read_scope(
    admin_token: &ValidatedAdminToken,
    requested: Option<&str>,
) -> AppResult<AdminScope> {
    admin_token.scope_caller()?.resolve(requested)
}

/// Refuse a caller bound to one tenant what belongs to the platform: the
/// standing allow-list and the sign-up codes.
///
/// A standing allow and a sign-up code are platform-wide: each lets a
/// pending account in on the spot, whatever tenant that account is or will
/// be in, and their lists name addresses and accounts of every tenant. A
/// `ManageUsers` token scoped to one tenant approves by id only a user
/// held within its tenant (`account_in_reach`), so through these doors
/// it would approve the sign-ups it is refused there, and read what its
/// user reads leave out. An admin's console session and a super-admin
/// token pass; a token bound to no tenant reaches no tenant and is refused
/// as on the rest of the users surface ([`ValidatedAdminToken::reach`]).
///
/// The refusal reads "Token is scoped to tenant T; `subject` platform-wide,
/// and it cannot `act`": `subject` is what is refused with its verb ("a
/// sign-up code is"), `act` what the caller asked for ("mint one").
///
/// # Errors
///
/// A permission error for a caller bound to one tenant or to none, and
/// `InvalidInput` when the tenant the caller carries is not a tenant id.
pub(super) fn require_platform_reach(
    admin_token: &ValidatedAdminToken,
    subject: &str,
    act: &str,
) -> AppResult<()> {
    match admin_token.reach()? {
        AdminScope::AllTenants => Ok(()),
        AdminScope::Tenant { tenant_id: own } => Err(AppError::new(
            ErrorCode::PermissionDenied,
            format!(
                "Token is scoped to tenant {own}; {subject} platform-wide, and it cannot {act}"
            ),
        )),
    }
}

/// The line an audit row carries when `scope` reaches past the tenant the
/// caller acts in; `None` when the caller reads only their own.
fn cross_tenant_read_summary(
    scope: AdminScope,
    admin_token: &ValidatedAdminToken,
) -> Option<String> {
    match scope {
        AdminScope::AllTenants => Some("Read reply sentences across every tenant".to_owned()),
        AdminScope::Tenant { tenant_id } => {
            let tenant = tenant_id.to_string();
            (admin_token.tenant_id.as_deref() != Some(tenant.as_str()))
                .then(|| format!("Read reply sentences of tenant {tenant}"))
        }
    }
}

/// The action key of a cross-tenant sentence read on the trail.
const CROSS_TENANT_SENTENCES_READ: &str = "claim_sentences.read_cross_tenant";

/// How long one recorded cross-tenant read stands for its repeats. The console
/// refetches these screens every two minutes; a row per refetch buried every
/// other action on the trail without saying anything the first row had not.
const CROSS_TENANT_READ_WINDOW: Duration = Duration::hours(1);

/// Record a read of reply sentences that reached past the caller's own tenant.
/// A verdict quotes what an athlete was told, so another tenant's are on the
/// trail. `target` says what came back.
///
/// At most one row per admin, route and scope per
/// [`CROSS_TENANT_READ_WINDOW`]: a repeat within it is the access already on
/// the trail. A credential naming no admin (a service token) is recorded
/// every time, since nothing tells its reads apart from another token's; so is
/// any read whose check fails, as a trail with a repeat beats one with a gap.
pub(crate) async fn audit_cross_tenant_sentences(
    context: &AdminApiContext,
    admin_token: &ValidatedAdminToken,
    headers: &HeaderMap,
    route: &str,
    scope: AdminScope,
    target: Value,
) {
    let Some(summary) = cross_tenant_read_summary(scope, admin_token) else {
        return;
    };
    let audit = context.repos.admin_audit.as_ref();
    if let Some(actor_user_id) = admin_token.operator_user_id {
        let key = RecentReadKey {
            actor_user_id,
            action: CROSS_TENANT_SENTENCES_READ,
            route,
            summary: &summary,
            since: Utc::now() - CROSS_TENANT_READ_WINDOW,
        };
        match audit.has_recent_read(&key).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => {
                warn!(error = %e, route, "could not check the trail for a recent read; recording it");
            }
        }
    }
    record_admin_read(
        audit,
        route,
        admin_token,
        headers,
        AuditDetail::new(CROSS_TENANT_SENTENCES_READ, summary).target(target),
    )
    .await;
}

/// One tenant an admin can pick.
#[derive(Debug, Serialize)]
pub struct ScopeTenant {
    /// Tenant id, sent back as `tenant_id` on the scoped reads.
    pub id: String,
    /// Display name.
    pub name: String,
    /// URL slug, which tells apart workspaces that share a name.
    pub slug: String,
}

/// Response for `GET /api/admin/tenants`.
#[derive(Debug, Serialize)]
pub struct ScopeTenantsResponse {
    /// Every tenant, by name.
    pub tenants: Vec<ScopeTenant>,
}

/// Handle `GET /api/admin/tenants`: what the shared scope control offers.
///
/// # Errors
/// `PermissionDenied` for a scoped token, which has no other tenant to pick
/// and must not learn the names of the others; a database error on the read.
pub async fn handle_list_scope_tenants(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if !admin_token.is_super_admin {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only an admin can list every tenant",
        ));
    }
    let mut tenants: Vec<ScopeTenant> = context
        .repos
        .tenants
        .get_all()
        .await?
        .into_iter()
        .map(|tenant| ScopeTenant {
            id: tenant.id.to_string(),
            name: tenant.name,
            slug: tenant.slug,
        })
        .collect();
    tenants.sort_by(|a, b| match a.name.cmp(&b.name) {
        Ordering::Equal => a.slug.cmp(&b.slug),
        order => order,
    });
    Ok((StatusCode::OK, Json(ScopeTenantsResponse { tenants })))
}
