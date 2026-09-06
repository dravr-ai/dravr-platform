// ABOUTME: estimate_lactate_thresholds — LT1 and LT2 from a lactate step test the athlete reports, each construct named
// ABOUTME: Read-only sibling of set_physiology: analyses through dravr-cageux, writes nothing, points at the field to save
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Lactate step test → thresholds
//!
//! The coach hears "200 W 1.1, 225 W 1.4, 250 W 2.3, 275 W 4.1 mmol" and
//! needs LT1, LT2, the zones and the pace table. cageux's
//! [`LactateStepTest`] does the arithmetic by four named constructs; this
//! tool parses the stages, keeps the constructs apart in the reply, derives
//! power zones from the modified-Dmax LT2 through the same function
//! `set_physiology` persists an FTP with, and names the profile field to
//! save. It never writes — `set_physiology` stays the profile's only writer.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::info;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::implementations::configuration::{derive_power_zone_set, power_zones_payload};
use crate::implementations::data_helpers::read_only_annotations;
use crate::implementations::physiology::optional_number;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::environment::TrainingZonesConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserPhysiologicalProfile};
use pierre_intelligence::algorithms::lactate::{
    LactateIntensityUnit, LactateStage, LactateStepTest, LactateThresholdMethod, LactateThresholds,
    ThresholdOutcome, MAX_STAGES,
};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

/// Locate LT1 and LT2 from a graded lactate step test the athlete reports.
///
/// The stages carry intensity as watts or seconds per kilometre, blood
/// lactate in mmol/L and heart rate when a strap was worn. cageux's
/// [`LactateStepTest`] locates the thresholds by four named constructs —
/// the log-log breakpoint for LT1, modified Dmax, Dmax and the fixed
/// 4.0 mmol/L OBLA for LT2 — and the reply keeps them apart under their own
/// names because they do not coincide; a construct the protocol cannot
/// support comes back as not determinable with the reason, never as a rule
/// of thumb. Like `estimate_vo2max` this estimates and does not write:
/// `set_physiology` stays the profile's only writer and the reply names the
/// field to save.
pub struct EstimateLactateThresholdsTool;

/// The intensity units the tool accepts, in the spelling the schema
/// advertises. Each maps to one [`LactateIntensityUnit`].
const LACTATE_UNITS: [&str; 2] = ["watts", "seconds_per_km"];

/// Round to a fixed number of decimals for the reply.
fn round_to(value: f64, decimals: i32) -> f64 {
    let factor = 10_f64.powi(decimals);
    (value * factor).round() / factor
}

impl EstimateLactateThresholdsTool {
    fn stage_schema() -> PropertySchema {
        let mut properties = BTreeMap::new();
        for (name, description) in [
            (
                "intensity",
                "The stage's intensity in the test's unit: watts, or seconds per kilometre for a running pace.",
            ),
            (
                "lactate_mmol",
                "Blood lactate sampled at the end of the stage, in mmol/L.",
            ),
            (
                "heart_rate",
                "Heart rate at the end of the stage in bpm, when a strap was worn. Omit otherwise.",
            ),
        ] {
            properties.insert(
                name.to_owned(),
                PropertySchema {
                    property_type: "number".to_owned(),
                    description: Some(description.to_owned()),
                    ..Default::default()
                },
            );
        }
        PropertySchema {
            property_type: "object".to_owned(),
            description: Some("One stage of the step test.".to_owned()),
            properties: Some(properties),
            required: Some(vec!["intensity".to_owned(), "lactate_mmol".to_owned()]),
            ..Default::default()
        }
    }

