// ABOUTME: Endurance per-user training-zone boundaries (HR + power) — distinct from per-activity HeartRateZone
// ABOUTME: Persisted copies of dravr-cageux's one zone model, used by latest.json + dossier.json and the profile
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's zones as the profile stores and renders them.
//!
//! Every boundary here is cut by `dravr_cageux::zones`, the one zone model:
//! heart-rate zones are [`HrZones`] (`VO2max`-tiered Karvonen, with a zone 6
//! for an athlete whose tier carries one) and power zones are [`PowerZones`]
//! (Coggan's seven levels). [`HrZoneSet::from_zones`] and
//! [`PowerZoneSet::from_zones`] are the only constructors, so a set this code
//! writes is always a copy of cageux's.
//!
//! Rows written before the zones moved to cageux hold five boundaries cut by a
//! table the platform carried itself. They still parse: every field added
//! since is `#[serde(default)]`, and the `model` tag defaults to `legacy`, so
//! a reader can tell such a row from a current one and the zone backfill can
//! find it.

use std::error::Error;
use std::fmt;

use dravr_cageux::zones::{HrZoneTier, HrZones, PowerZones, Vo2maxSource};
use serde::{Deserialize, Serialize};

use crate::models::UserPhysiologicalProfile;

/// Which model cut a stored set of heart-rate zones.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HrZoneModel {
    /// Five zones at fixed fractions of heart-rate reserve, written before
    /// the zones moved to cageux. Only ever read from an old row; nothing
    /// writes it.
    #[default]
    Legacy,
    /// cageux's `VO2max`-tiered Karvonen zones ([`HrZones`]).
    KarvonenVo2maxTiered,
}

/// Which model cut a stored set of power zones.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PowerZoneModel {
    /// Five zones at configurable FTP percentages, written before the zones
    /// moved to cageux. Only ever read from an old row; nothing writes it.
    #[default]
    Legacy,
    /// Coggan's seven power levels ([`PowerZones`]).
    Coggan,
}

/// Heart-rate zone boundaries for a single user.
///
/// Each `zN_max` is the upper bound (inclusive) of zone `N` in beats per
/// minute; zone `N+1` starts above it. `z6_max` is present when the athlete's
/// tier carries a zone 6 (`VO2max` of 50 or more and a zone 5 that stops
/// short of their maximum), and is then their maximum heart rate.
///
/// This is distinct from
/// [`pierre_core::models::HeartRateZone`](super::HeartRateZone), which carries
/// per-activity time-in-zone counts. `HrZoneSet` is the per-user definition;
/// `HeartRateZone` is the per-activity occupancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HrZoneSet {
    /// Lower bound of Zone 1 (recovery) in bpm, inclusive: the zone's floor
    /// on the athlete's reserve. Absent on a legacy row.
    #[serde(default)]
    pub z1_min: Option<u16>,
    /// Upper bound of Zone 1 (recovery) in bpm.
    pub z1_max: u16,
    /// Upper bound of Zone 2 (endurance) in bpm.
    pub z2_max: u16,
    /// Upper bound of Zone 3 (tempo) in bpm.
    pub z3_max: u16,
    /// Upper bound of Zone 4 (threshold) in bpm.
    pub z4_max: u16,
    /// Upper bound of Zone 5 (`VO2max`) in bpm.
    pub z5_max: u16,
    /// Upper bound of Zone 6 (anaerobic) in bpm, the athlete's maximum heart
    /// rate; absent when their zones have no zone 6, and on a legacy row.
    #[serde(default)]
    pub z6_max: Option<u16>,
    /// The `VO2max` tier that chose the zone fractions; absent on a legacy row.
    #[serde(default)]
    pub tier: Option<HrZoneTier>,
    /// Whether the `VO2max` that chose the tier was the athlete's own or
    /// estimated from their heart rates; absent on a legacy row.
    #[serde(default)]
    pub vo2_max_source: Option<Vo2maxSource>,
    /// Which model cut these boundaries. A row written before this field
    /// existed reads as [`HrZoneModel::Legacy`].
    #[serde(default)]
    pub model: HrZoneModel,
}

