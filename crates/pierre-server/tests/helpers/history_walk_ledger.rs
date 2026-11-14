// ABOUTME: Writes history-walk rows straight into the job ledger: one owed, or one that reached the end of the history
// ABOUTME: For the suites whose subject reads what a walk left, not the walk itself (the personal-best seed)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Only the suites a feature set compiles call it.
#![allow(dead_code)]

use chrono::Utc;
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobKind, BackfillJobState, JobLease, JobProgress,
    JobProgressStep,
};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// 2000-01-01T00:00:00Z, the floor a walk that names none of its own is
/// recorded with: the value its ledger rows hold, as a walk writes them.
pub(crate) const WALK_FLOOR: i64 = 946_684_800;

/// Record a history walk of `backend` owed for the user, leased for ten
/// minutes as a running walk holds it, so no resume sweep takes it.
pub(crate) async fn owe_history_walk(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) -> JobLease {
    let now_ms = Utc::now().timestamp_millis();
    let row = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id,
        user_id,
        provider: backend.to_owned(),
        kind: BackfillJobKind::HistoryWalk,
        after_ts: Some(WALK_FLOOR),
        before_ts: None,
        fetch_limit: Some(200),
        conversation_id: None,
        created_at_ms: now_ms,
        leased_until_ms: now_ms + 10 * 60_000,
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
        progress: JobProgress::default(),
        turn_id: None,
        asked_message_id: None,
        walk_backend: None,
    };
    assert!(
        repos
            .activity_backfill_jobs
            .record_backfill_job(&row)
            .await
            .unwrap(),
        "the walk is recorded"
    );
    JobLease {
        id: row.id,
        token: row.lease_token.unwrap(),
    }
}

/// End the walk `lease` holds at the floor: the ledger then reads it as
/// [`BackfillJobState::Walked`].
pub(crate) async fn end_history_walk(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    lease: &JobLease,
) {
    let step = JobProgressStep {
        before_ts: WALK_FLOOR,
        units_done: 12,
        rows_landed: 0,
        secs_per_unit: None,
        floor: None,
    };
    repos
        .activity_backfill_jobs
        .end_history_walk(lease, step)
        .await
        .unwrap()
        .expect("the lease holds the walk");
    assert_eq!(
        repos
            .activity_backfill_jobs
            .backfill_job_state(user_id, tenant_id, backend, BackfillJobKind::HistoryWalk)
            .await
            .unwrap(),
        Some(BackfillJobState::Walked),
        "the walk ended at the floor reads as walked"
    );
}

/// A walk of `backend` that reached the end of the user's history.
pub(crate) async fn walked_history(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    let lease = owe_history_walk(repos, user_id, tenant_id, backend).await;
    end_history_walk(repos, user_id, tenant_id, backend, &lease).await;
}
