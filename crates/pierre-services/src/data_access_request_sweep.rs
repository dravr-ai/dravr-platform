// ABOUTME: Daily sweep of the athlete's sent data access requests: one in-app reminder per stage, claimed before it is sent
// ABOUTME: Three days before the legal due date, and the day after it passes with no answer, naming the recourse

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Data access request sweep (carnet#1646)
//!
//! Once a day, every sent data access request whose due date is three days
//! away or fewer ([`ReminderStage::DueSoon`]), and every one past due with no
//! answer recorded ([`ReminderStage::Overdue`]), gets one reminder in the app.
//!
//! **Exactly once per stage.** Each reminder is claimed before it is sent,
//! by a conditional update that matches only while the stage's stamp is NULL
//! and the request still has the due date that was read. Two instances
//! sweeping at once both read the row; one claim matches, the other affects
//! nothing and sends nothing. A send that fails after its claim loses that
//! reminder: it is logged, never retried, because a retry is what could send
//! it twice. A due date the athlete moves (a corrected sending date, a GDPR
//! extension) clears both stamps, so the new date is reminded once again.
//!
//! The reminder goes to the app alone — the stored row and the device push —
//! never to the athlete's linked chat channels, at `REMINDER_TIER`.

use std::sync::Arc;
use std::time::Duration;

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::AppResult;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{DataAccessRequest, ReminderStage};
use pierre_database::RepositoryRegistry;
use pierre_notifications::events::{
    data_access_request_route, DATA_ACCESS_COMPANY_PARAM, DATA_ACCESS_DUE_ON_PARAM,
    DATA_ACCESS_REFERENCE_PARAM,
};
use pierre_notifications::models::NotificationCategory;
use pierre_notifications::{
    DispatchOutcome, EventDispatch, NotificationEvent, NotificationService, PushTier,
    TenantId as CommereTenantId,
};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::periodic::spawn_periodic;

/// The worker's name in the ledger and on every line it logs.
const WORKER_NAME: &str = "data access request sweep";

/// How often the sweep runs: the deadlines it watches are counted in days.
const SWEEP_PERIOD: Duration = Duration::from_hours(24);

/// The push tier a reminder is sent at: a legal deadline the athlete acts on,
/// as high-signal as a reauthentication prompt.
const REMINDER_TIER: PushTier = PushTier::P1;

/// How many requests one read of a stage returns.
const SWEEP_BATCH: i64 = 200;

/// How many reads of one stage a pass makes at most: a claimed row leaves the
/// next read, so this bounds a pass that keeps reading rows it cannot claim.
const MAX_READS_PER_STAGE: usize = 50;

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepOutcome {
    /// Reminders this pass claimed.
    pub claimed: u32,
    /// Claimed reminders the pipeline stored for the athlete.
    pub sent: u32,
    /// Claimed reminders the athlete's notification settings suppressed.
    pub suppressed: u32,
    /// Claimed reminders whose send failed: lost, by design.
    pub lost: u32,
    /// Due reminders another pass claimed first, or whose request moved on.
    pub skipped: u32,
}

/// The event a stage raises.
const fn event(stage: ReminderStage) -> NotificationEvent {
    match stage {
        ReminderStage::DueSoon => NotificationEvent::DataAccessRequestDueSoon,
        ReminderStage::Overdue => NotificationEvent::DataAccessRequestOverdue,
    }
}

/// The reminder for `request` at `stage`, due on `due_on`.
fn reminder(request: &DataAccessRequest, stage: ReminderStage, due_on: CivilDate) -> EventDispatch {
    EventDispatch {
        user_id: request.user_id,
        tenant_id: CommereTenantId(request.tenant_id.as_uuid()),
        category: NotificationCategory::System,
        event: event(stage),
        params: json!({
            DATA_ACCESS_COMPANY_PARAM: request.controller,
            DATA_ACCESS_REFERENCE_PARAM: request.reference,
            DATA_ACCESS_DUE_ON_PARAM: due_on.to_string(),
        }),
        // A tap opens the request in Settings → Privacy & Data. The body
        // still names where to go, for a client too old to know the screen.
        route: data_access_request_route(request.id),
        actions: None,
        // At most two per request per due date, each about a legal deadline:
        // the daily cap must not swallow one.
        bypass_frequency_cap: true,
        transport_policy: TransportPolicy::AnyTransport,
    }
}

/// One pass: claim and send every due reminder of both stages on `today`.
///
/// # Errors
/// Returns a database error when a read or a claim fails; reminders claimed
/// before it stay claimed.
pub async fn sweep(
    repos: &RepositoryRegistry,
    notifications: &NotificationService,
    today: CivilDate,
    now: Timestamp,
) -> AppResult<SweepOutcome> {
    let mut outcome = SweepOutcome::default();
    for stage in [ReminderStage::Overdue, ReminderStage::DueSoon] {
        for _ in 0..MAX_READS_PER_STAGE {
            let due = repos
                .data_access_requests
                .due_for_reminder(stage, today, SWEEP_BATCH)
                .await?;
            let read = due.len();
            for request in due {
                remind(repos, notifications, &request, stage, now, &mut outcome).await?;
            }
            if i64::try_from(read).unwrap_or(i64::MAX) < SWEEP_BATCH {
                break;
            }
        }
    }
    Ok(outcome)
}

/// Claim `request`'s `stage` reminder and send it.
async fn remind(
    repos: &RepositoryRegistry,
    notifications: &NotificationService,
    request: &DataAccessRequest,
    stage: ReminderStage,
    now: Timestamp,
    outcome: &mut SweepOutcome,
) -> AppResult<()> {
    let Some(due_on) = request.due_on else {
        outcome.skipped += 1;
        return Ok(());
    };
    if !repos
        .data_access_requests
        .claim_reminder(stage, request, due_on, now)
        .await?
    {
        outcome.skipped += 1;
        return Ok(());
    }
    outcome.claimed += 1;
    match notifications
        .dispatch_event_in_app(&reminder(request, stage, due_on), REMINDER_TIER)
        .await
    {
        Ok(delivery) => {
            if let DispatchOutcome::Suppressed(reason) = delivery.outcome {
                outcome.suppressed += 1;
                info!(
                    worker = WORKER_NAME,
                    request_id = %request.id,
                    user_id = %request.user_id,
                    stage = ?stage,
                    reason = ?reason,
                    "data access reminder suppressed by the athlete's notification settings"
                );
            } else {
                outcome.sent += 1;
            }
        }
        Err(e) => {
            outcome.lost += 1;
            warn!(
                worker = WORKER_NAME,
                request_id = %request.id,
                user_id = %request.user_id,
                stage = ?stage,
                error = %e,
                "data access reminder send failed after its claim; this reminder is lost"
            );
        }
    }
    Ok(())
}

/// Spawn the daily sweep.
///
/// Fire-and-forget, like every sibling scheduler; the worker ledger decides
/// when the next pass is due, so a restart resumes the cadence.
pub fn spawn_data_access_request_sweep(
    repos: Arc<RepositoryRegistry>,
    notifications: Arc<NotificationService>,
) {
    let ledger = Arc::clone(&repos.worker_runs);
    spawn_periodic(WORKER_NAME, SWEEP_PERIOD, ledger, move || {
        let repos = Arc::clone(&repos);
        let notifications = Arc::clone(&notifications);
        async move {
            let now = Timestamp::now();
            let outcome = sweep(&repos, &notifications, now.utc_date(), now).await?;
            if outcome.claimed > 0 {
                debug!(
                    worker = WORKER_NAME,
                    claimed = outcome.claimed,
                    sent = outcome.sent,
                    suppressed = outcome.suppressed,
                    lost = outcome.lost,
                    skipped = outcome.skipped,
                    "data access request sweep pass complete"
                );
            }
            Ok(())
        }
    });
}