impl HrZoneSet {
    /// The stored copy of cageux's zones for an athlete.
    #[must_use]
    pub fn from_zones(zones: &HrZones) -> Self {
        let upper = |zone: u8| {
            zones
                .zones()
                .iter()
                .find(|band| band.zone == zone)
                .map(|band| band.upper_bpm)
        };
        // cageux always cuts zones 1-5; zone 6 is the only optional one.
        let required = |zone: u8| upper(zone).unwrap_or(zones.max_hr);
        Self {
            z1_min: zones.zones().first().map(|band| band.lower_bpm),
            z1_max: required(1),
            z2_max: required(2),
            z3_max: required(3),
            z4_max: required(4),
            z5_max: required(5),
            z6_max: upper(6),
            tier: Some(zones.tier),
            vo2_max_source: Some(zones.vo2_max_source),
            model: HrZoneModel::KarvonenVo2maxTiered,
        }
    }
}

/// Power zone boundaries for a single user (cycling / running power).
///
/// Each `zN_max` is the upper bound (inclusive) of zone `N` in watts, rounded
/// to the watt. Zone 7 is everything above `z6_max` and has no upper bound;
/// a legacy row holds only five zones and no `z6_max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PowerZoneSet {
    /// Upper bound of Zone 1 (active recovery) in watts.
    pub z1_max: u32,
    /// Upper bound of Zone 2 (endurance) in watts.
    pub z2_max: u32,
    /// Upper bound of Zone 3 (tempo) in watts.
    pub z3_max: u32,
    /// Upper bound of Zone 4 (lactate threshold) in watts.
    pub z4_max: u32,
    /// Upper bound of Zone 5 (`VO2max`) in watts.
    pub z5_max: u32,
    /// Upper bound of Zone 6 (anaerobic capacity) in watts; zone 7
    /// (neuromuscular power) is everything above it. Absent on a legacy row.
    #[serde(default)]
    pub z6_max: Option<u32>,
    /// Which model cut these boundaries. A row written before this field
    /// existed reads as [`PowerZoneModel::Legacy`].
    #[serde(default)]
    pub model: PowerZoneModel,
}

impl PowerZoneSet {
    /// The stored copy of cageux's Coggan levels for an FTP, each upper edge
    /// rounded to the watt.
    #[must_use]
    pub fn from_zones(zones: &PowerZones) -> Self {
        let upper = |level: u8| {
            zones
                .zones()
                .iter()
                .find(|band| band.zone == level)
                .and_then(|band| band.upper_watts)
                .map(round_watts)
        };
        // cageux always cuts levels 1-6 with an upper edge; level 7 has none.
        let required = |level: u8| upper(level).unwrap_or(0);
        Self {
            z1_max: required(1),
            z2_max: required(2),
            z3_max: required(3),
            z4_max: required(4),
            z5_max: required(5),
            z6_max: upper(6),
            model: PowerZoneModel::Coggan,
        }
    }
}

/// Watts rounded to the nearest whole watt.
fn round_watts(watts: f64) -> u32 {
    // Safe: a Coggan edge is a positive fraction (at most 1.5) of an FTP the
    // profile's validation bounds far inside u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rounded = watts.round().max(0.0) as u32;
    rounded
}

/// Why an athlete's heart-rate zones could not be cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HrZonesUnavailable {
    /// No physiological profile is saved for the athlete.
    NoProfile,
    /// The profile lacks a resting heart rate, a maximum heart rate, or both;
    /// the zones are cut from the reserve between the two.
    MissingHeartRates {
        /// Whether the resting heart rate is missing.
        resting_hr: bool,
        /// Whether the maximum heart rate is missing.
        max_hr: bool,
    },
    /// cageux refused the saved values (a resting rate at or above the
    /// maximum or below the resting-rate floor, or a `VO2max` that is not a
    /// positive number), with its reason.
    Rejected(String),
}

impl HrZonesUnavailable {
    /// Why the zone reading is absent, worded for the reply that omits it.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::NoProfile
            | Self::MissingHeartRates {
                resting_hr: true,
                max_hr: true,
            } => "heart-rate zones need the athlete's resting and maximum heart rate; neither is \
                 saved (set_physiology)"
                .to_owned(),
            Self::MissingHeartRates {
                resting_hr: true, ..
            } => "heart-rate zones need the athlete's resting heart rate, which is not saved \
                 (set_physiology)"
                .to_owned(),
            Self::MissingHeartRates { .. } => {
                "heart-rate zones need the athlete's maximum heart rate, which is not saved \
                 (set_physiology)"
                    .to_owned()
            }
            Self::Rejected(reason) => {
                format!("heart-rate zones could not be cut from the saved heart rates: {reason}")
            }
        }
    }
}

impl fmt::Display for HrZonesUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason())
    }
}

impl Error for HrZonesUnavailable {}

