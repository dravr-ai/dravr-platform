// ABOUTME: User profile self-service routes — currently exposes the timezone setter
// ABOUTME: Web + mobile call /api/users/me/timezone after login to populate users.timezone
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/api/users/me/*` — user-profile self-service endpoints.
//!
//! These let an authenticated client update fields on its own user
//! row that aren't suitable for the OAuth2 ROPC token endpoint
//! (timezone, profile preferences, etc.). The chat prompt-assembly
//! stage reads `users.timezone` to resolve `{{CURRENT_DATE}}` to the
//! user's local calendar day, so the value must be writeable without
//! re-authenticating.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::put;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::errors::{AppError, AppResult};
use crate::mcp::resources::ServerContext;
use crate::middleware::extractors::AuthenticatedUser;

/// Mount the user-profile routes on the supplied router.
pub fn routes() -> Router<Arc<ServerContext>> {
    Router::new().route("/api/users/me/timezone", put(set_timezone))
}

/// Request body for `PUT /api/users/me/timezone`.
#[derive(Debug, Deserialize)]
pub struct SetTimezoneRequest {
    /// IANA timezone database name, e.g. `"America/Toronto"`.
    pub timezone: String,
}

/// Response body for `PUT /api/users/me/timezone`.
#[derive(Debug, Serialize)]
pub struct SetTimezoneResponse {
    /// The timezone now stored on the user row.
    pub timezone: String,
}

/// `PUT /api/users/me/timezone` — persist the authenticated user's
/// IANA timezone so the chat prompt can render `{{CURRENT_DATE}}` in
/// the user's local calendar.
///
/// The body shape is `{"timezone": "<IANA name>"}`. The server
/// validates that the string parses as a known timezone before
/// touching the database — junk values (empty string, made-up names)
/// return `400` so the client knows to re-read the user's `Intl`
/// settings instead of silently storing garbage that the prompt
/// resolver will reject at read time.
async fn set_timezone(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(req): Json<SetTimezoneRequest>,
) -> AppResult<Json<SetTimezoneResponse>> {
    let tz = req.timezone.trim();
    if tz.is_empty() {
        return Err(AppError::invalid_input("timezone must not be empty"));
    }
    if tz.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::invalid_input(format!(
            "{tz} is not a valid IANA timezone database name",
        )));
    }

    resources
        .repos
        .users
        .set_timezone(auth.user_id, tz)
        .await
        .map_err(|e| {
            warn!(user_id = %auth.user_id, error = %e, "Failed to persist user timezone");
            AppError::database(format!("set_timezone failed: {e}"))
        })?;

    Ok(Json(SetTimezoneResponse {
        timezone: tz.to_owned(),
    }))
}
