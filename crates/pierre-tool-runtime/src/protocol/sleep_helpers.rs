// ABOUTME: Sleep and recovery reads shared by the sleep, recovery and analytics tools
// ABOUTME: Stored rows every connected source synced, merged per night or per day

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sleep and recovery reads.
//!
//! Every sleep-scoring tool reads the same place: the sleep and recovery rows
//! dravr-enforme syncs from each connected source (WHOOP, Garmin and
//! intervals.icu for nights and mornings; COROS for mornings only — its resting
//! heart rate and sleep HRV, since the Training Hub records no night), merged
//! so one night — or one day — reported by two sources is one record carrying
//! both sources' metrics. The chat pipeline refreshes a stale API source
//! before the turn runs; a scrape-backed source (Garmin, COROS) syncs when the
//! athlete connects it and then at most every six hours, never inside a turn.
//!
//! A caller naming a source (the tools' `sleep_provider` argument) narrows the
//! rows to that source before merging; otherwise every source contributes.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::models::{
    merge_recovery_metrics, merge_sleep_sessions, Merged, StoredRecoveryMetrics,
    StoredSleepSession, StoredSleepStageType, TenantId,
};
use pierre_intelligence::SleepData;
use tracing::warn;
use uuid::Uuid;

use crate::protocol::types::{UniversalResponse, UniversalToolExecutor};

/// Extra days read before the requested window, so a night that began the
/// evening before the window's first day still falls inside it.
const WINDOW_MARGIN_DAYS: i64 = 2;

fn failure(message: String) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: None,
        error: Some(message),
        metadata: None,
    }
}

fn tenant_of(tenant_id: Option<&str>) -> Option<TenantId> {
    tenant_id.and_then(|t| TenantId::parse_str(t).ok())
}

fn missing_tenant() -> UniversalResponse {
    failure(
        "Synced sleep is read per tenant and this request carries none; \
         pass sleep_data to analyze a night directly."
            .to_owned(),
    )
}

/// Nights of synced sleep over the last `days` days, one entry per night
/// merged across sources, newest first. `source` narrows the rows to one
/// source before merging.
///
/// # Errors
/// Returns a boxed `UniversalResponse` when the request has no tenant or the stored
/// rows cannot be read.
pub async fn stored_sleep_nights(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    source: Option<&str>,
    days: u32,
) -> Result<Vec<Merged<StoredSleepSession>>, Box<UniversalResponse>> {
    let tenant = tenant_of(tenant_id).ok_or_else(|| Box::new(missing_tenant()))?;
    let end = Utc::now();
    let start = end - Duration::days(i64::from(days) + WINDOW_MARGIN_DAYS);
    let mut sessions = executor
        .resources
        .repos()
        .sleep
        .get_sleep_sessions(user_uuid, &tenant, start, end)
        .await
        .map_err(|e| {
            warn!(error = %e, "stored sleep sessions unreadable");
            Box::new(failure(
                "Sleep data could not be read right now.".to_owned(),
            ))
        })?;
    if let Some(source) = source {
        sessions.retain(|s| s.source_name.eq_ignore_ascii_case(source));
    }
    let mut nights: Vec<_> = merge_sleep_sessions(sessions)
        .into_iter()
        .filter(|night| !night.record.is_nap)
        .collect();
    nights.reverse();
    Ok(nights)
}

/// Synced nights over the last `days` days as `SleepData`, newest first.
///
/// Each carries the sources it merges. A night without its own HRV or resting
/// heart rate takes them from the recovery reading of the morning it ended
/// on — WHOOP, for one, reports HRV on recovery rather than on sleep.
///
/// # Errors
/// Returns a boxed `UniversalResponse` when the request has no tenant or the stored
/// rows cannot be read.
pub async fn sleep_history_data(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    source: Option<&str>,
    days: u32,
) -> Result<Vec<(SleepData, Vec<String>)>, Box<UniversalResponse>> {
    let nights = stored_sleep_nights(executor, user_uuid, tenant_id, source, days).await?;
    let recovery = recovery_by_date(executor, user_uuid, tenant_id, days).await;
    Ok(nights
        .into_iter()
        .map(|night| {
            let mut data = stored_sleep_to_data(&night.record);
            if let Some(day) = recovery.get(&night.record.end_datetime.date_naive()) {
                if data.hrv_rmssd_ms.is_none() {
                    data.hrv_rmssd_ms = day.hrv_rmssd.or(day.hrv_ms);
                }
                if data.resting_hr_bpm.is_none() {
                    data.resting_hr_bpm = day.resting_heart_rate;
                }
            }
            (data, night.sources)
        })
        .collect())
}