impl UserPhysiologicalProfile {
    /// The athlete's heart-rate zones, cut by cageux from their saved resting
    /// and maximum heart rate and `VO2max` ([`HrZones::for_athlete`]: a saved
    /// `VO2max` chooses the tier, else Uth's heart-rate ratio estimates one,
    /// which the zones record as their `vo2_max_source`).
    ///
    /// # Errors
    ///
    /// [`HrZonesUnavailable`] when a heart rate is missing or cageux refuses
    /// them: a zone reading is then absent, never cut from a default.
    pub fn hr_zones_model(&self) -> Result<HrZones, HrZonesUnavailable> {
        let (Some(resting_hr), Some(max_hr)) = (self.resting_hr, self.max_hr) else {
            return Err(HrZonesUnavailable::MissingHeartRates {
                resting_hr: self.resting_hr.is_none(),
                max_hr: self.max_hr.is_none(),
            });
        };
        HrZones::for_athlete(self.vo2_max, resting_hr, max_hr)
            .map_err(|e| HrZonesUnavailable::Rejected(e.to_string()))
    }

    /// The athlete's Coggan power levels from their saved FTP, or `None`
    /// without one.
    #[must_use]
    pub fn power_zones_model(&self) -> Option<PowerZones> {
        self.ftp_watts
            .and_then(|ftp| PowerZones::coggan(f64::from(ftp)).ok())
    }

    /// Re-cut the stored zone copies from the profile's own inputs.
    ///
    /// Each set is replaced by cageux's zones when its inputs are present and
    /// left as stored when they are not, so a profile whose heart rates were
    /// never saved keeps what it had.
    pub fn refresh_zones(&mut self) {
        if let Ok(zones) = self.hr_zones_model() {
            self.hr_zones = Some(HrZoneSet::from_zones(&zones));
        }
        if let Some(zones) = self.power_zones_model() {
            self.power_zones = Some(PowerZoneSet::from_zones(&zones));
        }
    }
}

/// The athlete's heart-rate zones from an optional profile.
///
/// # Errors
///
/// [`HrZonesUnavailable::NoProfile`] without a profile, else as
/// [`UserPhysiologicalProfile::hr_zones_model`].
pub fn athlete_hr_zones(
    profile: Option<&UserPhysiologicalProfile>,
) -> Result<HrZones, HrZonesUnavailable> {
    profile.map_or(
        Err(HrZonesUnavailable::NoProfile),
        UserPhysiologicalProfile::hr_zones_model,
    )
}

