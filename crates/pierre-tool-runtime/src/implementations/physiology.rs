// ABOUTME: set_physiology, the only production writer of user_physiological_profiles, and the physiology tool set
// ABOUTME: Read-modify-write so saving one measurement never nulls the rest of the athlete's profile
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete Physiology Tool
//!
//! `user_physiological_profiles` is read by every computation that can be
//! personalised — training-history TSS, the Endurance dossier and interval
//! export tools, and the athlete snapshot. Until this
//! tool existed nothing wrote it, so those readers always fell back to
//! `AthleteInputs::default()` and every TSS estimate dropped to the static
//! per-sport table or the duration-only rung.
//!
//! Two properties make this tool safe to hand to the agent mid-conversation:
//!
//! - **Read-modify-write.** The underlying upsert sets every column from
//!   `EXCLUDED.*`, so a naive "save just the FTP" would null out max HR,
//!   weight and both zone sets. This reads the stored row first and merges.
//! - **Read-back.** The response carries the profile as re-read from the
//!   database, not the arguments that were passed in. An agent that reports
//!   what the result says cannot confirm a save that did not land — which is
//!   the failure this tool was written for.
//!
//! Derived zones are persisted in the same write: supplying FTP populates
//! `power_zones`, and supplying both resting and max HR populates `hr_zones`.
//! `calculate_personalized_zones` derives the identical boundaries for display
//! from the same functions in [`super::configuration`].

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};
use tracing::info;

use crate::context::ToolExecutionContext;
use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::implementations::configuration::{
    derive_hr_zone_set, derive_power_zone_set, validate_parameter_ranges,
    validate_parameter_relationships,
};
use crate::implementations::lactate_thresholds::EstimateLactateThresholdsTool;
use crate::implementations::plan_flavour::RecommendPlanFlavourTool;
use crate::implementations::vo2max_estimate::EstimateVo2maxTool;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use crate::training_history_compute::{recompute_stored_history, HistoryRefresh};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    HrZoneSet, MeasurementKind, MetricProvenance, PowerZoneSet, ProvenancedValue, SportType,
    TenantId, UserPhysiologicalProfile, ATHLETE_REPORTED_ORIGIN,
};
use pierre_fitness_compute::AthleteInputs;
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// Lightest body weight accepted, in kilograms. Below this the value is a
/// child's weight or a pounds-for-kilograms unit error, not an adult athlete.
const WEIGHT_KG_MIN: f64 = 30.0;

/// Heaviest body weight accepted, in kilograms — above the heaviest recorded
/// competitor in any endurance discipline, so anything past it is a unit error.
const WEIGHT_KG_MAX: f64 = 250.0;

/// Youngest age accepted, in years. Matches the floor the platform's own
/// max-HR estimators are calibrated for.
const AGE_YEARS_MIN: u64 = 10;

/// Oldest age accepted, in years — past the verified human maximum.
const AGE_YEARS_MAX: u64 = 120;

/// Most training experience accepted, in years. A lifetime of training still
/// fits; anything beyond is an entry error.
const TRAINING_EXPERIENCE_YEARS_MAX: u64 = 80;

/// Fastest threshold pace accepted, in seconds per kilometre. 2:00/km is
/// quicker than the men's 10 km world-record pace, so a smaller number means
/// the athlete gave seconds per mile or minutes per kilometre by mistake.
const THRESHOLD_PACE_SEC_PER_KM_MIN: f64 = 120.0;

/// Slowest threshold pace accepted, in seconds per kilometre. 15:00/km is
/// slower than a walk, which is no longer a threshold effort.
const THRESHOLD_PACE_SEC_PER_KM_MAX: f64 = 900.0;

/// Lowest lactate threshold accepted, as a fraction of `VO2max`. Matches the
/// 0.65-0.95 range documented on
/// [`UserPhysiologicalProfile::lactate_threshold_percentage`] and the clamp
/// cageux's pace-zone calculator applies.
const LACTATE_THRESHOLD_PCT_MIN: f64 = 0.65;

/// Highest lactate threshold accepted, as a fraction of `VO2max`.
const LACTATE_THRESHOLD_PCT_MAX: f64 = 0.95;

/// Lowest critical power accepted, in watts. Below FTP's 50 W floor on
/// purpose: patients with COPD measure 46 ± 22 W (Tiller 2023), and a
/// floor at 50 would refuse half of such a cohort. Sources for every bound
/// below: `Methodology/Intelligence/Critical Power and Critical Speed`.
const CRITICAL_POWER_WATTS_MIN: u32 = 30;

/// Highest critical power accepted, in watts — above the 402 ± 33 W elite
/// track riders reach on a 3-minute all-out test (Bartram 2017).
const CRITICAL_POWER_WATTS_MAX: u32 = 600;

/// Smallest W′ accepted, in joules. Habitually active adults hold 15-16 kJ
/// (Vanhatalo 2007), so anything below 2 kJ is a kilojoules-for-joules slip.
const W_PRIME_JOULES_MIN: u32 = 2_000;

