// ABOUTME: PUT /api/chat/athletes/{athlete_user_id}/physiology/{sport}/{anchor} — a group's coach sets a coached athlete's anchor
// ABOUTME: The coaching gates, the tool switch, the one physiology writer, the athlete told; built with tools-config

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's edit of one of a coached athlete's anchors (carnet#1561).
//!
//! It saves through [`save_physiology`], the writer the athlete's own edit
//! and `set_physiology` save through, with the coach as the anchor's source
//! (origin `coach`) at the confidence the coach states. A saved anchor tells
//! the athlete (`coach_physiology_set`). As with the plan, a coach writes an
//! athlete's thresholds only from Dravr's own apps and channels: an API-key,
//! MCP or OAuth client acting for the coach is refused (carnet#1533).
//!
//! The writer is built by the `tools-config` feature, and so is this module:
//! without it the route does not exist, and the coach's `GET` still reads.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use pierre_contremaitre::anchor_strings::AnchorReader;
use pierre_core::errors::AppError;
use pierre_core::models::COACH_STATED_ORIGIN;
use pierre_middleware::AuthenticatedUser;
use pierre_services::coach_physiology_notice::{tell_threshold_set, CoachThresholdSet};
use pierre_tool_runtime::implementations::physiology::save_physiology;
use tracing::info;

use super::{athlete_in_path, coached_athlete, record_physiology_read, AthletePhysiologyQuery};
use crate::mcp::resources::ServerContext;
use crate::routes::athlete_home::physiology::edit::{
    anchor_edit_args, anchor_in_path, refuse_when_edits_off, AnchorEdit,
};
use crate::routes::athlete_home::physiology::{physiology_response, PhysiologyResponse};
use crate::routes::chat::common::refuse_external_coach_write;
use crate::tools::runtime_adapter::into_runtime;

/// `PUT /api/chat/athletes/{athlete_user_id}/physiology/{sport}/{anchor}?group_id=`:
/// the coach sets one of the athlete's anchors, stored as a coach's, the
/// athlete is told, and the coach reads the athlete's anchors back.
///
/// The gate refusals of the `GET`, 403 `external_transport` for a client
/// outside Dravr's own apps, 403 `physiology_disabled` when the athlete's
/// tenant switched `set_physiology` off (checked after the coaching gates,
/// as the plan read checks `get_training_plan`, and recording nothing), and
/// 400 for every edit the writer refuses.
pub(in super::super) async fn put_athlete_physiology_anchor(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((athlete_user_id, sport, anchor)): Path<(String, String, String)>,
    Query(query): Query<AthletePhysiologyQuery>,
    Json(edit): Json<AnchorEdit>,
) -> Result<Json<PhysiologyResponse>, AppError> {
    refuse_external_coach_write(&auth, "thresholds")?;
    let athlete = athlete_in_path(&athlete_user_id)?;
    let (sport, anchor) = anchor_in_path(&sport, &anchor)?;
    let coach = auth.user_id;
    let access = coached_athlete(&resources, &auth, athlete, query.group_id).await?;
    refuse_when_edits_off(&resources, access.tenant, AnchorReader::Coach).await?;
    record_physiology_read(&resources, &access, coach).await?;
    let args = anchor_edit_args(sport, anchor, &edit, COACH_STATED_ORIGIN)?;
    save_physiology(&into_runtime(&resources), access.tenant, athlete, &args).await?;
    let told = tell_threshold_set(
        &resources.common.repos,
        resources.common.notification_service.as_ref(),
        &CoachThresholdSet {
            tenant: access.tenant,
            athlete,
            coach,
            anchor,
        },
    )
    .await;
    info!(
        coach_user_id = %coach,
        athlete_user_id = %athlete,
        group_id = %access.group.id,
        sport = %sport,
        anchor = anchor.as_str(),
        athlete_notified = told,
        // The anchor's name only — the value is health data.
        "coach set a coached athlete's threshold"
    );
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
