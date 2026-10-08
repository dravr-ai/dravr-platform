// ABOUTME: GET /api/me/training-volume — the athlete's weekly distance, time and climbing per sport over twelve weeks
// ABOUTME: Reads the durable activity cache only, merges copies of one workout as the recent list does, and says how far the history reaches

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use chrono::{Duration, Utc};
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::errors::AppResult;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_tool_runtime::activity_fetch::activity_cache_retention_days;
use pierre_tool_runtime::runtime::ToolRuntime;

use super::{active_tenant, distinct_rows, served_rows, workouts};
use crate::mcp::resources::ServerContext;
use crate::services::training_volume::{
    day_start_utc, monday_of, volume_from_sessions, window_start, TrainingVolume, VolumeCoverage,
};

/// Most cached rows the volume read takes. Twelve weeks of one athlete's
/// workouts, with a copy per connection, sit far below it; a read that still
/// fills it drops the week its oldest row falls in, which the cut may have
/// left short, rather than under-count it.
const MAX_VOLUME_ROWS: i64 = 2_000;

/// The athlete's weekly volume on their own calendar.
///
/// Reaches no provider and starts no capture. How far back the weeks go is
/// read off the cache itself: when it holds an activity older than the window
/// every week is vouched for, otherwise the weeks start with the one the
/// oldest stored activity falls in.
pub(super) async fn get_training_volume(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<TrainingVolume>> {
    let user_id = auth.user_id;
    let tenant_id = active_tenant(&auth)?;
    let repos = resources.repos();
    let user = repos.users.get_global(user_id).await?;
    let zone = resolve_zone(user.as_ref().and_then(|u| u.timezone.as_deref()));
    let now = Utc::now();
    let today = clock_date(now, zone);
    let opens = day_start_utc(window_start(today), zone);

    let rows = served_rows(
        &resources,
        repos
            .activity_cache
            .get_cached_activity_rows(user_id, &tenant_id, opens, now, MAX_VOLUME_ROWS)
            .await?,
    );
    let read_full = usize::try_from(MAX_VOLUME_ROWS).is_ok_and(|cap| rows.len() >= cap);
    let oldest_read = rows
        .last()
        .map(|row| local_date(row.activity.start_date(), zone));

    let coverage = if read_full {
        // Rows come newest first, so the cut fell in the oldest row's week.
        oldest_read.map_or(VolumeCoverage::Nothing, |day| {
            VolumeCoverage::From(monday_of(day) + Duration::weeks(1))
        })
    } else {
        let since = now - Duration::days(activity_cache_retention_days());
        let older = served_rows(
            &resources,
            repos
                .activity_cache
                .get_cached_activity_rows(user_id, &tenant_id, since, opens, 1)
                .await?,
        );
        match (older.is_empty(), oldest_read) {
            (false, _) => VolumeCoverage::Whole,
            (true, Some(day)) => VolumeCoverage::From(day),
            (true, None) => VolumeCoverage::Nothing,
        }
    };

    let distinct = distinct_rows(rows);
    let sessions: Vec<_> = workouts(&distinct)
        .into_iter()
        .map(|workout| workout.session)
        .collect();
    Ok(Json(volume_from_sessions(today, zone, &sessions, coverage)))
}
