// ABOUTME: Pins the platform's inversion of Daniels' oxygen-cost curve to the curve it inverts
// ABOUTME: Guards the derived running pace zones against a linearised VDOT-to-velocity shortcut
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_fitness_compute::velocity_at_vo2max;

/// Daniels' oxygen-cost curve, evaluated forwards: the oxygen cost
/// (ml/kg/min) of running at `velocity` metres per minute.
fn oxygen_cost(velocity: f64) -> f64 {
    0.000_104_f64.mul_add(velocity * velocity, 0.182_258_f64.mul_add(velocity, -4.60))
}

#[test]
fn velocity_at_vo2max_inverts_the_oxygen_cost_curve() {
    for vo2_max in [35.0, 40.0, 50.0, 60.0, 70.0, 85.0] {
        let velocity = velocity_at_vo2max(vo2_max);
        let round_trip = oxygen_cost(velocity);
        assert!(
            (round_trip - vo2_max).abs() < 1e-9,
            "velocity {velocity} costs {round_trip} ml/kg/min, not {vo2_max}"
        );
    }
}

#[test]
fn velocity_at_vo2max_pins_the_known_daniels_values() {
    // Positive root of 0.000104 v^2 + 0.182258 v - (vo2max + 4.60) = 0.
    for (vo2_max, expected) in [
        (40.0, 217.671_57),
        (50.0, 260.772_02),
        (60.0, 302.297_29),
        (70.0, 342.408_42),
    ] {
        let velocity = velocity_at_vo2max(vo2_max);
        assert!(
            (velocity - expected).abs() < 0.01,
            "VO2max {vo2_max} gave {velocity} m/min, expected {expected}"
        );
    }
}

#[test]
fn velocity_at_vo2max_rejects_the_linearised_form() {
    // Dropping the squared term — (vo2max + 4.60) / 0.182258 — reads about
    // 15 % fast: 299.6 m/min at VO2max 50 against the curve's 260.8.
    let velocity = velocity_at_vo2max(50.0);
    let linearised = (50.0 + 4.60) / 0.182_258;
    assert!(
        linearised - velocity > 35.0,
        "the linearised shortcut is {linearised}; the curve gives {velocity}"
    );

    // What the athlete sees: the easy zone at 70 % of that velocity, in
    // seconds per kilometre. 5:29/km, not the linearised form's 4:46/km.
    let easy_seconds_per_km = 1000.0 / (velocity * 0.70) * 60.0;
    assert!(
        (easy_seconds_per_km - 328.69).abs() < 0.1,
        "easy pace was {easy_seconds_per_km} s/km, expected 328.69 (5:29/km)"
    );
}

#[test]
fn velocity_at_vo2max_rises_with_fitness() {
    let mut previous = velocity_at_vo2max(30.0);
    for vo2_max in [35.0, 40.0, 45.0, 50.0, 55.0, 60.0, 65.0, 70.0] {
        let velocity = velocity_at_vo2max(vo2_max);
        assert!(
            velocity > previous,
            "VO2max {vo2_max} gave {velocity} m/min, below the previous {previous}"
        );
        previous = velocity;
    }
}

#[test]
fn velocity_at_vo2max_returns_zero_for_unusable_input() {
    for vo2_max in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let velocity = velocity_at_vo2max(vo2_max);
        assert!(
            velocity.abs() < f64::EPSILON,
            "VO2max {vo2_max} gave {velocity} m/min instead of zero"
        );
    }
}
