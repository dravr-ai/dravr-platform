// ABOUTME: HTTP boundary for a coach's own coach-access request — read where it stands, ask in one tap, or withdraw
// ABOUTME: Opening one emails every super-admin and pings the notify channel; it grants nothing (ADR-018)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for the caller's coach-access request (carnet#738).
//!
//! - `GET /api/me/coach-access-request` — the caller's current request, or
//!   `null`, so the onboarding group step can show "request sent" or
//!   "declined" instead of the button. A decline the caller replaced by
//!   asking again is not current (carnet#1522): after withdrawing that ask
//!   they read `null`, the plain ask.
//! - `POST /api/me/coach-access-request` — ask for coach access, optionally
//!   naming the group the caller made (in their active tenant). `201` when a
//!   request was opened, `200` with the one already waiting otherwise.
//! - `DELETE /api/me/coach-access-request` — withdraw the request waiting
//!   (carnet#1522). The row is deleted, so the caller may ask again at once;
//!   `200` with their latest request after it (an earlier decided one, or
//!   `null`), `404` with none waiting, `409` when a decision claimed it first.
//!
//! The decision is a super-admin's, made in the admin console
//! (`pierre_routes_admin` coach-access handlers). Logic lives in
//! [`pierre_services::coach_access`], the operator email included; this
//! module is the HTTP boundary, handing that email the server's email
//! service.

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
use pierre_email::templates::CoachAccessOrigin;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::coach_access::{self, CoachAccessNotices, RequestOutcome};
use serde::{Deserialize, Serialize};
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

/// `GET /api/me/coach-access-request` — the caller's current request: the
/// latest, leaving out a decline a newer ask replaced.
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
        .current_for_user(auth.user_id)
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
    let origin = CoachAccessOrigin::Asked;
    let outcome = coach_access::request_access(repos, auth.user_id, group, origin).await?;
    let status = match &outcome {
        RequestOutcome::Opened(request) => {
            coach_access::email_super_admins(
                repos,
                resources.common.email_service.as_deref(),
                resources.common.config.frontend_url.as_deref(),
                request,
                origin,
            )
            .await;
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

/// `DELETE /api/me/coach-access-request` — withdraw the request waiting.
///
/// # Errors
///
/// Returns `AppError` when authentication fails, the caller has no request
/// waiting (not found), a super-admin decided it first (conflict), or
/// persistence fails.
pub async fn handle_delete(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let request = coach_access::withdraw(&resources.common.repos, auth.user_id).await?;
    Ok((StatusCode::OK, Json(CoachAccessRequestResponse { request })).into_response())
}

/// What a coach-access decision needs to reach its coach, from the assembled
/// server.
///
/// The notification service when the build has one, the mail service, the
/// string catalogue and the frontend origin. Every decision path (the console
/// queue, the Slack card) is handed the same.
#[must_use]
pub fn coach_access_notices(resources: &ServerContext) -> CoachAccessNotices {
    CoachAccessNotices {
        #[cfg(feature = "client-notifications")]
        notifications: resources.common.notification_service.clone(),
        #[cfg(not(feature = "client-notifications"))]
        notifications: None,
        email: resources.common.email_service.clone(),
        strings: resources.mcp.messaging_strings_registry.clone(),
        frontend_url: resources.common.config.frontend_url.clone(),
    }
}

/// Mount-helper for the caller's coach-access routes.
#[derive(Debug)]
pub struct CoachAccessRoutes;

impl CoachAccessRoutes {
    /// User-facing routes. Require a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route(
                "/api/me/coach-access-request",
                get(handle_get).post(handle_post).delete(handle_delete),
            )
            .with_state(resources)
    }
}
