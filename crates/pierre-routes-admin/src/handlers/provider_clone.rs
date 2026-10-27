// ABOUTME: Super-admin route that copies one account's scrape-session provider connection onto another
// ABOUTME: Refuses OAuth grants and dead sessions, audits every clone it runs, and never logs the session itself

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The operator clone of a provider connection.
//!
//! End-to-end testing onboarding on a deployed environment needs fresh
//! accounts with a working Garmin connection, and a Garmin login per account
//! means a captcha-prone SSO round per test. A sciotte scrape session is a
//! cookie jar the scraper replays, so it can be shared: this route copies the
//! source account's session onto the target, same session id, through the
//! same store and data warm-up a login runs
//! ([`pierre_services::scrape_session`] has the rules on what may be cloned
//! and why).
//!
//! Super-admin only, since it hands one account read access to another's
//! provider data, and a super-admin may act across tenants: the session lands
//! in the target's own default tenant (the first active one, the same
//! resolution the tenant middleware uses), wherever the source's row lives.
//! No `provider.connected` event is raised (an operator copy is not an
//! athlete connecting); an `admin_token_usage` row names the token, both
//! accounts and the backend before anything is written. Logs carry ids only,
//! never the session.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use serde_json::json;
use tracing::{error, info};
use uuid::Uuid;

use pierre_core::admin::models::{AdminAction, AdminTokenUsage, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_services::scrape_session::{find_clone_source, CloneRefusal};

use super::api_keys::json_response;
use super::strava_pool::deny_if_not_super_admin;
use super::types::{AdminResponse, CloneProviderRequest};
use super::user_removal::{load_user, refusal};
use crate::context::AdminApiContext;

/// The longest operator reason an audit row keeps, in characters.
const MAX_AUDIT_REASON_CHARS: usize = 200;

/// One clone attempt, as its audit row names it.
struct CloneAudit<'call> {
    from_user: Uuid,
    to_user: Uuid,
    backend: &'call str,
    reason: Option<&'call str>,
}

/// Append one audit row for this clone attempt.
async fn record_clone(
    context: &AdminApiContext,
    admin_token: &ValidatedAdminToken,
    audit: &CloneAudit<'_>,
    failure: Option<&AppError>,
) -> AppResult<()> {
    let mut target = format!(
        "user:{}:provider:{}:from:{}",
        audit.to_user, audit.backend, audit.from_user
    );
    // The reason is operator free text, so it comes last (the ids before it
    // stay parseable whatever it holds) and is capped.
    if let Some(reason) = audit.reason {
        target.push_str(":reason:");
        target.extend(reason.chars().take(MAX_AUDIT_REASON_CHARS));
    }
    context
        .repos
        .admin
        .record_token_usage(&AdminTokenUsage {
            id: None,
            admin_token_id: admin_token.token_id.clone(),
            timestamp: Utc::now(),
            action: AdminAction::CloneProviderConnection,
            target_resource: Some(target),
            ip_address: None,
            user_agent: None,
            request_size_bytes: None,
            success: failure.is_none(),
            error_message: failure.map(AppError::internal_details),
            response_time_ms: None,
        })
        .await
}

/// The status each refusal answers with.
const fn refusal_status(refusal: &CloneRefusal) -> StatusCode {
    match refusal {
        CloneRefusal::NotCloneable { .. } | CloneRefusal::NotASession { .. } => {
            StatusCode::BAD_REQUEST
        }
        CloneRefusal::NoSource { .. } => StatusCode::NOT_FOUND,
        CloneRefusal::SessionDead { .. } => StatusCode::CONFLICT,
    }
}

/// `POST /admin/users/{user_id}/providers/{provider}/clone` — copy
/// `from_user_id`'s scrape session for `provider` onto `user_id`.
///
/// `provider` is the user-facing name (`garmin`, `strava`) or the backend
/// itself (`sciotte_garmin`, `sciotte`). Answers 403 for anything but a
/// super-admin token; 404 for an unknown account or a source with no session
/// for the provider; 400 when source and target are one account, for a
/// provider that cannot be cloned (an OAuth grant, `TrainingPeaks`) and for a
/// target that belongs to no active tenant; 409 when the source's session
/// needs re-authentication. Nothing is written in any of those cases.
///
/// # Errors
///
/// Returns a resource-unavailable error when no session installer is wired,
/// an invalid-input error for a malformed id, and a database error when a
/// read, the audit row or the store fails.
pub async fn handle_clone_user_provider(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path((user_id, provider)): Path<(String, String)>,
    Json(request): Json<CloneProviderRequest>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let ctx = context.as_ref();
    let installer = ctx.session_installer.as_deref().ok_or_else(|| {
        AppError::resource_unavailable("Session clone is not wired on this server")
    })?;
    let (to_user, target) = load_user(ctx, &user_id).await?;
    let (from_user, source_user) = load_user(ctx, &request.from_user_id).await?;
    if from_user == to_user {
        return Ok(refusal(
            StatusCode::BAD_REQUEST,
            "Source and target are the same account".to_owned(),
            None,
        ));
    }

    let source = match find_clone_source(&ctx.repos, from_user, &provider).await? {
        Ok(source) => source,
        Err(refused) => {
            return Ok(refusal(refusal_status(&refused), refused.message(), None));
        }
    };
    // The target's default tenant: the first active one it belongs to.
    let Some(tenant_id) = ctx.repos.tenants.first_for_user(to_user).await? else {
        return Ok(refusal(
            StatusCode::BAD_REQUEST,
            "The target account belongs to no active tenant to connect the provider in".to_owned(),
            None,
        ));
    };

    let audit = CloneAudit {
        from_user,
        to_user,
        backend: &source.backend,
        reason: request.reason.as_deref(),
    };
    record_clone(ctx, &admin_token, &audit, None).await?;
    if let Err(e) = installer
        .install_session(to_user, tenant_id, &source.backend, &source.session)
        .await
    {
        if let Err(audit_error) = record_clone(ctx, &admin_token, &audit, Some(&e)).await {
            error!(
                to_user = %to_user,
                token_id = %admin_token.token_id,
                error = %audit_error.internal_details(),
                "Could not record the failed provider clone"
            );
        }
        return Err(e);
    }

    info!(
        from_user = %from_user,
        to_user = %to_user,
        backend = %source.backend,
        source_tenant = %source.tenant_id,
        target_tenant = %tenant_id,
        token_id = %admin_token.token_id,
        "Operator cloned a scrape-session provider connection"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!(
                "Cloned {} from {} onto {}",
                source.backend, source_user.email, target.email
            ),
            data: Some(json!({
                "from_user_id": from_user.to_string(),
                "to_user_id": to_user.to_string(),
                "provider": provider,
                "backend": source.backend,
            })),
        },
        StatusCode::OK,
    )
    .into_response())
}
