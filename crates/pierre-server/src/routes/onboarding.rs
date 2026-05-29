// ABOUTME: HTTP boundary for the onboarding-status check — single cheap call frontends use to gate routing
// ABOUTME: Returns {needs_provider_connection: bool} based on services::onboarding_gate as source of truth
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routes for onboarding state.
//!
//! - `GET /api/me/onboarding-status` — single cheap call that returns
//!   `{needs_provider_connection: bool}`. Web and mobile clients call this
//!   right after login to decide whether to render the onboarding screen or
//!   route to the main UI.
//!
//! Logic lives in [`pierre_services::onboarding_gate`]; this module is the
//! thin HTTP boundary.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;

use crate::mcp::resources::ServerContext;
use pierre_core::errors::AppError;
use pierre_middleware::extract_auth_from_headers;
use pierre_services::onboarding_gate;

/// Response body for `GET /api/me/onboarding-status`.
///
/// `needs_provider_connection` is `true` when the caller has no *real*
/// (non-`Synthetic`) row in `provider_connections` and should be routed to the
/// connect-a-provider onboarding screen. Synthetic seed/demo connections do not
/// count here — a demo user still needs to link a real provider.
#[derive(Debug, Serialize)]
pub struct OnboardingStatusResponse {
    /// `true` ⇒ frontend should redirect to the onboarding screen.
    pub needs_provider_connection: bool,
}

/// `GET /api/me/onboarding-status`
///
/// # Errors
///
/// Returns `AppError` when authentication fails or the provider-connections
/// query errors.
pub async fn handle_self_get(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let has_provider = onboarding_gate::user_has_real_provider(
        &resources.common.repos.provider_connections,
        auth.user_id,
    )
    .await?;
    Ok((
        StatusCode::OK,
        Json(OnboardingStatusResponse {
            needs_provider_connection: !has_provider,
        }),
    )
        .into_response())
}

/// Mount-helper for the onboarding-status endpoint. Same shape as
/// [`pierre_routes_admin::FeatureFlagsRoutes`].
pub struct OnboardingRoutes;

impl OnboardingRoutes {
    /// User-facing route. Requires a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/onboarding-status", get(handle_self_get))
            .with_state(resources)
    }
}
