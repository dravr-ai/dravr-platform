// ABOUTME: Endurance Phase 3 LT1/LT2 threshold estimators from HR / power / pace using cageux primitives
// ABOUTME: All estimators return Option — never substitute heuristics for missing physiology data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Threshold estimation (Endurance Phase 3)
//!
//! Lightweight estimators for the two ventilatory / lactate thresholds
//! coaches need:
//!
//! - **LT1** (aerobic threshold) — boundary between Zone 1 and Zone 2 on
//!   the polarized 3-zone model. We estimate it as 75 % of `lthr` when
//!   only LTHR is known, or via the Karvonen-style heart-rate reserve
//!   formula when both `max_hr` and `resting_hr` are supplied.
//! - **LT2** (lactate threshold / functional threshold heart rate) —
//!   boundary between Zone 2 and Zone 3. We surface the supplied LTHR
//!   directly; when only `max_hr` is known we fall back to
//!   `0.88 * max_hr` (Coggan rule of thumb for trained athletes).
//!
//! Pace and power thresholds are computed only when the matching
//! threshold input exists — `threshold_pace_sec_per_km` for running and
//! `ftp_watts` for cycling. All estimators return `None` when inputs are
//! missing per the Endurance deterministic-output rule.

use serde::{Deserialize, Serialize};

/// Coggan rule-of-thumb factor for LT2 / FTHR ≈ 0.88 × max HR.
const LT2_FRACTION_OF_MAX_HR: f64 = 0.88;

/// Default LT1 share of LT2 heart rate (Seiler / 3-zone model).
const LT1_FRACTION_OF_LT2: f64 = 0.85;

/// Per-user inputs needed by the threshold estimators. A subset of
/// [`pierre_core::models::UserPhysiologicalProfile`] expressed as `f64`
/// for arithmetic convenience.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThresholdInputs {
    /// Maximum heart rate in bpm.
    pub max_hr: Option<f64>,
    /// Resting heart rate in bpm.
    pub resting_hr: Option<f64>,
    /// Lactate threshold heart rate (LT2 in bpm).
    pub lthr: Option<f64>,
    /// Functional Threshold Power (LT2 in watts).
    pub ftp_watts: Option<f64>,
    /// Threshold pace in seconds per kilometre (running LT2).
    pub threshold_pace_sec_per_km: Option<f64>,
}

/// Per-modality LT1 / LT2 estimates.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ThresholdEstimate {
    /// Aerobic threshold heart rate in bpm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt1_hr: Option<f64>,
    /// Lactate threshold heart rate in bpm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt2_hr: Option<f64>,
    /// LT1 power (watts) — `0.75 * FTP` (Coggan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt1_power_watts: Option<f64>,
    /// LT2 power (watts) — defaults to FTP when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt2_power_watts: Option<f64>,
    /// LT1 pace (sec/km) — slower than LT2 pace by a Seiler-style factor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt1_pace_sec_per_km: Option<f64>,
    /// LT2 pace (sec/km) — defaults to threshold pace when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lt2_pace_sec_per_km: Option<f64>,
}

impl ThresholdEstimate {
    /// Compute the estimate from the supplied inputs.
    #[must_use]
    pub fn from_inputs(inputs: ThresholdInputs) -> Self {
        let lt2_hr = compute_lt2_hr(inputs);
        let lt1_hr = compute_lt1_hr(inputs, lt2_hr);
        let lt2_power_watts = inputs.ftp_watts;
        let lt1_power_watts = inputs.ftp_watts.map(|ftp| ftp * 0.75);
        let lt2_pace_sec_per_km = inputs.threshold_pace_sec_per_km;
        // LT1 pace is slower than LT2 pace; sec/km grows as pace slows.
        let lt1_pace_sec_per_km = inputs
            .threshold_pace_sec_per_km
            .map(|p| p * (1.0 / LT1_FRACTION_OF_LT2));
        Self {
            lt1_hr,
            lt2_hr,
            lt1_power_watts,
            lt2_power_watts,
            lt1_pace_sec_per_km,
            lt2_pace_sec_per_km,
        }
    }

    /// True when at least one threshold field could be estimated.
    #[must_use]
    pub const fn has_any(&self) -> bool {
        self.lt1_hr.is_some()
            || self.lt2_hr.is_some()
            || self.lt1_power_watts.is_some()
            || self.lt2_power_watts.is_some()
            || self.lt1_pace_sec_per_km.is_some()
            || self.lt2_pace_sec_per_km.is_some()
    }
}

fn compute_lt2_hr(inputs: ThresholdInputs) -> Option<f64> {
    if let Some(lthr) = inputs.lthr {
        return Some(lthr);
    }
    inputs.max_hr.map(|m| m * LT2_FRACTION_OF_MAX_HR)
}

fn compute_lt1_hr(inputs: ThresholdInputs, lt2_hr: Option<f64>) -> Option<f64> {
    let base_lt2 = lt2_hr?;
    if let (Some(max_hr), Some(resting_hr)) = (inputs.max_hr, inputs.resting_hr) {
        // Karvonen-style: LT1 ≈ resting + 0.65 * (max - resting), capped by
        // 85 % of LT2 so we never produce an LT1 above LT2.
        let karvonen = 0.65_f64.mul_add(max_hr - resting_hr, resting_hr);
        return Some(karvonen.min(base_lt2 * LT1_FRACTION_OF_LT2));
    }
    Some(base_lt2 * LT1_FRACTION_OF_LT2)
}
