// ABOUTME: Measured-vs-estimated provenance for a physiological value — the kind, who produced it, and when
// ABOUTME: A provenanced value never exists without its kind, and a provider-supplied threshold always names its provider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeSet;

use dravr_tronc::dates::CivilDate;
use serde::{Deserialize, Serialize};

use crate::ai_policy::SourceClass;

use super::UserPhysiologicalProfile;

/// The origin a value carries when the athlete stated it without naming the
/// test or the source behind it. It attributes nothing, so a sentence that
/// mentions it has not named a source.
pub const ATHLETE_REPORTED_ORIGIN: &str = "athlete-reported";

/// The origin of a per-sport anchor the athlete's human coach set.
///
/// The coach sets it from their view of the athlete (carnet#1561). A
/// persisted value: stored on the anchor row, read back by every reader.
pub const COACH_STATED_ORIGIN: &str = "coach";

/// Whether a physiological value was measured or estimated.
///
/// This is what decides how the value may be framed. A measured critical
/// power is "your CP is 312 W"; an estimated one is "Vekta estimates your CP
/// at 312 W", never stated bare. Where the value came from is a separate axis
/// ([`MetricProvenance::origin`]): a provider can hand over either kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementKind {
    /// Produced by a named test: a lab protocol, a 3-minute all-out test, a
    /// set of time trials fitted to the model, a track test.
    Measured,
    /// Produced by a model from training data (a provider's or an app's), or
    /// stated by the athlete without naming the test behind it.
    Estimated,
}

impl MeasurementKind {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Estimated => "estimated",
        }
    }

    /// Parse the DB string form. Anything but `measured` reads as
    /// [`Self::Estimated`]: a value whose kind cannot be read must never be
    /// presented as a measurement.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        if s == "measured" {
            Self::Measured
        } else {
            Self::Estimated
        }
    }
}

/// Where a physiological value came from and how much it can be trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MetricProvenance {
    /// Measured or estimated — the framing axis.
    pub kind: MeasurementKind,
    /// Who produced it: a provider (`vekta`, `intervals.icu`), a test (`lab`,
    /// `3-min all-out test`), or `athlete-reported`. The attribution an
    /// estimate is quoted with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The day the value was measured or estimated, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<CivilDate>,
}

/// A physiological value together with its [`MetricProvenance`].
///
/// The pairing is the invariant: there is no way to hold one of these
/// values without saying whether it was measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProvenancedValue<T> {
    /// The value, in the unit its field name carries.
    pub value: T,
    /// How the value was obtained.
    #[serde(flatten)]
    pub provenance: MetricProvenance,
}

impl<T> ProvenancedValue<T> {
    /// Pair a value with its provenance.
    #[must_use]
    pub const fn new(value: T, provenance: MetricProvenance) -> Self {
        Self { value, provenance }
    }
}

/// Where each of the profile's four scalar thresholds came from, for the ones
/// a provider supplied.
///
/// A threshold with no entry here is the athlete's own. That covers every
/// value stored before provenance was recorded and every value
/// `set_physiology` writes. An entry is written only by an import from a
/// provider, always [`MeasurementKind::Estimated`] with the provider as its
/// origin and the day the provider's setting last changed: the framing is
/// "intervals.icu has your FTP at 285 W", never a bare "your FTP is 285 W".
/// An import refreshes only an entry its own origin wrote, so a value the
/// athlete set is never replaced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ThresholdProvenance {
    /// Where the functional threshold power came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp_watts: Option<MetricProvenance>,
    /// Where the lactate threshold heart rate came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_hr: Option<MetricProvenance>,
    /// Where the maximum heart rate came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hr: Option<MetricProvenance>,
    /// Where the threshold pace came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_pace_sec_per_km: Option<MetricProvenance>,
}

impl ThresholdProvenance {
    /// Every threshold the athlete's own.
    pub(crate) const NONE: Self = Self {
        ftp_watts: None,
        threshold_hr: None,
        max_hr: None,
        threshold_pace_sec_per_km: None,
    };

