// ABOUTME: An athlete's physiology anchors per sport family, and the AthleteAnchors dravr-cageux resolves workout targets on
// ABOUTME: A stored anchor carries range, kind, origin, day, method and confidence; the profile row fills what none states
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-sport anchors (carnet#1496).
//!
//! A workout family's targets are domain coordinates, and dravr-cageux places
//! them on each sport's own anchors: the ratio of MAP to critical power, of
//! critical power to FTP, of `vVO2max` to critical speed. A ratio is the
//! athlete's own only when both of its ends are stated, and the I8 default
//! otherwise. The profile row holds one value per threshold for every sport;
//! a run critical speed never sets bike power and a bike maximum heart rate
//! never sets run heart-rate zones (rule I9), so anchors are also stored per
//! sport family, each with its range, kind, origin, day, method and
//! confidence ([`SportAnchor`]).
//!
//! [`athlete_anchors`] joins the two. A profile-row value is placed on the one
//! family it belongs to ([`ProfileTwin`]) and a stored per-sport anchor
//! replaces it. Confidence is never derived here: a profile-row value names no
//! test protocol, so it maps at [`AnchorConfidence::Low`], and only a writer
//! that states a confidence against a named method ranks an anchor higher.
//! Freshness and default ranges stay with the kernel, which reads `as_of` and
//! `method` off every [`AnchorValue`].

use chrono::NaiveDate;
use dravr_cageux::periodization::{
    AnchorConfidence, AnchorValue, AthleteAnchors, RelativeIntensity, SportAnchors, SportFamily,
    ThresholdBasis, WorkoutStep,
};
use serde::{Deserialize, Serialize};

use super::measurement::{AnchorOrigin, MeasurementKind, MetricProvenance, ProvenancedValue};
use super::{SportType, UserPhysiologicalProfile};
use crate::constants::units::METERS_PER_KM;

/// The unit an anchor's value, low and high are in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnchorUnit {
    /// Watts.
    Watts,
    /// Metres per second.
    MetresPerSecond,
    /// Beats per minute.
    BeatsPerMinute,
}

/// Which anchor of a sport family a stored row holds: a field of cageux's
/// [`SportAnchors`], less the resting heart rate, which is the athlete's
/// rather than a sport's and stays on the profile row.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub enum SportAnchorName {
    /// Functional threshold power, watts.
    #[serde(rename = "ftp")]
    Ftp,
    /// Critical power, watts.
    #[serde(rename = "critical_power")]
    CriticalPower,
    /// Maximal aerobic power, watts; its `method` names the definition it
    /// follows.
    #[serde(rename = "map")]
    Map,
    /// Critical speed, metres per second.
    #[serde(rename = "critical_speed")]
    CriticalSpeed,
    /// Speed at `VO2max`, or maximal aerobic speed, metres per second.
    #[serde(rename = "vvo2max")]
    Vvo2max,
    /// Threshold pace as a speed, metres per second.
    #[serde(rename = "threshold_speed")]
    ThresholdSpeed,
    /// Maximum heart rate reached in this sport, bpm.
    #[serde(rename = "max_heart_rate")]
    MaxHeartRate,
    /// Lactate-threshold heart rate in this sport, bpm.
    #[serde(rename = "threshold_heart_rate")]
    ThresholdHeartRate,
}

