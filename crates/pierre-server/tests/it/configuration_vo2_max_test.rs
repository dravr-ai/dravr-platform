// ABOUTME: Integration tests for VO2 max configuration and calculations
// ABOUTME: Tests extracted from embedded test module in src/configuration/vo2_max.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_cageux::config::intelligence::AlgorithmConfig;
use dravr_cageux::config::intelligence::{TrainingZonesConfig, VO2MaxCalculator};

#[test]
fn test_vo2_max_calculator_creation() {
    let calc = VO2MaxCalculator::new(50.0, 50, 180, 0.85);
    assert!(
        (calc.vdot - 50.0).abs() < f64::EPSILON,
        "bug: the calculator must hold the VDOT it was given"
    );
    assert!((calc.lactate_threshold - 0.85).abs() < f64::EPSILON);
}

#[test]
fn test_pace_zones_calculation() {
    let calc = VO2MaxCalculator::new(50.0, 50, 180, 0.85);
    let paces = calc
        .calculate_pace_zones(&TrainingZonesConfig::default())
        .expect("VDOT 50 cuts pace zones");

    // At a lactate threshold of 0.85 threshold pace is the speed whose
    // oxygen cost on Daniels' curve is 0.85 x 50 (262.46 s/km). Every zone
    // sits on the same curve, so the zones run slowest to fastest: easy,
    // marathon, threshold, VO2max, then the neuromuscular (mile) pace.
    assert!(
        (paces.threshold_pace_sec_per_km - 262.46).abs() < 0.05,
        "bug: threshold pace at 0.85 of VDOT 50 must be 262.46 s/km: {paces:?}"
    );
    assert!(
        paces.easy_pace_range.0 > paces.easy_pace_range.1,
        "bug: the easy range must run slow edge to fast edge: {paces:?}"
    );
    assert!(
        paces.easy_pace_range.1 > paces.marathon_pace_range.0,
        "bug: easy pace must be slower than marathon pace: {paces:?}"
    );
    assert!(
        paces.marathon_pace_range.1 > paces.threshold_pace_sec_per_km,
        "bug: marathon pace must be slower than threshold pace: {paces:?}"
    );
    assert!(
        paces.threshold_pace_range.0 > paces.threshold_pace_sec_per_km
            && paces.threshold_pace_sec_per_km > paces.threshold_pace_range.1,
        "bug: threshold pace must sit inside its band: {paces:?}"
    );
    assert!(
        paces.threshold_pace_range.1 > paces.vo2max_pace_range.1,
        "bug: threshold pace must be slower than VO2max pace: {paces:?}"
    );
    assert!(
        paces.vo2max_pace_range.1 > paces.vo2max_pace_range.0
            && paces.vo2max_pace_range.0 > paces.neuromuscular_pace_max,
        "bug: VO2max pace must run slow to fast and stay slower than the mile pace: {paces:?}"
    );
}

#[test]
fn test_pace_zones_refuse_a_vdot_that_is_no_number() {
    let calc = VO2MaxCalculator::new(f64::NAN, 50, 180, 0.85);
    assert!(
        calc.calculate_pace_zones(&TrainingZonesConfig::default())
            .is_err(),
        "a VDOT that is no number cuts no zones rather than inventing paces"
    );
}

#[test]
fn test_trimp_calculation() {
    let calc = VO2MaxCalculator::new(50.0, 50, 180, 0.85);

    let config = AlgorithmConfig::default();
    let trimp_male = calc.calculate_trimp(140, 30.0, "M", &config);
    let trimp_female = calc.calculate_trimp(140, 30.0, "F", &config);

    // Banister (1991): duration · x · k · e^(b·x), x the heart-rate reserve
    // fraction, here (140 − 50) / (180 − 50) = 9/13. The sexes differ in both
    // the multiplier and the exponent: k = 0.64, b = 1.92 male; k = 0.86,
    // b = 1.67 female. Over any valid reserve fraction the female weighting is
    // the higher one: male 50.22, female 56.76 for these 30 minutes.
    let x = 90.0_f64 / 130.0;
    let male = 30.0 * x * 0.64 * (1.92 * x).exp();
    let female = 30.0 * x * 0.86 * (1.67 * x).exp();
    assert!(
        (trimp_male - male).abs() < 1e-9,
        "male TRIMP should be {male:.2}, got {trimp_male}"
    );
    assert!(
        (trimp_female - female).abs() < 1e-9,
        "female TRIMP should be {female:.2}, got {trimp_female}"
    );
    assert!(trimp_female > trimp_male);
}
