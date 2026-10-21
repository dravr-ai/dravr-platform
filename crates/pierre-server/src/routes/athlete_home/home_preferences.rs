// ABOUTME: GET and PUT /api/me/home-preferences — the per-user choices Home's layout honours on web and mobile alike
// ABOUTME: The plan suggestion's dismissal (carnet#820) and Getting started's Hide (carnet#1098); a PUT merges

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's Home preferences.
//!
//! - `GET /api/me/home-preferences` — what the athlete chose, every field
//!   present; an athlete who chose nothing reads the defaults.
//! - `PUT /api/me/home-preferences` — merge the athlete's choices into what
//!   is stored, answering with every field as it now stands. A field the body
//!   leaves out keeps its stored value, so a client that predates a field (an
//!   older phone build) cannot reset it by writing the others.
//!
//! Stored server-side, keyed by the user alone, so the web and the phone show
//! the same Home whichever device the choice was made on.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use pierre_core::errors::AppResult;
use pierre_database::repositories::{HomePreferences, HomePreferencesUpdate};
use pierre_middleware::extractors::AuthenticatedUser;
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;

/// Body of `GET /api/me/home-preferences`, and the answer to a `PUT`: every
/// choice, present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomePreferencesBody {
    /// The athlete set aside the suggestion to build a training plan: Home
    /// stops offering it while they have none, until they bring it back.
    pub plan_suggestion_hidden: bool,
    /// The athlete hid Getting started, the first-steps list at the top of
    /// Today, until Settings brings it back.
    pub getting_started_hidden: bool,
}

impl From<HomePreferences> for HomePreferencesBody {
    fn from(prefs: HomePreferences) -> Self {
        Self {
            plan_suggestion_hidden: prefs.plan_suggestion_hidden,
            getting_started_hidden: prefs.getting_started_hidden,
        }
    }
}

/// Body of `PUT /api/me/home-preferences`: each field is optional, and one
/// left out keeps what is stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomePreferencesPatch {
    /// See [`HomePreferencesBody::plan_suggestion_hidden`].
    #[serde(default)]
    pub plan_suggestion_hidden: Option<bool>,
    /// See [`HomePreferencesBody::getting_started_hidden`].
    #[serde(default)]
    pub getting_started_hidden: Option<bool>,
}

impl From<HomePreferencesPatch> for HomePreferencesUpdate {
    fn from(patch: HomePreferencesPatch) -> Self {
        Self {
            plan_suggestion_hidden: patch.plan_suggestion_hidden,
            getting_started_hidden: patch.getting_started_hidden,
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
    Json(body): Json<HomePreferencesPatch>,
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
