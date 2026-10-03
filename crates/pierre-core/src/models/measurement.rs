// ABOUTME: Measured-vs-estimated provenance for a physiological value — the kind, who produced it, and when
// ABOUTME: A stored value never exists without its kind, so an estimate can never reach an athlete as a measurement
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
}
