// ABOUTME: Cookie-authenticated admin handlers for the standing pre-approval allow-list
// ABOUTME: The console's add-a-user-by-email path; bearer twins live in pierre-routes-admin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/api/admin/pre-approved-emails` — the console's half of the allow-list.
//!
//! Cookie twins of the bearer endpoints `pierre-cli user allow / disallow /
//! list-allowed` drives, sharing [`pierre_services::pre_approval`] so an allow
//! means the same thing whichever surface records it. Here the operator is the
//! signed-in admin, so `allowed_by` attribution is exact.
//!
//! Every handler authenticates through `WebAdminRoutes::authenticate_admin`
//! first: these decide who gets an account, so a valid session is not
//! sufficient on its own.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::info;

use pierre_core::errors::AppError;
use pierre_services::pre_approval;

use super::WebAdminContext;

/// Pre-approval request — record a standing allow for one address.
#[derive(Debug, Deserialize)]
pub struct AllowEmailRequest {
    /// Address to pre-approve; normalized (trimmed, lower-cased) server-side.
    pub email: String,
    /// Operator note recorded with the allow (cohort, reason).
    pub note: Option<String>,
}

/// The allow-list's cookie-auth route table.
///
/// Owned here rather than in `WebAdminRoutes::routes` so the handlers and the
/// paths that reach them stay in one file.
pub fn routes() -> Router<WebAdminContext> {
    Router::new()
        .route(
            "/api/admin/pre-approved-emails",
            get(handle_list).post(handle_allow),
        )
        .route(
            "/api/admin/pre-approved-emails/{email}",
            delete(handle_disallow),
        )
}

/// `GET /api/admin/pre-approved-emails` — the standing allow-list.
///
/// Both this and `pierre-cli user list-allowed` go through
/// [`pre_approval::list`], so the console and the CLI cannot disagree about
/// who is allowed.
///
/// # Errors
///
/// Returns the authentication error for a caller who is not an admin, or the
/// repository's error when the listing fails.
pub async fn handle_list(
    State(resources): State<WebAdminContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    let entries = pre_approval::list(&resources.repos).await?;

    info!(
        admin_user_id = %auth.user_id,
        count = entries.len(),
        "Web admin listed pre-approved emails"
    );

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": format!("{} pre-approved email(s)", entries.len()),
            "data": { "emails": entries, "total": entries.len() },
        })),
    )
        .into_response())
}

/// `POST /api/admin/pre-approved-emails` — pre-approve one address.
///
/// This is the console's answer to "add a user by email": the address needs no
/// account, and registration against it lands active with `approved_by` set to
/// the admin who allowed it. An address that already has a pending account is
/// approved on the spot and notified, exactly as the pending-queue approve
/// button would.
///
/// # Errors
///
/// Returns the authentication error for a caller who is not an admin,
/// [`AppError::invalid_input`] for a malformed address, or the repository's
/// error when the write fails.
pub async fn handle_allow(
    State(resources): State<WebAdminContext>,
    headers: HeaderMap,
    Json(request): Json<AllowEmailRequest>,
) -> Result<Response, AppError> {
    let auth = super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    let result = pre_approval::allow(
        &resources.repos,
        &request.email,
        Some(auth.user_id),
        request.note.as_deref(),
    )
    .await?;

    if let Some(approved) = result.approved_user.as_ref() {
        if let Some(notifier) = resources.approval_notifier.as_ref() {
            notifier
                .notify_user_approved(
                    approved.id,
                    &approved.email,
                    approved.display_name.as_deref(),
                )
                .await;
        }
    }

    info!(
        admin_user_id = %auth.user_id,
        outcome = ?result.outcome,
        "Web admin pre-approved an email"
    );

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": result.message(),
            "data": {
                "email": result.email,
                "outcome": result.outcome,
                "approved_user_id": result.approved_user.as_ref().map(|u| u.id.to_string()),
            },
        })),
    )
        .into_response())
}

/// `DELETE /api/admin/pre-approved-emails/{email}` — drop a standing allow.
///
/// An account that already registered against the address keeps whatever
/// status it holds; removing the allow only stops a *future* registration from
/// skipping the queue.
///
/// # Errors
///
/// Returns the authentication error for a caller who is not an admin,
/// [`AppError::invalid_input`] for a malformed address, or the repository's
/// error when the delete fails.
pub async fn handle_disallow(
    State(resources): State<WebAdminContext>,
    headers: HeaderMap,
    Path(email): Path<String>,
) -> Result<Response, AppError> {
    let auth = super::WebAdminRoutes::authenticate_admin(&headers, &resources).await?;

    let result = pre_approval::disallow(&resources.repos, &email).await?;

    info!(
        admin_user_id = %auth.user_id,
        removed = result.removed,
        "Web admin removed a pre-approved email"
    );

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": result.message(),
            "data": {
                "email": result.email,
                "removed": result.removed,
                "account_status": result.account_status,
            },
        })),
    )
        .into_response())
}