impl SportAnchorName {
    /// Every anchor, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Ftp,
        Self::CriticalPower,
        Self::Map,
        Self::CriticalSpeed,
        Self::Vvo2max,
        Self::ThresholdSpeed,
        Self::MaxHeartRate,
        Self::ThresholdHeartRate,
    ];

    /// The stored and serialized name, byte for byte what serde reads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ftp => "ftp",
            Self::CriticalPower => "critical_power",
            Self::Map => "map",
            Self::CriticalSpeed => "critical_speed",
            Self::Vvo2max => "vvo2max",
            Self::ThresholdSpeed => "threshold_speed",
            Self::MaxHeartRate => "max_heart_rate",
            Self::ThresholdHeartRate => "threshold_heart_rate",
        }
    }

    /// The anchor a stored name denotes; `None` for a name no build knows.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|anchor| anchor.as_str() == name)
    }

    /// The unit the anchor is in.
    #[must_use]
    pub const fn unit(self) -> AnchorUnit {
        match self {
            Self::Ftp | Self::CriticalPower | Self::Map => AnchorUnit::Watts,
            Self::CriticalSpeed | Self::Vvo2max | Self::ThresholdSpeed => {
                AnchorUnit::MetresPerSecond
            }
            Self::MaxHeartRate | Self::ThresholdHeartRate => AnchorUnit::BeatsPerMinute,
        }
    }

    /// Whether `sport` can hold this anchor. Power anchors belong to the bike
    /// and to a run power meter, never to the pool; speed anchors belong to
    /// the run and the pool, where speed is what the athlete controls, never
    /// to the bike; heart rate belongs to every sport.
    #[must_use]
    pub const fn offered_in(self, sport: SportFamily) -> bool {
        match self.unit() {
            AnchorUnit::Watts => !matches!(sport, SportFamily::Swim),
            AnchorUnit::MetresPerSecond => !matches!(sport, SportFamily::Ride),
            AnchorUnit::BeatsPerMinute => true,
        }
    }

    /// The value this anchor holds in one sport family's anchors.
    #[must_use]
    pub const fn get(self, anchors: &SportAnchors) -> Option<&AnchorValue> {
        match self {
            Self::Ftp => anchors.ftp.as_ref(),
            Self::CriticalPower => anchors.critical_power.as_ref(),
            Self::Map => anchors.map.as_ref(),
            Self::CriticalSpeed => anchors.critical_speed.as_ref(),
            Self::Vvo2max => anchors.vvo2max.as_ref(),
            Self::ThresholdSpeed => anchors.threshold_speed.as_ref(),
            Self::MaxHeartRate => anchors.max_heart_rate.as_ref(),
            Self::ThresholdHeartRate => anchors.threshold_heart_rate.as_ref(),
        }
    }

    /// The slot this anchor fills in one sport family's anchors, for writing.
    const fn get_mut(self, anchors: &mut SportAnchors) -> &mut Option<AnchorValue> {
        match self {
            Self::Ftp => &mut anchors.ftp,
            Self::CriticalPower => &mut anchors.critical_power,
            Self::Map => &mut anchors.map,
            Self::CriticalSpeed => &mut anchors.critical_speed,
            Self::Vvo2max => &mut anchors.vvo2max,
            Self::ThresholdSpeed => &mut anchors.threshold_speed,
            Self::MaxHeartRate => &mut anchors.max_heart_rate,
            Self::ThresholdHeartRate => &mut anchors.threshold_heart_rate,
        }
    }

    /// The profile-row field this anchor mirrors in `sport`, when the athlete
    /// trains `primary` as their main sport: the inverse of
    /// [`ProfileTwin::anchor`], which holds the one mapping.
    #[must_use]
    pub fn profile_twin(
        self,
        sport: SportFamily,
        primary: Option<SportFamily>,
    ) -> Option<ProfileTwin> {
        ProfileTwin::ALL
            .into_iter()
            .find(|twin| twin.anchor(primary) == Some((sport, self)))
    }

    /// The stored anchor a target's percent basis is a percentage of; `None`
    /// for heart-rate reserve and best power for a duration, which no single
    /// stored anchor holds.
    const fn of_basis(basis: ThresholdBasis) -> Option<Self> {
        match basis {
            ThresholdBasis::Ftp => Some(Self::Ftp),
            ThresholdBasis::HeartRate => Some(Self::ThresholdHeartRate),
            ThresholdBasis::Pace => Some(Self::ThresholdSpeed),
            ThresholdBasis::CriticalPower => Some(Self::CriticalPower),
            ThresholdBasis::CriticalSpeed => Some(Self::CriticalSpeed),
            ThresholdBasis::Map => Some(Self::Map),
            ThresholdBasis::VVo2max => Some(Self::Vvo2max),
            ThresholdBasis::MaxHeartRate => Some(Self::MaxHeartRate),
            ThresholdBasis::HeartRateReserve | ThresholdBasis::Mmp { .. } => None,
        }
    }
}

