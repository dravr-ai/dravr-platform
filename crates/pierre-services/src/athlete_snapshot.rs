// ABOUTME: Builds an AthleteMetrics snapshot (personalized-layer input) from a user's stored physiology + recent activities
// ABOUTME: Reads the physiological profile, activity cache and synced nights; runs cageux pace/zone math + the training-history form compute
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete snapshot builder
//!
//! [`build_athlete_metrics`] assembles the [`AthleteMetrics`] snapshot that the
//! personalized-physiology layer scores claims against. It is the one
//! place that reaches for the athlete's data — the `pierre_evals` crate stays
//! dependency-light and only sees the resulting plain struct.
//!
//! Three reads (physiology profile, recent activities, synced nights) plus
//! pure cageux compute:
//!
//! - The `UserPhysiologicalProfile` supplies VO2max/VDOT, max HR, and FTP, and
//!   feeds a [`VO2MaxCalculator`] for the Daniels training-pace ranges.
//! - The activity cache supplies `data_days` (the stale-estimate guard) and,
//!   via [`compute_training_history`], today's form reading: CTL, TSB, TSB as a
//!   share of CTL, and its band.
//! - The sleep sessions and recovery readings health sync stores supply the
//!   overnight HRV, resting heart rate and sleep ([`RecoveryReadings`]), read
//!   through [`stored_health`] so each source's provider terms apply before
//!   the sources merge (carnet#853). WHOOP's own scores never enter: only
//!   measurements are read, and health sync drops the scores at ingestion.
//!
//! Any missing input degrades to `None` for that metric; a user with no
//! profile, a thin history and no synced nights yields a snapshot whose
//! [`AthleteMetrics::is_usable`] is false, so the personalized layer stays
//! silent.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::config::intelligence::{TrainingZonesConfig, VO2MaxCalculator};
use dravr_cageux::physiological_constants::physiological_defaults::{
    DEFAULT_LACTATE_THRESHOLD, DEFAULT_SPORT_EFFICIENCY,
};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::civil_time::{clock_date, local_date, resolve_zone};
use pierre_core::constants::time_constants::SECONDS_PER_HOUR_F64;
use pierre_core::models::ProvenancedValue;
use pierre_core::models::{
    merge_recovery_metrics, merge_sleep_sessions, StoredRecoveryMetrics, StoredSleepSession,
};
use pierre_core::models::{
    Activity, DailyTrainingState, FormReading, TenantId, UserPhysiologicalProfile,
};
use pierre_database::RepositoryRegistry;
use pierre_evals::personalized_recovery::BASELINE_DAYS;
use pierre_evals::{AthleteMetrics, NightlyMetric, RecoveryReadings, StoredMetric};
use pierre_fitness_compute::{compute_training_history, AthleteInputs};
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::stored_health;

/// Lookback window for the activity read backing `data_days` and the form compute.
const ACTIVITY_LOOKBACK_DAYS: i64 = 180;
/// Compute window for the training-history form series (CTL warm-up is supplied
/// by the wider activity read above).
const TSB_COMPUTE_DAYS: i64 = 90;
/// Cap on activities loaded for the snapshot.
const ACTIVITY_READ_LIMIT: i64 = 500;
/// Days read before the overnight-readings window, so a night that began the
/// evening before its first day is still read.
const NIGHTS_READ_MARGIN_DAYS: i64 = 2;

/// The athlete's stored IANA timezone, or `None` when it cannot be read.
///
/// A failure here is not worth failing the snapshot over: every caller degrades
/// to UTC, which is what an athlete with no zone on file already gets.
async fn load_user_timezone(repos: &RepositoryRegistry, user_id: Uuid) -> Option<String> {
    match repos.users.get_global(user_id).await {
        Ok(user) => user.and_then(|u| u.timezone),
        Err(e) => {
            tracing::warn!(error = %e, "personalized verification: user timezone load failed");
            None
        }
    }
}

