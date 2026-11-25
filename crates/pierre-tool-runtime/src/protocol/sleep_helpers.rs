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
//! before the turn runs; a session-read source (Garmin, COROS) syncs when the
//! athlete connects it and then at most every six hours, never inside a turn.
//!
//! A caller naming a source (the tools' `sleep_provider` argument) narrows the
//! rows to that source before merging; otherwise every source contributes.
//! Either way `pierre_services::stored_health` reads, gates and merges them:
//! only the rows each source's provider terms let the reader see reach the
//! merge. A night is dated by the morning it ended on, in the athlete's zone.

use std::collections::{BTreeMap, HashMap};

use dravr_cageux::config::intelligence::SleepRecoveryConfig;
use dravr_cageux::error::IntelligenceError;
use dravr_cageux::sleep_analysis::{HrvTrendAnalysis, NightlyHrv, SleepAnalyzer, SleepData};
use dravr_tronc::dates::{CivilDate, CivilTime, Elapsed, Timestamp, Zone};
use pierre_core::models::{
    Merged, StoredRecoveryMetrics, StoredSleepSession, StoredSleepStageType, TenantId,
};
use tracing::warn;
use uuid::Uuid;

use crate::protocol::types::{UniversalResponse, UniversalToolExecutor};
use pierre_services::athlete_clock::athlete_zone;
use pierre_services::stored_health::{self, SourceFilter};

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
    let end = Timestamp::now();
    let start = end - Elapsed::days(i64::from(days) + WINDOW_MARGIN_DAYS);
    let filter = source.map_or(SourceFilter::All, SourceFilter::Service);
    let sessions = stored_health::merged_sleep(
        executor.resources.repos(),
        executor.resources.provider_registry().as_ref(),
        user_uuid,
        &tenant,
        (start, end),
        filter,
        stored_health::as_stored,
    )
    .await
    .map(|read| read.records)
    .map_err(|e| {
        warn!(error = %e, "stored sleep sessions unreadable");
        Box::new(failure(
            "Sleep data could not be read right now.".to_owned(),
        ))
    })?;
    let mut nights: Vec<_> = sessions
        .into_iter()
        .filter(|night| !night.record.is_nap)
        .collect();
    nights.reverse();
    Ok(nights)
}

/// The HRV trend of the night `sleep_data` describes: the athlete's own
/// stored nightly RMSSD, judged by cageux as the week ending that night
/// against the weeks before it ([`SleepAnalyzer::analyze_hrv_trends`]).
///
/// The nights come from the same source as the analysed one when a
/// `source` is named, so two devices' RMSSD, which run at different levels,
/// never share one baseline, and each is dated by the day it ended on in the
/// athlete's zone, as cageux's `NightlyHrv` expects. The analysed night's own
/// HRV joins the series when it is not a stored night (a `sleep_data` passed
/// by hand); without a user there is no stored history and the week reads
/// `insufficient`. `None` when that night carries no HRV; a stored history too
/// short to judge reads `insufficient`, never `normal`.
///
/// # Errors
/// Returns cageux's error when the configured HRV band or windows are not
/// positive.
pub async fn hrv_trend(
    executor: &UniversalToolExecutor,
    user_uuid: Option<Uuid>,
    tenant_id: Option<&str>,
    source: Option<&str>,
    sleep_data: &SleepData,
    config: &SleepRecoveryConfig,
) -> Result<Option<HrvTrendAnalysis>, IntelligenceError> {
    let Some(rmssd) = sleep_data.hrv_rmssd_ms else {
        return Ok(None);
    };
    // The baseline, the week, and a night either side for the window's edges.
    let days = config.hrv.baseline_days.saturating_add(8);
    let (zone, mut by_night) = match user_uuid {
        Some(user_uuid) => {
            let zone = athlete_zone(executor.resources.repos(), user_uuid).await;
            let stored =
                stored_hrv_by_night(executor, user_uuid, tenant_id, source, days, &zone).await;
            (zone, stored)
        }
        None => (Zone::utc(), BTreeMap::new()),
    };
    let as_of = analysed_night_date(sleep_data, &zone);
    by_night.entry(as_of).or_insert(rmssd);
    let nights: Vec<NightlyHrv> = by_night
        .into_iter()
        .map(|(date, rmssd_ms)| NightlyHrv { date, rmssd_ms })
        .collect();
    SleepAnalyzer::analyze_hrv_trends(&nights, as_of, config).map(Some)
}