/// The athlete's anchor a session's targets are written on, as it stood when
/// the session was measured: what a notice about the session names ("FTP
/// 285 W from intervals.icu"), with how far to trust it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TargetAnchor {
    /// Which anchor the targets are a percentage of.
    pub anchor: SportAnchorName,
    /// Its value with range, origin, day, method and confidence.
    #[serde(flatten)]
    pub value: AnchorValue,
    /// Whether the conversions between this sport's anchors (MAP to CP, CP
    /// to FTP, `vVO2max` to CS) were the athlete's own rather than the
    /// default ratios.
    pub own_conversions: bool,
}

/// The anchor `steps` for `sport` are written on.
///
/// It is the basis of the first step whose target is a percentage of an
/// anchor, when the athlete holds that anchor in the sport's family. `None`
/// for a sport no family covers, or targets that name no anchor the athlete
/// holds.
#[must_use]
pub fn target_anchor(
    steps: &[WorkoutStep],
    sport: &SportType,
    anchors: &AthleteAnchors,
) -> Option<TargetAnchor> {
    let family = SportFamily::of(sport)?;
    let held = anchors.for_family(family);
    let name =
        steps
            .iter()
            .find_map(|step| match RelativeIntensity::parse(&step.target_zone)? {
                RelativeIntensity::Percent {
                    threshold: Some(basis),
                    ..
                } => SportAnchorName::of_basis(basis),
                _ => None,
            })?;
    Some(TargetAnchor {
        anchor: name,
        value: name.get(held)?.clone(),
        own_conversions: !held.ratios().defaults_only,
    })
}

/// A profile-row field that also stands for one sport family's anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileTwin {
    /// `ftp_watts`: the bike's FTP.
    FtpWatts,
    /// `critical_power_watts`: the bike's critical power.
    CriticalPowerWatts,
    /// `critical_speed_mps`: the run's critical speed.
    CriticalSpeedMps,
    /// `threshold_pace_sec_per_km`: the run's threshold pace.
    ThresholdPaceSecPerKm,
    /// `max_hr`: the main sport's maximum heart rate.
    MaxHr,
    /// `threshold_hr`: the main sport's threshold heart rate.
    ThresholdHr,
}

