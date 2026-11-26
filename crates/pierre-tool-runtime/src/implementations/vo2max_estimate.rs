// ABOUTME: estimate_vo2max — a VO2max or VDOT estimate from a field test or race the athlete describes, which it never saves
// ABOUTME: Read-only: set_physiology is the one writer of the profile; this tool only estimates and reports

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use crate::read_subject::DataClass;
use crate::tool_data::ToolData;
use dravr_barrage::Stream;
use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::CivilDate;
use serde::Serialize;
use serde_json::Value;
use tracing::info;

use crate::context::ToolExecutionContext;
use crate::conversions::{answers_with, ok_typed, tool_definition, tool_result_to_response};
use crate::implementations::physiology_args::{optional_number, optional_whole_number};
use crate::read_subject;
use crate::runtime::ToolRuntime;
use dravr_cageux::algorithms::{
    VdotAlgorithm, Vo2maxAlgorithm, DANIELS_TABLE_DISTANCE_METERS, PREDICTABLE_VDOT,
};
use dravr_cageux::config::intelligence::VO2MaxCalculator;
use dravr_cageux::performance_prediction::PerformancePredictor;
use dravr_cageux::plausibility::computation::VO2MAX_ML_KG_MIN;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_config::environment::TrainingZonesConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AnchorOrigin, TenantId, UserPhysiologicalProfile};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_services::athlete_vdot::{self, vdot_beyond_tables_warning};
use pierre_services::effective_physiology::{effective_physiology, EffectivePhysiology};
use pierre_services::stored_health::Anchors;
use pierre_tools_core::ToolResult;

/// Seconds in Cooper's test: the 12-minute run read as a race through
/// Daniels' curve gives a VDOT beside Cooper's `VO2max`.
const COOPER_TEST_SECONDS: f64 = 720.0;

/// Where a heart rate a Swain estimate used came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeartRateSource {
    /// Given in this call.
    Call,
    /// The athlete's own saved value.
    Profile,
    /// A provider's value imported into the profile; `origin` names it.
    ProviderImport,
    /// Read from the athlete's devices: the weekly promotion of their morning
    /// readings, or their latest reading when nothing is saved.
    DeviceReadings,
    /// An estimate standing in on the profile until a measurement exists.
    Estimate,
}

/// One heart rate an estimate used, and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct HeartRateUsed {
    /// The heart rate, bpm.
    pub bpm: f64,
    /// Where it came from.
    pub source: HeartRateSource,
    /// Who produced it, when known: the provider for an import, the device
    /// source for a reading.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The day it was stated, imported or measured, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<CivilDate>,
}

/// The maximum and resting heart rate a Swain estimate read the stage
/// against: the estimate is as good as these, so the athlete sees both.
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct HeartRatesUsed {
    /// Maximum heart rate.
    pub max_hr: HeartRateUsed,
    /// Resting heart rate.
    pub resting_hr: HeartRateUsed,
}

