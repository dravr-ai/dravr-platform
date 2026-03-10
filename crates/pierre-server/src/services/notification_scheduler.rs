// ABOUTME: Background worker that polls scheduled_notifications table every 60 seconds
// ABOUTME: Fires due notifications via the dispatch pipeline and computes next fire times using cron
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Notification Scheduler
//!
//! Spawns a background tokio task that polls the `scheduled_notifications` table every
//! 60 seconds. For each due notification (`next_fire_at <= now` and `enabled = true`):
//!
//! 1. Builds a `DispatchRequest` based on the notification type
//! 2. Dispatches via `NotificationDispatcher`
//! 3. Computes the next fire time using the cron expression and user's timezone
//! 4. Updates `last_fired_at` and `next_fire_at` in the database
//!
//! Errors are logged but never block processing of other due notifications.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use tokio::task::AbortHandle;
use tokio::time::{interval_at, Duration, Instant};
use tracing::{debug, info, warn};

use pierre_core::models::notifications::{
    NotificationCategory, ScheduledNotification, UpdateScheduledNotificationParams,
};
use pierre_database::plugins::factory::Database;
use pierre_database::repositories::PushNotificationRepository;

use super::notification_dispatch::{DispatchRequest, NotificationDispatcher};

/// Scheduler poll interval in seconds
const POLL_INTERVAL_SECS: u64 = 60;

/// Start the notification scheduler background task
///
/// Spawns a tokio task that polls every 60 seconds for due scheduled notifications
/// and dispatches them through the notification pipeline.
pub fn start_notification_scheduler(
    database: Arc<Database>,
    dispatcher: Arc<NotificationDispatcher>,
) -> AbortHandle {
    let poll_duration = Duration::from_secs(POLL_INTERVAL_SECS);
    let handle = tokio::spawn(async move {
        // First tick after one interval (avoids startup load)
        let mut ticker = interval_at(Instant::now() + poll_duration, poll_duration);
        loop {
            ticker.tick().await;
            run_scheduler_cycle(&database, &dispatcher).await;
        }
    });
    let abort_handle = handle.abort_handle();
    info!("Notification scheduler started (polling every {POLL_INTERVAL_SECS}s)");
    abort_handle
}

/// Execute one scheduler cycle: query due notifications and dispatch each one
async fn run_scheduler_cycle(database: &Database, dispatcher: &NotificationDispatcher) {
    let now = Utc::now();
    let due_notifications = query_due_notifications(database, now).await;

    for scheduled in &due_notifications {
        dispatch_scheduled(database, dispatcher, scheduled).await;
        update_next_fire_time(database, scheduled, now).await;
    }
}

/// Query due scheduled notifications, returning empty vec on error
async fn query_due_notifications(
    database: &Database,
    now: DateTime<Utc>,
) -> Vec<ScheduledNotification> {
    match database.get_due_scheduled_notifications(now).await {
        Ok(n) if n.is_empty() => {
            debug!("Scheduler tick: no due notifications");
            Vec::new()
        }
        Ok(n) => {
            info!(count = n.len(), "Processing due scheduled notifications");
            n
        }
        Err(e) => {
            warn!("Failed to query due scheduled notifications: {e}");
            Vec::new()
        }
    }
}

/// Dispatch the notification (errors logged, not propagated)
async fn dispatch_scheduled(
    database: &Database,
    dispatcher: &NotificationDispatcher,
    scheduled: &ScheduledNotification,
) {
    let request = build_dispatch_request(database, scheduled).await;
    if let Err(e) = dispatcher.dispatch(&request).await {
        warn!(
            scheduled_id = %scheduled.id,
            notification_type = %scheduled.notification_type,
            user_id = %scheduled.user_id,
            error = %e,
            "Failed to dispatch scheduled notification"
        );
    }
}

/// Compute and persist the next fire time, or disable the schedule if none can be computed
async fn update_next_fire_time(
    database: &Database,
    scheduled: &ScheduledNotification,
    now: DateTime<Utc>,
) {
    let next_fire_at = compute_next_fire_time(&scheduled.schedule_cron, &scheduled.timezone, now);

    if let Some(next) = next_fire_at {
        if let Err(e) = database
            .update_scheduled_notification_fired(scheduled.id, now, next)
            .await
        {
            warn!(
                scheduled_id = %scheduled.id,
                error = %e,
                "Failed to update scheduled notification fire times"
            );
        }
        return;
    }

    warn!(
        scheduled_id = %scheduled.id,
        cron = %scheduled.schedule_cron,
        "Could not compute next fire time — disabling schedule"
    );
    let disable_params = UpdateScheduledNotificationParams {
        id: scheduled.id,
        user_id: scheduled.user_id,
        tenant_id: scheduled.tenant_id,
        enabled: Some(false),
        schedule_cron: None,
        timezone: None,
        next_fire_at: None,
    };
    let _ = database
        .update_scheduled_notification(&disable_params)
        .await;
}

