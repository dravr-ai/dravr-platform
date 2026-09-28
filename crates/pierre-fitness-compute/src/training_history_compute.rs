// ABOUTME: Endurance Phase 2 daily training-history compute — CTL/ATL/TSB + ACWR/monotony/strain/ramp_rate from activity stream
// ABOUTME: Pure computation, no IO; produces DailyTrainingState rows the TrainingHistoryRepository persists
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Endurance training-history compute
//!
//! [`compute_training_history`] turns a slice of activities + per-user
//! physiology into one [`DailyTrainingState`] per day in the requested
//! window.
//!
//! Frameworks:
//!
//! - `ctl` / `atl` / `tsb` / `form_ctl` / `daily_load` — the configured
//!   training-load smoothing (EMA/SMA/WMA/Kalman, default EMA 42-day chronic /
//!   7-day acute) read off cageux's per-day series,
//!   [`TrainingLoadAlgorithm::daily_series`], over each civil day's summed
//!   TSS; per-activity TSS honors the configured TSS algorithm. It is the same
//!   series `TrainingLoadCalculator` reads its as-of day from, so a history row
//!   and the current training load cannot disagree. `ctl` and `atl` are the
//!   day's end-of-day values; `tsb` and `form_ctl` come from the end of the day
//!   before (the Coggan/TrainingPeaks form convention cageux applies).
//! - `acwr` — Gabbett 7d / 28d ratio. `None` until 28+ days of data.
//! - `monotony` — Foster (mean weekly daily-load / std dev). `None` until
//!   7+ days of non-zero load.
//! - `strain` — Foster (weekly sum × monotony). `None` when monotony is `None`.
//! - `ramp_rate` — `CTL_today - CTL_7_days_ago`. `None` until 7+ days of CTL.
//!
//! Missing inputs propagate as `None` per the Endurance
//! deterministic-output rule: never substitute zero for "insufficient
//! history".

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate};
use dravr_cageux::algorithms::training_load::{DailyTrainingLoad, TrainingLoadAlgorithm};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::error::IntelligenceResult;
use dravr_cageux::metrics::MetricsCalculator;
use dravr_cageux::models::activity::Activity;
use pierre_core::civil_time::{local_date, resolve_zone};
use pierre_core::models::DailyTrainingState;

/// Default chronic window (Coggan).
pub const CTL_WINDOW_DAYS: i64 = 42;
/// Default acute window (Coggan).
pub const ATL_WINDOW_DAYS: i64 = 7;
/// ACWR acute window (Gabbett).
pub const ACWR_ACUTE_DAYS: i64 = 7;
/// ACWR chronic window (Gabbett).
pub const ACWR_CHRONIC_DAYS: i64 = 28;
/// Monotony / strain weekly window (Foster).
pub const FOSTER_WINDOW_DAYS: i64 = 7;
/// Ramp-rate lookback in days.
pub const RAMP_RATE_LOOKBACK_DAYS: i64 = 7;
/// Maximum backfill window in days — bounded to keep compute cost predictable.
pub const MAX_BACKFILL_DAYS: i64 = 365;

/// Extra days of activity history the CTL EMA needs on top of its own window
/// before the series it produces can be stood behind.
///
/// `ctl_prev` seeds at `0.0`, so a day computed with less than this behind it
/// carries a chronic load that is wrong low — and `ctl`/`atl`/`tsb` are plain
/// `f64` with no `None` arm to say so. Callers that source activities from a
/// bounded store must check they actually hold this depth before `from`.
pub const CTL_WARMUP_MARGIN_DAYS: i64 = 30;

/// Days of history that must precede `from` for that day's CTL/ATL/TSB to be
/// honest, given the configured chronic window.
#[must_use]
pub const fn warmup_days(ctl_window_days: i64) -> i64 {
    ctl_window_days + CTL_WARMUP_MARGIN_DAYS
}

/// Per-user physiology inputs for TSS computation.
#[derive(Debug, Clone, Copy, Default)]
pub struct AthleteInputs {
    /// Functional Threshold Power (watts) for power-based TSS.
    pub ftp_watts: Option<f64>,
    /// Lactate Threshold Heart Rate (bpm) for HR-based TSS (Banister TRIMP).
    pub lthr: Option<f64>,
    /// Maximum heart rate (bpm).
    pub max_hr: Option<f64>,
    /// Resting heart rate (bpm).
    pub resting_hr: Option<f64>,
    /// Body weight (kg).
    pub weight_kg: Option<f64>,
}