/// What `estimate_vo2max` answers with.
///
/// Deliberately does not save. Each number comes off a published equation
/// fitted on a field test, or Daniels' curves read on a race, and an athlete
/// should confirm it before it starts shaping their zones — which is what
/// `to_store` says to do. A `VO2max` and a VDOT are different numbers, and
/// each is reported under its own name.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct EstimateVo2maxResult {
    /// Which field test, race or statement the estimate came from.
    pub method: String,
    /// The `VO2max` estimate, in ml/kg/min, rounded to one decimal; never
    /// clamped. Absent for `race_result` and `from_vdot`, which give a VDOT.
    pub vo2max_ml_kg_min: Option<f64>,
    /// The VDOT, Daniels' performance index read from a race, rounded to one
    /// decimal: from `race_result`, `from_vdot`, or `cooper_test`'s 12-minute
    /// run read as a race. Never a `VO2max`. Absent for the other methods.
    pub vdot: Option<f64>,
    /// The equations used, named so the numbers are auditable.
    pub formula: String,
    /// What the athlete should know before trusting the numbers: an estimate
    /// outside the human range (with the inputs to recheck), a race outside
    /// the distances Daniels' tables cover.
    pub warnings: Vec<String>,
    /// `swain_vo2_reserve`: the maximum and resting heart rate used, and
    /// where each came from.
    pub heart_rates_used: Option<HeartRatesUsed>,
    /// Which inputs were taken from the stored profile rather than given in
    /// the call, by name — so the athlete can see what the estimate assumed.
    pub defaults_from_profile: Vec<&'static str>,
    /// The `VO2max` already on the profile, for comparison. Absent when none
    /// is stored.
    pub stored_vo2_max: Option<f64>,
    /// The VDOT already on the profile, for comparison. Absent when none is
    /// stored.
    pub stored_vdot: Option<f64>,
    /// The threshold pace the VDOT implies, in seconds per kilometre: the pace
    /// whose oxygen cost on Daniels' curve is the athlete's threshold share of
    /// VDOT, which `set_physiology` stores as `threshold_pace_sec_per_km`.
    /// Only from a VDOT: absent for `rockport_walk` and `swain_vo2_reserve`,
    /// whose `VO2max` says nothing about running pace.
    pub implied_threshold_pace_sec_per_km: Option<f64>,
    /// Always false: this tool estimates, it does not write.
    pub saved: bool,
    /// What to do with the numbers once the athlete confirms them.
    pub to_store: String,
}

/// What one method estimates: a `VO2max` from a field test, a VDOT from a
/// race or a statement, or Cooper's both.
enum Estimate {
    /// A `VO2max` from one of cageux's field-test equations.
    Vo2max(Vo2maxAlgorithm),
    /// A race read through Daniels' curves.
    Race {
        /// The race distance, metres.
        distance_meters: f64,
        /// The finishing time, seconds.
        time_seconds: f64,
        /// The VDOT it reads as.
        vdot: f64,
    },
    /// A VDOT the athlete states.
    StatedVdot {
        /// The VDOT.
        vdot: f64,
    },
    /// Cooper's test: its `VO2max`, and its 12-minute run read as a race.
    Cooper {
        /// Metres covered in 12 minutes.
        distance_meters: f64,
    },
}

/// The time `seconds` as `h:mm:ss` or `m:ss`.
fn clock(seconds: f64) -> String {
    PerformancePredictor::format_time(seconds)
}

/// A distance in kilometres as an athlete says it: "10 km", "21.1 km".
fn kilometres(distance_meters: f64) -> String {
    let km = distance_meters / 1_000.0;
    if km.fract().abs() < 0.05 {
        format!("{km:.0} km")
    } else {
        format!("{km:.1} km")
    }
}

/// The warning for a `VO2max` outside the human range, naming the inputs of
/// `method` to recheck. It is reported as computed, never clamped.
fn implausible_vo2max_warning(vo2max: f64, method: &str) -> Option<String> {
    if VO2MAX_ML_KG_MIN.in_range(vo2max) {
        return None;
    }
    let inputs = match method {
        "swain_vo2_reserve" => "the watts, the body weight and the heart rates",
        "rockport_walk" => "the walk time, the finishing heart rate, the weight and the age",
        _ => "the distance",
    };
    Some(format!(
        "VO2max {vo2max:.1} ml/kg/min is outside the {:.0}-{:.0} range people measure; recheck {inputs}",
        VO2MAX_ML_KG_MIN.min,
        VO2MAX_ML_KG_MIN.max
    ))
}

/// The warning for a VDOT outside Daniels' tables (30-85): less certain
/// within the 20-90 race times are predicted for; outside that, no race
/// times are predicted for it and `set_physiology` does not save it.
fn vdot_range_warning(vdot: f64) -> Option<String> {
    if PREDICTABLE_VDOT.contains(&vdot) {
        return vdot_beyond_tables_warning(vdot);
    }
    Some(format!(
        "VDOT {vdot:.1} is outside the {:.0}-{:.0} range race times are predicted for: none \
         are predicted for it and set_physiology does not save it",
        PREDICTABLE_VDOT.start(),
        PREDICTABLE_VDOT.end()
    ))
}