    fn properties() -> BTreeMap<String, PropertySchema> {
        let mut properties = BTreeMap::new();
        properties.insert(
            "unit".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "How every stage's intensity is expressed — watts for power, seconds_per_km for a running pace."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "stages".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some(
                    "The stages in the order they were run, easiest first, at least four, each harder than the last."
                        .to_owned(),
                ),
                items: Some(Box::new(Self::stage_schema())),
                ..Default::default()
            },
        );
        properties
    }

    fn unit(args: &Value) -> AppResult<LactateIntensityUnit> {
        match args
            .get("unit")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("watts") => Ok(LactateIntensityUnit::Watts),
            Some("seconds_per_km") => Ok(LactateIntensityUnit::SecondsPerKm),
            Some(other) => Err(AppError::invalid_input(format!(
                "unknown unit '{other}': expected one of {}",
                LACTATE_UNITS.join(", ")
            ))),
            None => Err(AppError::invalid_input(format!(
                "'unit' is required: one of {}",
                LACTATE_UNITS.join(", ")
            ))),
        }
    }

    /// Read a required number off one stage, so the error names the stage.
    fn stage_number(raw: &Value, key: &str, index: usize) -> AppResult<f64> {
        optional_number(raw, key)?
            .ok_or_else(|| AppError::invalid_input(format!("stages[{index}] needs '{key}'")))
    }

    fn stages(args: &Value) -> AppResult<Vec<LactateStage>> {
        let raw_stages = args.get("stages").and_then(Value::as_array).ok_or_else(|| {
            AppError::invalid_input(
                "'stages' is required: the test's stages in order, each with intensity and lactate_mmol",
            )
        })?;
        // The engine refuses an oversized test too, and its ceiling is the one
        // that counts; refusing here as well means a caller that sends a whole
        // time series is answered before the tool allocates a stage per point.
        if raw_stages.len() > MAX_STAGES {
            return Err(AppError::invalid_input(format!(
                "a graded step test runs at most {MAX_STAGES} stages; got {}. Send the test's own stages, not a time series",
                raw_stages.len()
            )));
        }
        raw_stages
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                Ok(LactateStage {
                    intensity: Self::stage_number(raw, "intensity", index)?,
                    lactate_mmol: Self::stage_number(raw, "lactate_mmol", index)?,
                    heart_rate: optional_number(raw, "heart_rate")?,
                })
            })
            .collect()
    }

    fn test(args: &Value) -> AppResult<LactateStepTest> {
        Ok(LactateStepTest {
            unit: Self::unit(args)?,
            stages: Self::stages(args)?,
        })
    }

    /// One construct's verdict: the method, what it marks, its paper, and
    /// either the located point or the reason it could not be located.
    fn outcome_payload(method: LactateThresholdMethod, outcome: &ThresholdOutcome) -> Value {
        let mut payload = json!({
            "method": method.as_str(),
            "marks": method.threshold(),
            "reference": method.reference(),
        });
        match outcome {
            ThresholdOutcome::Determined(point) => {
                payload["outcome"] = json!("determined");
                payload["intensity"] = json!(round_to(point.intensity, 1));
                payload["lactate_mmol"] = json!(round_to(point.lactate_mmol, 2));
                payload["heart_rate"] = json!(point.heart_rate.map(|hr| round_to(hr, 0)));
            }
            ThresholdOutcome::NotDeterminable { reason } => {
                payload["outcome"] = json!("not_determinable");
                payload["reason"] = json!(reason);
            }
        }
        payload
    }

    /// Power zones anchored on the modified-Dmax LT2 when the stages are in
    /// watts — the same derivation `set_physiology` persists for an FTP.
    fn power_zones(thresholds: &LactateThresholds, config: &TrainingZonesConfig) -> Value {
        const ANCHOR: &str = "lt2_modified_dmax";
        if thresholds.unit != LactateIntensityUnit::Watts {
            return json!({
                "anchor": ANCHOR,
                "available": false,
                "reason": "power zones need stages in watts; the band table carries the paces",
            });
        }
        let Some(point) = thresholds.lt2_modified_dmax.point() else {
            return json!({
                "anchor": ANCHOR,
                "available": false,
                "reason": "modified Dmax could not locate LT2 on these stages",
            });
        };
        // cageux bounds every stage to 1–2500 W, so the rounded threshold
        // is positive and far inside u32.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ftp_watts = point.intensity.round() as u32;
        derive_power_zone_set(ftp_watts, config).map_or_else(
            || {
                json!({
                    "anchor": ANCHOR,
                    "available": false,
                    "reason": "the configured zone percentages do not produce increasing boundaries at this threshold",
                })
            },
            |zones| {
                json!({
                    "anchor": ANCHOR,
                    "available": true,
                    "ftp_watts": ftp_watts,
                    "zones": power_zones_payload(&zones),
                })
            },
        )
    }

    /// The constructs are independent, so nothing in the arithmetic forces
    /// LT1 below LT2. When a determined pair comes back inverted the test
    /// cannot support both numbers, and the reply says so rather than
    /// leaving a coach to quote a physiologically incoherent pair.
    fn ordering_note(thresholds: &LactateThresholds) -> Option<String> {
        let lt1 = thresholds.lt1_log_log.point()?;
        let inverted: Vec<&'static str> = [
            (
                LactateThresholdMethod::ModifiedDmax,
                &thresholds.lt2_modified_dmax,
            ),
            (LactateThresholdMethod::Dmax, &thresholds.lt2_dmax),
            (LactateThresholdMethod::Obla4, &thresholds.lt2_obla_4mmol),
        ]
        .into_iter()
        .filter_map(|(method, outcome)| {
            let lt2 = outcome.point()?;
            // Compare on the effort axis: for pace, a *lower* number is harder.
            let harder = match thresholds.unit {
                LactateIntensityUnit::Watts => lt2.intensity <= lt1.intensity,
                LactateIntensityUnit::SecondsPerKm => lt2.intensity >= lt1.intensity,
            };
            harder.then_some(method.as_str())
        })
        .collect();
        (!inverted.is_empty()).then(|| {
            format!(
                "LT2 by {} did not come out above LT1, which no graded test should produce. Treat both as unusable and re-run the test rather than reporting either.",
                inverted.join(" and ")
            )
        })
    }

    fn to_store(unit: LactateIntensityUnit) -> &'static str {
        match unit {
            LactateIntensityUnit::Watts => {
                "call set_physiology with ftp_watts at the LT2 the athlete confirms; the profile has no field for heart rate at threshold, so that number stays in this reply"
            }
            LactateIntensityUnit::SecondsPerKm => {
                "call set_physiology with threshold_pace_sec_per_km at the LT2 the athlete confirms; the profile has no field for heart rate at threshold, so that number stays in this reply"
            }
        }
    }

    fn payload(
        thresholds: &LactateThresholds,
        profile: Option<&UserPhysiologicalProfile>,
        config: &TrainingZonesConfig,
    ) -> Value {
        json!({
            "unit": thresholds.unit.as_str(),
            "stage_count": thresholds.stage_count,
            "lt1": Self::outcome_payload(LactateThresholdMethod::LogLog, &thresholds.lt1_log_log),
            "lt2": [
                Self::outcome_payload(LactateThresholdMethod::ModifiedDmax, &thresholds.lt2_modified_dmax),
                Self::outcome_payload(LactateThresholdMethod::Dmax, &thresholds.lt2_dmax),
                Self::outcome_payload(LactateThresholdMethod::Obla4, &thresholds.lt2_obla_4mmol),
            ],
            "band_table": thresholds.band_table.iter().map(|row| json!({
                "lactate_mmol": row.lactate_mmol,
                "intensity": round_to(row.intensity, 1),
                "heart_rate": row.heart_rate.map(|hr| round_to(hr, 0)),
            })).collect::<Vec<_>>(),
            "curve_fit": {
                "model": "lactate = c0 + c1·t + c2·t² + c3·t³, t = effort normalised to 0..1 across the stages",
                "coefficients": thresholds.curve.coefficients.iter().map(|c| round_to(*c, 4)).collect::<Vec<_>>(),
                "r_squared": round_to(thresholds.curve.r_squared, 4),
            },
            "power_zones": Self::power_zones(thresholds, config),
            "stored_profile": {
                "ftp_watts": profile.and_then(|p| p.ftp_watts),
                "threshold_pace_sec_per_km": profile.and_then(|p| p.threshold_pace_sec_per_km),
                "max_hr": profile.and_then(|p| p.max_hr),
            },
            "framing": "Name the construct with every number: the four do not coincide (Jamnick 2020). 4.0 mmol/L is a convention (Heck 1985; critique Faude 2009), not the athlete's threshold; trained athletes turn at 2.5–4.0 mmol/L (Seiler-Viken 2025).",
            "ordering_warning": Self::ordering_note(thresholds),
            "saved": false,
            "to_store": Self::to_store(thresholds.unit),
        })
    }
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for EstimateLactateThresholdsTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(Self::properties()),
            required: Some(vec!["unit".to_owned(), "stages".to_owned()]),
            ..Default::default()
        };
        tool_definition(
            "estimate_lactate_thresholds",
            "Locate the athlete's lactate thresholds from a step test they report — each stage's power in watts or pace in seconds per kilometre, its blood lactate in mmol/L, and heart rate if a strap was worn. Returns LT1 by the log-log breakpoint and LT2 by modified Dmax, Dmax and the 4.0 mmol/L convention, each under its own name with the intensity, lactate and heart rate at that point; the lactate band table from 1.0 to 4.0 mmol/L; and power zones anchored on the modified-Dmax LT2 when the stages are in watts. Call it when the athlete reports a test such as '200 W 1.1, 225 W 1.4, 250 W 2.3, 275 W 4.1 mmol'. Needs at least four stages, each harder than the last. This only estimates: to keep a threshold, call set_physiology with ftp_watts or threshold_pace_sec_per_km after the athlete confirms it.",
            schema,
            Some(read_only_annotations()),
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        // The analysis itself is pure arithmetic over what the athlete typed,
        // but the reply echoes their stored FTP, threshold pace and max HR, so
        // the call reads the profile and must carry `profile:read`. Declaring
        // only the runtime requirements would let a client holding the
        // read-only default grant (`fitness:read`) read identity data the
        // scope split exists to keep separate.
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
            let test = Self::test(&args)?;

            // Every failure the analysis can raise is a protocol the athlete
            // can correct — too few stages, a stage not harder than the last,
            // a reading outside what a meter produces — not a server fault.
            let thresholds = test.analyze().map_err(|e| {
                AppError::invalid_input(format!("cannot analyze the lactate test: {e}"))
            })?;

            let profile = context
                .resources
                .repos()
                .user_physiological_profile
                .get_user_physiological_profile(tenant_id, user_id)
                .await?;
            let zones_config = &context.resources.config().training_zones;

            info!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                unit = thresholds.unit.as_str(),
                stage_count = thresholds.stage_count,
                // Which constructs resolved, never the measurements — they
                // are health data.
                lt1_determined = thresholds.lt1_log_log.point().is_some(),
                lt2_modified_dmax_determined = thresholds.lt2_modified_dmax.point().is_some(),
                lt2_obla_determined = thresholds.lt2_obla_4mmol.point().is_some(),
                "analyzed a lactate step test"
            );

            Ok(ToolResult::ok(Self::payload(
                &thresholds,
                profile.as_ref(),
                zones_config,
            )))
        }
        .await;
        tool_result_to_response(result)
    }
}
// Pure computation over the stages the athlete states plus a read of their
// own profile. Nothing is written, nothing leaves the process, and the
// response carries no third-party text.
crate::declare_security!(EstimateLactateThresholdsTool => empty);