/// Build a dense series of [`DailyTrainingState`] rows for `[from, to]`.
///
/// `activities` may extend before `from` — this is required, not merely
/// desirable, because the CTL window needs warm-up data to converge. Activities
/// outside `[from - warmup_days(ctl_window_days), to]` are dropped to bound
/// compute; at the default 42-day chronic window that lower bound is 72 days
/// before `from`. A caller supplying less than that gets a series warmed from a
/// zero seed, which reads as a real chronic load and is wrong low — see
/// [`warmup_days`].
///
/// The output has one row per calendar day in `[from, to]` even when
/// `daily_load == 0` (rest day). Days with insufficient history for a
/// derived metric leave that metric as `None`.
///
/// # Errors
///
/// Returns the `IntelligenceError` cageux raises when the configured
/// training-load algorithm refuses its parameters (a CTL/ATL window outside
/// 1 to 365 days, non-positive Kalman noise).
pub fn compute_training_history(
    activities: &[Activity],
    inputs: AthleteInputs,
    from: NaiveDate,
    to: NaiveDate,
    algorithm_config: &AlgorithmConfig,
    user_timezone: Option<&str>,
) -> IntelligenceResult<Vec<DailyTrainingState>> {
    if to < from {
        return Ok(Vec::new());
    }
    let span_days = (to - from).num_days();
    if span_days > MAX_BACKFILL_DAYS {
        // Defensive: caller should clamp, but never grow unboundedly.
        return Ok(Vec::new());
    }

    // Anchor the warm-up window on the configured chronic window so CTL/ATL
    // converge before `from`.
    let warmup = from - Duration::days(warmup_days(algorithm_config.params.training_load_ctl_days));
    // Bucket on the athlete's civil day, not the server's. A 21:00
    // America/Toronto session lands on the next UTC date, which shifted the
    // whole per-day series one day against the athlete's own calendar and made
    // every "what did I do Tuesday" answer disagree with them (registre#200).
    let zone = resolve_zone(user_timezone);
    let activity_dates: Vec<(NaiveDate, f64)> = activities
        .iter()
        .filter(|a| {
            let d = local_date(a.start_date(), zone);
            d >= warmup && d <= to
        })
        .map(|a| {
            (
                local_date(a.start_date(), zone),
                tss_for(a, inputs, algorithm_config),
            )
        })
        .collect();

    // Build daily load map across [warmup, to].
    let total_days_with_warmup = (to - warmup).num_days();
    let cap = usize::try_from(total_days_with_warmup)
        .unwrap_or(0)
        .saturating_add(1);
    let mut daily_load: Vec<(NaiveDate, f64)> = Vec::with_capacity(cap);
    let mut cursor = warmup;
    while cursor <= to {
        let load: f64 = activity_dates
            .iter()
            .filter(|(d, _)| *d == cursor)
            .map(|(_, t)| *t)
            .sum();
        daily_load.push((cursor, load));
        cursor += Duration::days(1);
    }

    let load = load_series(algorithm_config, &daily_load, from, to)?;

    // Track the index of the first day with non-zero load so derived
    // metrics (ACWR, monotony, strain, ramp_rate) only fire once we have
    // enough *real* history. Warm-up zeros must not be counted.
    let first_load_idx = daily_load.iter().position(|(_, l)| *l > f64::EPSILON);

    let span_usize = usize::try_from(span_days).unwrap_or(0).saturating_add(1);
    let mut out: Vec<DailyTrainingState> = Vec::with_capacity(span_usize);
    for (idx, (date, day_load)) in daily_load.iter().enumerate() {
        if *date < from {
            continue;
        }
        // `load` starts RAMP_RATE_LOOKBACK_DAYS before `from`.
        let series_idx = usize::try_from((*date - from).num_days() + RAMP_RATE_LOOKBACK_DAYS)
            .unwrap_or(usize::MAX);
        let Some(today) = load.get(series_idx) else {
            continue;
        };
        let days_of_history =
            first_load_idx.map_or(0, |first| idx.saturating_sub(first).saturating_add(1));
        let acwr = compute_acwr(&daily_load, idx, days_of_history);
        let (monotony, strain) = compute_monotony_strain(&daily_load, idx, days_of_history);
        let ramp_rate = compute_ramp_rate(&load, series_idx, days_of_history);
        out.push(DailyTrainingState {
            date: *date,
            ctl: today.ctl,
            atl: today.atl,
            tsb: today.tsb,
            form_ctl: today.form_ctl,
            acwr,
            monotony,
            strain,
            ramp_rate,
            daily_load: *day_load,
        });
    }
    Ok(out)
}

