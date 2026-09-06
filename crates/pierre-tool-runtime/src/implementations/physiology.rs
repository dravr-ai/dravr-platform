// ABOUTME: set_physiology, the only production writer of user_physiological_profiles, and estimate_vo2max beside it
// ABOUTME: Read-modify-write so saving one measurement never nulls the rest of the athlete's profile
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete Physiology Tool
//!
//! `user_physiological_profiles` is read by every computation that can be
//! personalised — training-history TSS, the Endurance dossier and interval
//! exports, `GET /api/v1/endurance/*`, and the athlete snapshot. Until this
//! tool existed nothing wrote it, so those readers always fell back to
//! `AthleteInputs::default()` and every TSS estimate dropped to the static
//! per-sport table or the duration-only rung.
//!
//! Two properties make this tool safe to hand to the coach mid-conversation:
//!
//! - **Read-modify-write.** The underlying upsert sets every column from
//!   `EXCLUDED.*`, so a naive "save just the FTP" would null out max HR,
//!   weight and both zone sets. This reads the stored row first and merges.
//! - **Read-back.** The response carries the profile as re-read from the
//!   database, not the arguments that were passed in. A coach that reports
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
use serde_json::{json, Map, Value};
use tracing::info;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::implementations::configuration::{
    derive_hr_zone_set, derive_power_zone_set, validate_parameter_ranges,
    validate_parameter_relationships,
};
use crate::implementations::data_helpers::read_only_annotations;
use crate::implementations::lactate_thresholds::EstimateLactateThresholdsTool;
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{SportType, TenantId, UserPhysiologicalProfile};
use pierre_intelligence::algorithms::Vo2maxAlgorithm;
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

/// Lowest lactate threshold accepted, as a fraction of max HR. Matches the
/// 0.65-0.95 range documented on
/// [`UserPhysiologicalProfile::lactate_threshold_percentage`].
const LACTATE_THRESHOLD_PCT_MIN: f64 = 0.65;