/// Time-in-zone distribution computed across one or more activities.
///
/// Values are absolute seconds spent in each of the athlete's heart-rate
/// zones. Sum across all zones plus `above_max_seconds` equals total samples
/// in the analyzed window. Used by the Endurance `latest.json` payload to
/// surface polarized-distribution stats (Z1+Z2 share vs Z3 share vs Z4 and up)
/// to agents.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct ZoneDistribution {
    /// Seconds spent in zone 1 (recovery).
    pub z1_seconds: u32,
    /// Seconds spent in zone 2 (endurance).
    pub z2_seconds: u32,
    /// Seconds spent in zone 3 (tempo).
    pub z3_seconds: u32,
    /// Seconds spent in zone 4 (threshold).
    pub z4_seconds: u32,
    /// Seconds spent in zone 5 (`VO2max`).
    pub z5_seconds: u32,
    /// Seconds spent in zone 6 (anaerobic); zero for an athlete whose zones
    /// have no zone 6.
    #[serde(default)]
    pub z6_seconds: u32,
    /// Seconds with a heart rate above the athlete's maximum.
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
            + self.z6_seconds
            + self.above_max_seconds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SportType;

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn a_row_written_before_the_one_zone_model_still_parses_as_legacy() -> TestResult {
        // The exact shape set_physiology stored before the zones moved to
        // cageux: five upper bounds and nothing else.
        let hr: HrZoneSet = serde_json::from_str(
            r#"{"z1_max":128,"z2_max":141,"z3_max":154,"z4_max":167,"z5_max":190}"#,
        )?;
        assert_eq!(hr.model, HrZoneModel::Legacy);
        assert_eq!((hr.z1_max, hr.z5_max), (128, 190));
        assert_eq!(hr.z6_max, None);
        assert_eq!(hr.z1_min, None);
        assert_eq!(hr.tier, None);
        assert_eq!(hr.vo2_max_source, None);

        let power: PowerZoneSet = serde_json::from_str(
            r#"{"z1_max":137,"z2_max":187,"z3_max":225,"z4_max":262,"z5_max":300}"#,
        )?;
        assert_eq!(power.model, PowerZoneModel::Legacy);
        assert_eq!((power.z4_max, power.z5_max), (262, 300));
        assert_eq!(power.z6_max, None);

        let distribution: ZoneDistribution = serde_json::from_str(
            r#"{"z1_seconds":1,"z2_seconds":2,"z3_seconds":3,"z4_seconds":4,"z5_seconds":5,"above_max_seconds":6}"#,
        )?;
        assert_eq!(distribution.z6_seconds, 0);
        assert_eq!(distribution.total_seconds(), 21);
        Ok(())
    }

    #[test]
    fn a_stored_set_copies_cageux_and_round_trips() -> TestResult {
        let zones = HrZones::karvonen_tiered(55.0, 50, 190)?;
        let set = HrZoneSet::from_zones(&zones);
        let bounds: Vec<u16> = zones.zones().iter().map(|band| band.upper_bpm).collect();
        assert_eq!(bounds.len(), 6, "an advanced athlete has a zone 6");
        let stored = vec![
            set.z1_max,
            set.z2_max,
            set.z3_max,
            set.z4_max,
            set.z5_max,
            set.z6_max.ok_or("zone 6 missing")?,
        ];
        assert_eq!(stored, bounds);
        assert_eq!(set.z6_max, Some(190), "zone 6 ends at the maximum");
        assert_eq!(set.z1_min, zones.zones().first().map(|band| band.lower_bpm));
        assert_eq!(set.tier, Some(HrZoneTier::Advanced));
        assert_eq!(set.vo2_max_source, Some(Vo2maxSource::Provided));
        assert_eq!(set.model, HrZoneModel::KarvonenVo2maxTiered);
        let reread: HrZoneSet = serde_json::from_value(serde_json::to_value(set)?)?;
        assert_eq!(reread, set);
        Ok(())
    }

    #[test]
    fn coggan_levels_at_250_watts_are_stored_to_the_watt() -> TestResult {
        let set = PowerZoneSet::from_zones(&PowerZones::coggan(250.0)?);
        // 55 / 75 / 90 / 105 / 120 / 150 % of 250 W, rounded half away from zero.
        assert_eq!(
            [set.z1_max, set.z2_max, set.z3_max, set.z4_max, set.z5_max],
            [138, 188, 225, 263, 300]
        );
        assert_eq!(set.z6_max, Some(375));
        assert_eq!(set.model, PowerZoneModel::Coggan);
        Ok(())
    }

    #[test]
    fn zones_need_both_heart_rates_and_say_which_is_missing() -> TestResult {
        let mut profile = UserPhysiologicalProfile::new(uuid::Uuid::new_v4(), SportType::Run);
        assert_eq!(
            profile.hr_zones_model(),
            Err(HrZonesUnavailable::MissingHeartRates {
                resting_hr: true,
                max_hr: true
            })
        );
        profile.max_hr = Some(190);
        let Err(missing) = profile.hr_zones_model() else {
            return Err("zones cut without a resting heart rate".into());
        };
        assert!(
            missing.reason().contains("resting heart rate"),
            "{missing:?}"
        );
        profile.resting_hr = Some(50);
        let zones = profile.hr_zones_model()?;
        // No VO2max saved: Uth's ratio, 15.3 x 190 / 50 = 58.1, chooses the tier.
        assert_eq!(zones.vo2_max_source, Vo2maxSource::HeartRateRatio);
        assert_eq!(zones.tier, HrZoneTier::Advanced);
        assert_eq!(athlete_hr_zones(None), Err(HrZonesUnavailable::NoProfile));
        Ok(())
    }

    #[test]
    fn refreshing_rewrites_a_legacy_row_and_keeps_one_without_inputs() -> TestResult {
        let legacy: HrZoneSet = serde_json::from_str(
            r#"{"z1_max":128,"z2_max":141,"z3_max":154,"z4_max":167,"z5_max":190}"#,
        )?;
        let mut profile = UserPhysiologicalProfile::new(uuid::Uuid::new_v4(), SportType::Ride);
        profile.hr_zones = Some(legacy);
        // No heart rates: the stored set stays as it is.
        profile.refresh_zones();
        assert_eq!(profile.hr_zones, Some(legacy));

        profile.resting_hr = Some(50);
        profile.max_hr = Some(190);
        profile.ftp_watts = Some(250);
        profile.refresh_zones();
        let expected = HrZoneSet::from_zones(&profile.hr_zones_model()?);
        assert_eq!(profile.hr_zones, Some(expected));
        assert_eq!(
            profile.power_zones.map(|zones| zones.model),
            Some(PowerZoneModel::Coggan)
        );
        Ok(())
    }
}