/// Largest W′ accepted, in joules — well above the 24 ± 4 kJ of elite track
/// endurance riders (Bartram 2017).
const W_PRIME_JOULES_MAX: u32 = 60_000;

/// Slowest critical speed accepted, in metres per second (11:07/km).
const CRITICAL_SPEED_MPS_MIN: f64 = 1.5;

/// Fastest critical speed accepted, in metres per second (2:23/km). Critical
/// speed sits below 5000 m race speed, and the world record is about
/// 6.6 m/s; Kipchoge's is 6.04 m/s (Jones & Vanhatalo 2017).
const CRITICAL_SPEED_MPS_MAX: f64 = 7.0;

/// Smallest D′ accepted, in metres.
const D_PRIME_METERS_MIN: f64 = 30.0;

/// Largest D′ accepted, in metres. Elite marathoners reach 616 m (Jones &
/// Vanhatalo 2017, Table 1) and middle-distance runners plausibly more.
const D_PRIME_METERS_MAX: f64 = 1_000.0;

/// Longest `measurement_source` accepted, in characters: room for "3-min
/// all-out test on the velodrome", short of a pasted paragraph.
const MEASUREMENT_SOURCE_MAX_CHARS: usize = 80;

/// The fields one call's `measurement_kind`, `measurement_source` and
/// `measured_on` describe, as the error messages name them.
const PROVENANCED_FIELDS: &str =
    "critical_power_watts, w_prime_joules, critical_speed_mps or d_prime_meters";

/// Annotation set for the physiology write.
fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        // An upsert of the same values lands the same row, but a second call
        // carrying different values legitimately changes the profile.
        idempotent_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

/// Read an optional number, rejecting a non-numeric value rather than
/// silently ignoring it — a dropped measurement is exactly the failure this
/// tool exists to end.
pub(super) fn optional_number(args: &Value, key: &str) -> AppResult<Option<f64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => raw
            .as_f64()
            .ok_or_else(|| AppError::invalid_input(format!("'{key}' must be a number")))
            .map(Some),
    }
}

/// Read an optional whole number, tolerating the `285.0` an LLM emits where
/// the schema says integer — the same leniency `commitment_create` needed
/// after strict rejection killed live calls.
pub(super) fn optional_whole_number(args: &Value, key: &str) -> AppResult<Option<u64>> {
    let Some(n) = optional_number(args, key)? else {
        return Ok(None);
    };
    if n.fract() != 0.0 || !(0.0..=1_000_000.0).contains(&n) {
        return Err(AppError::invalid_input(format!(
            "'{key}' must be a whole number, got {n}"
        )));
    }
    // Guarded above: non-negative, integral, and far inside u64.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Some(n as u64))
}

/// Parse a fitness level from the athlete-facing label, case-insensitively.
///
/// Spelled out rather than deferring to serde: the enum derives `Deserialize`
/// with no rename rule, so serde would accept only `"Recreational"` and
/// reject the `"recreational"` an LLM writes.
fn parse_fitness_level(raw: &str) -> AppResult<FitnessLevel> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "beginner" => Ok(FitnessLevel::Beginner),
        "recreational" => Ok(FitnessLevel::Recreational),
        "intermediate" => Ok(FitnessLevel::Intermediate),
        "advanced" => Ok(FitnessLevel::Advanced),
        "elite" => Ok(FitnessLevel::Elite),
        "professional" => Ok(FitnessLevel::Professional),
        other => Err(AppError::invalid_input(format!(
            "fitness_level must be one of beginner, recreational, intermediate, advanced, elite, professional; got '{other}'"
        ))),
    }
}

/// Parse `measurement_kind`. Strict: a kind the tool cannot read must not be
/// guessed into a measurement.
fn parse_measurement_kind(raw: &str) -> AppResult<MeasurementKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "measured" => Ok(MeasurementKind::Measured),
        "estimated" => Ok(MeasurementKind::Estimated),
        other => Err(AppError::invalid_input(format!(
            "measurement_kind must be measured or estimated; got '{other}'"
        ))),
    }
}

/// Parse the provenance one call gives its critical-power family values:
/// `None` when the call names none of `measurement_kind`,
/// `measurement_source` or `measured_on`.
///
/// # Errors
/// Returns an invalid-input error when `measurement_source` or `measured_on`
/// arrives without `measurement_kind`, the source is too long, or the date is
/// not a `YYYY-MM-DD` day up to today.
fn provenance_from_args(args: &Value) -> AppResult<Option<MetricProvenance>> {
    let kind = args
        .get("measurement_kind")
        .and_then(Value::as_str)
        .map(parse_measurement_kind)
        .transpose()?;
    let origin = args
        .get("measurement_source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if let Some(ref source) = origin {
        if source.chars().count() > MEASUREMENT_SOURCE_MAX_CHARS {
            return Err(AppError::invalid_input(format!(
                "measurement_source must be at most {MEASUREMENT_SOURCE_MAX_CHARS} characters"
            )));
        }
    }
    let as_of = args
        .get("measured_on")
        .and_then(Value::as_str)
        .map(|raw| {
            NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| {
                AppError::invalid_input(format!(
                    "measured_on must be a date as YYYY-MM-DD; got '{raw}'"
                ))
            })
        })
        .transpose()?;
    if let Some(day) = as_of {
        if day > Utc::now().date_naive() {
            return Err(AppError::invalid_input(format!(
                "measured_on cannot be in the future; got {day}"
            )));
        }
    }
    match kind {
        Some(kind) => Ok(Some(MetricProvenance { kind, origin, as_of })),
        None if origin.is_some() || as_of.is_some() => Err(AppError::invalid_input(
            "measurement_source and measured_on describe a measurement_kind; pass measurement_kind too",
        )),
        None => Ok(None),
    }
}

