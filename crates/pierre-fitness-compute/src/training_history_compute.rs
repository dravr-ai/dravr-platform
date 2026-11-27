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
//!   training-load smoothing (EMA/SMA/WMA, default EMA 42-day chronic /
//!   7-day acute) read off cageux's per-day series,
//!   [`TrainingLoadAlgorithm::daily_series`], over cageux's daily split of
//!   the athlete's civil days ([`TrainingLoadCalculator::daily_tss`]);
//!   per-activity TSS honors the configured TSS algorithm. It is the same
//!   split and series a `TrainingLoadCalculator` in the athlete's zone reads
//!   its as-of day from, so a history row, the roster and the current
//!   training load cannot disagree. `ctl` and `atl` are the
//!   day's end-of-day values; `tsb` and `form_ctl` come from the end of the day
//!   before (the Coggan/TrainingPeaks form convention cageux applies).
//! - `acwr` — Gabbett acute:chronic ratio. `None` until the chronic window
//!   of load history.
//! - `monotony` — Foster (mean weekly daily-load / std dev). `None` until a
//!   week of load history, and for a week without load or without spread.
//! - `strain` — Foster (weekly sum × monotony). `None` when monotony is `None`.
//! - `ramp_rate` — CTL today minus CTL a lookback ago. `None` until a
//!   lookback of load history.
//!
//! The four derived metrics and their windows are cageux's
//! ([`load_metrics`]): each day reads them off the load and CTL series
//! ending on it.
//!
//! Missing inputs propagate as `None` per the Endurance
//! deterministic-output rule: never substitute zero for "insufficient
//! history".

use std::collections::BTreeMap;
use std::iter;

use dravr_cageux::algorithms::training_load::{DailyTrainingLoad, TrainingLoadAlgorithm};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::constants::limits::MAX_TIMEFRAME_DAYS;
use dravr_cageux::error::IntelligenceResult;
use dravr_cageux::models::activity::Activity;
use dravr_cageux::periodization::load_metrics::{self, RAMP_RATE_LOOKBACK_DAYS};
use dravr_cageux::training_load::{TrainingLoad, TrainingLoadCalculator};
use dravr_tronc::dates::civil_clock::resolve_zone;
use dravr_tronc::dates::{CivilDate, Elapsed};
use pierre_core::models::{DailyTrainingState, UserPhysiologicalProfile};

/// Default chronic window (Coggan).
pub const CTL_WINDOW_DAYS: i64 = 42;
/// Default acute window (Coggan).
pub const ATL_WINDOW_DAYS: i64 = 7;
/// Maximum backfill window in days — bounded to keep compute cost predictable.
pub const MAX_BACKFILL_DAYS: i64 = 365;

/// Extra days of activity history the CTL EMA needs on top of its own window
/// before the series it produces can be stood behind.
///
/// cageux seeds each load at the history's mean daily TSS over its opening
/// window (42 days for CTL), so a steady athlete reads true from the first
/// day. What a short history still gets wrong is an athlete whose load before
/// the history began differed from its opening weeks: that gap survives as
/// `(prior - seed) x (41/42)^n` after `n` days, a third of it at 42 days and
/// a sixth at the 72 days [`warmup_days`] asks for. `ctl`/`atl`/`tsb` are
/// plain `f64` with no `None` arm to say so. Callers that source activities
/// from a bounded store must check they actually hold this depth before
/// `from`.
pub const CTL_WARMUP_MARGIN_DAYS: i64 = 30;

/// Days of history that must precede `from` for that day's CTL/ATL/TSB to be
/// honest, given the configured chronic window.
#[must_use]
pub const fn warmup_days(ctl_window_days: i64) -> i64 {
    ctl_window_days + CTL_WARMUP_MARGIN_DAYS
}

/// Per-user physiology inputs for TSS computation.
///
/// Every training-load reader — `get_training_history`, `analyze_training_load`,
/// the recovery tools, `calculate_fitness_score`, `generate_recommendations`,
/// `predict_performance`, the group snapshot and the athlete snapshot — builds
/// these with [`Self::from_profile`] from the athlete's stored physiological
/// profile, the one place their thresholds live, so two surfaces asked about
/// the same day score each session against the same numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AthleteInputs {
    /// Functional Threshold Power (watts) for power-based TSS.
    pub ftp_watts: Option<f64>,
    /// Lactate Threshold Heart Rate (bpm) for heart-rate TSS, read on the
    /// heart-rate reserve with [`Self::resting_hr`].
    pub lthr: Option<f64>,
    /// Maximum heart rate (bpm).
    pub max_hr: Option<f64>,
    /// Resting heart rate (bpm).
    pub resting_hr: Option<f64>,
    /// Body weight (kg).
    pub weight_kg: Option<f64>,
    /// Critical power (watts): a ride's FTP stand-in when the profile has no
    /// FTP and the provider recorded none.
    pub critical_power_watts: Option<f64>,
    /// Threshold pace (seconds per km): what a run's pace is scored against.
    /// Without it, a run is scored by its session RPE or not at all.
    pub threshold_pace_s_per_km: Option<f64>,
}

