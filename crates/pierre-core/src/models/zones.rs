// ABOUTME: Endurance per-user training-zone boundaries (HR + power) — distinct from per-activity HeartRateZone
// ABOUTME: Used by latest.json + dossier.json to expose the athlete's zone definitions to coaches and analytics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

/// Five-zone heart-rate boundaries for a single user.
///
/// Each `zN_max` is the upper bound (inclusive) of zone `N` in beats per minute.
/// The lower bound of zone `N+1` is `zN_max + 1`. A zone above `z5_max` is
/// implicitly "above max" and not counted in distribution analytics.
///
/// This is distinct from
/// [`pierre_core::models::HeartRateZone`](super::HeartRateZone), which carries
/// per-activity time-in-zone counts. `HrZoneSet` is the per-user definition;
/// `HeartRateZone` is the per-activity occupancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HrZoneSet {
    /// Upper bound of Zone 1 (active recovery / very easy) in bpm.
    pub z1_max: u16,
    /// Upper bound of Zone 2 (aerobic base / endurance) in bpm.
    pub z2_max: u16,
    /// Upper bound of Zone 3 (tempo) in bpm.
    pub z3_max: u16,
    /// Upper bound of Zone 4 (threshold / lactate threshold) in bpm.
    pub z4_max: u16,
    /// Upper bound of Zone 5 (`VO2max` / anaerobic) in bpm.
    pub z5_max: u16,
}

impl HrZoneSet {
    /// Construct a zone set from explicit upper bounds.
    ///
    /// # Errors
    ///
    /// Returns an error string when the zones are not strictly increasing,
    /// which would make the distribution analytics ambiguous.
    pub fn new(
        z1_max: u16,
        z2_max: u16,
        z3_max: u16,
        z4_max: u16,
        z5_max: u16,
    ) -> Result<Self, &'static str> {
        if !(z1_max < z2_max && z2_max < z3_max && z3_max < z4_max && z4_max < z5_max) {
            return Err("HR zones must be strictly increasing (z1 < z2 < z3 < z4 < z5)");
        }
        Ok(Self {
            z1_max,
            z2_max,
            z3_max,
            z4_max,
            z5_max,
        })
    }

    /// Classify a single heart-rate sample (in bpm) into its zone index 1..=5,
    /// or `None` for samples above `z5_max`.
    #[must_use]
    pub const fn classify(&self, bpm: u16) -> Option<u8> {
        if bpm <= self.z1_max {
            Some(1)
        } else if bpm <= self.z2_max {
            Some(2)
        } else if bpm <= self.z3_max {
            Some(3)
        } else if bpm <= self.z4_max {
            Some(4)
        } else if bpm <= self.z5_max {
            Some(5)
        } else {
            None
        }
    }
}

/// Five-zone power boundaries for a single user (cycling / running power).
///
/// Each `zN_max` is the upper bound (inclusive) of zone `N` in watts. Same
/// monotonicity invariant as [`HrZoneSet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerZoneSet {
    /// Upper bound of Zone 1 (active recovery) in watts.
    pub z1_max: u32,
    /// Upper bound of Zone 2 (endurance) in watts.
    pub z2_max: u32,
    /// Upper bound of Zone 3 (tempo) in watts.
    pub z3_max: u32,
    /// Upper bound of Zone 4 (threshold / FTP) in watts.
    pub z4_max: u32,
    /// Upper bound of Zone 5 (`VO2max`) in watts.
    pub z5_max: u32,
}

impl PowerZoneSet {
    /// Construct a zone set from explicit upper bounds.
    ///
    /// # Errors
    ///
    /// Returns an error string when the zones are not strictly increasing.
    pub fn new(
        z1_max: u32,
        z2_max: u32,
        z3_max: u32,
        z4_max: u32,
        z5_max: u32,
    ) -> Result<Self, &'static str> {
        if !(z1_max < z2_max && z2_max < z3_max && z3_max < z4_max && z4_max < z5_max) {
            return Err("Power zones must be strictly increasing (z1 < z2 < z3 < z4 < z5)");
        }
        Ok(Self {
            z1_max,
            z2_max,
            z3_max,
            z4_max,
            z5_max,
        })
    }

    /// Classify a single power sample (in watts) into its zone index 1..=5,
    /// or `None` for samples above `z5_max`.
    #[must_use]
    pub const fn classify(&self, watts: u32) -> Option<u8> {
        if watts <= self.z1_max {
            Some(1)
        } else if watts <= self.z2_max {
            Some(2)
        } else if watts <= self.z3_max {
            Some(3)
        } else if watts <= self.z4_max {
            Some(4)
        } else if watts <= self.z5_max {
            Some(5)
        } else {
            None
        }
    }
}

/// Time-in-zone distribution computed across one or more activities.
///
/// Values are absolute seconds spent in each zone. Sum across all zones plus
/// `above_max_seconds` equals total samples in the analyzed window. Used by
/// the Endurance `latest.json` payload to surface polarized-distribution
/// stats (Z1+Z2 share vs Z3 share vs Z4+Z5 share) to coaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ZoneDistribution {
    /// Seconds spent in zone 1 (active recovery / very easy).
    pub z1_seconds: u32,
    /// Seconds spent in zone 2 (aerobic base / endurance).
    pub z2_seconds: u32,
    /// Seconds spent in zone 3 (tempo).
    pub z3_seconds: u32,
    /// Seconds spent in zone 4 (threshold).
    pub z4_seconds: u32,
    /// Seconds spent in zone 5 (`VO2max`).
    pub z5_seconds: u32,
    /// Seconds with intensity above `z5_max` — sprints, neuromuscular peaks.
    pub above_max_seconds: u32,
}

impl ZoneDistribution {
    /// Total seconds analyzed (sum of all zone buckets including above-max).
    #[must_use]
    pub const fn total_seconds(&self) -> u32 {
        self.z1_seconds
            + self.z2_seconds
            + self.z3_seconds
            + self.z4_seconds
            + self.z5_seconds
            + self.above_max_seconds
    }
}