/// The most recent synced night as `SleepData`, with the sources it merges.
///
/// # Errors
/// Returns a boxed `UniversalResponse` when no night was synced in the last `days`
/// days (from `source`, when one is named) or the rows cannot be read.
pub async fn latest_sleep_data(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    source: Option<&str>,
    days: u32,
) -> Result<(SleepData, Vec<String>), Box<UniversalResponse>> {
    sleep_history_data(executor, user_uuid, tenant_id, source, days)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            Box::new(failure(source.map_or_else(
                || {
                    format!(
                        "No synced sleep for the last {days} day(s). Connect a sleep-tracking \
                         source (WHOOP, Garmin, intervals.icu) or pass sleep_data manually."
                    )
                },
                |source| format!("No sleep synced from {source} for the last {days} day(s)."),
            )))
        })
}

/// Merged recovery readings over the window, keyed by date. Empty when the
/// rows cannot be read: recovery only enriches sleep, it never blocks it.
async fn recovery_by_date(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    days: u32,
) -> HashMap<NaiveDate, StoredRecoveryMetrics> {
    let Some(tenant) = tenant_of(tenant_id) else {
        return HashMap::new();
    };
    let end = Utc::now();
    let start = end - Duration::days(i64::from(days) + WINDOW_MARGIN_DAYS);
    match executor
        .resources
        .repos()
        .recovery
        .get_recovery_metrics(user_uuid, &tenant, start, end)
        .await
    {
        Ok(rows) => merge_recovery_metrics(rows)
            .into_iter()
            .map(|day| (day.record.date, day.record))
            .collect(),
        Err(e) => {
            warn!(error = %e, "stored recovery unreadable; sleep served without it");
            HashMap::new()
        }
    }
}

/// Convert a stored (merged) sleep session to the intelligence layer's `SleepData`.
///
/// Stage durations come from the per-stage totals, falling back
/// to summing the stage list when a source only sent the list.
#[must_use]
pub fn stored_sleep_to_data(session: &StoredSleepSession) -> SleepData {
    let hours = |explicit: Option<u32>, kind: StoredSleepStageType| -> Option<f64> {
        let seconds = explicit.unwrap_or_else(|| {
            session
                .stages
                .iter()
                .filter(|stage| stage.stage_type == kind)
                .map(|stage| stage.duration_seconds)
                .sum()
        });
        (seconds > 0).then(|| f64::from(seconds) / 3600.0)
    };
    let asleep_seconds = session.total_sleep_seconds.unwrap_or_else(|| {
        let span = (session.end_datetime - session.start_datetime).num_seconds();
        u32::try_from(span.max(0)).unwrap_or(u32::MAX)
    });

    SleepData {
        date: session.start_datetime,
        duration_hours: f64::from(asleep_seconds) / 3600.0,
        deep_sleep_hours: hours(session.deep_sleep_seconds, StoredSleepStageType::Deep),
        rem_sleep_hours: hours(session.rem_sleep_seconds, StoredSleepStageType::Rem),
        light_sleep_hours: hours(session.light_sleep_seconds, StoredSleepStageType::Light),
        awake_hours: hours(session.awake_seconds, StoredSleepStageType::Awake),
        efficiency_percent: session.sleep_efficiency,
        hrv_rmssd_ms: session.avg_hrv,
        resting_hr_bpm: session.min_heart_rate,
        provider_score: session.sleep_score.map(f64::from),
    }
}