/// Build a `DispatchRequest` from a scheduled notification's type.
///
/// Queries the database for dynamic content (activity counts, training duration)
/// and falls back to static text if the query fails.
async fn build_dispatch_request(
    database: &Database,
    scheduled: &ScheduledNotification,
) -> DispatchRequest {
    let (category, title, body, data) = match scheduled.notification_type.as_str() {
        "weekly_training_summary" => {
            let dynamic_body = build_weekly_summary_body(database, scheduled).await;
            (
                NotificationCategory::Training,
                "Weekly training summary".to_owned(),
                dynamic_body,
                Some(serde_json::json!({ "screen": "activities" })),
            )
        }
        "workout_reminder" => {
            let dynamic_body = build_workout_reminder_body(database, scheduled).await;
            (
                NotificationCategory::Reminders,
                "Workout reminder".to_owned(),
                dynamic_body,
                Some(serde_json::json!({ "screen": "activities" })),
            )
        }
        "weekly_digest" => (
            NotificationCategory::Reminders,
            "Your week in review".to_owned(),
            "Check out your weekly highlights".to_owned(),
            Some(serde_json::json!({ "screen": "activities" })),
        ),
        other => (
            NotificationCategory::System,
            "Scheduled notification".to_owned(),
            format!("Scheduled: {other}"),
            None,
        ),
    };

    DispatchRequest {
        user_id: scheduled.user_id,
        tenant_id: scheduled.tenant_id,
        category,
        notification_type: scheduled.notification_type.clone(),
        title,
        body,
        data,
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    }
}

/// Query the user's `activity_synced` notification count from the past 7 days for a dynamic summary
async fn build_weekly_summary_body(
    database: &Database,
    scheduled: &ScheduledNotification,
) -> String {
    let seven_days_ago = Utc::now() - ChronoDuration::days(7);
    match database
        .count_notifications_since(
            scheduled.user_id,
            scheduled.tenant_id,
            "training",
            seven_days_ago,
        )
        .await
    {
        Ok(count) if count > 0 => {
            format!("You completed {count} training activities this week")
        }
        Ok(_) => "No training activities recorded this week — get moving!".to_owned(),
        Err(e) => {
            warn!(
                user_id = %scheduled.user_id,
                error = %e,
                "Failed to query weekly training data for scheduled notification"
            );
            "Your weekly training summary is ready".to_owned()
        }
    }
}

/// Check if the user already logged an activity today for a dynamic workout reminder
async fn build_workout_reminder_body(
    database: &Database,
    scheduled: &ScheduledNotification,
) -> String {
    let today_start = Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));

    let Some(since) = today_start else {
        return "Don't forget your workout today".to_owned();
    };

    match database
        .count_notifications_since(scheduled.user_id, scheduled.tenant_id, "training", since)
        .await
    {
        Ok(count) if count > 0 => "Great job on today's workout! Keep it up".to_owned(),
        Ok(_) => "No activity logged today — time for a workout!".to_owned(),
        Err(e) => {
            warn!(
                user_id = %scheduled.user_id,
                error = %e,
                "Failed to query today's activity data for workout reminder"
            );
            "Don't forget your workout today".to_owned()
        }
    }
}

/// Compute the next fire time from a cron expression in the given timezone
///
/// Returns `None` if the cron expression is invalid or no upcoming time can be found.
pub fn compute_next_fire_time(
    cron_expr: &str,
    timezone_str: &str,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    // Cron crate expects 6-field or 7-field expressions (with seconds).
    // Users provide 5-field (minute hour dom month dow), so prepend "0" for seconds.
    let full_cron = format!("0 {cron_expr}");

    let schedule = Schedule::from_str(&full_cron).ok()?;
    let tz: Tz = timezone_str.parse().unwrap_or(Tz::UTC);

    // Convert `after` to the user's timezone for correct cron evaluation
    let after_in_tz = after.with_timezone(&tz);

    schedule
        .after(&after_in_tz)
        .next()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Validate a cron expression (5-field: minute hour dom month dow)
///
/// Returns `Ok(())` if valid, `Err(message)` if invalid.
pub fn validate_cron_expression(cron_expr: &str) -> Result<(), String> {
    let full_cron = format!("0 {cron_expr}");
    Schedule::from_str(&full_cron)
        .map(|_| ())
        .map_err(|e| format!("Invalid cron expression '{cron_expr}': {e}"))
}
