// ABOUTME: estimate_vo2max — a VO2max estimate from a field test the athlete describes, which it never saves
// ABOUTME: Read-only: set_physiology is the one writer of the profile; this tool only estimates and reports
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tracing::info;

use crate::context::ToolExecutionContext;
use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::implementations::data_helpers::read_only_annotations;
use crate::implementations::physiology::{optional_number, optional_whole_number};
use crate::runtime::ToolRuntime;
use dravr_cageux::algorithms::{VdotAlgorithm, Vo2maxAlgorithm};
use dravr_cageux::config::intelligence::VO2MaxCalculator;
use dravr_cageux::physiological_constants::physiological_defaults::DEFAULT_LACTATE_THRESHOLD;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_config::environment::TrainingZonesConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserPhysiologicalProfile};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

/// What `estimate_vo2max` answers with.
///
/// Deliberately does not save. The estimate comes off a published equation
/// fitted on a field test, and an athlete should confirm a number before it
/// starts shaping their zones — which is what `to_store` says to do.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct EstimateVo2maxResult {
    /// Which field test the estimate came from.
    pub method: String,
    /// The estimate, in ml/kg/min, rounded to one decimal.
    pub vo2max_ml_kg_min: f64,
    /// The equation used, named so the number is auditable.
    pub formula: String,
    /// Which inputs were taken from the stored profile rather than given in
    /// the call, by name — so the athlete can see what the estimate assumed.
    pub defaults_from_profile: Vec<&'static str>,
    /// The `VO2max` already on the profile, for comparison. Absent when none
    /// is stored.
    pub stored_vo2_max: Option<f64>,
    /// The threshold pace this estimate implies, in seconds per kilometre —
    /// the running pace at the share of velocity-at-`VO2max` the training
    /// zones config states, which is what `set_physiology` stores as
    /// `threshold_pace_sec_per_km`. Absent when the estimate does not give a
    /// usable velocity.
    pub implied_threshold_pace_sec_per_km: Option<f64>,
    /// Always false: this tool estimates, it does not write.
    pub saved: bool,
    /// What to do with the number once the athlete confirms it.
    pub to_store: String,
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
/// agent confirm the value with the athlete before it becomes the basis for
/// every personalised calculation.
///
/// Body weight and age default to the stored profile when the athlete does
/// not restate them, and the response names which inputs came from there so
/// the agent can say so.
pub struct EstimateVo2maxTool;

/// The field-test methods the tool accepts, in the spelling the schema
/// advertises. Each maps to exactly one [`Vo2maxAlgorithm`] variant.
const VO2MAX_METHODS: [&str; 6] = [
    "cooper_test",
    "rockport_walk",
    "astrand_ryhming",
    "from_pace",
    "from_vdot",
    "race_result",
];

