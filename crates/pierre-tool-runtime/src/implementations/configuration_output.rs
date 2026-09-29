// ABOUTME: The shapes the configuration tools answer with, and their derived schemas
// ABOUTME: Split from configuration.rs, which is close to its size ceiling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Result types for the six configuration tools.
//!
//! A few of these carry `serde_json::Value` fields, which the derived schema
//! states as "any". That is honest here rather than lazy: a session override
//! map is whatever parameters the caller passed, and its keys are the
//! catalogue's, not a fixed set this type could name. Everything with a fixed
//! shape is typed.

use std::collections::BTreeMap;

use pierre_config::catalog::ConfigCatalog;
use pierre_core::config::profiles::ConfigProfile;
use serde::Serialize;
use serde_json::{Map, Value};

/// What `get_configuration_catalog` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ConfigurationCatalogResult {
    /// Every configurable parameter, by category and module, with its type,
    /// default and valid range.
    pub catalog: ConfigCatalog,
}

/// One named configuration profile.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ConfigurationProfileEntry {
    /// The name the profile is listed under. `update_user_configuration`
    /// takes it, or the profile's own name (`elite`, `sport_cycling`).
    pub name: String,
    /// The profile itself.
    pub profile: ConfigProfile,
    /// A one-line description, composed from the name.
    pub description: String,
}

/// What `get_configuration_profiles` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ConfigurationProfilesResult {
    /// The profiles on offer.
    pub profiles: Vec<ConfigurationProfileEntry>,
    /// How many there are.
    pub total_count: usize,
}

/// A saved training configuration: the profile template applied, the
/// athlete's own overrides on top, and what the two amount to.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ConfigurationDocument {
    /// The profile template in effect, with its values.
    pub profile: ConfigProfile,
    /// The parameter values the template applies, by name.
    pub profile_parameters: BTreeMap<String, f64>,
    /// The athlete's own overrides, by catalogue key. Open-ended: the keys
    /// are the catalogue's, not a fixed set this type could name.
    pub session_overrides: Map<String, Value>,
    /// The template's values with the overrides on top — what the
    /// configuration amounts to.
    pub effective_parameters: Map<String, Value>,
    /// When the configuration was last written, RFC 3339.
    pub last_modified: String,
}

impl ConfigurationDocument {
    /// The name the applied template's profile answers to (`default`,
    /// `elite`, `sport_cycling`), which is what `active_profile` reports.
    #[must_use]
    pub fn active_profile(&self) -> String {
        self.profile.name()
    }
}

/// What `get_user_configuration` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct UserConfigurationResult {
    /// The athlete this belongs to.
    pub user_id: String,
    /// The profile name of the template in effect: the one last applied with
    /// `update_user_configuration`, `default` when none was.
    pub active_profile: String,
    /// The template's values, the athlete's overrides and what they amount
    /// to.
    pub configuration: ConfigurationDocument,
    /// How many parameters the catalogue offers, so a client can show
    /// "3 of 47 overridden" without a second call.
    pub available_parameters: usize,
}

/// What `update_user_configuration` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct UpdateUserConfigurationResult {
    /// The athlete whose configuration was written.
    pub user_id: String,
    /// The document as stored — `active_profile`, the template's `profile`
    /// and `profile_parameters`, the `session_overrides`, the
    /// `effective_parameters` they amount to, `applied_overrides` and
    /// `last_modified` — echoed so the caller need not read it back.
    pub updated_configuration: Value,
    /// How many changes this call made: one per override set or removed, plus
    /// one when it named a profile.
    pub changes_applied: usize,
    /// What to tell the athlete, already written.
    pub message: String,
}

/// What `validate_configuration` answers with.
///
/// A validation failure is a reported outcome, not an error: the athlete
/// asked whether their parameters are sound and "no, and here is why" is the
/// answer to that question.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ValidateConfigurationResult {
    /// Whether every parameter passed.
    pub validation_passed: bool,
    /// How many were checked.
    pub parameters_validated: usize,
    /// The verdict in plain language.
    pub message: String,
    /// What failed, one entry per problem. Null rather than an empty list
    /// when everything passed — the two are different answers and the wire
    /// has always distinguished them.
    pub errors: Option<Vec<String>>,
}

// ----------------------------------------------------------------------------
// calculate_personalized_zones
// ----------------------------------------------------------------------------

/// The inputs the zones were derived from, as resolved.
///
/// Every one is optional because the athlete may have supplied none of them:
/// the tool reports what it had rather than substituting a default, which is
/// why `unavailable` on the result exists at all.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ZoneInputProfile {
    /// `VO2max` in ml/kg/min — what the pace zones need.
    pub vo2_max: Option<f64>,
    /// Resting heart rate in bpm.
    pub resting_hr: Option<u16>,
    /// Maximum heart rate in bpm. With `resting_hr`, this gives the HR zones.
    pub max_hr: Option<u16>,
    /// Functional threshold power in watts — what the power zones need.
    pub ftp: Option<u32>,
    /// Lactate threshold as a fraction of `VO2max` — what places threshold
    /// pace in the pace zones.
    pub lactate_threshold: f64,
    /// Sport efficiency factor used in the pace and power formulas.
    pub sport_efficiency: f64,
}

