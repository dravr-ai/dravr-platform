// ABOUTME: GET /api/me/training-volume — the athlete's weekly distance, time and climbing per sport over twelve weeks
// ABOUTME: One count of the stored workouts by week and sport, merged across sources, standing on Home's own vouching rule

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use dravr_tronc::dates::civil_clock::{clock_date, resolve_zone};
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_runtime_context::require_tenant;
use pierre_tool_runtime::activity_query::{
    query_activities, ActivityPredicate, ActivityQuery, ReadGate,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::warn;

use crate::mcp::resources::ServerContext;
use crate::services::training_volume::{
    volume_from_answer, window_start, TrainingVolume, VOLUME_GROUPS, VOLUME_METRICS,
};

/// The athlete's weekly volume on their own calendar.
///
/// One count of the stored workouts over the twelve weeks
/// ([`query_activities`]), read the way the athlete's own surfaces serve
/// rows ([`ReadGate::Display`]): every key the athlete holds rows under, no
/// row cap. Reaches no provider and starts no capture. How far back the
/// weeks go is read off the store itself: when it holds an activity older
/// than the window every week is vouched for, otherwise the weeks start with
/// the one the oldest stored activity falls in.
pub(super) async fn get_training_volume(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<TrainingVolume>> {
    let user_id = auth.user_id;
    let tenant_id = require_tenant(&resources, &auth).await?;
    let user = resources.repos().users.get_global(user_id).await?;
    let zone = resolve_zone(user.as_ref().and_then(|u| u.timezone.as_deref()));
    let today = clock_date(Timestamp::now(), &zone);
    let query = ActivityQuery {
        from: Some(window_start(today)),
        to: today,
        filter: ActivityPredicate::default(),
        group_by: VOLUME_GROUPS.to_vec(),
        metrics: VOLUME_METRICS.to_vec(),
        provider: None,
    };
    let runtime: Arc<dyn ToolRuntime> = resources;
    let answer = query_activities(&runtime, user_id, tenant_id, &query, ReadGate::Display).await?;
    if answer.groups_truncated {
        warn!(
            %user_id,
            groups = answer.groups_total,
            "training volume answer cut its week-by-sport groups; the smallest are missing"
        );
    }
    Ok(Json(volume_from_answer(today, &answer)))
}
