// ABOUTME: The declared shape of estimate_lactate_thresholds — four constructs, a band table, and what may be stored
// ABOUTME: Each threshold names its own method and paper, because the four do not coincide and a bare number misleads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The reply to a step test.
//!
//! Every located point travels with the construct that located it and the
//! paper behind it. That is not decoration: LT1 by log-log, LT2 by modified
//! Dmax, by Dmax and by the 4.0 mmol/L convention are four different
//! questions with four different answers (Jamnick 2020), and a client that
//! renders one of them as "your threshold" is reporting a number the athlete
//! did not get.

use serde::Serialize;

use crate::implementations::configuration_output::PowerZones;

/// What `estimate_lactate_thresholds` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LactateThresholdsResult {
    /// `watts` or `seconds_per_km` — which axis the stages were on.
    pub unit: String,
    /// How many stages the athlete reported.
    pub stage_count: usize,
    /// LT1, by the log-log breakpoint. One construct, one answer.
    pub lt1: ThresholdReport,
    /// LT2 three ways: modified Dmax, Dmax, and the 4.0 mmol/L convention.
    /// A list rather than three keys because they are the same question
    /// answered by three methods, and a client showing one must be able to
    /// show that the others disagreed.
    pub lt2: Vec<ThresholdReport>,
    /// Intensity and heart rate at each lactate concentration from 1.0 to
    /// 4.0 mmol/L, so the athlete can read across the curve.
    pub band_table: Vec<BandRow>,
    /// The cubic that was fitted, and how well it fitted.
    pub curve_fit: CurveFit,
    /// Zones anchored on the modified-Dmax LT2, when the stages were watts.
    pub power_zones: LactatePowerZones,
    /// What the athlete's profile already holds, so the reply can be compared
    /// against it rather than silently replacing it.
    pub stored_profile: StoredThresholds,
    /// How to talk about these numbers. Carried on the payload because a
    /// client that drops it will report 4.0 mmol/L as the threshold.
    pub framing: String,
    /// Set when an LT2 did not come out above LT1, which no graded test
    /// should produce. Explicitly null when they ordered correctly — a
    /// reader can tell "checked and fine" from "not checked", which is the
    /// distinction `a_coherent_test_carries_no_ordering_warning` pins.
    pub ordering_warning: Option<String>,
    /// Always false: this tool estimates and never writes. The athlete
    /// confirms a number and `set_physiology` stores it.
    pub saved: bool,
    /// Which `set_physiology` field would hold the confirmed LT2, named for
    /// the unit the test was run in.
    pub to_store: String,
}

/// One construct's verdict: either a located point or the reason there is
/// none.
///
/// Two arms rather than one shape with optionals, because the two answers
/// carry disjoint keys — a determined threshold has `intensity`,
/// `lactate_mmol` and `heart_rate`, an undetermined one has `reason`, and
/// neither set is a subset of the other. A client can therefore tell them
/// apart from the payload alone, and `heart_rate` stays explicitly null on a
/// determined point rather than vanishing: "the strap was not worn" and
/// "this method found nothing" are different facts.
#[derive(Debug, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ThresholdReport {
    /// The method located a point.
    Determined(DeterminedThreshold),
    /// It could not, and says why.
    NotDeterminable(UndeterminedThreshold),
}

/// A located threshold.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeterminedThreshold {
    /// The method that was applied.
    pub method: String,
    /// What that method marks, in words.
    pub marks: String,
    /// The paper it comes from.
    pub reference: String,
    /// Always `determined`.
    pub outcome: String,
    /// Watts or seconds per kilometre at the located point.
    pub intensity: f64,
    /// Blood lactate there.
    pub lactate_mmol: f64,
    /// Heart rate there. Explicitly null when no strap was worn, so a reader
    /// can tell "measured and absent" from "not part of this answer".
    pub heart_rate: Option<f64>,
}

/// A threshold the method could not locate.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct UndeterminedThreshold {
    /// The method that was applied.
    pub method: String,
    /// What that method marks, in words.
    pub marks: String,
    /// The paper it comes from.
    pub reference: String,
    /// Always `not_determinable`.
    pub outcome: String,
    /// Why the method could not locate a point on these stages.
    pub reason: String,
}

/// One row of the lactate band table.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct BandRow {
    /// The concentration this row is for.
    pub lactate_mmol: f64,
    /// The intensity the fitted curve puts it at.
    pub intensity: f64,
    /// Heart rate there, when the stages carried one.
    pub heart_rate: Option<f64>,
}

/// The curve that was fitted to the stages.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct CurveFit {
    /// The model, written out, so the coefficients mean something.
    pub model: String,
    /// `c0` through `c3`, in that order.
    pub coefficients: Vec<f64>,
    /// How much of the variance the fit explains. A low value is the signal
    /// that the located points should not be trusted.
    pub r_squared: f64,
}

/// Zones derived from the modified-Dmax LT2, or why there are none.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LactatePowerZones {
    /// Always `lt2_modified_dmax` — named so a client cannot mistake these
    /// for zones anchored on a different construct.
    pub anchor: String,
    /// Whether zones were derived. Read this before `zones`.
    pub available: bool,
    /// Why not: the stages were paces, the anchor could not be located, or
    /// the configured percentages do not increase at this threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The threshold the zones were built on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ftp_watts: Option<u32>,
    /// The zones themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zones: Option<PowerZones>,
}

/// What the athlete's profile already holds on these axes.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct StoredThresholds {
    /// Stored FTP, when one is set.
    pub ftp_watts: Option<u32>,
    /// Stored threshold pace, when one is set.
    pub threshold_pace_sec_per_km: Option<f64>,
    /// Stored maximum heart rate, when one is set.
    pub max_hr: Option<u16>,
}
