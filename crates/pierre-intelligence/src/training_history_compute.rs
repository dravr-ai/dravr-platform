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
//! - `ctl` / `atl` / `tsb` / `daily_load` — Coggan TSS-based EMA
//!   (42-day chronic, 7-day acute). Same exponential filter cageux's
//!   `TrainingLoadCalculator` exposes; we replicate the math here so the
//!   per-day rollup is self-contained.
//! - `acwr` — Gabbett 7d / 28d ratio. `None` until 28+ days of data.
//! - `monotony` — Foster (mean weekly daily-load / std dev). `None` until
//!   7+ days of non-zero load.
//! - `strain` — Foster (weekly sum × monotony). `None` when monotony is `None`.
//! - `ramp_rate` — `CTL_today - CTL_7_days_ago`. `None` until 7+ days of CTL.
//!
//! Missing inputs propagate as `None` per the Endurance
//! deterministic-output rule: never substitute zero for "insufficient
//! history".

use chrono::{Duration, NaiveDate};
use dravr_cageux::metrics::MetricsCalculator;
use dravr_cageux::models::activity::Activity;
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
/// `activities` may extend before `from` — this is desirable because the
/// 42-day CTL window needs warm-up data to converge. Activities outside
/// `[from - 60d, to]` are dropped to bound compute.
///
/// The output has one row per calendar day in `[from, to]` even when
/// `daily_load == 0` (rest day). Days with insufficient history for a
/// derived metric leave that metric as `None`.
#[must_use]
pub fn compute_training_history(
    activities: &[Activity],
    inputs: AthleteInputs,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<DailyTrainingState> {
    if to < from {
        return Vec::new();
    }
    let span_days = (to - from).num_days();
    if span_days > MAX_BACKFILL_DAYS {
        // Defensive: caller should clamp, but never grow unboundedly.
        return Vec::new();
    }

    // Anchor the warm-up window so CTL/ATL EMAs converge before `from`.
    let warmup = from - Duration::days(CTL_WINDOW_DAYS + 30);
    let activity_dates: Vec<(NaiveDate, f64)> = activities
        .iter()
        .filter(|a| {
            let d = a.start_date().date_naive();
            d >= warmup && d <= to
        })
        .map(|a| (a.start_date().date_naive(), tss_for(a, inputs)))
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

    let ctl_alpha = ema_alpha(CTL_WINDOW_DAYS);
    let atl_alpha = ema_alpha(ATL_WINDOW_DAYS);
    let mut ctl_series: Vec<(NaiveDate, f64)> = Vec::with_capacity(daily_load.len());
    let mut atl_series: Vec<(NaiveDate, f64)> = Vec::with_capacity(daily_load.len());
    let mut ctl_prev = 0.0_f64;
    let mut atl_prev = 0.0_f64;
    for (date, load) in &daily_load {
        let ctl = ema_step(ctl_prev, *load, ctl_alpha);
        let atl = ema_step(atl_prev, *load, atl_alpha);
        ctl_series.push((*date, ctl));
        atl_series.push((*date, atl));
        ctl_prev = ctl;
        atl_prev = atl;
    }

    // Track the index of the first day with non-zero load so derived
    // metrics (ACWR, monotony, strain, ramp_rate) only fire once we have
    // enough *real* history. Warm-up zeros must not be counted.
    let first_load_idx = daily_load.iter().position(|(_, l)| *l > f64::EPSILON);

    let span_usize = usize::try_from(span_days).unwrap_or(0).saturating_add(1);
    let mut out: Vec<DailyTrainingState> = Vec::with_capacity(span_usize);
    for (idx, (date, load)) in daily_load.iter().enumerate() {
        if *date < from {
            continue;
        }
        let ctl = ctl_series[idx].1;
        let atl = atl_series[idx].1;
        let days_of_history =
            first_load_idx.map_or(0, |first| idx.saturating_sub(first).saturating_add(1));
        let acwr = compute_acwr(&daily_load, idx, days_of_history);
        let (monotony, strain) = compute_monotony_strain(&daily_load, idx, days_of_history);
        let ramp_rate = compute_ramp_rate(&ctl_series, idx, days_of_history);
        out.push(DailyTrainingState {
            date: *date,
            ctl,
            atl,
            tsb: ctl - atl,
            acwr,
            monotony,
            strain,
            ramp_rate,
            daily_load: *load,
        });
    }
    out
}

fn tss_for(activity: &Activity, inputs: AthleteInputs) -> f64 {
    let calc = MetricsCalculator::new().with_user_data(
        inputs.ftp_watts,
        inputs.lthr,
        inputs.max_hr,
        inputs.resting_hr,
        inputs.weight_kg,
    );
    calc.calculate_metrics(activity)
        .map(|m| m.training_stress_score.unwrap_or(0.0))
        .unwrap_or(0.0)
}

#[allow(clippy::cast_precision_loss)]
fn ema_alpha(window_days: i64) -> f64 {
    // Coggan's exponentially-weighted average uses 2 / (N + 1).
    2.0 / (window_days as f64 + 1.0)
}

fn ema_step(previous: f64, todays_load: f64, alpha: f64) -> f64 {
    todays_load.mul_add(alpha, previous * (1.0 - alpha))
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
    ctl_series: &[(NaiveDate, f64)],
    idx: usize,
    days_of_history: usize,
) -> Option<f64> {
    let lookback = usize::try_from(RAMP_RATE_LOOKBACK_DAYS).unwrap_or(7);
    if days_of_history < lookback || idx < lookback {
        return None;
    }
    let today = ctl_series[idx].1;
    let prior = ctl_series[idx - lookback].1;
    Some(today - prior)
}