/// The warning for a race outside the distances Daniels' tables cover.
fn race_distance_warning(distance_meters: f64) -> Option<String> {
    (!DANIELS_TABLE_DISTANCE_METERS.contains(&distance_meters)).then(|| {
        format!(
            "Daniels' tables cover races from 1500 m to the marathon; a VDOT from {} rests on \
             his curves beyond them and is less certain",
            kilometres(distance_meters)
        )
    })
}

// ============================================================================
// EstimateVo2maxTool
// ============================================================================

/// Estimates `VO₂max`, or a VDOT, from a field test or race the athlete
/// describes in conversation.
///
/// Cooper's run, the Rockport walk and Swain's `VO2`-reserve cycle stage give
/// a `VO2max` through cageux's estimators; a race, or Cooper's run read as a
/// 12-minute race, gives a VDOT through Daniels' curves; an athlete may also
/// state a VDOT. Each needs a measured result no provider capture path
/// supplies, so this is the capture path. It estimates and reports; it does
/// not write. Storing a number is `set_physiology`'s job, which keeps one
/// writer for the profile and lets the agent confirm the value with the
/// athlete first.
///
/// Body weight, age and the heart rates default to the stored profile when
/// the athlete does not restate them, and the response names which inputs
/// came from there so the agent can say so.
#[derive(Debug)]
pub struct EstimateVo2maxTool;

/// The methods the tool accepts, in the spelling the schema advertises.
const VO2MAX_METHODS: [&str; 5] = [
    "cooper_test",
    "rockport_walk",
    "swain_vo2_reserve",
    "race_result",
    "from_vdot",
];

/// The methods whose equation takes a body weight (`Self::weight_kg`), which
/// the athlete's own defaults in when the call gives none.
const WEIGHT_METHODS: [&str; 2] = ["rockport_walk", "swain_vo2_reserve"];

