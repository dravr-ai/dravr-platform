// ABOUTME: Background scheduler that activates `coach_followups.due_at`
// ABOUTME: Fires push notifications + marks followups delivered when they elapse
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach Followup Scheduler
//!
//! Closes the long-standing gap where `coach_followups.due_at` was
//! advisory metadata — a coach committed "I'll check in Tuesday at 9am",
//! the row carried `due_at='Tue 09:00Z'`, and nothing happened until the
//! user happened to start the next chat turn.
//!
//! This service ticks every [`DEFAULT_TICK_INTERVAL`] from a Tokio task
//! spawned at `ServerContext` construction. On each tick it:
//!
//! 1. Calls [`HarnessMemoryRepository::list_due_followups`] for rows
//!    where `status='pending' AND due_at IS NOT NULL AND due_at <= now`.
//! 2. For each row, dispatches a user-facing push notification through
//!    the configured [`pierre_notifications::NotificationService`] (when
//!    available). Failure is logged but does not block delivery.
//! 3. Calls [`HarnessMemoryRepository::mark_followup_delivered`] so the
//!    row transitions `pending → delivered` regardless of dispatch
//!    outcome — the `due_at` window has fired exactly once, and we don't
//!    want the next tick to re-process the same row.
//!
//! ## Idempotency
//!
//! `mark_followup_delivered` only transitions a row that's still
//! `pending`. If two ticks race (e.g. operator restarted the server mid-
//! tick), the second `UPDATE` returns `0 rows_affected` and we treat the
//! followup as already handled.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use pierre_database::repositories::HarnessMemoryRepository;
use pierre_memory::CoachFollowup;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::mcp::resources::ServerContext;

#[cfg(feature = "client-notifications")]
use pierre_notifications::{
    models::NotificationCategory as CommNotifCategory, DispatchRequest, NotificationService,
    TenantId as CommTenantId,
};

use crate::errors::AppResult;

/// How often the scheduler tick fires.
///
/// 60 seconds keeps latency under one minute past `due_at` while keeping
/// DB load modest. Tunable via [`Self::with_interval`] for tests that
/// want to drive ticks manually.
pub const DEFAULT_TICK_INTERVAL: StdDuration = StdDuration::from_secs(60);

/// How many overdue followups to fetch per tick.
///
/// Higher caps risk a single slow tick if many followups land in the
/// same window; lower caps risk lag. 200 matches the admin triage list
/// limit so a single human-perceptible queue length corresponds to one
/// tick's worth of dispatches.
pub const DEFAULT_BATCH_SIZE: i64 = 200;

/// Outcome of a single scheduler tick — exposed for tests and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickOutcome {
    /// Number of overdue followups the tick fetched from the database.
    pub processed: usize,
    /// Number of notification dispatches the tick attempted (a no-op
    /// when `notification_service` is `None`).
    pub dispatched: usize,
    /// Number of followups successfully transitioned `pending → delivered`.
    pub marked_delivered: usize,
    /// Number of followups where dispatch *or* delivery-marking returned
    /// an error. Logged at warn level; the tick continues processing the
    /// rest of the batch.
    pub errors: usize,
}

/// Run a single scheduler tick.
///
/// Exposed (rather than only a long-running loop) so integration tests
/// can drive the scheduler one tick at a time without `tokio::sleep`.
/// Production callers wrap this in [`run_loop`].
///
/// `notification_service` is taken as a borrowed `Option` because some
/// build profiles compile out the notifications feature entirely.
///
/// # Errors
///
/// Returns the database error if the initial `list_due_followups` query
/// fails. Per-row errors are counted in `TickOutcome.errors` but do not
/// abort the batch.
pub async fn tick<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    #[cfg(feature = "client-notifications")] notification_service: Option<&NotificationService>,
    now: DateTime<Utc>,
    batch_size: i64,
) -> AppResult<TickOutcome> {
    let due = repo.list_due_followups(now, batch_size).await?;

    let mut outcome = TickOutcome {
        processed: due.len(),
        ..TickOutcome::default()
    };

    for followup in due {
        process_followup(
            repo,
            #[cfg(feature = "client-notifications")]
            notification_service,
            &followup,
            &mut outcome,
        )
        .await;
    }

    if outcome.processed > 0 {
        info!(
            processed = outcome.processed,
            dispatched = outcome.dispatched,
            marked_delivered = outcome.marked_delivered,
            errors = outcome.errors,
            "coach followup scheduler tick complete"
        );
    } else {
        debug!("coach followup scheduler tick: no overdue rows");
    }

    Ok(outcome)
}

async fn process_followup<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    #[cfg(feature = "client-notifications")] notification_service: Option<&NotificationService>,
    followup: &CoachFollowup,
    outcome: &mut TickOutcome,
) {
    let Some(tenant_uuid) = parse_tenant_uuid(followup, outcome) else {
        return;
    };
    let tenant_id = TenantId::from(tenant_uuid);

    #[cfg(feature = "client-notifications")]
    try_dispatch(notification_service, followup, tenant_uuid, outcome).await;

    transition_to_delivered(repo, followup, tenant_id, outcome).await;
}