/// The verifier's copy of a provenanced profile value: the number, whether it
/// is an estimate the reply must quote as one, and who produced it.
fn stored_metric<T>(stored: &ProvenancedValue<T>, value: f64) -> StoredMetric {
    StoredMetric {
        value,
        kind: stored.provenance.kind,
        origin: stored.provenance.origin.clone(),
    }
}

/// Assemble the [`AthleteMetrics`] snapshot for one athlete.
///
/// Best-effort: a failed read logs and leaves the affected metrics `None`,
/// never erroring the turn.
pub async fn build_athlete_metrics(
    repos: &RepositoryRegistry,
    policies: &dyn ProviderTerms,
    algorithm_config: &AlgorithmConfig,
    training_zones: &TrainingZonesConfig,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AthleteMetrics {
    let mut metrics = AthleteMetrics::default();

    let profile = match repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await
    {
        // A profile written from first-party-only data is withheld from an
        // external turn (carnet#769).
        Ok(p) => p.filter(|profile| ai_scope::admit_derived(profile.transport_policy)),
        Err(e) => {
            tracing::warn!(error = %e, "personalized verification: physiology profile load failed");
            None
        }
    };

    if let Some(p) = &profile {
        // cageux treats VDOT as the VO2max-equivalent feeding the Daniels tables.
        metrics.vo2max = p.vo2_max;
        metrics.vdot = p.vo2_max;
        metrics.ftp_watts = p.ftp_watts.map(f64::from);
        metrics.max_hr = p.max_hr.map(f64::from);
        metrics.critical_power_watts = p
            .critical_power_watts
            .as_ref()
            .map(|v| stored_metric(v, f64::from(v.value)));
        metrics.w_prime_joules = p
            .w_prime_joules
            .as_ref()
            .map(|v| stored_metric(v, f64::from(v.value)));
        metrics.critical_speed_mps = p
            .critical_speed_mps
            .as_ref()
            .map(|v| stored_metric(v, v.value));
        metrics.d_prime_meters = p.d_prime_meters.as_ref().map(|v| stored_metric(v, v.value));

        if let Some(vo2) = p.vo2_max {
            // Pace zones read only VO2max and the lactate threshold (cageux's
            // pace-zone default when the profile omits it); the calculator's
            // heart-rate fields are not read by them.
            let calculator = VO2MaxCalculator::new(
                vo2,
                0,
                0,
                p.lactate_threshold_percentage
                    .unwrap_or(DEFAULT_LACTATE_THRESHOLD),
                DEFAULT_SPORT_EFFICIENCY,
            );
            let paces = calculator.calculate_pace_zones(training_zones);
            metrics.easy_pace_range = Some(paces.easy_pace_range);
            metrics.marathon_pace_range = Some(paces.marathon_pace_range);
            metrics.threshold_pace_range = Some(paces.threshold_pace_range);
            metrics.interval_pace_range = Some(paces.vo2max_pace_range);
        }
    }

    // The athlete's own zone: every calendar-day boundary below is theirs, not
    // the server's. A 21:00 America/Toronto session belongs to the day they
    // trained (registre#200).
    let user_timezone = load_user_timezone(repos, user_id).await;

    let now = Utc::now();
    let activities = match repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            None,
            now - Duration::days(ACTIVITY_LOOKBACK_DAYS),
            now,
            ACTIVITY_READ_LIMIT,
        )
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "personalized verification: activity load failed");
            Vec::new()
        }
    };
    // The metrics judge what a model said, so they read what it may see
    // (carnet#723).
    let activities = ai_scope::filter_activities(policies, activities);

    // data_days = distinct calendar days with an activity — the density signal
    // gating whether the personalized layer trusts the snapshot enough to contradict an agent.
    let zone = resolve_zone(user_timezone.as_deref());
    let mut activity_days: Vec<_> = activities
        .iter()
        .map(|a| local_date(a.start_date(), zone))
        .collect();
    activity_days.sort_unstable();
    activity_days.dedup();
    metrics.data_days = u32::try_from(activity_days.len()).unwrap_or(u32::MAX);

    metrics.recovery = load_recovery_readings(repos, policies, tenant_id, user_id, zone, now).await;

    if !activities.is_empty() {
        metrics.form = form_today(
            &activities,
            profile.as_ref(),
            algorithm_config,
            clock_date(now, zone),
            user_timezone.as_deref(),
        );
    }

    metrics
}