impl ProfileTwin {
    /// Every twin, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::FtpWatts,
        Self::CriticalPowerWatts,
        Self::CriticalSpeedMps,
        Self::ThresholdPaceSecPerKm,
        Self::MaxHr,
        Self::ThresholdHr,
    ];

    /// The profile field's name, as `set_physiology` takes it.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::FtpWatts => "ftp_watts",
            Self::CriticalPowerWatts => "critical_power_watts",
            Self::CriticalSpeedMps => "critical_speed_mps",
            Self::ThresholdPaceSecPerKm => "threshold_pace_sec_per_km",
            Self::MaxHr => "max_hr",
            Self::ThresholdHr => "threshold_hr",
        }
    }

    /// The sport family and anchor this field stands for, given the athlete's
    /// main sport; `None` for a heart rate when the main sport has no family.
    /// The profile row's power values are the bike's and its speeds the
    /// run's; its heart rates are one value for every sport, which rule I9
    /// lets stand only for the sport the athlete mostly trains.
    #[must_use]
    pub const fn anchor(
        self,
        primary: Option<SportFamily>,
    ) -> Option<(SportFamily, SportAnchorName)> {
        match self {
            Self::FtpWatts => Some((SportFamily::Ride, SportAnchorName::Ftp)),
            Self::CriticalPowerWatts => Some((SportFamily::Ride, SportAnchorName::CriticalPower)),
            Self::CriticalSpeedMps => Some((SportFamily::Run, SportAnchorName::CriticalSpeed)),
            Self::ThresholdPaceSecPerKm => {
                Some((SportFamily::Run, SportAnchorName::ThresholdSpeed))
            }
            Self::MaxHr => match primary {
                Some(sport) => Some((sport, SportAnchorName::MaxHeartRate)),
                None => None,
            },
            Self::ThresholdHr => match primary {
                Some(sport) => Some((sport, SportAnchorName::ThresholdHeartRate)),
                None => None,
            },
        }
    }

    /// The value the profile row holds for this field, as an anchor: in the
    /// anchor's unit, with the origin and day the row records and
    /// [`AnchorConfidence::Low`], since the row names no test protocol.
    fn profile_value(self, profile: &UserPhysiologicalProfile) -> Option<AnchorValue> {
        let provenance = &profile.threshold_provenance;
        let stated = &profile.threshold_stated_on;
        match self {
            Self::FtpWatts => profile.ftp_watts.map(|watts| {
                threshold_anchor(
                    f64::from(watts),
                    provenance.ftp_watts.as_ref(),
                    stated.ftp_watts,
                )
            }),
            Self::CriticalPowerWatts => profile
                .critical_power_watts
                .as_ref()
                .map(|cp| provenanced_anchor(f64::from(cp.value), cp)),
            Self::CriticalSpeedMps => profile
                .critical_speed_mps
                .as_ref()
                .map(|cs| provenanced_anchor(cs.value, cs)),
            Self::ThresholdPaceSecPerKm => profile
                .threshold_pace_sec_per_km
                .filter(|pace| *pace > 0.0)
                .map(|pace| {
                    threshold_anchor(
                        METERS_PER_KM / pace,
                        provenance.threshold_pace_sec_per_km.as_ref(),
                        stated.threshold_pace_sec_per_km,
                    )
                }),
            Self::MaxHr => profile.max_hr.map(|bpm| {
                threshold_anchor(f64::from(bpm), provenance.max_hr.as_ref(), stated.max_hr)
            }),
            Self::ThresholdHr => profile.threshold_hr.map(|bpm| {
                threshold_anchor(
                    f64::from(bpm),
                    provenance.threshold_hr.as_ref(),
                    stated.threshold_hr,
                )
            }),
        }
    }
}

/// A profile-row threshold as an anchor: the provider's origin and day when an
/// import wrote it, the athlete's own and the day they stated it otherwise.
fn threshold_anchor(
    value: f64,
    import: Option<&MetricProvenance>,
    stated_on: Option<NaiveDate>,
) -> AnchorValue {
    let (origin, as_of) = import.map_or_else(
        || (AnchorOrigin::Athlete.as_str().to_owned(), stated_on),
        |provenance| {
            (
                provenance
                    .origin
                    .clone()
                    .unwrap_or_else(|| AnchorOrigin::Imported.as_str().to_owned()),
                provenance.as_of,
            )
        },
    );
    row_anchor(value, Some(origin), as_of)
}

/// A profile-row provenanced value as an anchor, keeping its origin and day.
fn provenanced_anchor<T>(value: f64, stored: &ProvenancedValue<T>) -> AnchorValue {
    row_anchor(
        value,
        stored.provenance.origin.clone(),
        stored.provenance.as_of,
    )
}

/// An anchor read off the profile row: a point with no stated range or
/// method, at low confidence.
fn row_anchor(value: f64, origin: Option<String>, as_of: Option<NaiveDate>) -> AnchorValue {
    // A physiological magnitude far inside f32.
    #[allow(clippy::cast_possible_truncation)]
    let value = value as f32;
    AnchorValue {
        value,
        low: None,
        high: None,
        origin,
        as_of,
        method: None,
        confidence: AnchorConfidence::Low,
    }
}

/// One stored anchor: one athlete's value of one anchor in one sport family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SportAnchor {
    /// The sport family it belongs to.
    pub sport: SportFamily,
    /// Which anchor it is.
    pub anchor: SportAnchorName,
    /// Measured by a named test, or estimated: how it may be framed.
    pub kind: MeasurementKind,
    /// The value with its range, origin, day, method and confidence.
    #[serde(flatten)]
    pub value: AnchorValue,
}

