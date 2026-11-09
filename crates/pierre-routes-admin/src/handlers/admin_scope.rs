// ABOUTME: The console admin's side of the scope rule: resolves a read's tenants and lists the tenants a super admin can pick
// ABOUTME: Every screen scoped by carnet#1440 reads through here; the rule itself is pierre_core::admin::scope
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which tenants a console read covers (carnet#1440).
//!
//! A plain admin stays on their own tenant, a super admin reads the platform
//! or picks one tenant. These read routes check that rule instead of
//! `ViewConfiguration`, which `console_admin` deliberately lacks: without
//! this, a plain admin on one of these screens would be refused outright rather
//! than shown their own tenant, while harness configuration stays out of reach.

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
use pierre_core::admin::scope::{AdminScope, ScopeCaller};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::repositories::RecentReadKey;
use pierre_database::RepositoryRegistry;

use crate::audit::{record_admin_read, AuditDetail};
use crate::context::AdminApiContext;

/// Resolve the tenants a read covers for a console or CLI admin credential.
///
/// A session's tenant comes from a signed claim that outlives a membership, so
/// a plain admin's is checked against the membership table: a removed admin
/// holding a token issued earlier reads nothing. A service token names no
/// person and carries its binding in its own stored row.
///
/// # Errors
/// What [`ScopeCaller::resolve`] refuses, plus a refusal when the operator no
/// longer belongs to the tenant, and a database error on the membership read.
pub(crate) async fn console_read_scope(
    repos: &RepositoryRegistry,
    admin_token: &ValidatedAdminToken,
    requested: Option<&str>,
) -> AppResult<AdminScope> {
    let caller = admin_token.scope_caller()?;
    let scope = caller.resolve(requested)?;
    if let (ScopeCaller::BoundTo(own), Some(operator)) = (caller, admin_token.operator_user_id) {
        if repos.tenants.get_user_role(operator, own).await?.is_none() {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "You no longer belong to the tenant this session acts in",
            ));
        }
    }
    Ok(scope)
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
/// At most one row per operator, route and scope per
/// [`CROSS_TENANT_READ_WINDOW`]: a repeat within it is the access already on
/// the trail. A credential naming no operator (a service token) is recorded
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

/// One tenant a super admin can pick.
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
/// `PermissionDenied` for anyone but a super admin, who has no other tenant to
/// pick and must not learn the names of the others; a database error on the read.
pub async fn handle_list_scope_tenants(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if !admin_token.is_super_admin {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only a super admin can list every tenant",
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