/// The fields one `set_physiology` call carries. Every field is optional; the
/// tool rejects a call that sets none of them.
struct PhysiologyUpdate {
    ftp_watts: Option<u32>,
    threshold_pace_sec_per_km: Option<f64>,
    max_hr: Option<u16>,
    resting_hr: Option<u16>,
    threshold_hr: Option<u16>,
    lactate_threshold_percentage: Option<f64>,
    vo2_max: Option<f64>,
    weight: Option<f64>,
    age: Option<u16>,
    fitness_level: Option<FitnessLevel>,
    primary_sport: Option<SportType>,
    training_experience_years: Option<u8>,
    critical_power_watts: Option<u32>,
    w_prime_joules: Option<u32>,
    critical_speed_mps: Option<f64>,
    d_prime_meters: Option<f64>,
    /// How every critical-power family value in this call was obtained.
    /// Present exactly when one of them is.
    provenance: Option<MetricProvenance>,
}

impl PhysiologyUpdate {
    /// Parse the tool arguments, converting each numeric field into the width
    /// the stored profile uses. An out-of-width value is reported as a range
    /// error rather than wrapping.
    fn from_args(args: &Value) -> AppResult<Self> {
        let ftp_watts = optional_whole_number(args, "ftp_watts")?
            .map(|v| {
                u32::try_from(v)
                    .map_err(|_| AppError::invalid_input(format!("ftp_watts is out of range: {v}")))
            })
            .transpose()?;
        let max_hr = optional_whole_number(args, "max_hr")?
            .map(|v| {
                u16::try_from(v)
                    .map_err(|_| AppError::invalid_input(format!("max_hr is out of range: {v}")))
            })
            .transpose()?;
        let resting_hr = optional_whole_number(args, "resting_hr")?
            .map(|v| {
                u16::try_from(v).map_err(|_| {
                    AppError::invalid_input(format!("resting_hr is out of range: {v}"))
                })
            })
            .transpose()?;
        let threshold_hr = optional_whole_number(args, "threshold_hr")?
            .map(|v| {
                u16::try_from(v).map_err(|_| {
                    AppError::invalid_input(format!("threshold_hr is out of range: {v}"))
                })
            })
            .transpose()?;
        let age = optional_whole_number(args, "age")?
            .map(|v| {
                u16::try_from(v)
                    .map_err(|_| AppError::invalid_input(format!("age is out of range: {v}")))
            })
            .transpose()?;
        let training_experience_years = optional_whole_number(args, "training_experience_years")?
            .map(|v| {
                u8::try_from(v).map_err(|_| {
                    AppError::invalid_input(format!(
                        "training_experience_years is out of range: {v}"
                    ))
                })
            })
            .transpose()?;

        let fitness_level = args
            .get("fitness_level")
            .and_then(Value::as_str)
            .map(parse_fitness_level)
            .transpose()?;
        // `from_provider_string` is the platform's canonical sport parser and
        // maps an unrecognised label to `SportType::Other` instead of failing,
        // so an athlete naming a sport the enum has no variant for still gets
        // the rest of their measurements saved.
        let primary_sport = args
            .get("primary_sport")
            .and_then(Value::as_str)
            .map(|s| SportType::from_provider_string(s, None));

        let critical_power_watts = optional_whole_number(args, "critical_power_watts")?
            .map(|v| {
                u32::try_from(v).map_err(|_| {
                    AppError::invalid_input(format!("critical_power_watts is out of range: {v}"))
                })
            })
            .transpose()?;
        let w_prime_joules = optional_whole_number(args, "w_prime_joules")?
            .map(|v| {
                u32::try_from(v).map_err(|_| {
                    AppError::invalid_input(format!("w_prime_joules is out of range: {v}"))
                })
            })
            .transpose()?;
        let critical_speed_mps = optional_number(args, "critical_speed_mps")?;
        let d_prime_meters = optional_number(args, "d_prime_meters")?;
        // A critical-power family value never lands without saying whether it
        // was measured: that kind is what keeps a modelled number from
        // reaching the athlete as a measurement. A kind with nothing to
        // describe is refused too, so no caller believes it qualified the FTP.
        let provenance = provenance_from_args(args)?;
        let carries_provenanced_value = critical_power_watts.is_some()
            || w_prime_joules.is_some()
            || critical_speed_mps.is_some()
            || d_prime_meters.is_some();
        match (carries_provenanced_value, provenance.is_some()) {
            (true, false) => {
                return Err(AppError::invalid_input(format!(
                    "measurement_kind is required with {PROVENANCED_FIELDS}: measured only when the athlete named the test that produced it, estimated otherwise"
                )));
            }
            (false, true) => {
                return Err(AppError::invalid_input(format!(
                    "measurement_kind describes {PROVENANCED_FIELDS}, and this call sets none of them"
                )));
            }
            _ => {}
        }

        Ok(Self {
            ftp_watts,
            threshold_pace_sec_per_km: optional_number(args, "threshold_pace_sec_per_km")?,
            max_hr,
            resting_hr,
            threshold_hr,
            lactate_threshold_percentage: optional_number(args, "lactate_threshold_percentage")?,
            vo2_max: optional_number(args, "vo2_max")?,
            weight: optional_number(args, "weight")?,
            age,
            fitness_level,
            primary_sport,
            training_experience_years,
            critical_power_watts,
            w_prime_joules,
            critical_speed_mps,
            d_prime_meters,
            provenance,
        })
    }

