// ABOUTME: The shapes of the athlete's latest anchor measurements: which anchors a read asks for, what it found
// ABOUTME: Resting heart rate, VO2max and weight, each with the day it was measured and the source standing for it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The types of [`super::latest_anchor_measurements`].

use chrono::NaiveDate;
use dravr_equilibre_sync::Merged;
use pierre_providers::ai_scope::StoredHealthRecord;

/// One measurement of an anchor reading: its value, the day it was
/// measured, and the source standing for that day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured<T> {
    /// The value, in the unit its field names.
    pub value: T,
    /// The athlete's local day it was measured on.
    pub date: NaiveDate,
    /// The key of the source standing for that day (`garmin`, `athlete`, ...).
    pub source: String,
}

/// Which training anchors a read asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Anchors {
    /// The resting heart rate.
    pub resting_hr: bool,
    /// The `VO2max`.
    pub vo2_max: bool,
    /// The weight.
    pub weight: bool,
}

/// The athlete's latest measurement of each training anchor the wellness
/// rows hold: resting heart rate, `VO2max`, weight.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnchorMeasurements {
    /// The latest resting heart rate, bpm.
    pub resting_hr: Option<Measured<u32>>,
    /// The latest `VO2max`, ml/kg/min.
    pub vo2_max: Option<Measured<f64>>,
    /// The latest weight, kg.
    pub weight_kg: Option<Measured<f64>>,
}

/// The newest merged day of `days` (ordered by date) carrying `field`.
pub(super) fn newest<T, V>(
    days: &[Merged<T>],
    field: impl Fn(&T) -> Option<V>,
    date: impl Fn(&T) -> NaiveDate,
) -> Option<Measured<V>>
where
    T: StoredHealthRecord,
{
    days.iter().rev().find_map(|day| {
        field(&day.record).map(|value| Measured {
            value,
            date: date(&day.record),
            source: day.record.source_name().to_owned(),
        })
    })
}