impl EstimateVo2maxTool {
    fn properties() -> BTreeMap<String, PropertySchema> {
        let mut properties = BTreeMap::new();
        properties.insert(
            "method".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "cooper_test (metres in 12 min: VO2max, VDOT), rockport_walk (mile walk \
                     time, finish HR), swain_vo2_reserve (6-min cycle stage at 65-75% of HR \
                     reserve: watts, end HR), race_result (race or 3-8 min effort: VDOT), \
                     from_vdot (a known VDOT)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        for (name, property_type, description) in [
            (
                "distance_meters",
                "number",
                "cooper_test: metres covered in 12 minutes. race_result: the distance in metres (5 km is 5000).",
            ),
            (
                "time_seconds",
                "number",
                "rockport_walk: seconds to walk one mile. race_result: the finishing time in seconds (19:30 is 1170).",
            ),
            (
                "heart_rate",
                "number",
                "rockport_walk: bpm at the finish. swain_vo2_reserve: bpm at the end of the stage.",
            ),
            (
                "power_watts",
                "number",
                "swain_vo2_reserve: watts held through the stage.",
            ),
            (
                "max_hr",
                "number",
                "swain_vo2_reserve: maximum heart rate; omitted, the saved one.",
            ),
            (
                "resting_hr",
                "number",
                "swain_vo2_reserve: resting heart rate; omitted, the saved one.",
            ),
            (
                "weight_kg",
                "number",
                "rockport_walk, swain_vo2_reserve: body weight in kg; omitted, the stored one.",
            ),
            (
                "age",
                "integer",
                "Age in years. rockport_walk needs it; when omitted the stored profile age is used.",
            ),
            ("vdot", "number", "from_vdot: the VDOT, 20-90."),
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
                    "rockport_walk: the sex the equation was fitted on, female or male.".to_owned(),
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

    /// The refusal of a method that needs an input of the athlete's wellness
    /// (`key`) the call did not give: why none stands in.
    fn wellness_missing(method: &str, key: &str, what: &str) -> AppError {
        // A coach's read without the athlete's wellness grant leaves it out,
        // so none stands in, and the coach is told why rather than that the
        // athlete has none (carnet#1387).
        AppError::invalid_input(read_subject::wellness_withheld().map_or_else(
            || {
                format!(
                    "{method} needs '{key}' — none was given and the athlete has no {what} on \
                     file; ask the athlete or save it with set_physiology"
                )
            },
            |withheld| {
                format!(
                    "{method} needs '{key}' — none was given, and the athlete's {what} is their \
                     wellness. {} Give {key} to estimate without it.",
                    withheld.note()
                )
            },
        ))
    }

    /// Weight in kg from the call, else the athlete's effective weight (the
    /// profile's, else their latest weigh-in), recording the source.
    fn weight_kg(
        args: &Value,
        effective_weight: Option<f64>,
        defaults: &mut Vec<&'static str>,
        method: &str,
    ) -> AppResult<f64> {
        if let Some(w) = optional_number(args, "weight_kg")? {
            return Ok(w);
        }
        if let Some(w) = effective_weight {
            defaults.push("weight_kg");
            return Ok(w);
        }
        Err(Self::wellness_missing(method, "weight_kg", "body weight"))
    }

    /// Whether the call reads the athlete's wellness: a method that needs a
    /// body weight with no `weight_kg` given, or Swain's stage with no
    /// `resting_hr` given.
    fn reads_wellness(args: &Value) -> bool {
        let method = args
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase);
        let absent = |key: &str| args.get(key).is_none_or(Value::is_null);
        method.is_some_and(|method| {
            (WEIGHT_METHODS.contains(&method.as_str()) && absent("weight_kg"))
                || (method == "swain_vo2_reserve" && absent("resting_hr"))
        })
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

    /// The maximum heart rate from the call, else the profile's, with where
    /// it came from: the athlete's own, or a provider's import. Never one
    /// predicted from age: Swain's estimate is only as good as this number.
    fn max_hr(
        args: &Value,
        profile: Option<&UserPhysiologicalProfile>,
        defaults: &mut Vec<&'static str>,
    ) -> AppResult<HeartRateUsed> {
        if let Some(bpm) = optional_number(args, "max_hr")? {
            return Ok(HeartRateUsed {
                bpm,
                source: HeartRateSource::Call,
                origin: None,
                as_of: None,
            });
        }
        let Some((profile, saved)) = profile.and_then(|p| p.max_hr.map(|bpm| (p, bpm))) else {
            return Err(AppError::invalid_input(
                "swain_vo2_reserve needs 'max_hr' — none was given and the profile has none; ask \
                 the athlete for their measured maximum heart rate (an age-predicted one is too \
                 rough for this test)",
            ));
        };
        defaults.push("max_hr");
        let bpm = f64::from(saved);
        Ok(profile.threshold_provenance.max_hr.as_ref().map_or(
            HeartRateUsed {
                bpm,
                source: HeartRateSource::Profile,
                origin: None,
                as_of: profile.threshold_stated_on.max_hr,
            },
            |import| HeartRateUsed {
                bpm,
                source: HeartRateSource::ProviderImport,
                origin: import.origin.clone(),
                as_of: import.as_of,
            },
        ))
    }

    /// The resting heart rate from the call, else the profile's (with its
    /// recorded origin), else the athlete's latest device reading.
    fn resting_hr(
        args: &Value,
        physiology: &EffectivePhysiology,
        defaults: &mut Vec<&'static str>,
    ) -> AppResult<HeartRateUsed> {
        if let Some(bpm) = optional_number(args, "resting_hr")? {
            return Ok(HeartRateUsed {
                bpm,
                source: HeartRateSource::Call,
                origin: None,
                as_of: None,
            });
        }
        let profile = physiology.profile.as_ref();
        if let Some(bpm) = profile.and_then(|p| p.resting_hr) {
            defaults.push("resting_hr");
            let stamp = profile.and_then(|p| p.anchor_provenance.resting_hr);
            let source = match stamp.map(|s| s.origin) {
                None | Some(AnchorOrigin::Athlete) => HeartRateSource::Profile,
                Some(AnchorOrigin::Imported) => HeartRateSource::ProviderImport,
                Some(AnchorOrigin::Promoted) => HeartRateSource::DeviceReadings,
                Some(AnchorOrigin::Estimated) => HeartRateSource::Estimate,
            };
            return Ok(HeartRateUsed {
                bpm: f64::from(bpm),
                source,
                origin: None,
                as_of: stamp.and_then(|s| s.as_of),
            });
        }
        if let Some(reading) = &physiology.measured.resting_hr {
            defaults.push("resting_hr");
            return Ok(HeartRateUsed {
                bpm: f64::from(reading.value),
                source: HeartRateSource::DeviceReadings,
                origin: Some(reading.source.clone()),
                as_of: Some(reading.date),
            });
        }
        Err(Self::wellness_missing(
            "swain_vo2_reserve",
            "resting_hr",
            "resting heart rate",
        ))
    }

    /// Build the estimate from the call, filling weight, age and the heart
    /// rates from the athlete's physiology where they did not restate them.
    fn estimate(
        args: &Value,
        physiology: &EffectivePhysiology,
        defaults: &mut Vec<&'static str>,
    ) -> AppResult<(String, Estimate, Option<HeartRatesUsed>)> {
        let profile = physiology.profile.as_ref();
        let weight = physiology.weight();
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
        let mut heart_rates = None;
        let estimate = match method.as_str() {
            "cooper_test" => Estimate::Cooper {
                distance_meters: Self::required_number(args, "distance_meters", &method)?,
            },
            "rockport_walk" => Estimate::Vo2max(Vo2maxAlgorithm::RockportWalk {
                weight_kg: Self::weight_kg(args, weight, defaults, &method)?,
                age: Self::age(args, profile, defaults)?,
                gender: Self::gender(args, &method)?,
                time_seconds: Self::required_number(args, "time_seconds", &method)?,
                heart_rate: Self::required_number(args, "heart_rate", &method)?,
            }),
            "swain_vo2_reserve" => {
                let power_watts = Self::required_number(args, "power_watts", &method)?;
                let heart_rate = Self::required_number(args, "heart_rate", &method)?;
                let max_hr = Self::max_hr(args, profile, defaults)?;
                let resting_hr = Self::resting_hr(args, physiology, defaults)?;
                let algorithm = Vo2maxAlgorithm::SwainVo2Reserve {
                    power_watts,
                    heart_rate,
                    max_hr: max_hr.bpm,
                    resting_hr: resting_hr.bpm,
                    weight_kg: Self::weight_kg(args, weight, defaults, &method)?,
                };
                heart_rates = Some(HeartRatesUsed { max_hr, resting_hr });
                Estimate::Vo2max(algorithm)
            }
            // A race the athlete ran gives VDOT through Daniels' own curves;
            // nothing is invented in between, and it is reported as a VDOT.
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
                Estimate::Race {
                    distance_meters,
                    time_seconds,
                    vdot,
                }
            }
            "from_vdot" => {
                let vdot = Self::required_number(args, "vdot", &method)?;
                if !PREDICTABLE_VDOT.contains(&vdot) {
                    return Err(AppError::invalid_input(format!(
                        "VDOT {vdot:.1} is outside the {:.0}-{:.0} range race times are \
                         predicted for",
                        PREDICTABLE_VDOT.start(),
                        PREDICTABLE_VDOT.end()
                    )));
                }
                Estimate::StatedVdot { vdot }
            }
            other => {
                return Err(AppError::invalid_input(format!(
                    "unknown method '{other}': expected one of {}",
                    VO2MAX_METHODS.join(", ")
                )))
            }
        };
        Ok((method, estimate, heart_rates))
    }
}

/// Every failure a `VO2max` estimator raises is an input outside the range
/// its published equation was fitted on (a Swain stage outside 65-75 % of
/// heart-rate reserve names the athlete's bpm window), so it is the athlete's
/// number to correct, not a server fault; cageux's message is passed through.
fn vo2max_of(algorithm: &Vo2maxAlgorithm) -> AppResult<f64> {
    algorithm
        .estimate_vo2max()
        .map_err(|e| AppError::invalid_input(format!("cannot estimate VO2max: {e}")))
}

/// Round to one decimal.
fn tenth(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// The numbers one estimate gives, before they are rounded for the result.
struct Numbers {
    vo2max: Option<f64>,
    vdot: Option<f64>,
    /// How the VDOT was found, for `vdot_method`.
    vdot_basis: Option<String>,
    formula: String,
    warnings: Vec<String>,
}

/// Compute `estimate` for `method`.
fn numbers(method: &str, estimate: &Estimate) -> AppResult<Numbers> {
    Ok(match estimate {
        Estimate::Vo2max(algorithm) => {
            let vo2max = vo2max_of(algorithm)?;
            Numbers {
                vo2max: Some(vo2max),
                vdot: None,
                vdot_basis: None,
                formula: format!("{}: {}", algorithm.description(), algorithm.formula()),
                warnings: implausible_vo2max_warning(vo2max, method)
                    .into_iter()
                    .collect(),
            }
        }
        Estimate::Race {
            distance_meters,
            time_seconds,
            vdot,
        } => Numbers {
            vo2max: None,
            vdot: Some(*vdot),
            vdot_basis: Some(format!(
                "race: {} in {}",
                kilometres(*distance_meters),
                clock(*time_seconds)
            )),
            formula: "VDOT (Daniels): the race's oxygen cost divided by the share of VO2max \
                      held for its duration"
                .to_owned(),
            warnings: race_distance_warning(*distance_meters)
                .into_iter()
                .chain(vdot_range_warning(*vdot))
                .collect(),
        },
        Estimate::StatedVdot { vdot } => Numbers {
            vo2max: None,
            vdot: Some(*vdot),
            vdot_basis: Some("stated".to_owned()),
            formula: "VDOT as the athlete states it".to_owned(),
            warnings: vdot_range_warning(*vdot).into_iter().collect(),
        },
        Estimate::Cooper { distance_meters } => {
            let algorithm = Vo2maxAlgorithm::CooperTest {
                distance_meters: *distance_meters,
            };
            let vo2max = vo2max_of(&algorithm)?;
            let mut warnings: Vec<String> = implausible_vo2max_warning(vo2max, method)
                .into_iter()
                .collect();
            let vdot = match VdotAlgorithm::Daniels
                .calculate_vdot(*distance_meters, COOPER_TEST_SECONDS)
            {
                Ok(vdot) => {
                    warnings.extend(vdot_range_warning(vdot));
                    Some(vdot)
                }
                Err(e) => {
                    warnings.push(format!(
                        "no VDOT from this run read as a 12-minute race: {e}"
                    ));
                    None
                }
            };
            Numbers {
                vo2max: Some(vo2max),
                vdot,
                vdot_basis: vdot
                    .map(|_| format!("cooper_test: {} in 12:00", kilometres(*distance_meters))),
                formula: format!(
                    "{}: {}; VDOT (Daniels): the 12-minute run read as a race",
                    algorithm.description(),
                    algorithm.formula()
                ),
                warnings,
            }
        }
    })
}

/// What `to_store` tells the agent to save, for what this estimate gave.
fn to_store(method: &str, numbers: &Numbers) -> String {
    let mut steps = Vec::new();
    if numbers.vo2max.is_some() {
        steps.push(format!(
            "call set_physiology with vo2_max and vo2_max_method '{method}' once the athlete \
             confirms it"
        ));
    }
    if numbers.vdot.is_some() {
        let method_label = numbers.vdot_basis.as_deref().unwrap_or("stated");
        steps.push(format!(
            "save the VDOT with set_physiology vdot, vdot_as_of (the day it was run or stated) \
             and vdot_method '{method_label}', and threshold_pace_sec_per_km when the athlete \
             confirms the implied pace too"
        ));
    }
    steps.join("; ")
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
            "Estimate VO2max (ml/kg/min) or VDOT from a field test, race or known VDOT the athlete reports (see `method`), e.g. 'I ran 2.8 km in 12 minutes'. Weight, age and heart rates default to stored data, named in the result. Estimates only: save with set_physiology once confirmed (`to_store`).",
            schema,
            annotations("Estimate VO2max", Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // Reads the stored profile for weight, age and heart-rate defaults
        // and echoes `stored_vo2_max`, so it discloses identity data. Runtime
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

            // A method that needs a body weight or resting heart rate the call
            // did not give reads the athlete's: their wellness, which on a
            // coach's read is on the athlete's record before it is read
            // (carnet#1445).
            if Self::reads_wellness(&args) {
                read_subject::record_wellness_read(context.resources.repos()).await?;
            }

            // The stored profile (withheld from an external caller when it was
            // written from first-party-only data, carnet#769) and the latest
            // measurement of each anchor it lacks.
            let absent = |key: &str| args.get(key).is_none_or(Value::is_null);
            let mut physiology = effective_physiology(
                context.resources.repos(),
                context.resources.provider_registry().as_ref(),
                tenant_id,
                user_id,
                // A value the caller gives is never looked up, and a coach's
                // read of an athlete looks up none of their wellness readings.
                read_subject::readable_anchors(Anchors {
                    weight: absent("weight_kg"),
                    resting_hr: absent("resting_hr"),
                    ..Anchors::default()
                }),
            )
            .await?;
            // The defaults and the echo come from the physiology this read
            // may show: a coach reads the athlete's training physiology, with
            // no body weight or resting heart rate to default to
            // (carnet#1387).
            physiology.profile = read_subject::readable_physiology(physiology.profile);
            let profile = physiology.profile.as_ref();

            let mut defaults_from_profile: Vec<&'static str> = Vec::new();
            let (method, estimate, heart_rates_used) =
                Self::estimate(&args, &physiology, &mut defaults_from_profile)?;
            let numbers = numbers(&method, &estimate)?;

            info!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                method = %method,
                // The method and which inputs were defaulted, never the
                // measurements themselves — they are health data.
                defaults = %defaults_from_profile.join(","),
                "estimated VO2max or VDOT from a field test"
            );

            ok_typed(
                "estimate_vo2max",
                EstimateVo2maxResult {
                    vo2max_ml_kg_min: numbers.vo2max.map(tenth),
                    vdot: numbers.vdot.map(tenth),
                    // A threshold pace comes from a VDOT only: a VO2max says
                    // nothing about running economy, and a cycling one
                    // nothing about running at all.
                    implied_threshold_pace_sec_per_km: numbers.vdot.and_then(|vdot| {
                        implied_threshold_pace(
                            vdot,
                            athlete_vdot::lactate_threshold(profile),
                            &context.resources.config().training_zones,
                        )
                    }),
                    to_store: to_store(&method, &numbers),
                    formula: numbers.formula,
                    warnings: numbers.warnings,
                    heart_rates_used,
                    defaults_from_profile,
                    stored_vo2_max: profile.and_then(|p| p.vo2_max),
                    stored_vdot: profile.and_then(|p| p.vdot.as_ref()).map(|v| v.value),
                    saved: false,
                    method,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The threshold pace a VDOT implies, in seconds per kilometre.
///
/// Read off cageux's [`VO2MaxCalculator::calculate_pace_zones`], the function
/// the pace zones are cut by: threshold pace is the speed whose oxygen cost on
/// Daniels' curve is the athlete's lactate threshold share of VDOT (Daniels'
/// 0.88 when none is saved). So a pace offered here and a pace drawn in the
/// zones cannot disagree. `None` when the VDOT cuts no zones — inventing a
/// pace would be worse than saying nothing.
fn implied_threshold_pace(
    vdot: f64,
    lactate_threshold: f64,
    zones: &TrainingZonesConfig,
) -> Option<f64> {
    // Pace zones read only the VDOT and the lactate threshold; the
    // calculator's heart-rate fields feed nothing asked for here.
    let pace = VO2MaxCalculator::new(vdot, 0, 0, lactate_threshold)
        .calculate_pace_zones(zones)
        .ok()?
        .threshold_pace_sec_per_km;
    (pace.is_finite() && pace > 0.0).then(|| tenth(pace))
}

// Pure computation over inputs the athlete states plus a read of their own
// profile. Nothing is written, nothing leaves the process, and the response
// carries no third-party text.
// Training physiology, the estimates read against it, and the zones and
// configuration it sets.
crate::declare_security!(
    EstimateVo2maxTool => empty,
    reads: ToolData::of(DataClass::Training, &[Stream::Fitness, Stream::Profile]),
);