/// CTL/ATL/TSB for every day from `RAMP_RATE_LOOKBACK_DAYS` before `from`
/// through `to`, smoothed by the configured training-load algorithm.
///
/// Days before `from` in `daily_load` warm the smoothing up. The lookback days
/// are asked for separately from `[from, to]` so the ramp rate of `from`
/// itself has a prior value without widening one call past cageux's 365-day
/// series cap, which `[from, to]` alone can already reach.
fn load_series(
    algorithm_config: &AlgorithmConfig,
    daily_load: &[(NaiveDate, f64)],
    from: NaiveDate,
    to: NaiveDate,
) -> IntelligenceResult<Vec<DailyTrainingLoad>> {
    let algorithm: TrainingLoadAlgorithm = algorithm_config.training_load_algorithm();
    let daily_tss: BTreeMap<NaiveDate, f64> = daily_load
        .iter()
        .filter(|(_, load)| *load > 0.0)
        .copied()
        .collect();
    let lookback_from = from - Duration::days(RAMP_RATE_LOOKBACK_DAYS);
    let mut series = algorithm.daily_series(&daily_tss, lookback_from, from - Duration::days(1))?;
    series.extend(algorithm.daily_series(&daily_tss, from, to)?);
    Ok(series)
}

fn tss_for(activity: &Activity, inputs: AthleteInputs, algorithm_config: &AlgorithmConfig) -> f64 {
    let calc = MetricsCalculator::new()
        .with_user_data(
            inputs.ftp_watts,
            inputs.lthr,
            inputs.max_hr,
            inputs.resting_hr,
            inputs.weight_kg,
        )
        .with_algorithm_config(algorithm_config.clone());
    calc.calculate_metrics(activity)
        .map_or(0.0, |m| m.training_stress_score.unwrap_or(0.0))
}

#[allow(clippy::cast_precision_loss)]
fn compute_acwr(
    daily_load: &[(NaiveDate, f64)],
    idx: usize,
    days_of_history: usize,
) -> Option<f64> {
    let acute_days = usize::try_from(ACWR_ACUTE_DAYS).unwrap_or(7);
    let chronic_days = usize::try_from(ACWR_CHRONIC_DAYS).unwrap_or(28);
    if days_of_history < chronic_days || idx + 1 < chronic_days {
        return None;
    }
    let acute_start = idx + 1 - acute_days;
    let chronic_start = idx + 1 - chronic_days;
    let acute_sum: f64 = daily_load[acute_start..=idx].iter().map(|(_, l)| *l).sum();
    let chronic_sum: f64 = daily_load[chronic_start..=idx]
        .iter()
        .map(|(_, l)| *l)
        .sum();
    let acute_avg = acute_sum / ACWR_ACUTE_DAYS as f64;
    let chronic_avg = chronic_sum / ACWR_CHRONIC_DAYS as f64;
    if chronic_avg <= f64::EPSILON {
        None
    } else {
        Some(acute_avg / chronic_avg)
    }
}

#[allow(clippy::cast_precision_loss)]
fn compute_monotony_strain(
    daily_load: &[(NaiveDate, f64)],
    idx: usize,
    days_of_history: usize,
) -> (Option<f64>, Option<f64>) {
    let window = usize::try_from(FOSTER_WINDOW_DAYS).unwrap_or(7);
    if days_of_history < window || idx + 1 < window {
        return (None, None);
    }
    let start = idx + 1 - window;
    let loads: Vec<f64> = daily_load[start..=idx].iter().map(|(_, l)| *l).collect();
    let weekly_sum: f64 = loads.iter().sum();
    if weekly_sum <= f64::EPSILON {
        return (None, None);
    }
    let mean = weekly_sum / loads.len() as f64;
    let variance = loads
        .iter()
        .map(|l| {
            let d = *l - mean;
            d * d
        })
        .sum::<f64>()
        / loads.len() as f64;
    let std_dev = variance.sqrt();
    if std_dev <= f64::EPSILON {
        return (None, None);
    }
    let monotony = mean / std_dev;
    let strain = weekly_sum * monotony;
    (Some(monotony), Some(strain))
}

fn compute_ramp_rate(
    load: &[DailyTrainingLoad],
    series_idx: usize,
    days_of_history: usize,
) -> Option<f64> {
    let lookback = usize::try_from(RAMP_RATE_LOOKBACK_DAYS).unwrap_or(7);
    if days_of_history < lookback {
        return None;
    }
    let today = load.get(series_idx)?.ctl;
    let prior = load.get(series_idx.checked_sub(lookback)?)?.ctl;
    Some(today - prior)
}