/// One heart-rate zone.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HeartRateZone {
    /// What the zone is called, so a client need not map numbers to names.
    pub name: String,
    /// Lower bound in bpm, inclusive.
    pub min_hr: u64,
    /// Upper bound in bpm, inclusive.
    pub max_hr: u64,
}

/// The five heart-rate zones.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HeartRateZones {
    /// Active recovery.
    pub zone_1: HeartRateZone,
    /// Aerobic base.
    pub zone_2: HeartRateZone,
    /// Aerobic threshold.
    pub zone_3: HeartRateZone,
    /// Lactate threshold.
    pub zone_4: HeartRateZone,
    /// VO2 max.
    pub zone_5: HeartRateZone,
}

/// One pace zone, as `m:ss` per kilometre.
///
/// Strings rather than seconds: these are for an athlete to read off, and the
/// numbers they are derived from are already in `zone_calculations`.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PaceZone {
    /// Fastest pace in the zone.
    pub min_pace: String,
    /// Slowest pace in the zone.
    pub max_pace: String,
}

/// The Daniels VDOT pace zones, named for what they are for.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PaceZones {
    /// Easy running: 59-74 % of velocity at `VO2max` by default.
    pub zone_1_easy: PaceZone,
    /// Marathon pace, cut from threshold pace.
    pub zone_2_marathon: PaceZone,
    /// Threshold pace, around the athlete's lactate threshold.
    pub zone_3_threshold: PaceZone,
    /// `VO2max` intervals, up to velocity at `VO2max`.
    pub zone_4_vo2max: PaceZone,
    /// Neuromuscular (repetition) work: the slowest pace of the zone, which
    /// runs on to any faster pace, as `m:ss` per kilometre.
    pub zone_5_neuromuscular_max_pace: String,
}

/// One power zone, in watts.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PowerZoneBand {
    /// Lower bound in watts, inclusive.
    pub min_watts: u32,
    /// Upper bound in watts, inclusive.
    pub max_watts: u32,
}

/// The five power zones.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PowerZones {
    /// Active recovery.
    pub zone_1: PowerZoneBand,
    /// Endurance.
    pub zone_2: PowerZoneBand,
    /// Tempo.
    pub zone_3: PowerZoneBand,
    /// Threshold.
    pub zone_4: PowerZoneBand,
    /// VO2 max.
    pub zone_5: PowerZoneBand,
}

/// The zones that could be derived from what the athlete supplied.
///
/// Each is null when its inputs were missing, and `unavailable` on the result
/// says which input would unlock it. Substituting a default here would hand
/// an athlete zone boundaries computed from somebody else's physiology.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PersonalizedZones {
    /// Needs both a resting and a maximum heart rate.
    pub heart_rate_zones: Option<HeartRateZones>,
    /// Needs `VO2max`.
    pub pace_zones: Option<PaceZones>,
    /// Needs FTP.
    pub power_zones: Option<PowerZones>,
    /// The FTP the power zones were built from, echoed back.
    pub ftp: Option<u32>,
}

/// How the zones were derived, and the two threshold heart rates.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ZoneCalculations {
    /// Always `heart_rate_reserve` — the Karvonen method.
    pub method: String,
    /// The lactate threshold heart rate the heart-rate-reserve model places
    /// (85% of the reserve), in bpm — the reference of these zones. Training
    /// load scores against the athlete's own `threshold_hr`, saved with
    /// `set_physiology`.
    pub lactate_threshold_hr: u64,
    /// Aerobic threshold heart rate in bpm.
    pub aerobic_threshold_hr: u64,
    /// The efficiency factor the pace and power formulas used.
    pub sport_efficiency_factor: f64,
    /// The pace formula, stated so the number is auditable.
    pub pace_formula: String,
    /// The power formula, likewise.
    pub power_estimation: String,
}

/// What `calculate_personalized_zones` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PersonalizedZonesResult {
    /// The inputs, as resolved.
    pub user_profile: ZoneInputProfile,
    /// Where each input came from, keyed by field name: `provided`, `profile`,
    /// or `estimated_from_age` for the one documented estimator. An athlete
    /// should be able to tell a measured number from an estimated one.
    pub input_sources: Value,
    /// The zones that could be derived.
    pub personalized_zones: PersonalizedZones,
    /// For each zone that could not be derived, which input would unlock it.
    pub unavailable: Value,
    /// How the heart-rate zones were computed. Absent when they could not be.
    pub zone_calculations: Option<ZoneCalculations>,
}