impl AthleteInputs {
    /// The inputs a stored profile gives, each absent where the profile is
    /// silent.
    ///
    /// The resting heart rate and weight are the profile's stored anchors:
    /// the athlete's own, or promoted weekly from their measurements when they
    /// set none — never a daily reading, which would rescore every stored day
    /// each morning.
    ///
    /// The LTHR is [`UserPhysiologicalProfile::lactate_threshold_hr`]: the
    /// measured `threshold_hr` first, else the estimate from the lactate
    /// threshold (a fraction of `VO2max`) and max HR. No profile gives
    /// [`Self::default`]: a session is then scored by what it carries alone
    /// (the provider's TSS, a run's session RPE, another session's duration),
    /// or left out.
    #[must_use]
    pub fn from_profile(profile: Option<&UserPhysiologicalProfile>) -> Self {
        profile.map_or_else(Self::default, |p| Self {
            ftp_watts: p.ftp_watts.map(f64::from),
            lthr: p.lactate_threshold_hr(),
            max_hr: p.max_hr.map(f64::from),
            resting_hr: p.resting_hr.map(f64::from),
            weight_kg: p.weight,
            critical_power_watts: p
                .critical_power_watts
                .as_ref()
                .map(|cp| f64::from(cp.value)),
            threshold_pace_s_per_km: p.threshold_pace_sec_per_km,
        })
    }

    /// `calculator` scoring against these thresholds' critical power and
    /// threshold pace, which cageux reads off the calculator rather than the
    /// per-call arguments.
    fn calibrated(&self, calculator: TrainingLoadCalculator) -> TrainingLoadCalculator {
        calculator
            .with_critical_power(self.critical_power_watts)
            .with_threshold_pace(self.threshold_pace_s_per_km)
    }

    /// CTL, ATL and TSB of `activities` by `calculator`, each session's stress
    /// scored against these thresholds.
    ///
    /// # Errors
    /// Returns the calculator's error when the configured training-load
    /// algorithm refuses its parameters.
    pub fn training_load(
        &self,
        calculator: TrainingLoadCalculator,
        activities: &[Activity],
    ) -> IntelligenceResult<TrainingLoad> {
        self.calibrated(calculator).calculate_training_load(
            activities,
            self.ftp_watts,
            self.lthr,
            self.max_hr,
            self.resting_hr,
            self.weight_kg,
        )
    }

    /// Each civil day's summed training stress of `activities`, the daily
    /// split cageux smooths: every session scored by `calculator` against
    /// these thresholds and counted on the day it started on in the
    /// calculator's zone ([`TrainingLoadCalculator::daily_tss`]). A session
    /// no estimator can score is left out, and a day whose sessions carry no
    /// stress is absent.
    #[must_use]
    pub fn daily_tss(
        &self,
        calculator: TrainingLoadCalculator,
        activities: &[Activity],
    ) -> BTreeMap<CivilDate, f64> {
        self.calibrated(calculator).daily_tss(
            activities,
            self.ftp_watts,
            self.lthr,
            self.max_hr,
            self.resting_hr,
            self.weight_kg,
        )
    }
}