/// One sport family's anchors in `anchors`, for writing.
const fn family_mut(anchors: &mut AthleteAnchors, sport: SportFamily) -> &mut SportAnchors {
    match sport {
        SportFamily::Ride => &mut anchors.ride,
        SportFamily::Run => &mut anchors.run,
        SportFamily::Swim => &mut anchors.swim,
    }
}

/// The anchors cageux resolves `profile`'s athlete's workout targets on.
///
/// Each profile-row value lands on the one family it belongs to (rule I9);
/// the resting heart rate, the athlete's rather than a sport's, lands on all
/// three. A stored per-sport anchor then replaces whatever the row put in its
/// slot: it carries a stated range, method and confidence the row cannot.
#[must_use]
pub fn athlete_anchors(
    profile: &UserPhysiologicalProfile,
    stored: &[SportAnchor],
) -> AthleteAnchors {
    let primary = SportFamily::of(&profile.primary_sport);
    let mut anchors = AthleteAnchors::default();
    for twin in ProfileTwin::ALL {
        let (Some((sport, name)), Some(value)) =
            (twin.anchor(primary), twin.profile_value(profile))
        else {
            continue;
        };
        // A heart rate the row holds as the mirror of another sport's stored
        // anchor still describes that sport: the athlete changed main sport
        // after stating it, and rule I9 keeps it off the new one.
        let mirrors_another_sport = matches!(twin, ProfileTwin::MaxHr | ProfileTwin::ThresholdHr)
            && stored.iter().any(|held| {
                held.anchor == name
                    && held.sport != sport
                    && (held.value.value.round() - value.value).abs() < 0.5
            });
        if mirrors_another_sport {
            continue;
        }
        *name.get_mut(family_mut(&mut anchors, sport)) = Some(value);
    }
    if let Some(bpm) = profile.resting_hr {
        let stamp = profile.anchor_provenance.resting_hr;
        let resting = row_anchor(
            f64::from(bpm),
            Some(
                stamp
                    .map_or(AnchorOrigin::Athlete, |stamp| stamp.origin)
                    .as_str()
                    .to_owned(),
            ),
            stamp.and_then(|stamp| stamp.as_of),
        );
        for sport in SportFamily::ALL {
            family_mut(&mut anchors, *sport).resting_heart_rate = Some(resting.clone());
        }
    }
    for anchor in stored {
        *anchor
            .anchor
            .get_mut(family_mut(&mut anchors, anchor.sport)) = Some(anchor.value.clone());
    }
    anchors
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::models::{AnchorStamp, SportType};
    use uuid::Uuid;

    type TestResult = Result<(), Box<dyn Error>>;

    fn day(year: i32, month: u32, date: u32) -> Result<NaiveDate, Box<dyn Error>> {
        NaiveDate::from_ymd_opt(year, month, date).ok_or_else(|| "fixture date".into())
    }

    fn stored(
        sport: SportFamily,
        anchor: SportAnchorName,
        value: f32,
    ) -> Result<SportAnchor, Box<dyn Error>> {
        Ok(SportAnchor {
            sport,
            anchor,
            kind: MeasurementKind::Measured,
            value: AnchorValue {
                value,
                low: Some(value * 0.95),
                high: Some(value * 1.05),
                origin: Some("lab".to_owned()),
                as_of: Some(day(2026, 11, 1)?),
                method: Some("ramp_peak".to_owned()),
                confidence: AnchorConfidence::High,
            },
        })
    }

    #[test]
    fn every_anchor_name_round_trips_through_serde_and_parse() -> TestResult {
        for anchor in SportAnchorName::ALL {
            let json = serde_json::to_value(anchor)?;
            assert_eq!(json, serde_json::json!(anchor.as_str()));
            assert_eq!(SportAnchorName::parse(anchor.as_str()), Some(anchor));
        }
        assert_eq!(SportAnchorName::parse("resting_heart_rate"), None);
        Ok(())
    }

    #[test]
    fn power_never_reaches_the_pool_and_speed_never_reaches_the_bike() {
        assert!(SportAnchorName::Map.offered_in(SportFamily::Run));
        assert!(!SportAnchorName::Map.offered_in(SportFamily::Swim));
        assert!(SportAnchorName::CriticalSpeed.offered_in(SportFamily::Swim));
        assert!(!SportAnchorName::CriticalSpeed.offered_in(SportFamily::Ride));
        assert!(SportAnchorName::MaxHeartRate.offered_in(SportFamily::Swim));
    }

    #[test]
    fn profile_row_values_land_on_their_own_sport_at_low_confidence() -> TestResult {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Ride);
        profile.ftp_watts = Some(285);
        profile.threshold_stated_on.ftp_watts = Some(day(2026, 10, 20)?);
        profile.threshold_pace_sec_per_km = Some(250.0);
        profile.max_hr = Some(188);
        profile.resting_hr = Some(48);
        profile.anchor_provenance.resting_hr = Some(AnchorStamp::athlete(day(2026, 11, 2)?));

        let anchors = athlete_anchors(&profile, &[]);

        let ftp = anchors.ride.ftp.as_ref().ok_or("no bike FTP")?;
        assert!((ftp.value - 285.0).abs() < f32::EPSILON);
        assert_eq!(ftp.confidence, AnchorConfidence::Low);
        assert_eq!(ftp.origin.as_deref(), Some("athlete"));
        assert_eq!(ftp.as_of, Some(day(2026, 10, 20)?));
        assert!(anchors.run.ftp.is_none(), "bug: the bike FTP set run power");

        let speed = anchors
            .run
            .threshold_speed
            .as_ref()
            .ok_or("no run threshold")?;
        assert!((speed.value - 4.0).abs() < 1e-4, "250 s/km is 4 m/s");

        assert!(anchors.ride.max_heart_rate.is_some());
        assert!(
            anchors.run.max_heart_rate.is_none(),
            "bug: a max heart rate crossed sports (rule I9)"
        );
        for family in [&anchors.ride, &anchors.run, &anchors.swim] {
            let resting = family.resting_heart_rate.as_ref().ok_or("no resting HR")?;
            assert_eq!(resting.as_of, Some(day(2026, 11, 2)?));
        }
        Ok(())
    }

    #[test]
    fn an_imported_threshold_keeps_its_provider_and_day() -> TestResult {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run);
        profile.ftp_watts = Some(270);
        profile.threshold_provenance.ftp_watts = Some(MetricProvenance {
            kind: MeasurementKind::Estimated,
            origin: Some("intervals.icu".to_owned()),
            as_of: Some(day(2026, 9, 30)?),
        });
        let ftp = athlete_anchors(&profile, &[])
            .ride
            .ftp
            .ok_or("no bike FTP")?;
        assert_eq!(ftp.origin.as_deref(), Some("intervals.icu"));
        assert_eq!(ftp.as_of, Some(day(2026, 9, 30)?));
        Ok(())
    }

    #[test]
    fn a_stored_anchor_replaces_the_row_and_makes_the_ratio_the_athletes_own() -> TestResult {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Ride);
        profile.critical_power_watts = Some(ProvenancedValue::new(
            280,
            MetricProvenance {
                kind: MeasurementKind::Measured,
                origin: Some("3-min all-out test".to_owned()),
                as_of: Some(day(2026, 10, 1)?),
            },
        ));
        assert!(athlete_anchors(&profile, &[]).ride.ratios().defaults_only);

        let anchors = athlete_anchors(
            &profile,
            &[
                stored(SportFamily::Ride, SportAnchorName::Map, 378.0)?,
                stored(SportFamily::Run, SportAnchorName::MaxHeartRate, 194.0)?,
            ],
        );
        let ratios = anchors.ride.ratios();
        assert!(!ratios.defaults_only);
        assert!((ratios.map_over_cp - 1.35).abs() < 1e-3, "378 / 280");
        assert_eq!(
            anchors.run.max_heart_rate.as_ref().map(|hr| hr.confidence),
            Some(AnchorConfidence::High)
        );
        Ok(())
    }

    #[test]
    fn a_heart_rate_mirrored_from_the_old_main_sport_stays_off_the_new_one() -> TestResult {
        // Stated as the run's max while running was the main sport, which
        // mirrored it onto the row; the athlete then made cycling the main sport.
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Ride);
        profile.max_hr = Some(194);
        let anchors = athlete_anchors(
            &profile,
            &[stored(
                SportFamily::Run,
                SportAnchorName::MaxHeartRate,
                194.0,
            )?],
        );
        assert!(
            anchors.ride.max_heart_rate.is_none(),
            "bug: the run's max heart rate reached the bike (rule I9)"
        );
        assert!(anchors.run.max_heart_rate.is_some());

        profile.max_hr = Some(186);
        let own = athlete_anchors(
            &profile,
            &[stored(
                SportFamily::Run,
                SportAnchorName::MaxHeartRate,
                194.0,
            )?],
        );
        assert_eq!(
            own.ride.max_heart_rate.as_ref().map(|hr| hr.value.round()),
            Some(186.0),
            "a row value of the main sport's own still lands on it"
        );
        Ok(())
    }

    #[test]
    fn a_heart_rate_only_reaches_a_main_sport_with_a_family() {
        let mut profile =
            UserPhysiologicalProfile::new(Uuid::nil(), SportType::Other("rowing".to_owned()));
        profile.max_hr = Some(190);
        let anchors = athlete_anchors(&profile, &[]);
        assert!(anchors.ride.max_heart_rate.is_none());
        assert!(anchors.run.max_heart_rate.is_none());
        assert!(anchors.swim.max_heart_rate.is_none());
    }

    fn step(target: &str) -> WorkoutStep {
        WorkoutStep {
            label: "Work".to_owned(),
            duration_seconds: 30,
            distance_meters: None,
            target_zone: target.to_owned(),
            repeat: 1,
            repeat_group: None,
            note: None,
        }
    }

    #[test]
    fn a_session_names_the_held_anchor_its_targets_are_written_on() -> TestResult {
        let mut profile = UserPhysiologicalProfile::new(Uuid::nil(), SportType::Ride);
        profile.ftp_watts = Some(285);
        let row_only = athlete_anchors(&profile, &[]);
        let steps = [step("Z2"), step("100-110% MAP"), step("50% FTP")];
        assert_eq!(
            target_anchor(&steps, &SportType::Ride, &row_only),
            None,
            "the first anchor named is MAP, which the athlete does not hold"
        );

        let held = athlete_anchors(
            &profile,
            &[stored(SportFamily::Ride, SportAnchorName::Map, 380.0)?],
        );
        let anchor = target_anchor(&steps, &SportType::GravelRide, &held).ok_or("MAP anchor")?;
        assert_eq!(anchor.anchor, SportAnchorName::Map);
        assert!((anchor.value.value - 380.0).abs() < f32::EPSILON);
        assert!(
            !anchor.own_conversions,
            "MAP alone states one end of no ratio, so conversions stay the defaults"
        );

        let ftp_steps = [step("119% FTP")];
        let ftp = target_anchor(&ftp_steps, &SportType::Ride, &row_only).ok_or("FTP anchor")?;
        assert_eq!(ftp.anchor, SportAnchorName::Ftp);
        assert_eq!(ftp.value.origin.as_deref(), Some("athlete"));
        assert_eq!(target_anchor(&ftp_steps, &SportType::Yoga, &row_only), None);
        Ok(())
    }

    #[test]
    fn each_twin_names_the_anchor_it_mirrors() -> TestResult {
        let primary = Some(SportFamily::Run);
        for twin in ProfileTwin::ALL {
            let (sport, name) = twin.anchor(primary).ok_or("a run athlete's twin")?;
            assert_eq!(
                name.profile_twin(sport, primary),
                Some(twin),
                "{}",
                twin.field()
            );
        }
        assert_eq!(
            SportAnchorName::MaxHeartRate.profile_twin(SportFamily::Ride, primary),
            None,
            "bug: a non-main sport's heart rate mirrored onto the row"
        );
        Ok(())
    }
}
