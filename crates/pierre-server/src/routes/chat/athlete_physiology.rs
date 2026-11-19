// ABOUTME: GET and PUT /api/chat/athletes/{athlete_user_id}/physiology — a group's coach reads and sets a coached athlete's anchors
// ABOUTME: Group info's "Thresholds": the coaching gates and consent on every request, the one physiology writer, the athlete told

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1561. From Group info, the human coach of a group reads a coached
//! athlete's per-sport anchors and sets one:
//!
//! - `GET /api/chat/athletes/{id}/physiology?group_id=` — the athlete's
//!   anchors in the shape `GET /api/me/physiology` answers with, worded for
//!   the coach (in the coach's locale and units; an anchor the athlete set
//!   reads as the athlete's own);
//! - `PUT /api/chat/athletes/{id}/physiology/{sport}/{anchor}?group_id=` —
//!   one anchor set, in the body `PUT /api/me/physiology/{sport}/{anchor}`
//!   takes, answered with the anchors as the `GET` reads them.
//!
//! Both run the coaching gates ([`coached_athlete_access`]) on every
//! request: the caller holds coach access and is the active group's coach,
//! the athlete is a live member who still shares their data with the coach
//! (their consent), and they have one home tenant, where their profile
//! lives. The `PUT` is refused, too, when that tenant switched
//! `set_physiology` off; the `GET` reads the anchors whatever the write
//! tool's switch says. Each request that passes goes on the athlete's own
//! record as a `physiology_view` read before anything of theirs is read
//! (carnet#1445).
//!
//! The write, its writer and who it tells are `edit`, built with the
//! `tools-config` feature that builds the physiology writer; the `GET` reads
//! whatever that feature says.

/// The coach's edit of one anchor: built only where its writer is.
#[cfg(feature = "tools-config")]
pub(super) mod edit;

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use pierre_auth::auth::AuthResult;
use pierre_contremaitre::anchor_strings::AnchorReader;
use pierre_core::errors::AppError;
use pierre_database::repositories::{CoachReadClass, CoachReadSurface};
use pierre_middleware::AuthenticatedUser;
use pierre_services::coached_athlete::{coached_athlete_access, CoachedAthleteAccess};
use pierre_tool_runtime::read_subject::record_coach_read;
use serde::Deserialize;
use uuid::Uuid;

use super::common::coaching_refusal;
use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::physiology::{physiology_response, PhysiologyResponse};

/// Query parameters for the coach's physiology routes.
#[derive(Debug, Deserialize)]
pub(super) struct AthletePhysiologyQuery {
    /// The group the caller coaches the athlete in.
    group_id: Uuid,
    /// The locale the anchors' words render in; the coach's own when absent.
    #[serde(default)]
    locale: Option<String>,
}

/// The athlete id a path names.
fn athlete_in_path(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw.trim())
        .map_err(|_| AppError::invalid_input("athlete_user_id must be a UUID"))
}

/// The coaching gates for the caller, `auth`, acting for `athlete` in
/// `group_id`, run now; a refusal answered as `auth` may be told it
/// ([`coaching_refusal`]).
async fn coached_athlete(
    resources: &Arc<ServerContext>,
    auth: &AuthResult,
    athlete: Uuid,
    group_id: Uuid,
) -> Result<CoachedAthleteAccess, AppError> {
    coached_athlete_access(&resources.common.repos, auth.user_id, athlete, group_id)
        .await?
        .map_err(|refused| coaching_refusal(auth, refused))
}

/// Record on the athlete's own record that `coach` read their anchors, once
/// every gate let the request through. A row that cannot be written fails
/// the request: no read goes unrecorded.
async fn record_physiology_read(
    resources: &Arc<ServerContext>,
    access: &CoachedAthleteAccess,
    coach: Uuid,
) -> Result<(), AppError> {
    record_coach_read(
        &resources.common.repos,
        access,
        coach,
        CoachReadSurface::PhysiologyView.as_str(),
        CoachReadClass::Training,
    )
    .await
}

/// `GET /api/chat/athletes/{athlete_user_id}/physiology?group_id=`: the
/// athlete's anchors, worded for the coach.
///
/// A refusal carries `details.reason`: 403 `no_coach_access`, 403
/// `not_group_coach`, 404 `not_a_member`, 403 `coach_sharing_revoked`, 409
/// `several_tenants` or `no_tenant`.
pub(super) async fn get_athlete_physiology(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(athlete_user_id): Path<String>,
    Query(query): Query<AthletePhysiologyQuery>,
) -> Result<Json<PhysiologyResponse>, AppError> {
    let athlete = athlete_in_path(&athlete_user_id)?;
    let coach = auth.user_id;
    let access = coached_athlete(&resources, &auth, athlete, query.group_id).await?;
    record_physiology_read(&resources, &access, coach).await?;
    let body = physiology_response(
        &resources,
        access.tenant,
        athlete,
        coach,
        AnchorReader::Coach,
        query.locale.as_deref(),
    )
    .await?;
    Ok(Json(body))
}