    /// Whether every threshold is the athlete's own.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ftp_watts.is_none()
            && self.threshold_hr.is_none()
            && self.max_hr.is_none()
            && self.threshold_pace_sec_per_km.is_none()
    }
}

/// Who set one of the profile's training anchors: its resting heart rate,
/// `VO2max` or weight (Athlete Data Consolidation §5, DECISION 5).
///
/// The anchors are what stored training load and the heart-rate zones are
/// scored against, so which number fills one is decided once and recorded:
/// only an anchor nobody vouches for — an estimate, or one an earlier
/// promotion wrote — is ever refreshed from the athlete's measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnchorOrigin {
    /// The athlete stated it (`set_physiology`).
    Athlete,
    /// Brought in from another system that holds the athlete's own number.
    Imported,
    /// A model's estimate standing in until a measurement exists.
    Estimated,
    /// Promoted from the athlete's own measurements by the weekly promotion.
    Promoted,
}

impl AnchorOrigin {
    /// Every origin, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Athlete,
        Self::Imported,
        Self::Estimated,
        Self::Promoted,
    ];

    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Athlete => "athlete",
            Self::Imported => "imported",
            Self::Estimated => "estimated",
            Self::Promoted => "promoted",
        }
    }

    /// The origin a stored origin string names: one of the forms
    /// [`Self::as_str`] writes, or [`ATHLETE_REPORTED_ORIGIN`], which a value
    /// the athlete stated without naming its source carries. `None` for
    /// anything else, which is a provider, an app or a test named as the
    /// origin (`intervals.icu`, `lab`).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        if s == ATHLETE_REPORTED_ORIGIN {
            return Some(Self::Athlete);
        }
        Self::ALL.into_iter().find(|origin| origin.as_str() == s)
    }

    /// Parse the DB string form. Anything unrecognised reads as
    /// [`Self::Athlete`]: an anchor whose origin cannot be read must never be
    /// overwritten automatically.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        Self::parse(s).unwrap_or(Self::Athlete)
    }

    /// Whether the weekly promotion may replace an anchor of this origin:
    /// only one nobody vouches for. An athlete's or an imported number stands.
    #[must_use]
    pub const fn is_promotable(self) -> bool {
        matches!(self, Self::Estimated | Self::Promoted)
    }
}

/// Where one training anchor came from and as of which day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AnchorStamp {
    /// Who set it.
    pub origin: AnchorOrigin,
    /// The day it was stated, imported, estimated or measured, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<CivilDate>,
}

impl AnchorStamp {
    /// The stamp of a number the athlete stated on `as_of`.
    #[must_use]
    pub const fn athlete(as_of: CivilDate) -> Self {
        Self {
            origin: AnchorOrigin::Athlete,
            as_of: Some(as_of),
        }
    }

    /// The stamp of a number promoted from a measurement taken on `as_of`.
    #[must_use]
    pub const fn promoted(as_of: CivilDate) -> Self {
        Self {
            origin: AnchorOrigin::Promoted,
            as_of: Some(as_of),
        }
    }
}

/// Where each of the profile's three training anchors came from.
///
/// An anchor with no entry holds no value, or holds one stored before origins
/// were recorded — which only `set_physiology` and the athlete's own
/// configuration ever wrote — so it reads as the athlete's own. An entry is
/// kept only beside a value.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct AnchorProvenance {
    /// Where the resting heart rate came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resting_hr: Option<AnchorStamp>,
    /// Where the `VO2max` came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vo2_max: Option<AnchorStamp>,
    /// Where the weight came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<AnchorStamp>,
}

impl AnchorProvenance {
    /// No anchor carries a recorded origin.
    pub(crate) const NONE: Self = Self {
        resting_hr: None,
        vo2_max: None,
        weight: None,
    };

