// ABOUTME: The training thresholds a provider keeps for the athlete — FTP, threshold and max heart rate, threshold pace — each with its as-of day
// ABOUTME: What an import reads off a provider before the precedence rule decides which of them reach the physiological profile
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::NaiveDate;

/// One threshold a provider holds, in the unit its field name carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderThreshold<T> {
    /// The value.
    pub value: T,
    /// The day the provider's setting holding it last changed, when the
    /// provider says.
    pub as_of: Option<NaiveDate>,
}

/// The thresholds a provider holds for the athlete, as the provider set them.
///
/// Every value is the provider's estimate: a provider names no test behind a
/// setting, so an imported value is quoted with [`Self::origin`]. A field is
/// `None` when the provider holds no usable value for it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderThresholds {
    /// Who holds the values, as an imported value's provenance names it
    /// (`intervals.icu`).
    pub origin: &'static str,
    /// Functional threshold power, in watts.
    pub ftp_watts: Option<ProviderThreshold<u32>>,
    /// Lactate threshold heart rate, in bpm.
    pub threshold_hr: Option<ProviderThreshold<u16>>,
    /// Maximum heart rate, in bpm.
    pub max_hr: Option<ProviderThreshold<u16>>,
    /// Threshold pace, in seconds per kilometre.
    pub threshold_pace_sec_per_km: Option<ProviderThreshold<f64>>,
}
