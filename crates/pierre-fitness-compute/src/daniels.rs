// ABOUTME: The platform's inversion of Jack Daniels' oxygen-cost curve — VO2max to velocity at VO2max
// ABOUTME: Pure math; every platform surface deriving running pace zones from VO2max reads it here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Daniels velocity
//!
//! Jack Daniels' oxygen-cost curve maps running velocity `v` (metres per
//! minute) to its oxygen cost (millilitres per kilogram per minute):
//!
//! ```text
//! vo2 = -4.60 + 0.182258 * v + 0.000104 * v^2
//! ```
//!
//! [`velocity_at_vo2max`] inverts it: the positive root of
//! `0.000104 v^2 + 0.182258 v - (vo2max + 4.60) = 0` is the velocity an athlete
//! of that VDOT holds at VO2max, and every pace zone the platform quotes is a
//! fraction of it. The squared term is worth about 7 ml/kg/min at racing
//! speeds — roughly 15 % of the velocity — so it carries the relation rather
//! than decorating it.

/// Coefficient of the squared velocity term in Daniels' oxygen-cost curve.
const DANIELS_A: f64 = 0.000_104;

/// Coefficient of the linear velocity term in Daniels' oxygen-cost curve.
const DANIELS_B: f64 = 0.182_258;

/// Oxygen cost (ml/kg/min) the curve attributes to standing still, negated.
const DANIELS_C: f64 = 4.60;

/// Velocity at `VO2max`, in metres per minute, for an athlete of `vo2_max`
/// (ml/kg/min).
///
/// Returns `0.0` when `vo2_max` is not a finite positive number, which callers
/// render as an unusable pace rather than an invented one.
#[must_use]
pub fn velocity_at_vo2max(vo2_max: f64) -> f64 {
    if !vo2_max.is_finite() || vo2_max <= 0.0 {
        return 0.0;
    }
    // c is at most -4.60 here, so -4ac is positive and the discriminant
    // exceeds DANIELS_B^2: the positive root always exists.
    let c = -(vo2_max + DANIELS_C);
    let discriminant = DANIELS_B.mul_add(DANIELS_B, -(4.0 * DANIELS_A * c));
    (-DANIELS_B + discriminant.sqrt()) / (2.0 * DANIELS_A)
}