    /// Names of the fields this call sets, for the response and the log line.
    ///
    /// Only names travel — the measurements themselves are health data and
    /// stay out of the operator log.
    fn field_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.ftp_watts.is_some() {
            names.push("ftp_watts");
        }
        if self.threshold_pace_sec_per_km.is_some() {
            names.push("threshold_pace_sec_per_km");
        }
        if self.max_hr.is_some() {
            names.push("max_hr");
        }
        if self.resting_hr.is_some() {
            names.push("resting_hr");
        }
        if self.threshold_hr.is_some() {
            names.push("threshold_hr");
        }
        if self.lactate_threshold_percentage.is_some() {
            names.push("lactate_threshold_percentage");
        }
        if self.vo2_max.is_some() {
            names.push("vo2_max");
        }
        if self.weight.is_some() {
            names.push("weight");
        }
        if self.age.is_some() {
            names.push("age");
        }
        if self.fitness_level.is_some() {
            names.push("fitness_level");
        }
        if self.primary_sport.is_some() {
            names.push("primary_sport");
        }
        if self.training_experience_years.is_some() {
            names.push("training_experience_years");
        }
        if self.critical_power_watts.is_some() {
            names.push("critical_power_watts");
        }
        if self.w_prime_joules.is_some() {
            names.push("w_prime_joules");
        }
        if self.critical_speed_mps.is_some() {
            names.push("critical_speed_mps");
        }
        if self.d_prime_meters.is_some() {
            names.push("d_prime_meters");
        }
        names
    }

    /// Overlay the supplied fields onto the stored profile, leaving every
    /// field this call did not mention exactly as it was.
    fn apply_to(&self, profile: &mut UserPhysiologicalProfile) {
        if let Some(v) = self.ftp_watts {
            profile.ftp_watts = Some(v);
        }
        if let Some(v) = self.threshold_pace_sec_per_km {
            profile.threshold_pace_sec_per_km = Some(v);
        }
        if let Some(v) = self.max_hr {
            profile.max_hr = Some(v);
        }
        if let Some(v) = self.resting_hr {
            profile.resting_hr = Some(v);
        }
        if let Some(v) = self.threshold_hr {
            profile.threshold_hr = Some(v);
        }
        if let Some(v) = self.lactate_threshold_percentage {
            profile.lactate_threshold_percentage = Some(v);
        }
        if let Some(v) = self.vo2_max {
            profile.vo2_max = Some(v);
        }
        if let Some(v) = self.weight {
            profile.weight = Some(v);
        }
        if let Some(v) = self.age {
            profile.age = Some(v);
        }
        if let Some(v) = self.fitness_level {
            profile.fitness_level = v;
        }
        if let Some(ref v) = self.primary_sport {
            profile.primary_sport = v.clone();
        }
        if let Some(v) = self.training_experience_years {
            profile.training_experience_years = Some(v);
        }
        // `from_args` guarantees a provenance whenever one of these is set.
        if let Some(ref provenance) = self.provenance {
            overlay(
                &mut profile.critical_power_watts,
                self.critical_power_watts,
                provenance,
            );
            overlay(&mut profile.w_prime_joules, self.w_prime_joules, provenance);
            overlay(
                &mut profile.critical_speed_mps,
                self.critical_speed_mps,
                provenance,
            );
            overlay(&mut profile.d_prime_meters, self.d_prime_meters, provenance);
        }
    }
}

/// Replace a stored provenanced value when the call supplies a new one,
/// pairing it with the call's provenance; leave it untouched otherwise.
fn overlay<T>(
    stored: &mut Option<ProvenancedValue<T>>,
    supplied: Option<T>,
    provenance: &MetricProvenance,
) {
    if let Some(value) = supplied {
        *stored = Some(ProvenancedValue::new(value, provenance.clone()));
    }
}

