// ABOUTME: GET /api/me/getting-started — Home's Getting started list: connect, then the first-run flows, done or not
// ABOUTME: Read from the catalogue's first_run order, the athlete's state and first_run_progress (carnet#1098, carnet#1099)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Getting started (carnet#1098, carnet#1099).
//!
//! Five rows at the top of Home's Today: connect your data, then the four
//! first-run flows in the catalogue's `first_run:` order. Each row says
//! whether its prerequisites hold, whether it is done, and whether it is the
//! current one; an open row carries the `gs:<slot>:<id>` postback a tap sends
//! into the Home thread. The rows offered as links are reported shown
//! (`suggestion.shown`, surface `getting_started`, carnet#1100) and never
//! counted toward retiring a starter.
//!
//! The read is also where Finish your profile is noticed done when the dossier
//! filled outside any turn — Settings, fact extraction after the last reply.

use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, Json};
use chrono::{DateTime, Utc};
use pierre_chat_pipeline::suggestions::report_getting_started_shown;
use pierre_contremaitre::use_case_catalogue::UseCaseCatalogue;
use pierre_core::errors::AppError;
use pierre_middleware::AuthenticatedUser;
use pierre_providers::ai_scope;
use pierre_services::athlete_state::AthleteState;
use pierre_services::first_run::{getting_started, observe_profile, FirstRun, GettingStartedRow};
use serde::Serialize;

use crate::mcp::resources::ServerContext;
use crate::routes::chat::{client_surface, get_tenant_id};

/// One row of `GET /api/me/getting-started`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GettingStartedRowResponse {
    /// `connect`, or the first-run flow's catalogue id.
    pub id: String,
    /// The row's place in the list, from 1.
    pub order: usize,
    /// Its prerequisites hold; a row that waits shows what it waits on.
    pub eligible: bool,
    /// Done.
    pub done: bool,
    /// The first row neither done nor waiting.
    pub current: bool,
    /// What a tap sends into the Home thread; `null` for a done or waiting
    /// row and for `connect`, which opens Settings › Connections.
    pub postback: Option<String>,
}

impl From<GettingStartedRow> for GettingStartedRowResponse {
    fn from(row: GettingStartedRow) -> Self {
        Self {
            id: row.id,
            order: row.order,
            eligible: row.eligible,
            done: row.done,
            current: row.current,
            postback: row.postback,
        }
    }
}

/// Body of `GET /api/me/getting-started`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GettingStartedResponse {
    /// Connect, then the first-run flows in flow order.
    pub rows: Vec<GettingStartedRowResponse>,
    /// Rows done.
    pub done_count: usize,
    /// Rows in all.
    pub total: usize,
    /// When the last row became done (RFC 3339); `null` until every row is.
    /// Home folds the list to one line then, and drops it seven days later.
    pub folded_at: Option<DateTime<Utc>>,
    /// The athlete hid the list (Home preferences); Settings brings it back.
    pub hidden: bool,
}

/// `GET /api/me/getting-started` — the caller's Getting started list.
///
/// # Errors
///
/// Returns the authentication error, a refusal for a caller outside Dravr's
/// apps when the list was read from first-party-only data, and the
/// repository error when the athlete's state, first run or Home preferences
/// cannot be read: a list built on a partial read could tick a row that is
/// not done.
pub async fn get_getting_started(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
) -> Result<Json<GettingStartedResponse>, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let repos = &resources.common.repos;
    let surface = client_surface(&headers);
    let (state, policy) = AthleteState::read(repos, tenant_id, auth.user_id, Utc::now()).await?;
    // Which rows are done says what the athlete's data holds, as a starter
    // does: an interface that may not carry that data is not told (carnet#769).
    if !ai_scope::admit_derived(policy) {
        return Err(AppError::unavailable_over_transport(
            "Getting started is not available over this interface",
        ));
    }
    let mut first_run = FirstRun::read(repos, tenant_id, auth.user_id).await?;
    if state.dossier.complete {
        observe_profile(repos, auth.user_id, tenant_id, &first_run, surface.as_str()).await;
        first_run = FirstRun::read(repos, tenant_id, auth.user_id).await?;
    }
    let list = getting_started(UseCaseCatalogue::pinned(), &state, &first_run);
    let hidden = repos
        .home_preferences
        .get_home_preferences(auth.user_id)
        .await?
        .getting_started_hidden;
    if !hidden {
        let offered: Vec<(usize, &str)> = list
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.postback.is_some())
            .map(|(slot, row)| (slot, row.id.as_str()))
            .collect();
        report_getting_started_shown(auth.user_id, tenant_id, surface, &offered);
    }
    Ok(Json(GettingStartedResponse {
        rows: list.rows.into_iter().map(Into::into).collect(),
        done_count: list.done_count,
        total: list.total,
        folded_at: list.folded_at,
        hidden,
    }))
}
