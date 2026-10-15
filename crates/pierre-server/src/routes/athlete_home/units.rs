// ABOUTME: GET and PUT /api/me/units — the unit system the athlete reads distances in, chosen in Settings
// ABOUTME: Metric unless the athlete chose imperial; nothing else decides it (carnet#835, carnet#1044)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's units.
//!
//! - `GET /api/me/units` — the system the athlete reads: `metric` unless they
//!   chose `imperial`.
//! - `PUT /api/me/units` — store the athlete's Settings choice, answering as
//!   `GET` does.
//!
//! Stored server-side, keyed by the user alone, so the web, the phone and the
//! agent's replies agree whichever device the choice was made on.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use pierre_core::errors::AppResult;
use pierre_core::models::UnitSystem;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_services::units::resolve_user_units;
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;

/// Body of `GET` and `PUT /api/me/units`, and of the `PUT` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitsBody {
    /// The system everything the athlete reads is written in.
    pub units: UnitSystem,
}

/// `GET /api/me/units`.
pub(super) async fn get_units(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Json<UnitsBody> {
    let units = resolve_user_units(
        resources.common.repos.unit_preferences.as_ref(),
        auth.user_id,
    )
    .await;
    Json(UnitsBody { units })
}

/// `PUT /api/me/units`.
pub(super) async fn put_units(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(update): Json<UnitsBody>,
) -> AppResult<Json<UnitsBody>> {
    resources
        .common
        .repos
        .unit_preferences
        .set_units(auth.user_id, update.units)
        .await?;
    Ok(Json(update))
}