/// The athlete's overnight readings over the last week, read under each
/// source's provider terms. A failed read logs and leaves that half empty.
async fn load_recovery_readings(
    repos: &RepositoryRegistry,
    policies: &dyn ProviderTerms,
    tenant_id: TenantId,
    user_id: Uuid,
    zone: Tz,
    now: DateTime<Utc>,
) -> RecoveryReadings {
    let start = now - Duration::days(BASELINE_DAYS + NIGHTS_READ_MARGIN_DAYS);
    let nights =
        match stored_health::sleep_sessions(repos, policies, user_id, &tenant_id, start, now).await
        {
            Ok(read) => read.records,
            Err(e) => {
                tracing::warn!(error = %e, "personalized verification: sleep sessions load failed");
                Vec::new()
            }
        };
    let mornings = match stored_health::recovery_metrics(
        repos, policies, user_id, &tenant_id, start, now,
    )
    .await
    {
        Ok(read) => read.records,
        Err(e) => {
            tracing::warn!(error = %e, "personalized verification: recovery readings load failed");
            Vec::new()
        }
    };
    recovery_readings(nights, mornings, zone, clock_date(now, zone))
}

/// One reading per civil day of HRV, resting HR and sleep, from the nights and
/// mornings each source synced.
///
/// Rows are merged across sources first, the way every sleep and recovery
/// tool reads them, so one night two devices recorded counts once. A night
/// belongs to the day it ended on, in the athlete's zone; naps are left out.
///
/// A day's HRV and resting HR are read two ways, because the night and the
/// next morning can each carry one: morning-first (the recovery reading's,
/// else the night's own average HRV and lowest heart rate), as the readiness
/// tools quote them, and night-first, as the sleep tools quote them.
fn recovery_readings(
    nights: Vec<StoredSleepSession>,
    mornings: Vec<StoredRecoveryMetrics>,
    zone: Tz,
    today: NaiveDate,
) -> RecoveryReadings {
    let mut sleep: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    let mut night_hrv: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    let mut night_min_hr: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for night in merge_sleep_sessions(nights) {
        let session = night.record;
        if session.is_nap {
            continue;
        }
        let day = local_date(session.end_datetime, zone);
        *sleep.entry(day).or_insert(0.0) += hours_asleep(&session);
        if let Some(value) = session.avg_hrv {
            night_hrv.entry(day).or_insert(value);
        }
        if let Some(value) = session.min_heart_rate {
            night_min_hr.entry(day).or_insert_with(|| f64::from(value));
        }
    }
    let mut morning_hrv: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    let mut morning_resting_hr: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for morning in merge_recovery_metrics(mornings) {
        let reading = morning.record;
        if let Some(value) = reading.hrv_rmssd.or(reading.hrv_ms) {
            morning_hrv.insert(reading.date, value);
        }
        if let Some(value) = reading.resting_heart_rate {
            morning_resting_hr.insert(reading.date, f64::from(value));
        }
    }
    let metric = |daily: BTreeMap<NaiveDate, f64>| {
        NightlyMetric::from_daily(&daily.into_iter().collect::<Vec<_>>(), today)
    };
    RecoveryReadings {
        hrv_ms: metric(preferring(&morning_hrv, &night_hrv)),
        hrv_ms_by_night: metric(preferring(&night_hrv, &morning_hrv)),
        resting_hr_bpm: metric(preferring(&morning_resting_hr, &night_min_hr)),
        resting_hr_bpm_by_night: metric(preferring(&night_min_hr, &morning_resting_hr)),
        sleep_hours: metric(sleep),
    }
}

/// One value per day from either series, `first`'s on a day both carry.
fn preferring(
    first: &BTreeMap<NaiveDate, f64>,
    second: &BTreeMap<NaiveDate, f64>,
) -> BTreeMap<NaiveDate, f64> {
    let mut merged = second.clone();
    merged.extend(first);
    merged
}

