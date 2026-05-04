// ABOUTME: Smoke test for pierre-intelligence's bridge to dravr-cageux algorithms
// ABOUTME: Verifies the re-exported MaxHrAlgorithm computes plausible values
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Smoke integration tests for the crate public API.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use pierre_intelligence::algorithms::maxhr::MaxHrAlgorithm;

#[test]
fn tanaka_max_hr_for_age_forty_is_in_expected_range() {
    let max_hr = MaxHrAlgorithm::Tanaka
        .estimate(40, None)
        .expect("valid age must succeed");
    // Tanaka: 208 - 0.7 * 40 = 180. Allow ±1 bpm for rounding behavior.
    assert!(
        (179.0..=181.0).contains(&max_hr),
        "expected ~180 bpm for Tanaka@40, got {max_hr}"
    );
}

#[test]
fn fox_max_hr_for_age_forty() {
    let max_hr = MaxHrAlgorithm::Fox
        .estimate(40, None)
        .expect("valid age must succeed");
    // Fox: 220 - 40 = 180.
    assert!(
        (179.0..=181.0).contains(&max_hr),
        "expected ~180 bpm for Fox@40, got {max_hr}"
    );
}

#[test]
fn max_hr_rejects_invalid_age() {
    assert!(
        MaxHrAlgorithm::Tanaka.estimate(0, None).is_err(),
        "age=0 must fail validation"
    );
    assert!(
        MaxHrAlgorithm::Tanaka.estimate(200, None).is_err(),
        "age=200 must fail validation"
    );
}

#[test]
fn default_max_hr_algorithm_is_tanaka() {
    let default = MaxHrAlgorithm::default();
    assert_eq!(default, MaxHrAlgorithm::Tanaka);
}