/// Range-check the columns `configuration_validation` does not cover.
///
/// The heart-rate, VO2 max and FTP bounds live in
/// `dravr_cageux::physiological_constants::configuration_validation`
/// and are applied by [`validate_parameter_ranges`]; these are the remaining
/// profile columns, whose bounds have no home there.
fn validate_uncovered_ranges(profile: &UserPhysiologicalProfile, errors: &mut Vec<String>) {
    if let Some(weight) = profile.weight {
        if !(WEIGHT_KG_MIN..=WEIGHT_KG_MAX).contains(&weight) {
            errors.push(format!(
                "weight must be between {WEIGHT_KG_MIN} and {WEIGHT_KG_MAX} kg, got {weight:.1}"
            ));
        }
    }
    if let Some(age) = profile.age {
        if !(AGE_YEARS_MIN..=AGE_YEARS_MAX).contains(&u64::from(age)) {
            errors.push(format!(
                "age must be between {AGE_YEARS_MIN} and {AGE_YEARS_MAX} years, got {age}"
            ));
        }
    }
    if let Some(years) = profile.training_experience_years {
        if u64::from(years) > TRAINING_EXPERIENCE_YEARS_MAX {
            errors.push(format!(
                "training_experience_years must be at most {TRAINING_EXPERIENCE_YEARS_MAX}, got {years}"
            ));
        }
    }
    if let Some(pace) = profile.threshold_pace_sec_per_km {
        if !(THRESHOLD_PACE_SEC_PER_KM_MIN..=THRESHOLD_PACE_SEC_PER_KM_MAX).contains(&pace) {
            errors.push(format!(
                "threshold_pace_sec_per_km must be between {THRESHOLD_PACE_SEC_PER_KM_MIN} and {THRESHOLD_PACE_SEC_PER_KM_MAX} seconds, got {pace:.1}"
            ));
        }
    }
    if let Some(pct) = profile.lactate_threshold_percentage {
        if !(LACTATE_THRESHOLD_PCT_MIN..=LACTATE_THRESHOLD_PCT_MAX).contains(&pct) {
            errors.push(format!(
                "lactate_threshold_percentage must be between {LACTATE_THRESHOLD_PCT_MIN} and {LACTATE_THRESHOLD_PCT_MAX} (fraction of VO2max), got {pct:.2}"
            ));
        }
    }
    if let Some(ref cp) = profile.critical_power_watts {
        if !(CRITICAL_POWER_WATTS_MIN..=CRITICAL_POWER_WATTS_MAX).contains(&cp.value) {
            errors.push(format!(
                "critical_power_watts must be between {CRITICAL_POWER_WATTS_MIN} and {CRITICAL_POWER_WATTS_MAX} W, got {}",
                cp.value
            ));
        }
    }
    if let Some(ref w_prime) = profile.w_prime_joules {
        if !(W_PRIME_JOULES_MIN..=W_PRIME_JOULES_MAX).contains(&w_prime.value) {
            errors.push(format!(
                "w_prime_joules must be between {W_PRIME_JOULES_MIN} and {W_PRIME_JOULES_MAX} J (W′ is in joules: 20 kJ is 20000), got {}",
                w_prime.value
            ));
        }
    }
    if let Some(ref cs) = profile.critical_speed_mps {
        if !(CRITICAL_SPEED_MPS_MIN..=CRITICAL_SPEED_MPS_MAX).contains(&cs.value) {
            errors.push(format!(
                "critical_speed_mps must be between {CRITICAL_SPEED_MPS_MIN} and {CRITICAL_SPEED_MPS_MAX} m/s (4:00/km is 4.17), got {:.2}",
                cs.value
            ));
        }
    }
    if let Some(ref d_prime) = profile.d_prime_meters {
        if !(D_PRIME_METERS_MIN..=D_PRIME_METERS_MAX).contains(&d_prime.value) {
            errors.push(format!(
                "d_prime_meters must be between {D_PRIME_METERS_MIN} and {D_PRIME_METERS_MAX} m, got {:.1}",
                d_prime.value
            ));
        }
    }
    if let Some(years) = profile.training_experience_years {
        if let Some(age) = profile.age {
            if u64::from(years) >= u64::from(age) {
                errors.push(format!(
                    "training_experience_years ({years}) must be less than age ({age})"
                ));
            }
        }
    }
}

