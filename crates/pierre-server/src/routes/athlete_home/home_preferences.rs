// ABOUTME: GET and PUT /api/me/home-preferences — the per-user choices Home's layout honours on web and mobile alike
// ABOUTME: Today one: whether the athlete set aside the suggestion to build a training plan (carnet#820)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's Home preferences.
//!
//! - `GET /api/me/home-preferences` — what the athlete chose, every field
//!   present; an athlete who chose nothing reads the defaults.
//! - `PUT /api/me/home-preferences` — store the athlete's choices, answering
//!   with what is now stored.
//!
//! Stored server-side, keyed by the user alone, so the web and the phone show
//! the same Home whichever device the choice was made on.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use pierre_core::errors::AppResult;
use pierre_database::repositories::HomePreferences;
use pierre_middleware::extractors::AuthenticatedUser;
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;

/// Body of `GET` and `PUT /api/me/home-preferences`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomePreferencesBody {
    /// The athlete set aside the suggestion to build a training plan: Home
    /// stops offering it while they have none, until they bring it back.
    pub plan_suggestion_hidden: bool,
}

impl From<HomePreferences> for HomePreferencesBody {
    fn from(prefs: HomePreferences) -> Self {
        Self {
            plan_suggestion_hidden: prefs.plan_suggestion_hidden,
        }
    }
}

impl From<HomePreferencesBody> for HomePreferences {
    fn from(body: HomePreferencesBody) -> Self {
        Self {
            plan_suggestion_hidden: body.plan_suggestion_hidden,
        }
    }
}

/// `GET /api/me/home-preferences`.
pub(super) async fn get_home_preferences(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<HomePreferencesBody>> {
    let prefs = resources
        .common
        .repos
        .home_preferences
        .get_home_preferences(auth.user_id)
        .await?;
    Ok(Json(prefs.into()))
}

/// `PUT /api/me/home-preferences`.
pub(super) async fn put_home_preferences(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(body): Json<HomePreferencesBody>,
) -> AppResult<Json<HomePreferencesBody>> {
    let repos = &resources.common.repos;
    repos
        .home_preferences
        .set_home_preferences(auth.user_id, body.into())
        .await?;
    let stored = repos
        .home_preferences
        .get_home_preferences(auth.user_id)
        .await?;
    Ok(Json(stored.into()))
}