impl EstimateVo2maxTool {
    fn properties() -> BTreeMap<String, PropertySchema> {
        let mut properties = BTreeMap::new();
        properties.insert(
            "method".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Which field test the athlete did — one of cooper_test, rockport_walk, astrand_ryhming, from_pace, from_vdot, race_result. cooper_test: distance run in 12 minutes. \
                     rockport_walk: a timed one-mile walk with heart rate at the finish. \
                     astrand_ryhming: steady-state cycling at a known power with heart rate. \
                     from_pace: a hard 3–8 minute speed and an easy speed. \
                     from_vdot: a VDOT the athlete already knows. \
                     race_result: a race or time trial the athlete ran — its distance and its time."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        for (name, property_type, description) in [
            (
                "distance_meters",
                "number",
                "cooper_test: metres covered in 12 minutes on flat ground. race_result: the race distance in metres (5 km is 5000).",
            ),
            (
                "time_seconds",
                "number",
                "rockport_walk: seconds taken to walk one mile (1,609 m) as fast as possible. race_result: the finishing time in seconds (19:30 is 1170).",
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
            // A race the athlete actually ran is a field test with a much
            // longer history than the rest: the distance and the time give
            // VDOT through Daniels' own curve, and VDOT is already a method
            // here. Nothing is invented in between.
            "race_result" => {
                let distance_meters = Self::required_number(args, "distance_meters", &method)?;
                let time_seconds = Self::required_number(args, "time_seconds", &method)?;
                let vdot = VdotAlgorithm::Daniels
                    .calculate_vdot(distance_meters, time_seconds)
                    .map_err(|e| {
                        AppError::invalid_input(format!(
                            "race_result: {distance_meters:.0} m in {time_seconds:.0} s does not \
                             give a usable VDOT: {e}"
                        ))
                    })?;
                Vo2maxAlgorithm::FromVdot { vdot }
            }
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
        answers_with::<EstimateVo2maxResult>(tool_definition(
            "estimate_vo2max",
            "Estimate the athlete's VO2max in ml/kg/min from a field test they describe — a Cooper 12-minute run distance, a Rockport timed mile walk with finishing heart rate, an Astrand-Ryhming steady-state ride at a known power, a hard-versus-easy pace ratio, a VDOT they already know, or a race or time trial they ran (its distance and time). Call it when the athlete reports a test result such as 'I ran 2.8 km in 12 minutes' or 'I walked a mile in 13 minutes and my heart rate was 140'. Body weight and age come from the stored profile when not restated, and the result says which inputs were defaulted. This only estimates: to keep the number, call set_physiology with vo2_max after the athlete confirms it.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // Reads the stored profile for weight and age defaults and echoes
        // `stored_vo2_max`, so it discloses identity data. Runtime
        // requirements alone resolve to an empty scope list, which the
        // read-only default grant satisfies.
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
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

            ok_typed(
                "estimate_vo2max",
                EstimateVo2maxResult {
                    method,
                    vo2max_ml_kg_min: (vo2max * 10.0).round() / 10.0,
                    formula: algorithm.description(),
                    defaults_from_profile,
                    stored_vo2_max: profile.as_ref().and_then(|p| p.vo2_max),
                    implied_threshold_pace_sec_per_km: implied_threshold_pace(
                        vo2max,
                        profile
                            .as_ref()
                            .and_then(|p| p.lactate_threshold_percentage)
                            .unwrap_or(DEFAULT_LACTATE_THRESHOLD),
                        &context.resources.config().training_zones,
                    ),
                    saved: false,
                    to_store:
                        "call set_physiology with vo2_max, and with threshold_pace_sec_per_km when \
                         the athlete confirms the implied pace too"
                            .to_owned(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The threshold pace an estimated `VO2max` implies, in seconds per
/// kilometre.
///
/// Read off cageux's [`VO2MaxCalculator::calculate_pace_zones`], the function
/// the pace zones are cut by: threshold pace is the pace the athlete's lactate
/// threshold (`lactate_threshold`, a fraction of `VO2max`) puts them at on
/// Daniels' curve, and the threshold zone's slow edge is a configured multiple
/// of it. So a pace offered here and a pace drawn in the zones cannot
/// disagree. `None` when the pace is not usable — an estimate that low
/// describes no running pace, and inventing one would be worse than saying
/// nothing.
fn implied_threshold_pace(
    vo2max: f64,
    lactate_threshold: f64,
    zones: &TrainingZonesConfig,
) -> Option<f64> {
    if !vo2max.is_finite() || vo2max <= 0.0 || zones.vdot_threshold_zone_slow_factor <= 0.0 {
        return None;
    }
    // Pace zones read only VO2max and the lactate threshold; the calculator's
    // heart-rate fields and sport efficiency feed nothing asked for here.
    let paces =
        VO2MaxCalculator::new(vo2max, 0, 0, lactate_threshold, 1.0).calculate_pace_zones(zones);
    let pace = paces.threshold_pace_range.0 / zones.vdot_threshold_zone_slow_factor;
    (pace.is_finite() && pace > 0.0).then(|| (pace * 10.0).round() / 10.0)
}

// Pure computation over inputs the athlete states plus a read of their own
// profile. Nothing is written, nothing leaves the process, and the response
// carries no third-party text.
crate::declare_security!(EstimateVo2maxTool => empty);
