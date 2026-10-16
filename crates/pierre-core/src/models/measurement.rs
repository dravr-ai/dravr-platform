// ABOUTME: Measured-vs-estimated provenance for a physiological value — the kind, who produced it, and when
// ABOUTME: A provenanced value never exists without its kind, and a provider-supplied threshold always names its provider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// The origin a value carries when the athlete stated it without naming the
/// test or the source behind it. It attributes nothing, so a sentence that
/// mentions it has not named a source.
pub const ATHLETE_REPORTED_ORIGIN: &str = "athlete-reported";

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
    pub as_of: Option<NaiveDate>,
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

#[cfg(test)]
mod tests {
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
                as_of: NaiveDate::from_ymd_opt(2026, 10, 1),
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
                as_of: NaiveDate::from_ymd_opt(2026, 9, 30),
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
}
