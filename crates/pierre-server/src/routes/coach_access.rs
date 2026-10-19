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
use pierre_services::coach_access::{self, RequestOutcome};
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
