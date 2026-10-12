// ABOUTME: GET and PUT /api/me/check-in — how often the coach checks in with the athlete on its own (carnet#831)
// ABOUTME: Asked during onboarding on web and mobile, changed in Settings; off is always a valid answer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's check-in cadence.
//!
//! - `GET /api/me/check-in` — `{"cadence": "daily" | "three_per_week" |
//!   "weekly" | "off" | null}`. `null` is an athlete who never chose: they are
//!   never checked in on, and the onboarding step asks them.
//! - `PUT /api/me/check-in` with `{"cadence": "…"}` — store the choice and
//!   answer as `GET` does.
//!
//! Stored per athlete and per tenant: the tenant is the one whose threads the
//! check-in lands in, the athlete's active tenant here.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::CheckInCadence;
use pierre_middleware::extractors::AuthenticatedUser;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::mcp::resources::ServerContext;
use crate::routes::onboarding::active_tenant;

/// Body of `GET` and `PUT /api/me/check-in`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct CheckInBody {
    /// The cadence the athlete chose, or `null` when they never chose one.
    pub cadence: Option<CheckInCadence>,
}

/// Body of `PUT /api/me/check-in`.
#[derive(Debug, Deserialize)]
struct CheckInUpdate {
    /// The cadence to store.
    pub cadence: CheckInCadence,
}

/// `GET /api/me/check-in`.
async fn get_check_in(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<CheckInBody>> {
    let tenant_id = active_tenant(&auth)?;
    let cadence = resources
        .common
        .repos
        .check_in_preferences
        .check_in_cadence(tenant_id, auth.user_id)
        .await?;
    Ok(Json(CheckInBody { cadence }))
}

/// `PUT /api/me/check-in`.
async fn put_check_in(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(update): Json<CheckInUpdate>,
) -> AppResult<Json<CheckInBody>> {
    let tenant_id = active_tenant(&auth)?;
    resources
        .common
        .repos
        .check_in_preferences
        .set_check_in_cadence(tenant_id, auth.user_id, update.cadence, Utc::now())
        .await?;
    info!(
        target: "notify",
        event = "coach.check_in_cadence_set",
        user_id = %auth.user_id,
        tenant_id = %tenant_id,
        cadence = update.cadence.as_str(),
        surface = "app",
        "check-in cadence chosen"
    );
    Ok(Json(CheckInBody {
        cadence: Some(update.cadence),
    }))
}

/// Mount-helper for the check-in cadence endpoints.
pub struct CheckInRoutes;

impl CheckInRoutes {
    /// User-facing routes. Require a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/check-in", get(get_check_in).put(put_check_in))
            .with_state(resources)
    }
}
