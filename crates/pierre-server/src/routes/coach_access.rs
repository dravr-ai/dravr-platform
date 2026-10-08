// ABOUTME: HTTP boundary for a coach's own coach-access request — read where it stands, or ask in one tap
// ABOUTME: Opening one emails every super-admin and pings the notify channel; it grants nothing (ADR-018)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for the caller's coach-access request (carnet#738).
//!
//! - `GET /api/me/coach-access-request` — the caller's latest request, or
//!   `null`, so the onboarding group step can show "request sent" or
//!   "declined" instead of the button.
//! - `POST /api/me/coach-access-request` — ask for coach access, optionally
//!   naming the group the caller made (in their active tenant). `201` when a
//!   request was opened, `200` with the one already waiting otherwise.
//!
//! The decision is a super-admin's, made in the admin console
//! (`pierre_routes_admin` coach-access handlers). Logic lives in
//! [`pierre_services::coach_access`]; this module is the HTTP boundary plus
//! the operator email, which needs the server's email service.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use pierre_core::errors::AppError;
use pierre_core::models::{CoachAccessRequest, TenantId};
use pierre_core::permissions::UserRole;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_middleware::redaction::mask_email;
use pierre_services::coach_access::{self, RequestOutcome};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// Body of `POST /api/me/coach-access-request`.
#[derive(Debug, Default, Deserialize)]
pub struct CoachAccessRequestBody {
    /// The group the coach made and wants to coach, in their active tenant.
    #[serde(default)]
    pub group_id: Option<String>,
}

/// Response of both coach-access routes.
#[derive(Debug, Serialize)]
pub struct CoachAccessRequestResponse {
    /// The caller's request; `null` when they never asked.
    pub request: Option<CoachAccessRequest>,
}

/// `GET /api/me/coach-access-request` — the caller's latest request.
///
/// # Errors
///
/// Returns `AppError` when authentication fails or the read errors.
pub async fn handle_get(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let request = resources
        .common
        .repos
        .coach_access_requests
        .latest_for_user(auth.user_id)
        .await?;
    Ok((StatusCode::OK, Json(CoachAccessRequestResponse { request })).into_response())
}

/// `POST /api/me/coach-access-request` — ask for coach access in one tap.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, the group id is malformed or
/// named without an active tenant, the group is not the caller's or already
/// has a coach, the caller already holds coach access, or persistence fails.
pub async fn handle_post(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    body: Option<Json<CoachAccessRequestBody>>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let body = body.map(|Json(b)| b).unwrap_or_default();

    let group = match body.group_id.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => {
            let group_id = Uuid::parse_str(raw)
                .map_err(|e| AppError::invalid_input(format!("Invalid group id: {e}")))?;
            let tenant = auth.active_tenant_id.ok_or_else(|| {
                AppError::invalid_input("Naming a group requires an active tenant")
            })?;
            Some((group_id, TenantId::from_uuid(tenant)))
        }
    };

    let repos = &resources.common.repos;
    let outcome = coach_access::request_access(repos, auth.user_id, group).await?;
    let status = match &outcome {
        RequestOutcome::Opened(request) => {
            email_super_admins(&resources, request).await;
            StatusCode::CREATED
        }
        RequestOutcome::AlreadyPending(_) => StatusCode::OK,
    };
    let request = match outcome {
        RequestOutcome::Opened(request) | RequestOutcome::AlreadyPending(request) => request,
    };
    Ok((
        status,
        Json(CoachAccessRequestResponse {
            request: Some(request),
        }),
    )
        .into_response())
}

/// Email every active super-admin about a new request. Best-effort: a failed
/// or unconfigured send is logged and never fails the request, which the
/// notify channel and the console queue carry regardless.
async fn email_super_admins(resources: &ServerContext, request: &CoachAccessRequest) {
    let Some(email) = resources.common.email_service.as_ref() else {
        warn!(request_id = %request.id, "Email service not configured — skipping coach access emails");
        return;
    };
    let repos = &resources.common.repos;
    let Some(recipients) = super_admin_emails(resources, request.id).await else {
        return;
    };
    let Some(requester) = repos.users.get_global(request.user_id).await.ok().flatten() else {
        warn!(request_id = %request.id, "Coach access requester vanished before the email");
        return;
    };
    let group_name = requested_group_name(resources, request).await;
    let review_url = resources
        .common
        .config
        .frontend_url
        .as_deref()
        .map(|base| format!("{}/#users", base.trim_end_matches('/')));

    for to in &recipients {
        let sent = email
            .send_coach_access_requested(
                to,
                &requester.email,
                requester.display_name.as_deref(),
                group_name.as_deref(),
                review_url.as_deref(),
            )
            .await;
        log_email_outcome(to, request.id, sent);
    }
}

/// The addresses of every active super-admin; `None` (logged) when they
/// cannot be read.
async fn super_admin_emails(resources: &ServerContext, request_id: Uuid) -> Option<Vec<String>> {
    match resources.common.repos.users.list_admins().await {
        Ok(admins) => Some(
            admins
                .into_iter()
                .filter(|a| a.is_active && matches!(a.role, UserRole::SuperAdmin))
                .map(|a| a.email)
                .collect(),
        ),
        Err(e) => {
            warn!(%request_id, error = %e, "Could not list super-admins for a coach access request");
            None
        }
    }
}

/// The name of the group a request names, for the email; `None` when it names
/// none or the group is gone.
async fn requested_group_name(
    resources: &ServerContext,
    request: &CoachAccessRequest,
) -> Option<String> {
    let group_id = request.group_id?;
    let tenant = request.group_tenant_id.as_deref()?.parse::<Uuid>().ok()?;
    resources
        .common
        .repos
        .groups
        .get_group(&group_id.to_string(), TenantId::from_uuid(tenant))
        .await
        .ok()
        .flatten()
        .map(|g| g.name)
}

/// Log one operator email's outcome against the masked recipient.
fn log_email_outcome(to: &str, request_id: Uuid, sent: Result<(), AppError>) {
    let recipient = mask_email(to);
    match sent {
        Ok(()) => info!(%recipient, %request_id, "Coach access request email sent"),
        Err(e) => {
            warn!(%recipient, %request_id, error = %e, "Failed to send coach access request email");
        }
    }
}

/// Mount-helper for the caller's coach-access routes.
pub struct CoachAccessRoutes;

impl CoachAccessRoutes {
    /// User-facing routes. Require a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route(
                "/api/me/coach-access-request",
                get(handle_get).post(handle_post),
            )
            .with_state(resources)
    }
}