/// Validate the profile as it would stand after the merge.
///
/// Validating the merged row rather than the incoming arguments is what
/// catches a contradiction spread across two calls — saving `resting_hr: 60`
/// on Monday and `max_hr: 55` on Tuesday is just as wrong as sending both at
/// once, and only the merged view sees it.
fn validate_merged(profile: &UserPhysiologicalProfile) -> AppResult<()> {
    let mut ranges = Map::new();
    if let Some(v) = profile.max_hr {
        ranges.insert("max_hr".to_owned(), json!(v));
    }
    if let Some(v) = profile.resting_hr {
        ranges.insert("resting_hr".to_owned(), json!(v));
    }
    if let Some(v) = profile.vo2_max {
        ranges.insert("vo2_max".to_owned(), json!(v));
    }
    if let Some(v) = profile.ftp_watts {
        ranges.insert("ftp".to_owned(), json!(v));
    }
    if let Some(v) = profile.threshold_hr {
        ranges.insert("threshold_hr".to_owned(), json!(v));
    }

    let mut errors = Vec::new();
    validate_parameter_ranges(&ranges, &mut errors);
    validate_uncovered_ranges(profile, &mut errors);
    // Every relationship check below reads values the range pass just
    // cleared, so a rejected number never reaches the derived lactate
    // threshold.
    if !errors.is_empty() {
        return Err(AppError::invalid_input(errors.join("; ")));
    }

    let mut relationships = ranges;
    // The LTHR the TSS engine will score against — the measured one, else the
    // estimate from the lactate threshold and max HR — through the same
    // `lactate_threshold_hr` every training-load reader uses. Checking that
    // number keeps the relationship test honest about the value the engine
    // will consume.
    //
    // An estimate is deliberately kept out of the range map above:
    // `THRESHOLD_HR_MIN` is 100 bpm, which the estimate for a legitimate
    // 100-125 bpm max HR at the 0.65 floor falls under, and rejecting that
    // profile would be wrong. A measured value was range-checked there.
    if let Some(lthr) = profile.lactate_threshold_hr() {
        // Bounded by the cleared ranges: threshold HR <= 200, or max HR <= 220
        // times a fraction below one.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lthr_bpm = lthr.round() as u64;
        relationships.insert("threshold_hr".to_owned(), json!(lthr_bpm));
    }
    validate_parameter_relationships(&relationships, &mut errors);
    errors
        .is_empty()
        .ok_or_else(|| AppError::invalid_input(errors.join("; ")))
}

/// Render a profile for the tool response.
fn profile_payload(profile: &UserPhysiologicalProfile) -> PhysiologyProfile {
    PhysiologyProfile {
        ftp_watts: profile.ftp_watts,
        threshold_pace_sec_per_km: profile.threshold_pace_sec_per_km,
        max_hr: profile.max_hr,
        resting_hr: profile.resting_hr,
        threshold_hr: profile.threshold_hr,
        lactate_threshold_percentage: profile.lactate_threshold_percentage,
        vo2_max: profile.vo2_max,
        weight: profile.weight,
        age: profile.age,
        fitness_level: profile.fitness_level,
        primary_sport: profile.primary_sport.clone(),
        training_experience_years: profile.training_experience_years,
        hr_zones: profile.hr_zones,
        power_zones: profile.power_zones,
        critical_power_watts: profile.critical_power_watts.clone(),
        w_prime_joules: profile.w_prime_joules.clone(),
        critical_speed_mps: profile.critical_speed_mps.clone(),
        d_prime_meters: profile.d_prime_meters.clone(),
    }
}

/// The athlete's physiological profile as the tools echo it back.
///
/// Every measurement is optional: a profile is built up over time, and an
/// athlete who has only ever given a resting heart rate has a profile with
/// one field set. Reporting the absent ones as absent rather than as zero is
/// what keeps an agent from reasoning off a fabricated number.
///
/// `fitness_level` and `primary_sport` are NOT optional — they carry their
/// own defaults — which is why they are the enums rather than strings.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PhysiologyProfile {
    /// Functional threshold power, in watts.
    pub ftp_watts: Option<u32>,
    /// Threshold pace, in seconds per kilometre.
    pub threshold_pace_sec_per_km: Option<f64>,
    /// Maximum heart rate, in bpm.
    pub max_hr: Option<u16>,
    /// Resting heart rate, in bpm.
    pub resting_hr: Option<u16>,
    /// Measured lactate threshold heart rate, in bpm — what heart-rate
    /// training stress is scored against.
    pub threshold_hr: Option<u16>,
    /// Lactate threshold as a fraction of `VO2max`. Sets threshold pace, and
    /// estimates the LTHR when none is measured.
    pub lactate_threshold_percentage: Option<f64>,
    /// `VO2max`, in ml/kg/min.
    pub vo2_max: Option<f64>,
    /// Body mass, in kilograms.
    pub weight: Option<f64>,
    /// Age in years.
    pub age: Option<u16>,
    /// Self-reported level: beginner, intermediate, advanced.
    pub fitness_level: FitnessLevel,
    /// The sport the athlete mostly trains. Open-ended: a provider-specific
    /// sport no enum anticipated arrives as `Other`.
    pub primary_sport: SportType,
    /// Years of structured training.
    pub training_experience_years: Option<u8>,
    /// Heart-rate zone boundaries. Absent until both a resting and a maximum
    /// heart rate are on the profile, because they are derived from the pair.
    pub hr_zones: Option<HrZoneSet>,
    /// Power zone boundaries. Absent for an athlete with no power meter or
    /// saved FTP.
    pub power_zones: Option<PowerZoneSet>,
    /// Critical power, in watts, with whether it was measured or estimated
    /// and by whom. Quote an estimated value as an estimate, attributed to
    /// its origin ("Vekta estimates your CP at 312 W"), never as a
    /// measurement.
    pub critical_power_watts: Option<ProvenancedValue<u32>>,
    /// W′, in joules, with its provenance. Quoted the same way as
    /// `critical_power_watts`.
    pub w_prime_joules: Option<ProvenancedValue<u32>>,
    /// Critical speed, in metres per second, with its provenance. Quoted the
    /// same way as `critical_power_watts`.
    pub critical_speed_mps: Option<ProvenancedValue<f64>>,
    /// D′, in metres, with its provenance. Quoted the same way as
    /// `critical_power_watts`.
    pub d_prime_meters: Option<ProvenancedValue<f64>>,
}

