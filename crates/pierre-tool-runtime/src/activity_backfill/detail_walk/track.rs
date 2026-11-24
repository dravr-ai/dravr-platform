// ABOUTME: Asking the track walk: every outdoor ride whose track is unsettled is read once for it, newest first
// ABOUTME: Once per session connection, after any owed detail walk, within the detail walk's own day budget

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use dravr_tronc::dates::Timestamp;
use pierre_core::models::TenantId;
use pierre_database::repositories::{BackfillJobKind, BackfillJobState, JobProgress};
use pierre_providers::core::ActivityQueryParams;
use tracing::info;
use uuid::Uuid;

use super::candidates::Athlete;
use super::{session_walk_window, walk_row};
use crate::activity_backfill::{enqueue, provider_tenant_id_str, ActivityBackfillJob, Enqueued};
use crate::runtime::ToolRuntime;

/// What asking for a track walk came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackWalkAsked {
    /// A walk was recorded and started: from the newest ride, or from where
    /// the connection's last one stopped short.
    Started,
    /// The connection has one owed, or one that walked its whole history: no
    /// second is recorded, so no ride is read for its track twice.
    Standing,
    /// The backend is not a session the limiter counts, or the ledger could
    /// not say: nothing was asked.
    NotAsked,
}

/// Ask for the track walk of `backend`'s session for the athlete, once
/// (carnet#1679).
///
/// A session's list row carries no route, so a ride's track is only what a
/// detail read of it kept: a ride a detail read reached before the route was
/// kept, and one no detail read ever reached (a detail walk reads only the
/// window it is asked for; carnet#1779), hold none. The track walk reads each
/// outdoor ride whose track is unsettled once, newest first, over the
/// athlete's whole stored history, as the places pass reads it, through the
/// step and the day budget the detail walk reads in ([`super::step`]), and
/// waits while a detail walk is owed through the backend.
///
/// Only a session the limiter counts is walked: its detail carries the
/// route, which a grant's does not, and a grant's list row carries its route
/// overview already. A connection whose walk is owed or walked is left
/// alone; one that ended short (its session died, its budget left no call,
/// its last attempt failed) is recorded again from the cursor it stopped at,
/// so no ride it walked past is read again. Asked once a connection's head
/// refresh succeeds, so the session is live: by the capture sweep, and by a
/// chat turn's live read of the session's list head, both through
/// `activity_backfill::after_refresh`.
pub async fn ask_track_walk(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> TrackWalkAsked {
    if session_walk_window(runtime, user_id, tenant_id, backend)
        .await
        .is_none()
    {
        return TrackWalkAsked::NotAsked;
    }
    let athlete = Athlete {
        runtime,
        user_id,
        tenant_id,
    };
    let stopped_at = match walk_row(&athlete, backend, BackfillJobKind::TrackWalk).await {
        Err(()) => return TrackWalkAsked::NotAsked,
        Ok(Some(row)) if matches!(row.state, BackfillJobState::Owed | BackfillJobState::Walked) => {
            return TrackWalkAsked::Standing;
        }
        Ok(row) => row.and_then(|row| row.before_ts),
    };
    let job = ActivityBackfillJob {
        resources: Arc::clone(runtime),
        user_id,
        tenant_id,
        tenant_id_str: provider_tenant_id_str(tenant_id),
        provider_name: backend.to_owned(),
        query_params: ActivityQueryParams {
            after: None,
            before: Some(stopped_at.unwrap_or_else(|| Timestamp::now().epoch_seconds())),
            limit: None,
            offset: None,
        },
        // No conversation waits on it, and nobody is told when it ends: the
        // places pass picks each new track up.
        pierre_conversation_id: None,
        kind: BackfillJobKind::TrackWalk,
        turn_id: None,
        asked_message_id: None,
        walk_sport: None,
    };
    match enqueue(job, None, JobProgress::default()).await {
        Enqueued::Started => {
            info!(user_id = %user_id, backend, resumed = stopped_at.is_some(), "Track walk: asked");
            TrackWalkAsked::Started
        }
        Enqueued::InFlightHere(..) | Enqueued::Owed(..) => TrackWalkAsked::Standing,
        Enqueued::Unrecorded => TrackWalkAsked::NotAsked,
    }
}
