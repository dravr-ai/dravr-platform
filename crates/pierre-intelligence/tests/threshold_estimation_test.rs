// ABOUTME: Endurance Phase 3 unit tests for ThresholdEstimate — LT1 / LT2 from HR / power / pace
// ABOUTME: Pure tests; no IO; locks in Coggan + Karvonen + Seiler factors
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_intelligence::threshold_estimation::{ThresholdEstimate, ThresholdInputs};

#[test]
fn empty_inputs_yield_no_thresholds() {
    let estimate = ThresholdEstimate::from_inputs(ThresholdInputs::default());
    assert!(!estimate.has_any());
    assert!(estimate.lt1_hr.is_none());
    assert!(estimate.lt2_hr.is_none());
    assert!(estimate.lt1_power_watts.is_none());
    assert!(estimate.lt2_power_watts.is_none());
    assert!(estimate.lt1_pace_sec_per_km.is_none());
    assert!(estimate.lt2_pace_sec_per_km.is_none());
}

#[test]
fn lthr_drives_lt2_directly() {
    let estimate = ThresholdEstimate::from_inputs(ThresholdInputs {
        lthr: Some(170.0),
        ..ThresholdInputs::default()
    });
    assert_eq!(estimate.lt2_hr, Some(170.0));
}

#[test]
fn max_hr_only_falls_back_to_88_percent_lt2() {
    let estimate = ThresholdEstimate::from_inputs(ThresholdInputs {
        max_hr: Some(190.0),
        ..ThresholdInputs::default()
    });
    let lt2 = estimate.lt2_hr.expect("lt2 from max_hr fallback");
    assert!(
        (lt2 - 167.2).abs() < 1e-6,
        "expected 0.88*190=167.2, got {lt2}"
    );
}

#[test]
fn karvonen_lt1_caps_at_85_percent_of_lt2() {
    // Resting 50, max 200 → Karvonen LT1 = 50 + 0.65*(200-50) = 147.5
    // LT2 from lthr = 170 → 0.85*170 = 144.5; cap wins.
    let estimate = ThresholdEstimate::from_inputs(ThresholdInputs {
        lthr: Some(170.0),
        max_hr: Some(200.0),
        resting_hr: Some(50.0),
        ..ThresholdInputs::default()
    });
    let lt1 = estimate.lt1_hr.expect("lt1 present");
    assert!(
        (lt1 - 144.5).abs() < 1e-6,
        "expected cap = 0.85 * lt2, got {lt1}"
    );
}

#[test]
fn ftp_drives_lt2_power_and_lt1_at_75_percent() {
    let estimate = ThresholdEstimate::from_inputs(ThresholdInputs {
        ftp_watts: Some(280.0),
        ..ThresholdInputs::default()
    });
    assert_eq!(estimate.lt2_power_watts, Some(280.0));
    let lt1 = estimate.lt1_power_watts.expect("lt1 power");
    assert!(
        (lt1 - 210.0).abs() < 1e-9,
        "expected 0.75 * 280 = 210, got {lt1}"
    );
}

#[test]
fn threshold_pace_drives_lt2_and_lt1_pace_slower() {
    // LT2 pace = 240 sec/km. LT1 should be slower → larger sec/km value.
    let estimate = ThresholdEstimate::from_inputs(ThresholdInputs {
        threshold_pace_sec_per_km: Some(240.0),
        ..ThresholdInputs::default()
    });
    assert_eq!(estimate.lt2_pace_sec_per_km, Some(240.0));
    let lt1 = estimate.lt1_pace_sec_per_km.expect("lt1 pace");
    assert!(
        lt1 > 240.0,
        "LT1 pace must be slower (greater sec/km) than LT2"
    );
    // 240 / 0.85 = 282.35
    assert!((lt1 - 282.352_941_176_5).abs() < 1e-3);
}

#[test]
fn has_any_true_when_at_least_one_field_present() {
    let estimate = ThresholdEstimate::from_inputs(ThresholdInputs {
        ftp_watts: Some(250.0),
        ..ThresholdInputs::default()
    });
    assert!(estimate.has_any());
}
