// ABOUTME: The walks a session's head refresh asks: its tag walk and its track walk, each once per connection
// ABOUTME: Asked by the capture sweep's refresh, and off the turn by a chat read that carried the session's list head

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_providers::request_budget;
use tracing::warn;
use uuid::Uuid;

use super::{detail_walk, tag_walk, ActivityBackfillJob};
use crate::activity_fetch::sync_verdict::{self, HeadRead, HeadVerdict};
use crate::runtime::ToolRuntime;

/// A connection whose head just refreshed has a live session: ask for its
/// tag walk, which fills the tags and gear id of the stored history the head
/// read never carries, once per connection and only after its history walk
/// ended ([`tag_walk::ask_tag_walk`]), and for its track walk, which reads
/// once more the rides a detail read reached before it kept their route
/// ([`detail_walk::ask_track_walk`]). A backend that is no counted session is asked
/// neither.
pub(crate) async fn ask_walks_after_refresh(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    tag_walk::ask_tag_walk(runtime, user_id, tenant_id, backend).await;
    detail_walk::ask_track_walk(runtime, user_id, tenant_id, backend).await;
}

/// Ask the walks a head refresh asks ([`ask_walks_after_refresh`]) once a
/// chat turn's live read `job` answered `read` with the head of its own
/// list: a session the capture sweep passes over is asked them too.
///
/// Only a read the backend itself answered complete over the list head
/// counts. A capture that never reached the head, a grant answering in the
/// session's place, and a closed window or a later page say nothing of a
/// live session. The ask is spawned off the turn, in the background share:
/// it never delays the answer, and what it fails at is logged where it
/// fails.
pub(super) fn ask_walks_after_live_read(job: &ActivityBackfillJob, read: &AppResult<HeadRead>) {
    if !refreshed_own_head(job, read) {
        return;
    }
    let runtime = Arc::clone(&job.resources);
    let (user_id, tenant_id, backend) = (job.user_id, job.tenant_id, job.provider_name.clone());
    let task = tokio::spawn(request_budget::in_background(async move {
        ask_walks_after_refresh(&runtime, user_id, tenant_id, &backend).await;
    }));
    let backend = job.provider_name.clone();
    tokio::spawn(async move {
        if let Err(join_error) = task.await {
            warn!(
                user_id = %user_id,
                backend = %backend,
                error = %join_error,
                "Walks after a live read: the ask's task failed"
            );
        }
    });
}

/// Whether `read` refreshed the head of `job`'s own list: complete, from
/// the backend the job reads, over an open window's first page.
fn refreshed_own_head(job: &ActivityBackfillJob, read: &AppResult<HeadRead>) -> bool {
    matches!(
        read,
        Ok(HeadRead {
            verdict: HeadVerdict::Complete,
            served_by_grant: None,
            ..
        })
    ) && sync_verdict::covers_list_head(&job.query_params, Utc::now().timestamp())
}