/// The athlete's stored nightly HRV over the last `days` days, one reading
/// per night, keyed by the night it is (the day it ended on in `zone`, as
/// [`StoredSleepSession::night_date`] dates it): the night's own average HRV,
/// else that morning's recovery reading, the way [`sleep_history_data`] fills
/// a night. Empty when the rows cannot be read.
async fn stored_hrv_by_night(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    source: Option<&str>,
    days: u32,
    zone: &Zone,
) -> BTreeMap<CivilDate, f64> {
    let nights = match stored_sleep_nights(executor, user_uuid, tenant_id, source, days).await {
        Ok(nights) => nights,
        Err(unread) => {
            warn!(
                error = ?unread.error,
                "stored sleep unreadable; HRV trend judged on the analysed night alone"
            );
            return BTreeMap::new();
        }
    };
    let recovery = recovery_by_date(executor, user_uuid, tenant_id, source, days).await;
    let mut by_night = BTreeMap::new();
    // Newest first: a night two sessions share keeps the later one's reading.
    for night in nights {
        let day = night.record.night_date(zone);
        let hrv = night.record.avg_hrv.or_else(|| {
            recovery
                .get(&day)
                .and_then(StoredRecoveryMetrics::daily_hrv)
        });
        if let Some(hrv) = hrv {
            by_night.entry(day).or_insert(hrv);
        }
    }
    by_night
}

/// The night `sleep_data` describes, dated as a stored night is: a night
/// anchored at midnight UTC (a day summary, or a `sleep_data` dated by hand
/// with a bare `YYYY-MM-DD`) is that date; otherwise the day its end falls on
/// in `zone`, the end being its start plus the time asleep and awake.
fn analysed_night_date(sleep_data: &SleepData, zone: &Zone) -> CivilDate {
    if sleep_data.date.utc_time() == CivilTime::MIDNIGHT {
        return sleep_data.date.utc_date();
    }
    let hours_in_bed = sleep_data.duration_hours + sleep_data.awake_hours.unwrap_or(0.0);
    // Bounded: a night in bed is under a day of seconds.
    #[allow(clippy::cast_possible_truncation)]
    let seconds_in_bed = (hours_in_bed.clamp(0.0, 24.0) * 3600.0).round() as i64;
    zone.date_at(sleep_data.date + Elapsed::seconds(seconds_in_bed))
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
    let recovery = recovery_by_date(executor, user_uuid, tenant_id, source, days).await;
    let zone = athlete_zone(executor.resources.repos(), user_uuid).await;
    Ok(nights
        .into_iter()
        .map(|night| {
            let mut data = stored_sleep_to_data(&night.record);
            if let Some(day) = recovery.get(&night.record.night_date(&zone)) {
                if data.hrv_rmssd_ms.is_none() {
                    data.hrv_rmssd_ms = day.daily_hrv();
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

/// Merged recovery readings over the window, keyed by date, narrowed to
/// `source` like the nights they fill. Empty when the rows cannot be read:
/// recovery only enriches sleep, it never blocks it.
async fn recovery_by_date(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    source: Option<&str>,
    days: u32,
) -> HashMap<CivilDate, StoredRecoveryMetrics> {
    let Some(tenant) = tenant_of(tenant_id) else {
        return HashMap::new();
    };
    let end = Timestamp::now();
    let start = end - Elapsed::days(i64::from(days) + WINDOW_MARGIN_DAYS);
    match stored_health::merged_recovery(
        executor.resources.repos(),
        executor.resources.provider_registry().as_ref(),
        user_uuid,
        &tenant,
        (start, end),
        source.map_or(SourceFilter::All, SourceFilter::Service),
        stored_health::as_stored,
    )
    .await
    {
        Ok(read) => read
            .records
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