/// Parse the followup's stringly-typed `tenant_id` into a `Uuid`, charging
/// `outcome.errors` and logging on failure. Returns `None` so the caller
/// can short-circuit without further branching (keeping cognitive
/// complexity low).
fn parse_tenant_uuid(followup: &CoachFollowup, outcome: &mut TickOutcome) -> Option<uuid::Uuid> {
    match followup.tenant_id.parse::<uuid::Uuid>() {
        Ok(u) => Some(u),
        Err(e) => {
            error!(
                followup_id = %followup.id,
                tenant_id = %followup.tenant_id,
                error = %e,
                "scheduler: followup has malformed tenant_id, skipping"
            );
            outcome.errors += 1;
            None
        }
    }
}

/// Best-effort notification dispatch. Compiled out when notifications
/// are disabled; updates `outcome.dispatched` / `outcome.errors` per
/// attempt.
#[cfg(feature = "client-notifications")]
async fn try_dispatch(
    service: Option<&NotificationService>,
    followup: &CoachFollowup,
    tenant_uuid: uuid::Uuid,
    outcome: &mut TickOutcome,
) {
    let Some(service) = service else { return };
    outcome.dispatched += 1;
    if let Err(e) = dispatch_notification(service, followup, tenant_uuid).await {
        warn!(
            followup_id = %followup.id,
            error = %e,
            "scheduler: notification dispatch failed (best-effort)"
        );
        outcome.errors += 1;
    }
}

/// Flip the row to `delivered` and tally the outcome. `Ok(false)` from
/// the repo means the row already transitioned (likely an in-prompt
/// delivery on a parallel turn) — not counted as an error.
async fn transition_to_delivered<R: HarnessMemoryRepository + ?Sized>(
    repo: &R,
    followup: &CoachFollowup,
    tenant_id: TenantId,
    outcome: &mut TickOutcome,
) {
    match repo.mark_followup_delivered(&followup.id, tenant_id).await {
        Ok(true) => {
            outcome.marked_delivered += 1;
        }
        Ok(false) => {
            debug!(
                followup_id = %followup.id,
                "scheduler: mark_delivered returned false (already transitioned)"
            );
        }
        Err(e) => {
            error!(
                followup_id = %followup.id,
                error = %e,
                "scheduler: mark_followup_delivered failed"
            );
            outcome.errors += 1;
        }
    }
}

#[cfg(feature = "client-notifications")]
async fn dispatch_notification(
    service: &NotificationService,
    followup: &CoachFollowup,
    tenant_uuid: uuid::Uuid,
) -> Result<pierre_notifications::DispatchOutcome, pierre_notifications::CommereError> {
    let user_uuid = followup.user_id.parse::<uuid::Uuid>().map_err(|_| {
        pierre_notifications::CommereError::Validation {
            field: "user_id".to_owned(),
            reason: format!("not a UUID: {}", followup.user_id),
        }
    })?;

    let request = DispatchRequest {
        user_id: user_uuid,
        tenant_id: CommTenantId(tenant_uuid),
        category: CommNotifCategory::Coach,
        notification_type: "coach_followup_due".to_owned(),
        title: "Your coach has a followup for you".to_owned(),
        body: followup.content.clone(),
        data: None,
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    service.dispatch(&request).await
}

/// Long-running loop that ticks the scheduler at [`DEFAULT_TICK_INTERVAL`].
///
/// Errors during a tick are logged but do not stop the loop — the next
/// tick will retry. Stopping the loop requires aborting the spawned
/// task (held by the caller).
pub async fn run_loop<R>(
    repo: Arc<R>,
    #[cfg(feature = "client-notifications")] notification_service: Option<Arc<NotificationService>>,
    interval_duration: StdDuration,
    batch_size: i64,
) where
    R: HarnessMemoryRepository + ?Sized + 'static,
{
    let mut ticker = interval(interval_duration);
    // Skip the immediate initial tick so a server restart doesn't slam
    // the DB before warm-up completes.
    ticker.tick().await;

    info!(
        interval_secs = interval_duration.as_secs(),
        batch_size, "coach followup scheduler started"
    );

    loop {
        ticker.tick().await;
        let now = Utc::now();
        let result = tick(
            repo.as_ref(),
            #[cfg(feature = "client-notifications")]
            notification_service.as_deref(),
            now,
            batch_size,
        )
        .await;
        if let Err(e) = result {
            error!(error = %e, "coach followup scheduler tick errored — retrying next interval");
        }
    }
}

/// Spawn the followup scheduler as a background tokio task.
///
/// Mirrors [`crate::services::messaging_outbound::start_outbound_worker`]
/// — called once at server bootstrap from
/// `bin/pierre-mcp-server.rs::spawn_background_workers`. The task runs
/// for the server's lifetime; aborting it requires capturing the
/// returned `JoinHandle`, which we currently discard since the scheduler
/// is best-effort and a server restart re-arms it.
pub fn start_followup_scheduler(resources: &Arc<ServerContext>) {
    let repo = Arc::clone(&resources.repos.memory);
    #[cfg(feature = "client-notifications")]
    let notification_service = resources.notification_service.clone();
    tokio::spawn(async move {
        run_loop(
            repo,
            #[cfg(feature = "client-notifications")]
            notification_service,
            DEFAULT_TICK_INTERVAL,
            DEFAULT_BATCH_SIZE,
        )
        .await;
    });
    info!("Coach followup scheduler spawned");
}