/// What `set_physiology` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct SetPhysiologyResult {
    /// Always true: the tool errors rather than reporting a failed save.
    pub saved: bool,
    /// Whether this call created the profile rather than updating one.
    pub created: bool,
    /// Which fields this call set, by name. Names only — the measurements
    /// themselves are health data and do not belong in a field list.
    pub updated_fields: Vec<&'static str>,
    /// The profile as it now stands, so the agent need not read it back.
    pub profile: PhysiologyProfile,
    /// What the save did to the stored daily training history
    /// (`get_training_history`): recomputed when it moved a number training
    /// load is scored against.
    pub training_history: HistoryRefresh,
}

// ============================================================================
// SetPhysiologyTool
// ============================================================================

/// Saves the athlete's physiological measurements to the profile every
/// personalised computation reads.
pub struct SetPhysiologyTool;

impl SetPhysiologyTool {
    /// Field descriptions for the tool schema, kept beside the parser.
    fn properties() -> BTreeMap<String, PropertySchema> {
        let mut properties = BTreeMap::new();
        for (name, property_type, description) in [
            (
                "ftp_watts",
                "integer",
                "Functional Threshold Power in watts. Saving it also derives and stores the athlete's power zones.",
            ),
            (
                "threshold_pace_sec_per_km",
                "number",
                "Threshold pace in seconds per kilometre — the running equivalent of FTP. A 4:10/km threshold is 250.",
            ),
            (
                "max_hr",
                "integer",
                "Maximum heart rate in bpm. Saving it together with resting_hr derives and stores the athlete's heart-rate zones.",
            ),
            ("resting_hr", "integer", "Resting heart rate in bpm."),
            (
                "threshold_hr",
                "integer",
                "Lactate threshold heart rate (LTHR) in bpm, from a 30-minute field test, a ramp test or a lab report. Heart-rate-based training load is scored against it.",
            ),
            (
                "lactate_threshold_percentage",
                "number",
                "Lactate threshold as a fraction of VO2 max, between 0.65 and 0.95 (about 0.75-0.90 for a trained athlete), as a lab test reports it. It places threshold pace in the pace zones, and with max_hr estimates the LTHR when no threshold_hr is saved.",
            ),
            ("vo2_max", "number", "VO2 max in ml/kg/min."),
            ("weight", "number", "Body weight in kilograms."),
            ("age", "integer", "Age in years."),
            (
                "fitness_level",
                "string",
                "One of beginner, recreational, intermediate, advanced, elite, professional.",
            ),
            (
                "primary_sport",
                "string",
                "The athlete's main sport, e.g. run, ride, swim, trail_running.",
            ),
            (
                "training_experience_years",
                "integer",
                "Years of structured training experience.",
            ),
            (
                "critical_power_watts",
                "integer",
                "Critical power (CP) in watts: the asymptote of the power-duration curve. Needs measurement_kind.",
            ),
            (
                "w_prime_joules",
                "integer",
                "W′ (W prime) in joules, the work capacity above critical power: 20 kJ is 20000. Needs measurement_kind.",
            ),
            (
                "critical_speed_mps",
                "number",
                "Critical speed (CS) in metres per second, the running analogue of critical power: 4:00/km is 4.17. Needs measurement_kind.",
            ),
            (
                "d_prime_meters",
                "number",
                "D′ (D prime) in metres, the distance capacity above critical speed. Needs measurement_kind.",
            ),
            (
                "measured_on",
                "string",
                "The day the critical power, W′, critical speed or D′ in this call was measured or estimated, as YYYY-MM-DD, when the athlete or the source gives it.",
            ),
        ] {
            properties.insert(
                name.to_owned(),
                PropertySchema {
                    property_type: property_type.to_owned(),
                    description: Some(description.to_owned()),
                    ..Default::default()
                },
            );
        }
        properties.insert(
            "measurement_source".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(format!(
                    "Who or what produced the critical power, W′, critical speed or D′ in this call: a provider or app (vekta, intervals.icu), a test (lab, 3-min all-out test), or {ATHLETE_REPORTED_ORIGIN} when the athlete named no source. An estimate is quoted with it."
                )),
                ..Default::default()
            },
        );
        properties.insert(
            "measurement_kind".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Required with critical_power_watts, w_prime_joules, critical_speed_mps or d_prime_meters, and applies to all of them in this call. measured only when the athlete names the test that produced the value (a lab test, a 3-min all-out test, time trials fitted to the model); estimated when an app or provider modelled it from training data, or the athlete gave the number without naming a test. One of: measured, estimated.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties
    }
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for SetPhysiologyTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(Self::properties()),
            // No field is individually required — the athlete states one
            // measurement at a time. The handler rejects a call that sets none.
            required: None,
            ..Default::default()
        };
        answers_with::<SetPhysiologyResult>(tool_definition(
            "set_physiology",
            "Save the athlete's physiological measurements — FTP, threshold pace, max, resting and threshold heart rate, lactate threshold, VO2 max, weight, age — so training load, zones and every personalised calculation use their real numbers instead of generic per-sport estimates. Training load is scored against what is saved here and nowhere else — power against the FTP, heart rate against the threshold heart rate (estimated from the lactate threshold and max HR when none is saved) — in analyze_training_load, get_training_history, calculate_fitness_score, generate_recommendations and the recovery tools. Also saves critical power, W′, critical speed and D′, each with whether it was measured or estimated (measurement_kind) and by whom (measurement_source); an estimated value is quoted as an estimate with its source, never as a measurement. Call this whenever the athlete states one of these values, for example 'my FTP is 285' or 'my max HR is 190'. Pass only the fields they actually gave you; everything else keeps its stored value. The result is the profile re-read from storage after the write, so report back only what it contains.",
            schema,
            Some(write_annotations()),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // The write lands in `user_physiological_profiles` and the reply is the
        // profile re-read from storage, so this both writes and reads identity
        // data. Without PROFILE the bits resolve to fitness:write, and the
        // read is not declared at all.
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::WRITES_DATA
            | ToolCapabilities::PROFILE
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let user_id = context.user_id;

            let update = PhysiologyUpdate::from_args(&args)?;
            let updated_fields = update.field_names();
            if updated_fields.is_empty() {
                return Err(AppError::invalid_input(
                    "set_physiology needs at least one measurement: ftp_watts, threshold_pace_sec_per_km, max_hr, resting_hr, threshold_hr, lactate_threshold_percentage, vo2_max, weight, age, fitness_level, primary_sport, training_experience_years, critical_power_watts, w_prime_joules, critical_speed_mps or d_prime_meters",
                ));
            }

            let repos = context.resources.repos();
            let existing = repos
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?;
            let created = existing.is_none();
            let load_inputs_before = AthleteInputs::from_profile(existing.as_ref());
            // A first save has no stored sport to keep. `Run` matches the
            // column's own schema default, so code and table agree rather than
            // offering a third answer; `primary_sport` is settable here, so the
            // athlete's real sport overwrites it as soon as they name it.
            let mut profile =
                existing.unwrap_or_else(|| UserPhysiologicalProfile::new(user_id, SportType::Run));
            profile.user_id = user_id;
            update.apply_to(&mut profile);
            validate_merged(&profile)?;

            // Zones are derived here rather than at read time so the stored
            // profile carries the boundaries every reader already expects to
            // find in `hr_zones_json` / `power_zones_json`.
            let zones_config = &context.resources.config().training_zones;
            if let Some(ftp) = profile.ftp_watts {
                if let Some(zones) = derive_power_zone_set(ftp, zones_config) {
                    profile.power_zones = Some(zones);
                }
            }
            if let (Some(resting_hr), Some(max_hr)) = (profile.resting_hr, profile.max_hr) {
                if let Some(zones) = derive_hr_zone_set(resting_hr, max_hr) {
                    profile.hr_zones = Some(zones);
                }
            }

            repos
                .user_physiological_profile
                .upsert_user_physiological_profile(tenant_id, user_id, &profile)
                .await?;

            // Re-read rather than echo the merged struct: the response is what
            // the agent will repeat to the athlete, and it should describe the
            // stored row, not the intent.
            let stored = repos
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?
                .ok_or_else(|| {
                    AppError::database(
                        "physiological profile was not readable immediately after its write",
                    )
                })?;

            info!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                created = created,
                // Field names only — the measurements are health data.
                fields = %updated_fields.join(","),
                "saved athlete physiology"
            );

            // The stored daily rollup was scored against the thresholds it
            // was computed with; a save that moved one recomputes it, so
            // get_training_history agrees with every tool computing load live.
            let training_history =
                if AthleteInputs::from_profile(Some(&stored)) == load_inputs_before {
                    HistoryRefresh::Unaffected
                } else {
                    recompute_stored_history(&context.resources, tenant_id, user_id).await
                };

            ok_typed(
                "set_physiology",
                SetPhysiologyResult {
                    saved: true,
                    created,
                    updated_fields,
                    profile: profile_payload(&stored),
                    training_history,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Build the physiology tool set for registration.
#[must_use]
pub fn create_physiology_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(SetPhysiologyTool),
        Box::new(EstimateVo2maxTool),
        Box::new(EstimateLactateThresholdsTool),
        Box::new(RecommendPlanFlavourTool),
    ]
}

// Guardian security classification (see `crate::security`). The write is
// internal and correctable, echoes no third-party text, and sends nothing
// outbound, so it carries no labels.
crate::declare_security!(SetPhysiologyTool => empty);