/// Highest lactate threshold accepted, as a fraction of max HR.
const LACTATE_THRESHOLD_PCT_MAX: f64 = 0.95;

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
fn optional_whole_number(args: &Value, key: &str) -> AppResult<Option<u64>> {
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

/// The fields one `set_physiology` call carries. Every field is optional; the
/// tool rejects a call that sets none of them.
struct PhysiologyUpdate {
    ftp_watts: Option<u32>,
    threshold_pace_sec_per_km: Option<f64>,
    max_hr: Option<u16>,
    resting_hr: Option<u16>,
    lactate_threshold_percentage: Option<f64>,
    vo2_max: Option<f64>,
    weight: Option<f64>,
    age: Option<u16>,
    fitness_level: Option<FitnessLevel>,
    primary_sport: Option<SportType>,
    training_experience_years: Option<u8>,
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

        Ok(Self {
            ftp_watts,
            threshold_pace_sec_per_km: optional_number(args, "threshold_pace_sec_per_km")?,
            max_hr,
            resting_hr,
            lactate_threshold_percentage: optional_number(args, "lactate_threshold_percentage")?,
            vo2_max: optional_number(args, "vo2_max")?,
            weight: optional_number(args, "weight")?,
            age,
            fitness_level,
            primary_sport,
            training_experience_years,
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
    }
}

/// Range-check the columns `configuration_validation` does not cover.
///
/// The heart-rate, VO2 max and FTP bounds live in
/// `pierre_intelligence::physiological_constants::configuration_validation`
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
                "lactate_threshold_percentage must be between {LACTATE_THRESHOLD_PCT_MIN} and {LACTATE_THRESHOLD_PCT_MAX} (fraction of max HR), got {pct:.2}"
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
    // Lactate threshold in bpm, derived the same way `training_history_compute`
    // derives the LTHR it feeds the TSS engine. Checking that number keeps the
    // relationship test honest about the value the engine will consume.
    //
    // It is deliberately absent from the range map above: `THRESHOLD_HR_MIN`
    // is 100 bpm, which a legitimate 150 bpm max HR at the 0.65 floor falls
    // under, and rejecting that profile would be wrong.
    if let (Some(pct), Some(max_hr)) = (profile.lactate_threshold_percentage, profile.max_hr) {
        let lthr = f64::from(max_hr) * pct;
        // Bounded by the cleared ranges: max HR <= 220 and pct <= 0.95.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lthr_bpm = lthr.round() as u64;
        relationships.insert("threshold_hr".to_owned(), json!(lthr_bpm));
    }
    validate_parameter_relationships(&relationships, &mut errors);
    if !errors.is_empty() {
        return Err(AppError::invalid_input(errors.join("; ")));
    }
    Ok(())
}

/// Render a profile for the tool response.
fn profile_payload(profile: &UserPhysiologicalProfile) -> Value {
    json!({
        "ftp_watts": profile.ftp_watts,
        "threshold_pace_sec_per_km": profile.threshold_pace_sec_per_km,
        "max_hr": profile.max_hr,
        "resting_hr": profile.resting_hr,
        "lactate_threshold_percentage": profile.lactate_threshold_percentage,
        "vo2_max": profile.vo2_max,
        "weight": profile.weight,
        "age": profile.age,
        "fitness_level": profile.fitness_level,
        "primary_sport": profile.primary_sport,
        "training_experience_years": profile.training_experience_years,
        "hr_zones": profile.hr_zones,
        "power_zones": profile.power_zones,
    })
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
                "lactate_threshold_percentage",
                "number",
                "Lactate threshold as a fraction of maximum heart rate, between 0.65 and 0.95. Combined with max_hr this gives the LTHR that heart-rate-based training load uses.",
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
        tool_definition(
            "set_physiology",
            "Save the athlete's physiological measurements — FTP, threshold pace, max and resting heart rate, lactate threshold, VO2 max, weight, age — so training load, zones and every personalised calculation use their real numbers instead of generic per-sport estimates. Call this whenever the athlete states one of these values, for example 'my FTP is 285' or 'my max HR is 190'. Pass only the fields they actually gave you; everything else keeps its stored value. The result is the profile re-read from storage after the write, so report back only what it contains.",
            schema,
            Some(write_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        // The write lands in `user_physiological_profiles` and the reply is the
        // profile re-read from storage, so this both writes and reads identity
        // data. Without PROFILE the bits resolve to fitness:write, and the
        // read is not declared at all.
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::PROFILE,
        )
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
                    "set_physiology needs at least one measurement: ftp_watts, threshold_pace_sec_per_km, max_hr, resting_hr, lactate_threshold_percentage, vo2_max, weight, age, fitness_level, primary_sport or training_experience_years",
                ));
            }

            let repos = context.resources.repos();
            let existing = repos
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?;
            let created = existing.is_none();
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
            // the coach will repeat to the athlete, and it should describe the
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

            Ok(ToolResult::ok(json!({
                "saved": true,
                "created": created,
                "updated_fields": updated_fields,
                "profile": profile_payload(&stored),
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// EstimateVo2maxTool
// ============================================================================

/// Estimates `VO₂max` from a field test the athlete describes in conversation.
///
/// The five estimators in `dravr_cageux::algorithms::vo2max` — Cooper,
/// Rockport, Åstrand-Ryhming, Daniels' VDOT and a pace ratio — each need a
/// measured test result that no provider capture path supplies: a 12-minute
/// run distance, a timed mile walk with the finishing heart rate, steady-state
/// ergometer watts. Those are things an athlete *says*, so this is the capture
/// path. It estimates and reports; it does not write. Storing the number is
/// `set_physiology`'s job, which keeps one writer for the profile and lets the
/// coach confirm the value with the athlete before it becomes the basis for
/// every personalised calculation.
///
/// Body weight and age default to the stored profile when the athlete does
/// not restate them, and the response names which inputs came from there so
/// the coach can say so.
pub struct EstimateVo2maxTool;

/// The field-test methods the tool accepts, in the spelling the schema
/// advertises. Each maps to exactly one [`Vo2maxAlgorithm`] variant.
const VO2MAX_METHODS: [&str; 5] = [
    "cooper_test",
    "rockport_walk",
    "astrand_ryhming",
    "from_pace",
    "from_vdot",
];

impl EstimateVo2maxTool {
    fn properties() -> BTreeMap<String, PropertySchema> {
        let mut properties = BTreeMap::new();
        properties.insert(
            "method".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Which field test the athlete did — one of cooper_test, rockport_walk, astrand_ryhming, from_pace, from_vdot. cooper_test: distance run in 12 minutes. \
                     rockport_walk: a timed one-mile walk with heart rate at the finish. \
                     astrand_ryhming: steady-state cycling at a known power with heart rate. \
                     from_pace: a hard 3–8 minute speed and an easy speed. \
                     from_vdot: a VDOT the athlete already knows."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        for (name, property_type, description) in [
            (
                "distance_meters",
                "number",
                "cooper_test: metres covered in 12 minutes on flat ground.",
            ),
            (
                "time_seconds",
                "number",
                "rockport_walk: seconds taken to walk one mile (1,609 m) as fast as possible.",
            ),
            (
                "heart_rate",
                "number",
                "rockport_walk: heart rate in bpm immediately at the finish. astrand_ryhming: steady-state heart rate during the ride, 120–170 bpm.",
            ),
            (
                "power_watts",
                "number",
                "astrand_ryhming: the steady power held on the ergometer, in watts.",
            ),
            (
                "weight_kg",
                "number",
                "Body weight in kilograms. rockport_walk and astrand_ryhming need it; when omitted the stored profile weight is used.",
            ),
            (
                "age",
                "integer",
                "Age in years. rockport_walk needs it; when omitted the stored profile age is used.",
            ),
            (
                "max_speed_ms",
                "number",
                "from_pace: the fastest speed in metres per second the athlete can hold for 3–8 minutes.",
            ),
            (
                "recovery_speed_ms",
                "number",
                "from_pace: the athlete's easy or recovery speed in metres per second.",
            ),
            (
                "vdot",
                "number",
                "from_vdot: the VDOT value, 30–85. It is already VO2max in ml/kg/min, so this reports it after range-checking.",
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
            "gender".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "rockport_walk and astrand_ryhming: the sex the published equation was fitted on, female or male."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties
    }

    /// Read a required number for the named method, so the error names both
    /// the field and the test it belongs to.
    fn required_number(args: &Value, key: &str, method: &str) -> AppResult<f64> {
        optional_number(args, key)?
            .ok_or_else(|| AppError::invalid_input(format!("{method} needs '{key}'")))
    }

    /// Weight in kg from the call, else from the profile, recording the source.
    fn weight_kg(
        args: &Value,
        profile: Option<&UserPhysiologicalProfile>,
        defaults: &mut Vec<&'static str>,
        method: &str,
    ) -> AppResult<f64> {
        if let Some(w) = optional_number(args, "weight_kg")? {
            return Ok(w);
        }
        if let Some(w) = profile.and_then(|p| p.weight) {
            defaults.push("weight_kg");
            return Ok(w);
        }
        Err(AppError::invalid_input(format!(
            "{method} needs 'weight_kg' — none was given and the profile has no weight; ask the athlete or save it with set_physiology"
        )))
    }

    /// Age in years from the call, else from the profile, recording the source.
    fn age(
        args: &Value,
        profile: Option<&UserPhysiologicalProfile>,
        defaults: &mut Vec<&'static str>,
    ) -> AppResult<u8> {
        let years = match optional_whole_number(args, "age")? {
            Some(a) => a,
            None => match profile.and_then(|p| p.age) {
                Some(a) => {
                    defaults.push("age");
                    u64::from(a)
                }
                None => {
                    return Err(AppError::invalid_input(
                        "rockport_walk needs 'age' — none was given and the profile has no age; ask the athlete or save it with set_physiology",
                    ))
                }
            },
        };
        u8::try_from(years)
            .map_err(|_| AppError::invalid_input(format!("'age' must be at most 255, got {years}")))
    }

    /// The published equations were fitted per sex; cageux encodes it as
    /// 0 = female, 1 = male.
    fn gender(args: &Value, method: &str) -> AppResult<u8> {
        match args.get("gender").and_then(Value::as_str) {
            Some(g) if g.eq_ignore_ascii_case("female") => Ok(0),
            Some(g) if g.eq_ignore_ascii_case("male") => Ok(1),
            Some(other) => Err(AppError::invalid_input(format!(
                "'gender' must be female or male, got '{other}'"
            ))),
            None => Err(AppError::invalid_input(format!(
                "{method} needs 'gender' (female or male) — the published equation is fitted per sex"
            ))),
        }
    }

    /// Build the estimator from the call, filling weight and age from the
    /// profile where the athlete did not restate them.
    fn algorithm(
        args: &Value,
        profile: Option<&UserPhysiologicalProfile>,
        defaults: &mut Vec<&'static str>,
    ) -> AppResult<(String, Vo2maxAlgorithm)> {
        let method = args
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "'method' is required: one of {}",
                    VO2MAX_METHODS.join(", ")
                ))
            })?;
        let algorithm = match method.as_str() {
            "cooper_test" => Vo2maxAlgorithm::CooperTest {
                distance_meters: Self::required_number(args, "distance_meters", &method)?,
            },
            "rockport_walk" => Vo2maxAlgorithm::RockportWalk {
                weight_kg: Self::weight_kg(args, profile, defaults, &method)?,
                age: Self::age(args, profile, defaults)?,
                gender: Self::gender(args, &method)?,
                time_seconds: Self::required_number(args, "time_seconds", &method)?,
                heart_rate: Self::required_number(args, "heart_rate", &method)?,
            },
            "astrand_ryhming" => Vo2maxAlgorithm::AstrandRyhming {
                gender: Self::gender(args, &method)?,
                heart_rate: Self::required_number(args, "heart_rate", &method)?,
                power_watts: Self::required_number(args, "power_watts", &method)?,
                weight_kg: Self::weight_kg(args, profile, defaults, &method)?,
            },
            "from_pace" => Vo2maxAlgorithm::FromPace {
                max_speed_ms: Self::required_number(args, "max_speed_ms", &method)?,
                recovery_speed_ms: Self::required_number(args, "recovery_speed_ms", &method)?,
            },
            "from_vdot" => Vo2maxAlgorithm::FromVdot {
                vdot: Self::required_number(args, "vdot", &method)?,
            },
            other => {
                return Err(AppError::invalid_input(format!(
                    "unknown method '{other}': expected one of {}",
                    VO2MAX_METHODS.join(", ")
                )))
            }
        };
        Ok((method, algorithm))
    }
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for EstimateVo2maxTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(Self::properties()),
            required: Some(vec!["method".to_owned()]),
            ..Default::default()
        };
        tool_definition(
            "estimate_vo2max",
            "Estimate the athlete's VO2max in ml/kg/min from a field test they describe — a Cooper 12-minute run distance, a Rockport timed mile walk with finishing heart rate, an Astrand-Ryhming steady-state ride at a known power, a hard-versus-easy pace ratio, or a VDOT they already know. Call it when the athlete reports a test result such as 'I ran 2.8 km in 12 minutes' or 'I walked a mile in 13 minutes and my heart rate was 140'. Body weight and age come from the stored profile when not restated, and the result says which inputs were defaulted. This only estimates: to keep the number, call set_physiology with vo2_max after the athlete confirms it.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        // Reads the stored profile for weight and age defaults and echoes
        // `stored_vo2_max`, so it discloses identity data. Runtime
        // requirements alone resolve to an empty scope list, which the
        // read-only default grant satisfies.
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::PROFILE,
        )
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

            let profile = context
                .resources
                .repos()
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?;

            let mut defaults_from_profile: Vec<&'static str> = Vec::new();
            let (method, algorithm) =
                Self::algorithm(&args, profile.as_ref(), &mut defaults_from_profile)?;

            // Every failure the estimator can raise is an input outside the
            // range the published equation was fitted on, so it is the
            // athlete's number to correct, not a server fault.
            let vo2max = algorithm
                .estimate_vo2max()
                .map_err(|e| AppError::invalid_input(format!("cannot estimate VO2max: {e}")))?;

            info!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                method = %method,
                // The method and which inputs were defaulted, never the
                // measurements themselves — they are health data.
                defaults = %defaults_from_profile.join(","),
                "estimated VO2max from a field test"
            );

            Ok(ToolResult::ok(json!({
                "method": method,
                "vo2max_ml_kg_min": (vo2max * 10.0).round() / 10.0,
                "formula": algorithm.description(),
                "defaults_from_profile": defaults_from_profile,
                "stored_vo2_max": profile.as_ref().and_then(|p| p.vo2_max),
                "saved": false,
                "to_store": "call set_physiology with vo2_max once the athlete confirms the number",
            })))
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
    ]
}

// Guardian security classification (see `crate::security`). The write is
// internal and correctable, echoes no third-party text, and sends nothing
// outbound, so it carries no labels.
crate::declare_security!(SetPhysiologyTool => empty);

// Pure computation over inputs the athlete states plus a read of their own
// profile. Nothing is written, nothing leaves the process, and the response
// carries no third-party text.
crate::declare_security!(EstimateVo2maxTool => empty);
