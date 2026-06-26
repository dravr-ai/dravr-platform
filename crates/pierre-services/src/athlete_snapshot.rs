// ABOUTME: Builds an AthleteMetrics snapshot (personalized-layer input) from a user's stored physiology + recent activities
// ABOUTME: Reads the physiological profile + activity cache, runs cageux pace/zone math + the training-history TSB compute
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
//! Two reads (physiology profile + recent activities) plus pure cageux compute:
//!
//! - The `UserPhysiologicalProfile` supplies VO2max/VDOT, max HR, and FTP, and
//!   feeds a [`VO2MaxCalculator`] for the Daniels training-pace ranges.
//! - The activity cache supplies `data_days` (the stale-estimate guard) and,
//!   via [`compute_training_history`], the latest training-stress balance.
//!
//! Any missing input degrades to `None` for that metric; a profile-less or
//! history-thin user yields a snapshot whose [`AthleteMetrics::is_usable`] is
//! false, so the personalized layer stays silent.

use chrono::{Duration, Utc};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_evals::AthleteMetrics;
use pierre_fitness_compute::{compute_training_history, AthleteInputs};
use pierre_intelligence::config::intelligence::VO2MaxCalculator;
use pierre_intelligence::AlgorithmConfig;
use uuid::Uuid;

/// Lookback window for the activity read backing `data_days` and the TSB compute.
const ACTIVITY_LOOKBACK_DAYS: i64 = 180;
/// Compute window for the training-history TSB series (CTL warm-up is supplied
/// by the wider activity read above).
const TSB_COMPUTE_DAYS: i64 = 90;
/// Cap on activities loaded for the snapshot.
const ACTIVITY_READ_LIMIT: i64 = 500;
/// Resting-HR anchor for the pace math when the profile omits it.
const DEFAULT_RESTING_HR: u16 = 60;
/// Max-HR anchor for the pace math when the profile omits it.
const DEFAULT_MAX_HR: u16 = 190;
/// Lactate-threshold fraction when the profile omits it (Daniels default ~0.85).
const DEFAULT_LACTATE_THRESHOLD: f64 = 0.85;
/// Sport-efficiency factor (1.0 = running baseline).
const DEFAULT_SPORT_EFFICIENCY: f64 = 1.0;

/// Assemble the [`AthleteMetrics`] snapshot for one athlete.
///
/// Best-effort: a failed read logs and leaves the affected metrics `None`,
/// never erroring the turn.
pub async fn build_athlete_metrics(
    repos: &RepositoryRegistry,
    algorithm_config: &AlgorithmConfig,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AthleteMetrics {
    let mut metrics = AthleteMetrics::default();

    let profile = match repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await
    {
        Ok(p) => p,
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

        if let Some(vo2) = p.vo2_max {
            let calculator = VO2MaxCalculator::new(
                vo2,
                p.resting_hr.unwrap_or(DEFAULT_RESTING_HR),
                p.max_hr.unwrap_or(DEFAULT_MAX_HR),
                p.lactate_threshold_percentage
                    .unwrap_or(DEFAULT_LACTATE_THRESHOLD),
                DEFAULT_SPORT_EFFICIENCY,
            );
            let paces = calculator.calculate_pace_zones();
            metrics.easy_pace_range = Some(paces.easy_pace_range);
            metrics.marathon_pace_range = Some(paces.marathon_pace_range);
            metrics.threshold_pace_range = Some(paces.threshold_pace_range);
            metrics.interval_pace_range = Some(paces.vo2max_pace_range);
        }
    }

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

    // data_days = distinct calendar days with an activity — the density signal
    // gating whether the personalized layer trusts the snapshot enough to contradict a coach.
    let mut activity_days: Vec<_> = activities
        .iter()
        .map(|a| a.start_date().date_naive())
        .collect();
    activity_days.sort_unstable();
    activity_days.dedup();
    metrics.data_days = u32::try_from(activity_days.len()).unwrap_or(u32::MAX);

    if !activities.is_empty() {
        let inputs = AthleteInputs {
            ftp_watts: profile.as_ref().and_then(|p| p.ftp_watts.map(f64::from)),
            lthr: profile.as_ref().and_then(|p| {
                p.lactate_threshold_percentage
                    .zip(p.max_hr)
                    .map(|(pct, mhr)| f64::from(mhr) * pct)
            }),
            max_hr: profile.as_ref().and_then(|p| p.max_hr.map(f64::from)),
            resting_hr: profile.as_ref().and_then(|p| p.resting_hr.map(f64::from)),
            weight_kg: profile.as_ref().and_then(|p| p.weight),
        };
        let today = now.date_naive();
        let states = compute_training_history(
            &activities,
            inputs,
            today - Duration::days(TSB_COMPUTE_DAYS),
            today,
            algorithm_config,
        );
        metrics.recent_tsb = states.last().map(|s| s.tsb);
    }

    metrics
}
