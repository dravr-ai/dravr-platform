// ABOUTME: Asking again for a history walk that ended incomplete: it resumes from its cursor with its counters
// ABOUTME: No sooner than a day after a failing end, so a walk that keeps failing never storms the read service

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Resuming an incomplete walk (carnet#1698).
//!
//! A walk ends incomplete when its source's credential died, or when a step
//! failed on every attempt. Started over from the head, it would read again
//! every year it already landed and fail where it failed before, so no ask
//! could ever finish it. Asked again for any reason (a connect, a first
//! sync, a depth miss, or, for a session the walk windows count, a head
//! refresh of it: the capture sweep's, or a chat turn's live read of the
//! list head), it is owed again in place instead: its cursor, floor, phase
//! (the grant an API phase reads) and progress are kept, and the resume
//! sweep runs its next step below where it stopped. The head refresh is the ask that needs no event: a
//! session walk nothing else asks again still resumes once its back-off is
//! over.
//!
//! A step that failed on every attempt stamped its end, and the walk is not
//! owed again until [`INCOMPLETE_WALK_BACK_OFF`] after it: a source that
//! always fails costs one step's attempts a day, never one per ask, against
//! the single session read service. A walk whose credential died is stamped
//! with no end: no walk reads its session until the athlete connects it
//! again, and then it resumes at once. A walk whose API phase lost its grant
//! resumes only once that grant is live beside the session again, so an ask
//! never pages a dead grant.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use dravr_tronc::dates::Timestamp;
use pierre_core::models::TenantId;
use pierre_database::repositories::{BackfillJobState, HistoryWalkRowState};
use pierre_services::training_history_read;
use tracing::{info, warn};
use uuid::Uuid;

use crate::runtime::{ToolRuntime, WalkTrigger};

/// How long a walk that ended on a failed last attempt waits before an ask
/// owes it again.
pub(crate) const INCOMPLETE_WALK_BACK_OFF: StdDuration = StdDuration::from_hours(24);

/// Owe again the walk of `backend` that ended incomplete, from where it
/// stopped. `None` when no walk under `backend`'s own key ended incomplete,
/// or the ledger could not say: a walk is asked as for no row. `Some(true)`
/// when it is owed again, `Some(false)` when its failing end is under
/// [`INCOMPLETE_WALK_BACK_OFF`] old, its API phase's grant is not live
/// beside the session, or the ledger could not be written.
pub(super) async fn resume_incomplete(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    trigger: WalkTrigger,
) -> Option<bool> {
    let row = ended_incomplete(runtime, user_id, tenant_id, backend).await?;
    if !phase_live(runtime, user_id, tenant_id, backend, &row).await {
        info!(user_id = %user_id, backend, grant = ?row.walk_backend, ?trigger, "History walk: an incomplete API phase waits for its grant to be live again");
        return Some(false);
    }
    Some(owe_again(runtime, user_id, tenant_id, backend, trigger).await)
}

/// Owe the incomplete walk filed under `backend` again, once its failing end
/// is [`INCOMPLETE_WALK_BACK_OFF`] old; whether it was.
async fn owe_again(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    trigger: WalkTrigger,
) -> bool {
    let back_off_ms = i64::try_from(INCOMPLETE_WALK_BACK_OFF.as_millis()).unwrap_or(i64::MAX);
    let ended_by_ms = Timestamp::now().epoch_millis().saturating_sub(back_off_ms);
    match runtime
        .repos()
        .activity_backfill_jobs
        .resume_incomplete_history_walk(user_id, tenant_id, backend, ended_by_ms)
        .await
    {
        Ok(resumed) => {
            info!(user_id = %user_id, backend, ?trigger, resumed, "History walk: an incomplete walk was asked again; it resumes from its cursor, or waits out its back-off");
            resumed
        }
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "History walk: the incomplete walk could not be owed again");
            false
        }
    }
}

/// Whether the phase the incomplete walk `row` reads can be read: its
/// session, or the grant its API phase reads while that grant is still the
/// one live beside the session ([`training_history_read::walk_hand_off`]).
async fn phase_live(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    row: &HistoryWalkRowState,
) -> bool {
    let Some(grant) = row.walk_backend.as_deref() else {
        return true;
    };
    training_history_read::walk_hand_off(runtime.repos(), user_id, tenant_id, backend)
        .await
        .is_some_and(|live| live.backend == grant)
}

/// The walk filed under `backend`, when it ended incomplete; `None` too
/// (logged) when the ledger could not say.
async fn ended_incomplete(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> Option<HistoryWalkRowState> {
    match runtime
        .repos()
        .activity_backfill_jobs
        .history_walk_row(user_id, tenant_id, backend)
        .await
    {
        Ok(row) => row.filter(|row| row.state == BackfillJobState::Incomplete),
        Err(e) => {
            warn!(user_id = %user_id, backend, error = %e, "History walk: the incomplete walk could not be read; a walk is asked as for no row");
            None
        }
    }
}