/// Build a dense series of [`DailyTrainingState`] rows for `[from, to]`.
///
/// `activities` may extend before `from` — this is required, not merely
/// desirable, because the CTL window needs warm-up data to converge. Days of
/// the athlete's calendar outside `[from - warmup_days(ctl_window_days), to]`
/// do not enter the series; at the default 42-day chronic window that lower
/// bound is 72 days
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
/// 1 to 365 days).
pub fn compute_training_history(
    activities: &[Activity],
    inputs: AthleteInputs,
    from: CivilDate,
    to: CivilDate,
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
    let warmup = from - Elapsed::days(warmup_days(algorithm_config.params.training_load_ctl_days));
    // Bucket on the athlete's civil day, not the server's. A 21:00
    // America/Toronto session lands on the next UTC date, which shifted the
    // whole per-day series one day against the athlete's own calendar and made
    // every "what did I do Tuesday" answer disagree with them (registre#200).
    // cageux owns the split: scoring, the civil day, and which sessions carry
    // stress.
    let calculator = TrainingLoadCalculator::from_config(algorithm_config.clone(), to)
        .in_zone(resolve_zone(user_timezone));
    let algorithm = calculator.algorithm();
    let daily_tss: BTreeMap<CivilDate, f64> = inputs
        .daily_tss(calculator, activities)
        .range(warmup..=to)
        .map(|(day, tss)| (*day, *tss))
        .collect();

    // Every day across [warmup, to], a rest day at zero.
    let daily_load: Vec<(CivilDate, f64)> = iter::successors(Some(warmup), |day| day.succ())
        .take_while(|day| *day <= to)
        .map(|day| (day, daily_tss.get(&day).copied().unwrap_or(0.0)))
        .collect();

    let load = load_series(&algorithm, &daily_tss, from, to)?;

    // cageux counts the history a derived metric needs (ACWR, monotony,
    // strain, ramp_rate) from the first day carrying load, so the warm-up
    // zeros in front of it are not counted.
    let loads: Vec<f64> = daily_load.iter().map(|(_, l)| *l).collect();
    let ctl: Vec<f64> = load.iter().map(|day| day.ctl).collect();

    let span_usize = usize::try_from(span_days).unwrap_or(0).saturating_add(1);
    let mut out: Vec<DailyTrainingState> = Vec::with_capacity(span_usize);
    for (idx, (date, day_load)) in daily_load.iter().enumerate() {
        if *date < from {
            continue;
        }
        // `load` starts RAMP_RATE_LOOKBACK_DAYS before `from`.
        let series_idx =
            usize::try_from((*date - from).num_days() + i64::from(RAMP_RATE_LOOKBACK_DAYS))
                .unwrap_or(usize::MAX);
        let Some(today) = load.get(series_idx) else {
            continue;
        };
        let loads_to_date = &loads[..=idx];
        let foster = load_metrics::foster_week(loads_to_date);
        let ramp_rate = load_metrics::ramp_rate(&ctl[..=series_idx], loads_to_date);
        out.push(DailyTrainingState {
            date: *date,
            ctl: today.ctl,
            atl: today.atl,
            tsb: today.tsb,
            form_ctl: today.form_ctl,
            acwr: load_metrics::acwr(loads_to_date),
            monotony: foster.map(|week| week.monotony),
            strain: foster.map(|week| week.strain),
            ramp_rate,
            daily_load: *day_load,
        });
    }
    Ok(out)
}

/// CTL/ATL/TSB for every day from `RAMP_RATE_LOOKBACK_DAYS` before `from`
/// through `to`, smoothed by the configured training-load algorithm.
///
/// Days before `from` in `daily_tss` warm the smoothing up. The lookback days
/// are asked for separately from `[from, to]` so the ramp rate of `from`
/// itself has a prior value without widening one call past cageux's 365-day
/// series cap, which `[from, to]` alone can already reach.
///
/// Both calls must read one seed. cageux starts each load at the history's
/// mean daily TSS over its opening window, read through the call's as-of day,
/// so a lookback series ending the day before `from` would seed a history
/// shorter than 42 days there differently from `[from, to]`. The lookback
/// series runs toward `to` instead, as far as the cap allows, and keeps its
/// first days: through `to` it reads the same seed, and when the cap stops it
/// short the history spans more than a chronic window by then, where the
/// seed no longer moves.
fn load_series(
    algorithm: &TrainingLoadAlgorithm,
    daily_tss: &BTreeMap<CivilDate, f64>,
    from: CivilDate,
    to: CivilDate,
) -> IntelligenceResult<Vec<DailyTrainingLoad>> {
    let lookback_days = i64::from(RAMP_RATE_LOOKBACK_DAYS);
    let lookback_from = from - Elapsed::days(lookback_days);
    let lookback_as_of = to
        .min(lookback_from + Elapsed::days(i64::from(MAX_TIMEFRAME_DAYS)))
        .max(from - Elapsed::days(1));
    let mut series = algorithm.daily_series(daily_tss, lookback_from, lookback_as_of)?;
    series.truncate(usize::try_from(RAMP_RATE_LOOKBACK_DAYS).unwrap_or(usize::MAX));
    series.extend(algorithm.daily_series(daily_tss, from, to)?);
    Ok(series)
}