    /// Whether no anchor carries a recorded origin.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.resting_hr.is_none() && self.vo2_max.is_none() && self.weight.is_none()
    }

    /// The origin of an anchor holding a value: its recorded one, else the
    /// athlete's (the value predates origins). `None` when it holds no value.
    fn origin_of(has_value: bool, stamp: Option<&AnchorStamp>) -> Option<AnchorOrigin> {
        has_value.then(|| stamp.map_or(AnchorOrigin::Athlete, |stamp| stamp.origin))
    }

    /// Whether the weekly promotion may fill or replace an anchor: it holds no
    /// value, or one whose origin [`AnchorOrigin::is_promotable`].
    #[must_use]
    pub fn promotable(has_value: bool, stamp: Option<&AnchorStamp>) -> bool {
        Self::origin_of(has_value, stamp).is_none_or(AnchorOrigin::is_promotable)
    }
}

/// The day the athlete stated each of the profile's four scalar thresholds.
///
/// [`ThresholdProvenance`] describes only a provider's import, and its absence
/// is what marks a value as the athlete's own, which an import never replaces.
/// So the day an athlete-set value was stated is kept apart: an entry here
/// exists exactly when the value is the athlete's own and was stated after
/// this was recorded (carnet#1496). A threshold with neither entry predates
/// both records, and its day is unknown.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct ThresholdStatedOn {
    /// The day the athlete stated their functional threshold power.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp_watts: Option<CivilDate>,
    /// The day the athlete stated their lactate threshold heart rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_hr: Option<CivilDate>,
    /// The day the athlete stated their maximum heart rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hr: Option<CivilDate>,
    /// The day the athlete stated their threshold pace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_pace_sec_per_km: Option<CivilDate>,
}

impl ThresholdStatedOn {
    /// No threshold carries a stated day.
    pub(crate) const NONE: Self = Self {
        ftp_watts: None,
        threshold_hr: None,
        max_hr: None,
        threshold_pace_sec_per_km: None,
    };

    /// Whether no threshold carries a stated day.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ftp_watts.is_none()
            && self.threshold_hr.is_none()
            && self.max_hr.is_none()
            && self.threshold_pace_sec_per_km.is_none()
    }
}

impl AnchorOrigin {
    /// The trust label's class of an anchor of this origin: the athlete's
    /// own statement, or — imported, estimated or promoted from measurements
    /// whose service the anchor does not record — the class that claims the
    /// least.
    #[must_use]
    pub const fn source_class(self) -> SourceClass {
        match self {
            Self::Athlete => SourceClass::PatientEntered,
            Self::Imported | Self::Estimated | Self::Promoted => SourceClass::ThirdPartyApp,
        }
    }
}

impl MetricProvenance {
    /// The trust label's class of a value the athlete stated with this
    /// provenance (`set_physiology`): theirs, whatever test they named, unless
    /// their human coach set it ([`COACH_STATED_ORIGIN`]), which is not the
    /// athlete's own record and claims the least.
    #[must_use]
    pub fn stated_class(&self) -> SourceClass {
        if self.origin.as_deref() == Some(COACH_STATED_ORIGIN) {
            SourceClass::ThirdPartyApp
        } else {
            SourceClass::PatientEntered
        }
    }
}