/// Time asleep in a session, in hours: the source's total, or the session's
/// span when the source reports none.
fn hours_asleep(session: &StoredSleepSession) -> f64 {
    let seconds = session.total_sleep_seconds.map_or_else(
        || {
            (session.end_datetime - session.start_datetime)
                .num_seconds()
                .max(0) as f64
        },
        f64::from,
    );
    seconds / SECONDS_PER_HOUR_F64
}

/// The athlete's form on `today`, their civil day: CTL minus ATL at the end of
/// yesterday, so a session today does not move it, read as the share of
/// yesterday's CTL the tools render.
///
/// The reading is the one a daily row hands every other surface
/// ([`DailyTrainingState::form_reading`]), so a reply is checked against the
/// CTL, share and band the athlete was shown. A series that stops short of
/// `today` yields no reading rather than an older day passed off as now.
///
/// `None` when cageux refuses the configured training-load parameters; the
/// refusal is logged and the snapshot carries no form rather than a made-up one.
fn form_today(
    activities: &[Activity],
    profile: Option<&UserPhysiologicalProfile>,
    algorithm_config: &AlgorithmConfig,
    today: NaiveDate,
    user_timezone: Option<&str>,
) -> Option<FormReading> {
    let inputs = AthleteInputs::from_profile(profile);
    // The window's end is the athlete's civil day, because the rollup
    // buckets each activity on theirs. Bounding it with the server's put
    // the current local day past `to` for every zone ahead of UTC, and the
    // series simply lost it (registre#260).
    match compute_training_history(
        activities,
        inputs,
        today - Duration::days(TSB_COMPUTE_DAYS),
        today,
        algorithm_config,
        user_timezone,
    ) {
        Ok(states) => states
            .last()
            .filter(|state| state.date == today)
            .map(DailyTrainingState::form_reading),
        Err(e) => {
            tracing::warn!(error = %e, "personalized verification: training-load series refused");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use chrono::TimeZone;
    use pierre_core::ai_policy::{AiUse, SourcePolicy};
    use pierre_core::models::{DataSource, DeviceType};
    use pierre_core::transport::TransportPolicy;
    use pierre_providers::ai_scope::ai_read;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

    fn night(
        source: &str,
        start: DateTime<Utc>,
        hours: u32,
        hrv: Option<f64>,
    ) -> StoredSleepSession {
        StoredSleepSession {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            data_source_id: format!("{source}-device"),
            is_nap: false,
            start_datetime: start,
            end_datetime: start + Duration::hours(i64::from(hours)),
            total_sleep_seconds: Some(hours * 3600),
            deep_sleep_seconds: None,
            light_sleep_seconds: None,
            rem_sleep_seconds: None,
            awake_seconds: None,
            sleep_efficiency: None,
            avg_heart_rate: None,
            min_heart_rate: Some(45),
            avg_hrv: hrv,
            sleep_score: None,
            stages: Vec::new(),
            source_name: source.to_owned(),
        }
    }

    fn morning(source: &str, date: NaiveDate, hrv: f64, resting_hr: u32) -> StoredRecoveryMetrics {
        StoredRecoveryMetrics {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            data_source_id: format!("{source}-device"),
            date,
            recovery_score: None,
            readiness_score: None,
            hrv_ms: None,
            hrv_rmssd: Some(hrv),
            resting_heart_rate: Some(resting_hr),
            stress_score: None,
            body_battery: None,
            spo2: None,
            respiratory_rate: None,
            skin_temp_deviation: None,
            daily_strain: None,
            athlete_note: None,
            source_name: source.to_owned(),
            recorded_at: date.and_hms_opt(8, 0, 0).unwrap().and_utc(),
        }
    }

    /// 23:00 UTC on the evening before `day`.
    fn bedtime(day: NaiveDate) -> DateTime<Utc> {
        (day - Duration::days(1))
            .and_hms_opt(23, 0, 0)
            .unwrap()
            .and_utc()
    }

    #[test]
    fn a_night_two_devices_recorded_counts_once_on_the_day_it_ended() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let mut nights: Vec<StoredSleepSession> = (1..=4)
            .map(|back| night("garmin", bedtime(today - Duration::days(back)), 8, None))
            .collect();
        // Last night, from a strap and a watch at once: one night, not 14 h.
        nights.push(night("whoop", bedtime(today), 7, None));
        nights.push(night("garmin", bedtime(today), 7, Some(41.0)));
        // An afternoon nap is not a night.
        let mut nap = night(
            "garmin",
            today.and_hms_opt(14, 0, 0).unwrap().and_utc(),
            1,
            None,
        );
        nap.is_nap = true;
        nights.push(nap);
        let mornings = vec![
            morning("garmin", today - Duration::days(2), 60.0, 47),
            morning("garmin", today - Duration::days(1), 58.0, 48),
        ];

        let readings = recovery_readings(nights, mornings, Tz::UTC, today);

        let sleep = readings.sleep_hours.unwrap();
        assert_eq!(sleep.last, Some(7.0));
        assert_eq!(sleep.prior_mean, Some(8.0));
        // Today has no morning reading, so the night's own HRV stands for it.
        let hrv = readings.hrv_ms.unwrap();
        assert_eq!(hrv.last, Some(41.0));
        assert_eq!(hrv.week_mean, Some(53.0), "two mornings and last night");
        assert_eq!(hrv.prior_mean, None, "two mornings make no baseline");
        // No night but last night's carries its own HRV, so read night-first
        // the days are the same.
        assert_eq!(readings.hrv_ms_by_night, Some(hrv));
        // Resting HR: the night's lowest heart rate fills days with no morning.
        let resting_hr = readings.resting_hr_bpm.unwrap();
        assert_eq!(resting_hr.last, Some(45.0));
        assert_eq!(
            resting_hr.prior_mean,
            Some(f64::from(45 + 45 + 47 + 48) / 4.0)
        );
        // Read night-first, as the sleep tools quote it, every night's own
        // lowest heart rate (45) stands over the mornings' 47 and 48.
        let by_night = readings.resting_hr_bpm_by_night.unwrap();
        assert_eq!(by_night.last, Some(45.0));
        assert_eq!(by_night.prior_mean, Some(45.0));
    }

    #[test]
    fn a_night_belongs_to_the_day_it_ended_on_in_the_athletes_zone() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let nights = || {
            vec![
                // Ends 03:00 UTC on the 9th: 23:00 on the 8th in Toronto.
                night(
                    "garmin",
                    Utc.with_ymd_and_hms(2026, 10, 8, 21, 0, 0).unwrap(),
                    6,
                    None,
                ),
                // Ends 11:00 UTC on the 9th: 07:00 on the 9th in Toronto.
                night(
                    "garmin",
                    Utc.with_ymd_and_hms(2026, 10, 9, 4, 0, 0).unwrap(),
                    7,
                    None,
                ),
            ]
        };
        let toronto = recovery_readings(nights(), Vec::new(), Tz::America__Toronto, today);
        assert_eq!(
            toronto.sleep_hours.unwrap().last,
            Some(7.0),
            "two nights, two days"
        );
        // Bucketed on UTC days both land on the 9th, as one 13-hour day.
        let utc = recovery_readings(nights(), Vec::new(), Tz::UTC, today);
        assert_eq!(utc.sleep_hours.unwrap().last, Some(13.0));
    }

    /// WHOOP's data barred from models, Garmin's allowed. No shipped provider
    /// bars measurements today (WHOOP's descriptor redacts only its scores);
    /// this proves the snapshot reads through the per-source gate rather than
    /// around it.
    struct Terms;

    const NEVER_AI: SourcePolicy = SourcePolicy {
        direct: AiUse::Deny,
        by_source: &[],
        other_sources: AiUse::Deny,
    };

    impl ProviderTerms for Terms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "whoop" => Some(&NEVER_AI),
                "garmin" => Some(&SourcePolicy::ALLOW_ALL),
                _ => None,
            }
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            matches!(provider, "whoop" | "garmin").then_some(TransportPolicy::AnyTransport)
        }
    }

    /// An athlete whose Garmin watch and WHOOP strap both synced the last
    /// seven nights and mornings: Garmin 7 h a night and HRV 55, then 6 h and
    /// 50 last night; WHOOP 9 h and HRV 80 every night.
    async fn athlete_with_two_sources() -> (Arc<RepositoryRegistry>, TenantId, Uuid) {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let user = create_test_user(&format!("nights-{}@example.com", Uuid::new_v4()), None);
        let user_id = repos.users.create(&user).await.unwrap();
        let tenant = TenantId::generate();
        let today = Utc::now().date_naive();
        for provider in ["garmin", "whoop"] {
            let data_source_id = repos
                .data_sources
                .upsert_data_source(
                    &tenant,
                    &DataSource {
                        id: String::new(),
                        user_id: user_id.to_string(),
                        provider: provider.to_owned(),
                        device_model: None,
                        software_version: None,
                        source: None,
                        device_type: DeviceType::Watch,
                        original_source_name: None,
                    },
                )
                .await
                .unwrap();
            for back in 0..7 {
                let day = today - Duration::days(back);
                let (hours, hrv) = match (provider, back) {
                    ("whoop", _) => (9, 80.0),
                    ("garmin", 0) => (6, 50.0),
                    _ => (7, 55.0),
                };
                let mut session = night(provider, bedtime(day), hours, None);
                session.user_id = user_id.to_string();
                session.data_source_id.clone_from(&data_source_id);
                repos
                    .sleep
                    .upsert_sleep_session(&tenant, &session)
                    .await
                    .unwrap();
                let mut reading = morning(provider, day, hrv, 48);
                reading.user_id = user_id.to_string();
                reading.data_source_id.clone_from(&data_source_id);
                repos
                    .recovery
                    .upsert_recovery_metrics(&tenant, &reading)
                    .await
                    .unwrap();
            }
        }
        (repos, tenant, user_id)
    }

    async fn snapshot(
        repos: &RepositoryRegistry,
        tenant: TenantId,
        user_id: Uuid,
    ) -> AthleteMetrics {
        build_athlete_metrics(
            repos,
            &Terms,
            &AlgorithmConfig::default(),
            &TrainingZonesConfig::default(),
            tenant,
            user_id,
        )
        .await
    }

    #[tokio::test]
    async fn the_snapshot_reads_the_synced_nights_a_model_may_see() {
        let (repos, tenant, user_id) = athlete_with_two_sources().await;

        let (metrics, withheld) = ai_read(snapshot(&repos, tenant, user_id)).await;

        assert!(
            metrics.is_usable(),
            "overnight readings alone make the layer usable"
        );
        assert_eq!(metrics.data_days, 0, "no activity history was needed");
        let sleep = metrics.recovery.sleep_hours.unwrap();
        assert_eq!(sleep.last, Some(6.0), "Garmin's night, not WHOOP's 9 h");
        assert_eq!(sleep.prior_mean, Some(7.0));
        let hrv = metrics.recovery.hrv_ms.unwrap();
        assert_eq!(hrv.last, Some(50.0));
        assert_eq!(hrv.prior_mean, Some(55.0));
        assert_eq!(metrics.recovery.resting_hr_bpm.unwrap().last, Some(48.0));
        // The sleep tools' reading of the same night: its lowest heart rate.
        assert_eq!(
            metrics.recovery.resting_hr_bpm_by_night.unwrap().last,
            Some(45.0)
        );
        assert_eq!(
            withheld.dropped, 14,
            "WHOOP's seven nights and seven mornings"
        );
    }

    #[tokio::test]
    async fn outside_a_model_read_every_source_merges() {
        let (repos, tenant, user_id) = athlete_with_two_sources().await;

        let metrics = snapshot(&repos, tenant, user_id).await;

        // WHOOP ranks first for sleep and recovery, so its values lead the merge.
        assert_eq!(metrics.recovery.sleep_hours.unwrap().last, Some(9.0));
        assert_eq!(metrics.recovery.hrv_ms.unwrap().last, Some(80.0));
    }
}
