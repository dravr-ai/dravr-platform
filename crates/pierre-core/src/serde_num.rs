// ABOUTME: Serde deserializers that accept a whole-valued JSON float where a schema declares an integer
// ABOUTME: LLM callers emit 60.0 for a number; strict serde rejects a float for u8/u32 and that rejection killed live tool calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Whole-number deserializers
//!
//! Tool schemas declare integers, and the models calling them routinely send
//! `3.0`, `60.0`, `480.0`. Strict serde rejects a float for a `u8` or a `u32`,
//! which rejected seven consecutive live `save_training_plan` calls on
//! 2026-07-12. These accept a whole-valued float and reject a fractional or
//! out-of-range one with a message the model can act on.

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};

/// A JSON number as a whole value inside `0..=max`, or the rejection.
fn whole<E: DeError>(n: f64, max: u32) -> Result<u32, E> {
    // In-range whole doubles convert exactly; the guard runs first.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    if n.fract() == 0.0 && (0.0..=f64::from(max)).contains(&n) {
        Ok(n as u32)
    } else {
        Err(E::custom(format!(
            "expected a whole number between 0 and {max}, got {n}"
        )))
    }
}

/// Deserialize a whole-valued JSON number (int or float) into `u8`.
///
/// # Errors
///
/// Rejects a fractional value and one outside `0..=255`.
pub fn whole_u8<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let n = f64::deserialize(deserializer)?;
    let value = whole(n, u32::from(u8::MAX))?;
    u8::try_from(value).map_err(D::Error::custom)
}

/// Deserialize a whole-valued JSON number (int or float) into `u32`.
///
/// # Errors
///
/// Rejects a fractional value and one outside `0..=u32::MAX`.
pub fn whole_u32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let n = f64::deserialize(deserializer)?;
    whole(n, u32::MAX)
}

/// Deserialize an optional whole-valued JSON number into `Option<u32>`; an
/// absent or `null` value is `None`.
///
/// # Errors
///
/// Rejects a fractional value and one outside `0..=u32::MAX`.
pub fn whole_u32_opt<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    Option::<f64>::deserialize(deserializer)?
        .map(|n| whole(n, u32::MAX))
        .transpose()
}