impl UserPhysiologicalProfile {
    /// Where the values this profile holds came from, for the trust label of
    /// a result that served it (owl phase 1); empty when it holds none.
    ///
    /// - A threshold with an import entry ([`ThresholdProvenance`]) is a
    ///   provider's setting, another application's; one without is the
    ///   athlete's own.
    /// - An anchor takes its origin's class ([`AnchorOrigin::source_class`]),
    ///   the athlete's when none was recorded.
    /// - A critical-power or critical-speed value takes its stated
    ///   provenance's class ([`MetricProvenance::stated_class`]).
    /// - The lactate fraction, age and training years are written only by
    ///   `set_physiology`, so they are the athlete's. A sex is the athlete's
    ///   or what a provider reported of them, which the row does not tell
    ///   apart, so it claims the least.
    ///
    /// Zones are derived by Dravr from the thresholds, and the sport and
    /// fitness level are defaults every row carries; none adds a class.
    #[must_use]
    pub fn source_classes(&self) -> BTreeSet<SourceClass> {
        let imported = &self.threshold_provenance;
        let thresholds = [
            (self.ftp_watts.is_some(), imported.ftp_watts.is_some()),
            (self.threshold_hr.is_some(), imported.threshold_hr.is_some()),
            (self.max_hr.is_some(), imported.max_hr.is_some()),
            (
                self.threshold_pace_sec_per_km.is_some(),
                imported.threshold_pace_sec_per_km.is_some(),
            ),
        ]
        .into_iter()
        .filter(|(held, _)| *held)
        .map(|(_, imported)| {
            if imported {
                SourceClass::ThirdPartyApp
            } else {
                SourceClass::PatientEntered
            }
        });
        let stamps = &self.anchor_provenance;
        let anchors = [
            AnchorProvenance::origin_of(self.resting_hr.is_some(), stamps.resting_hr.as_ref()),
            AnchorProvenance::origin_of(self.vo2_max.is_some(), stamps.vo2_max.as_ref()),
            AnchorProvenance::origin_of(self.weight.is_some(), stamps.weight.as_ref()),
        ]
        .into_iter()
        .flatten()
        .map(AnchorOrigin::source_class);
        let stated = [
            self.critical_power_watts.as_ref().map(|v| &v.provenance),
            self.w_prime_joules.as_ref().map(|v| &v.provenance),
            self.critical_speed_mps.as_ref().map(|v| &v.provenance),
            self.d_prime_meters.as_ref().map(|v| &v.provenance),
        ]
        .into_iter()
        .flatten()
        .map(MetricProvenance::stated_class);
        let athletes_own = (self.lactate_threshold_percentage.is_some()
            || self.age.is_some()
            || self.training_experience_years.is_some())
        .then_some(SourceClass::PatientEntered);
        let reported_sex = self.sex.is_some().then_some(SourceClass::ThirdPartyApp);
        thresholds
            .chain(anchors)
            .chain(stated)
            .chain(athletes_own)
            .chain(reported_sex)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn an_unreadable_kind_is_an_estimate_never_a_measurement() {
        assert_eq!(
            MeasurementKind::parse_lenient("measured"),
            MeasurementKind::Measured
        );
        assert_eq!(
            MeasurementKind::parse_lenient("estimated"),
            MeasurementKind::Estimated
        );
        assert_eq!(
            MeasurementKind::parse_lenient("provider"),
            MeasurementKind::Estimated
        );
        assert_eq!(
            MeasurementKind::parse_lenient(""),
            MeasurementKind::Estimated
        );
    }

    #[test]
    fn a_provenanced_value_serializes_flat() -> Result<(), serde_json::Error> {
        let value = ProvenancedValue::new(
            312_u32,
            MetricProvenance {
                kind: MeasurementKind::Estimated,
                origin: Some("vekta".to_owned()),
                as_of: CivilDate::from_ymd(2026, 10, 1).ok(),
            },
        );
        let json = serde_json::to_value(&value)?;
        assert_eq!(
            json,
            serde_json::json!({
                "value": 312,
                "kind": "estimated",
                "origin": "vekta",
                "as_of": "2026-10-01"
            })
        );
        Ok(())
    }

    #[test]
    fn threshold_provenance_names_only_what_a_provider_supplied() -> Result<(), serde_json::Error> {
        assert!(ThresholdProvenance::NONE.is_empty());
        assert_eq!(ThresholdProvenance::default(), ThresholdProvenance::NONE);
        assert_eq!(
            serde_json::to_value(ThresholdProvenance::NONE)?,
            serde_json::json!({})
        );

        let imported = ThresholdProvenance {
            ftp_watts: Some(MetricProvenance {
                kind: MeasurementKind::Estimated,
                origin: Some("intervals.icu".to_owned()),
                as_of: CivilDate::from_ymd(2026, 9, 30).ok(),
            }),
            ..ThresholdProvenance::NONE
        };
        assert!(!imported.is_empty());
        let json = serde_json::to_value(&imported)?;
        assert_eq!(
            json,
            serde_json::json!({
                "ftp_watts": { "kind": "estimated", "origin": "intervals.icu", "as_of": "2026-09-30" }
            })
        );
        // A stored or cached profile written before the sidecar existed reads
        // with every threshold the athlete's own.
        let read: ThresholdProvenance = serde_json::from_value(serde_json::json!({}))?;
        assert!(read.is_empty());
        assert_eq!(
            serde_json::from_value::<ThresholdProvenance>(json)?,
            imported
        );
        Ok(())
    }

    #[test]
    fn an_unreadable_anchor_origin_is_the_athletes_and_never_promotable() {
        for origin in [
            AnchorOrigin::Athlete,
            AnchorOrigin::Imported,
            AnchorOrigin::Estimated,
            AnchorOrigin::Promoted,
        ] {
            assert_eq!(
                AnchorOrigin::parse_lenient(origin.as_str()),
                origin,
                "{origin:?} must round-trip through its stored form"
            );
        }
        assert_eq!(
            AnchorOrigin::parse_lenient("garmin"),
            AnchorOrigin::Athlete,
            "an origin no build knows must read as the athlete's"
        );
        assert!(
            !AnchorOrigin::Athlete.is_promotable(),
            "bug: the athlete's number would be promoted over"
        );
        assert!(
            !AnchorOrigin::Imported.is_promotable(),
            "bug: an imported number would be promoted over"
        );
        assert!(
            AnchorOrigin::Estimated.is_promotable(),
            "bug: an estimate would never be replaced by a measurement"
        );
        assert!(
            AnchorOrigin::Promoted.is_promotable(),
            "bug: a promoted number would never be refreshed"
        );
    }

    #[test]
    fn only_a_stored_origin_code_parses_and_a_named_source_stays_a_name() {
        for origin in AnchorOrigin::ALL {
            assert_eq!(
                AnchorOrigin::parse(origin.as_str()),
                Some(origin),
                "bug: {origin:?} does not parse back from its stored form"
            );
        }
        assert_eq!(
            AnchorOrigin::parse(ATHLETE_REPORTED_ORIGIN),
            Some(AnchorOrigin::Athlete),
            "bug: a value the athlete stated without a source is not the athlete's"
        );
        for named in ["intervals.icu", "lab", "3-min all-out test", "Imported", ""] {
            assert_eq!(
                AnchorOrigin::parse(named),
                None,
                "bug: '{named}' is a named source, not a stored origin code"
            );
        }
    }

    #[test]
    fn a_value_with_no_recorded_origin_is_the_athletes_own() -> Result<(), String> {
        let day = CivilDate::from_ymd(2026, 10, 31)
            .ok()
            .ok_or("a valid day")?;
        assert!(
            !AnchorProvenance::promotable(true, None),
            "a legacy value must never be promoted over"
        );
        assert!(
            AnchorProvenance::promotable(false, None),
            "an empty anchor is promotable"
        );
        assert!(
            AnchorProvenance::promotable(true, Some(&AnchorStamp::promoted(day))),
            "bug: a promoted anchor would never be refreshed"
        );
        assert!(
            !AnchorProvenance::promotable(true, Some(&AnchorStamp::athlete(day))),
            "bug: the athlete's stamped anchor would be promoted over"
        );
        Ok(())
    }

    #[test]
    fn anchor_provenance_serializes_only_what_is_recorded() -> Result<(), Box<dyn Error>> {
        assert!(
            AnchorProvenance::NONE.is_empty(),
            "bug: NONE records an origin"
        );
        assert_eq!(
            AnchorProvenance::default(),
            AnchorProvenance::NONE,
            "bug: the default records an origin"
        );
        assert_eq!(
            serde_json::to_value(AnchorProvenance::NONE)?,
            serde_json::json!({}),
            "bug: an unrecorded origin is serialized"
        );
        let day = CivilDate::from_ymd(2026, 10, 25)
            .ok()
            .ok_or("a valid day")?;
        let promoted = AnchorProvenance {
            resting_hr: Some(AnchorStamp::promoted(day)),
            ..AnchorProvenance::NONE
        };
        let json = serde_json::to_value(promoted)?;
        assert_eq!(
            json,
            serde_json::json!({ "resting_hr": { "origin": "promoted", "as_of": "2026-10-25" } }),
            "bug: the promoted stamp serializes in another shape"
        );
        assert_eq!(
            serde_json::from_value::<AnchorProvenance>(json)?,
            promoted,
            "bug: the stamp does not round-trip"
        );
        Ok(())
    }

    mod source_classes {
        use std::collections::BTreeSet;

        use uuid::Uuid;

        use super::super::*;
        use crate::ai_policy::SourceClass::{PatientEntered, ThirdPartyApp};
        use crate::models::{PhysiologicalSex, SportType};

        fn profile() -> UserPhysiologicalProfile {
            UserPhysiologicalProfile::new(Uuid::nil(), SportType::Run)
        }

        fn provenance(origin: &str) -> MetricProvenance {
            MetricProvenance {
                kind: MeasurementKind::Measured,
                origin: Some(origin.to_owned()),
                as_of: None,
            }
        }

        #[test]
        fn a_profile_holding_nothing_names_no_source() {
            assert!(
                profile().source_classes().is_empty(),
                "a fresh profile's defaults add no class"
            );
        }

        #[test]
        fn a_threshold_is_the_athletes_unless_a_provider_imported_it() {
            let mut held = profile();
            held.ftp_watts = Some(285);
            assert_eq!(
                held.source_classes(),
                BTreeSet::from([PatientEntered]),
                "an FTP with no import entry is the athlete's"
            );
            held.threshold_provenance.ftp_watts = Some(MetricProvenance {
                kind: MeasurementKind::Estimated,
                origin: Some("intervals.icu".to_owned()),
                as_of: None,
            });
            assert_eq!(
                held.source_classes(),
                BTreeSet::from([ThirdPartyApp]),
                "an imported FTP is the provider's setting"
            );
        }

        #[test]
        fn an_anchor_takes_its_origins_class() {
            let mut held = profile();
            held.resting_hr = Some(46);
            assert_eq!(
                held.source_classes(),
                BTreeSet::from([PatientEntered]),
                "an anchor stored before origins is the athlete's own"
            );
            for origin in [
                AnchorOrigin::Imported,
                AnchorOrigin::Estimated,
                AnchorOrigin::Promoted,
            ] {
                held.anchor_provenance.resting_hr = Some(AnchorStamp {
                    origin,
                    as_of: None,
                });
                assert_eq!(
                    held.source_classes(),
                    BTreeSet::from([ThirdPartyApp]),
                    "{origin:?}"
                );
            }
        }

        #[test]
        fn a_stated_value_is_the_athletes_unless_their_coach_set_it() {
            let mut held = profile();
            held.critical_power_watts = Some(ProvenancedValue::new(312, provenance("lab")));
            assert_eq!(
                held.source_classes(),
                BTreeSet::from([PatientEntered]),
                "a value the athlete stated from a lab test is theirs"
            );
            held.critical_power_watts =
                Some(ProvenancedValue::new(312, provenance(COACH_STATED_ORIGIN)));
            assert_eq!(
                held.source_classes(),
                BTreeSet::from([ThirdPartyApp]),
                "a value their coach set claims the least"
            );
        }

        #[test]
        fn a_sex_claims_the_least_and_age_is_the_athletes() {
            let mut held = profile();
            held.age = Some(31);
            assert_eq!(
                held.source_classes(),
                BTreeSet::from([PatientEntered]),
                "an age is the athlete's"
            );
            held.sex = Some(PhysiologicalSex::Female);
            assert_eq!(
                held.source_classes(),
                BTreeSet::from([PatientEntered, ThirdPartyApp]),
                "a sex claims the least beside the athlete's age"
            );
        }
    }
}
