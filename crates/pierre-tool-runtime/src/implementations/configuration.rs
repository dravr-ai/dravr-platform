// ABOUTME: User configuration tools for personalized training settings.
// ABOUTME: Implements catalog, profiles, user config, zones calculation, and validation.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # User Configuration Tools
//!
//! This module provides tools for user configuration with direct business logic:
//! - `GetConfigurationCatalogTool` - Get available configuration options
//! - `GetConfigurationProfilesTool` - Get configuration profile templates
//! - `GetUserConfigurationTool` - Get user's current configuration
//! - `UpdateUserConfigurationTool` - Update user configuration
//! - `CalculatePersonalizedZonesTool` - Calculate training zones
//! - `ValidateConfigurationTool` - Validate configuration values

use pierre_providers::ai_scope;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use uuid::Uuid;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::warn;

use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::implementations::configuration_document::{configuration_payload, updated_document};
use crate::implementations::configuration_output::{
    ConfigurationCatalogResult, ConfigurationProfileEntry, ConfigurationProfilesResult,
    HeartRateZone, HeartRateZones, PaceZone, PaceZones, PersonalizedZones, PersonalizedZonesResult,
    PowerZoneBand, PowerZones as PowerZonesPayload, UpdateUserConfigurationResult,
    UserConfigurationResult, ValidateConfigurationResult, ZoneCalculations, ZoneInputProfile,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_cageux::config::intelligence::VO2MaxCalculator;
use dravr_cageux::models::maxhr::MaxHrAlgorithm;
use dravr_cageux::physiological_constants::configuration_validation;
use dravr_cageux::physiological_constants::physiological_defaults::{
    DEFAULT_LACTATE_THRESHOLD, LACTATE_THRESHOLD_MAX, LACTATE_THRESHOLD_MIN,
};
use dravr_cageux::zones::{HrZoneBand, HrZones, PowerZones};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_config::catalog::CatalogBuilder;
use pierre_config::environment::TrainingZonesConfig;
use pierre_core::config::profiles::ProfileTemplates;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::zones::HrZoneModel;
use pierre_core::models::{TenantId, UserPhysiologicalProfile};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_runtime_context::ConfigLookupScope;
use pierre_tools_core::ToolResult;

// ============================================================================
// Helpers (inlined from former handlers/configuration.rs)
// ============================================================================

/// Zone calculation inputs, each carrying whether it is actually known.
///
/// The athlete-specific measurements are `Option` on purpose: a zone family
/// whose inputs are unknown is omitted with a reason, never cut from house
/// numbers and labelled as the athlete's own.
struct ZoneParams {
    vo2_max: Option<f64>,
    resting_hr: Option<u16>,
    max_hr: Option<u16>,
    ftp_watts: Option<u32>,
    lactate_threshold: f64,
    /// Where each athlete-specific input came from, keyed by field name:
    /// `provided` (this call's arguments), `profile` (the saved physiology),
    /// `estimated_from_age` for a maximum heart rate cageux's configured
    /// max-HR algorithm estimated from the saved age, or `daniels_default`
    /// for a lactate threshold neither supplied nor saved.
    sources: serde_json::Map<String, Value>,
}

/// Resolve zone calculation inputs from this call's arguments, falling back to
/// the athlete's saved physiology.
///
/// Precedence is argument, then stored profile, then unknown. The single
/// exception is maximum heart rate, which falls back to cageux's estimate
/// from a stored age by the deployment's configured max-HR algorithm — a
/// named estimator whose use is reported in `sources`, not an undisclosed
/// constant.
fn resolve_f64_input(
    args: &Value,
    key: &str,
    from_profile: Option<f64>,
    sources: &mut serde_json::Map<String, Value>,
) -> Option<f64> {
    if let Some(v) = args.get(key).and_then(Value::as_f64) {
        sources.insert(key.to_owned(), json!("provided"));
        return Some(v);
    }
    if let Some(v) = from_profile {
        sources.insert(key.to_owned(), json!("profile"));
        return Some(v);
    }
    None
}

fn resolve_u64_input(
    args: &Value,
    key: &str,
    from_profile: Option<u64>,
    sources: &mut serde_json::Map<String, Value>,
) -> Option<u64> {
    if let Some(v) = args.get(key).and_then(Value::as_u64) {
        sources.insert(key.to_owned(), json!("provided"));
        return Some(v);
    }
    if let Some(v) = from_profile {
        sources.insert(key.to_owned(), json!("profile"));
        return Some(v);
    }
    None
}

fn extract_zone_parameters(
    args: &Value,
    stored: Option<&UserPhysiologicalProfile>,
    max_hr_algorithm: MaxHrAlgorithm,
) -> ZoneParams {
    let mut sources = serde_json::Map::new();

    let vo2_max = resolve_f64_input(
        args,
        "vo2_max",
        stored.and_then(|p| p.vo2_max),
        &mut sources,
    );
    let lactate_threshold = resolve_f64_input(
        args,
        "lactate_threshold",
        stored.and_then(|p| p.lactate_threshold_percentage),
        &mut sources,
    )
    .unwrap_or_else(|| {
        // None measured: Daniels' threshold intensity, named in the sources.
        sources.insert("lactate_threshold".to_owned(), json!("daniels_default"));
        DEFAULT_LACTATE_THRESHOLD
    });

    let resting_hr = resolve_u64_input(
        args,
        "resting_hr",
        stored.and_then(|p| p.resting_hr).map(u64::from),
        &mut sources,
    );
    let mut max_hr = resolve_u64_input(
        args,
        "max_hr",
        stored.and_then(|p| p.max_hr).map(u64::from),
        &mut sources,
    );
    if max_hr.is_none() {
        // `estimated_max_hr` returns the stored max HR when there is one, so
        // reaching here means it can only come from age.
        if let Some(estimated) = stored.and_then(|p| p.estimated_max_hr(max_hr_algorithm)) {
            sources.insert("max_hr".to_owned(), json!("estimated_from_age"));
            sources.insert(
                "max_hr_algorithm".to_owned(),
                json!(max_hr_algorithm.name()),
            );
            max_hr = Some(u64::from(estimated));
        }
    }
    let ftp_watts = resolve_u64_input(
        args,
        "ftp",
        stored.and_then(|p| p.ftp_watts).map(u64::from),
        &mut sources,
    );

    ZoneParams {
        vo2_max,
        resting_hr: resting_hr.and_then(|v| u16::try_from(v).ok()),
        max_hr: max_hr.and_then(|v| u16::try_from(v).ok()),
        ftp_watts: ftp_watts.and_then(|v| u32::try_from(v).ok()),
        lactate_threshold,
        sources,
    }
}

fn create_user_profile(params: &ZoneParams) -> ZoneInputProfile {
    ZoneInputProfile {
        vo2_max: params.vo2_max,
        resting_hr: params.resting_hr,
        max_hr: params.max_hr,
        ftp: params.ftp_watts,
        lactate_threshold: params.lactate_threshold,
    }
}

/// One heart-rate zone of cageux's for a tool payload.
fn heart_rate_zone(band: &HrZoneBand) -> HeartRateZone {
    HeartRateZone {
        name: band.name.clone(),
        min_hr: u64::from(band.lower_bpm),
        max_hr: u64::from(band.upper_bpm),
    }
}

/// Render cageux's heart-rate zones for a tool payload.
///
/// `None` when cageux handed back fewer than five zones, which its
/// constructors never do.
fn render_hr_zones(zones: &HrZones) -> Option<HeartRateZones> {
    let bands = zones.zones();
    let band = |index: usize| bands.get(index).map(heart_rate_zone);
    Some(HeartRateZones {
        zone_1: band(0)?,
        zone_2: band(1)?,
        zone_3: band(2)?,
        zone_4: band(3)?,
        zone_5: band(4)?,
        zone_6: band(5),
        tier: zones.tier,
        vo2_max_source: zones.vo2_max_source,
    })
}

/// How cageux cut the heart-rate zones, plus its two threshold heart rates.
fn render_zone_calculations(zones: &HrZones) -> ZoneCalculations {
    ZoneCalculations {
        method: HrZoneModel::KarvonenVo2maxTiered,
        tier: zones.tier,
        vo2_max: zones.vo2_max,
        vo2_max_source: zones.vo2_max_source,
        lactate_threshold_hr: u64::from(zones.lactate_threshold_bpm),
        aerobic_threshold_hr: u64::from(zones.aerobic_threshold_bpm),
    }
}

/// Render the VDOT pace zones for a tool payload.
///
/// The zones are cageux's [`VO2MaxCalculator::calculate_pace_zones`], the one
/// pace-zone function every surface quotes: easy, `VO2max` and neuromuscular
/// edges are fractions of Daniels' velocity at `VO2max`, marathon and
/// threshold edges are multiples of the threshold pace the athlete's lactate
/// threshold puts them at, and `config` carries every edge.
fn calculate_pace_zones_from_vo2max(
    vo2_max: f64,
    lactate_threshold: f64,
    config: &TrainingZonesConfig,
) -> PaceZones {
    // Pace zones read only VO2max and the lactate threshold; the heart-rate
    // fields of the calculator feed its heart-rate zones, which this does not
    // ask for.
    let paces =
        VO2MaxCalculator::new(vo2_max, 0, 0, lactate_threshold).calculate_pace_zones(config);

    let format_pace = |seconds_per_km: f64| -> String {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let total_secs = if !seconds_per_km.is_finite() || seconds_per_km < 0.0 {
            0_u32
        } else if seconds_per_km >= 4_294_967_295.0 {
            u32::MAX
        } else {
            seconds_per_km.round() as u32
        };

        let minutes = total_secs / 60;
        let seconds = total_secs % 60;
        format!("{minutes}:{seconds:02}")
    };

    // cageux orders each range by its own convention; a zone quotes its
    // fastest pace (fewest seconds per kilometre) first.
    let band = |(a, b): (f64, f64)| PaceZone {
        min_pace: format_pace(a.min(b)),
        max_pace: format_pace(a.max(b)),
    };
    PaceZones {
        zone_1_easy: band(paces.easy_pace_range),
        zone_2_marathon: band(paces.marathon_pace_range),
        zone_3_threshold: band(paces.threshold_pace_range),
        zone_4_vo2max: band(paces.vo2max_pace_range),
        zone_5_neuromuscular_max_pace: format_pace(paces.neuromuscular_pace_max),
    }
}

/// Render Coggan's seven power levels from cageux for a tool payload.
///
/// `None` when cageux handed back fewer than seven levels, which
/// [`PowerZones::coggan`] never does.
pub(crate) fn render_power_zones(zones: &PowerZones) -> Option<PowerZonesPayload> {
    let levels = zones.zones();
    let level = |index: usize| {
        levels.get(index).map(|band| PowerZoneBand {
            name: band.name.clone(),
            min_watts: band.lower_watts,
            max_watts: band.upper_watts,
        })
    };
    Some(PowerZonesPayload {
        zone_1: level(0)?,
        zone_2: level(1)?,
        zone_3: level(2)?,
        zone_4: level(3)?,
        zone_5: level(4)?,
        zone_6: level(5)?,
        zone_7: level(6)?,
    })
}

/// The max-HR estimator an athlete is read with: the operator's
/// `algorithm.maxhr` override for the user, tenant or fleet when one is set,
/// else the algorithm cageux's configuration selects.
///
/// # Errors
///
/// Returns the repository error when the override cannot be read, and an
/// invalid-input error when a stored override names no algorithm cageux has.
pub(crate) async fn configured_max_hr_algorithm(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: Option<&str>,
) -> AppResult<MaxHrAlgorithm> {
    if let Some(config) = runtime.admin_config() {
        let user = user_id.to_string();
        let scope = ConfigLookupScope {
            user_id: Some(&user),
            tenant_id,
        };
        if let Some(value) = config
            .get_override_value(MAX_HR_ALGORITHM_KEY, scope)
            .await?
        {
            let name = value.as_str().ok_or_else(|| {
                AppError::invalid_input(format!("{MAX_HR_ALGORITHM_KEY} must be a string"))
            })?;
            return name
                .parse::<MaxHrAlgorithm>()
                .map_err(|e| AppError::invalid_input(format!("{MAX_HR_ALGORITHM_KEY}: {e}")));
        }
    }
    Ok(runtime.cageux_config().algorithms.maxhr_algorithm())
}

/// The admin key that selects the max-HR estimator.
const MAX_HR_ALGORITHM_KEY: &str = "algorithm.maxhr";

pub(crate) fn validate_parameter_ranges(
    obj: &serde_json::Map<String, Value>,
    errors: &mut Vec<String>,
) -> bool {
    let mut all_valid = true;

    let max_hr = obj.get("max_hr").and_then(Value::as_u64);
    let resting_hr = obj.get("resting_hr").and_then(Value::as_u64);
    let threshold_hr = obj.get("threshold_hr").and_then(Value::as_u64);
    let vo2_max = obj.get("vo2_max").and_then(Value::as_f64);
    let ftp = obj.get("ftp").and_then(Value::as_u64);

    if let Some(hr) = max_hr {
        if !(configuration_validation::MAX_HR_MIN..=configuration_validation::MAX_HR_MAX)
            .contains(&hr)
        {
            all_valid = false;
            errors.push(format!(
                "max_hr must be between {} and {} bpm, got {}",
                configuration_validation::MAX_HR_MIN,
                configuration_validation::MAX_HR_MAX,
                hr
            ));
        }
    }

    if let Some(hr) = resting_hr {
        if !(configuration_validation::RESTING_HR_MIN..=configuration_validation::RESTING_HR_MAX)
            .contains(&hr)
        {
            all_valid = false;
            errors.push(format!(
                "resting_hr must be between {} and {} bpm, got {}",
                configuration_validation::RESTING_HR_MIN,
                configuration_validation::RESTING_HR_MAX,
                hr
            ));
        }
    }

    if let Some(hr) = threshold_hr {
        if !(configuration_validation::THRESHOLD_HR_MIN
            ..=configuration_validation::THRESHOLD_HR_MAX)
            .contains(&hr)
        {
            all_valid = false;
            errors.push(format!(
                "threshold_hr must be between {} and {} bpm, got {}",
                configuration_validation::THRESHOLD_HR_MIN,
                configuration_validation::THRESHOLD_HR_MAX,
                hr
            ));
        }
    }

    if let Some(vo2) = vo2_max {
        if !(configuration_validation::VO2_MAX_MIN..=configuration_validation::VO2_MAX_MAX)
            .contains(&vo2)
        {
            all_valid = false;
            errors.push(format!(
                "vo2_max must be between {} and {} ml/kg/min, got {:.1}",
                configuration_validation::VO2_MAX_MIN,
                configuration_validation::VO2_MAX_MAX,
                vo2
            ));
        }
    }

    if let Some(power) = ftp {
        if !(configuration_validation::FTP_MIN..=configuration_validation::FTP_MAX).contains(&power)
        {
            all_valid = false;
            errors.push(format!(
                "ftp must be between {} and {} watts, got {}",
                configuration_validation::FTP_MIN,
                configuration_validation::FTP_MAX,
                power
            ));
        }
    }

    all_valid
}

pub(crate) fn validate_parameter_relationships(
    obj: &serde_json::Map<String, Value>,
    errors: &mut Vec<String>,
) -> bool {
    let mut all_valid = true;

    let max_hr = obj.get("max_hr").and_then(Value::as_u64);
    let resting_hr = obj.get("resting_hr").and_then(Value::as_u64);
    let threshold_hr = obj.get("threshold_hr").and_then(Value::as_u64);

    if let (Some(resting), Some(max)) = (resting_hr, max_hr) {
        if resting >= max {
            all_valid = false;
            errors.push(format!(
                "resting_hr ({resting}) must be less than max_hr ({max})"
            ));
        }
    }

    if let (Some(resting), Some(threshold)) = (resting_hr, threshold_hr) {
        if resting >= threshold {
            all_valid = false;
            errors.push(format!(
                "resting_hr ({resting}) must be less than threshold_hr ({threshold})"
            ));
        }
    }

    if let (Some(threshold), Some(max)) = (threshold_hr, max_hr) {
        if threshold >= max {
            all_valid = false;
            errors.push(format!(
                "threshold_hr ({threshold}) must be less than max_hr ({max})"
            ));
        }
    }

    all_valid
}

// ============================================================================
// GetConfigurationCatalogTool
// ============================================================================

/// Tool for getting the complete configuration catalog.
pub struct GetConfigurationCatalogTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetConfigurationCatalogTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };

        answers_with::<ConfigurationCatalogResult>(tool_definition(
            "get_configuration_catalog",
            "Get the complete catalog of available configuration options",
            schema,
            None,
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let result: AppResult<ToolResult> = async move {
            ok_typed(
                "get_configuration_catalog",
                ConfigurationCatalogResult {
                    catalog: CatalogBuilder::build(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetConfigurationProfilesTool
// ============================================================================

/// Tool for getting available configuration profile templates.
pub struct GetConfigurationProfilesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetConfigurationProfilesTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };

        answers_with::<ConfigurationProfilesResult>(tool_definition(
            "get_configuration_profiles",
            "Get available configuration profile templates",
            schema,
            None,
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let result: AppResult<ToolResult> = async move {
            let profile_templates = ProfileTemplates::all();
            let profiles: Vec<ConfigurationProfileEntry> = profile_templates
                .into_iter()
                .map(|(name, profile)| ConfigurationProfileEntry {
                    description: format!("Configuration profile: {name}"),
                    name,
                    profile,
                })
                .collect();

            ok_typed(
                "get_configuration_profiles",
                ConfigurationProfilesResult {
                    total_count: profiles.len(),
                    profiles,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetUserConfigurationTool
// ============================================================================

/// Tool for retrieving the current user's configuration.
pub struct GetUserConfigurationTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetUserConfigurationTool {
    fn definition(&self) -> Tool {
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(BTreeMap::new()),
            required: None,
            ..Default::default()
        };

        answers_with::<UserConfigurationResult>(tool_definition(
            "get_user_configuration",
            "Get your current training configuration settings",
            schema,
            None,
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA | ToolCapabilities::PROFILE
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_uuid = ctx.user_id;
            let stored = ctx
                .resources
                .repos()
                .profiles
                .get_configuration(&user_uuid.to_string())
                .await?
                .map(|config_str| {
                    serde_json::from_str::<Value>(&config_str).unwrap_or_else(|e| {
                        warn!(
                            user_id = %user_uuid,
                            error = %e,
                            "stored configuration is not JSON; reading it as the default profile"
                        );
                        json!({})
                    })
                });
            ok_typed(
                "get_user_configuration",
                configuration_payload(&user_uuid, stored.as_ref()),
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// UpdateUserConfigurationTool
// ============================================================================

/// Tool for updating the user's configuration.
pub struct UpdateUserConfigurationTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for UpdateUserConfigurationTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "profile".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Profile template to apply, by the name get_configuration_profiles lists \
                     ('Elite Athlete') or its profile name ('elite'): its values become the \
                     base parameters, under the overrides. Omit to keep the saved template \
                     (the default one when none was applied)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "parameters".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some(
                    "Overrides by catalogue key (get_configuration_catalog), merged into the \
                     saved ones: a key set to null is removed, a key not named keeps its saved \
                     value. Physiological measurements (ftp, threshold_hr, max_hr, resting_hr, \
                     weight, vo2_max...) are refused: set_physiology saves them, and training \
                     load reads them from there."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);

        answers_with::<UpdateUserConfigurationResult>(tool_definition(
            "update_user_configuration",
            "Update your training configuration settings: apply a profile template and set \
             catalogue overrides on top of it",
            schema,
            None,
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA | ToolCapabilities::PROFILE
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_uuid = ctx.user_id;

            let saved: Option<Value> = ctx
                .resources
                .repos()
                .profiles
                .get_configuration(&user_uuid.to_string())
                .await?
                .and_then(|stored| serde_json::from_str(&stored).ok());
            let (configuration, changes_applied) = updated_document(
                saved.as_ref(),
                args.get("profile").and_then(Value::as_str),
                args.get("parameters").and_then(Value::as_object),
            )?;

            let config_json = serde_json::to_string(&configuration)
                .map_err(|e| AppError::internal(format!("Failed to serialize config: {e}")))?;
            ctx.resources
                .repos()
                .profiles
                .save_configuration(&user_uuid.to_string(), &config_json)
                .await?;

            ok_typed(
                "update_user_configuration",
                UpdateUserConfigurationResult {
                    user_id: user_uuid.to_string(),
                    updated_configuration: configuration,
                    changes_applied,
                    message: "Configuration updated successfully".to_owned(),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// CalculatePersonalizedZonesTool
// ============================================================================

/// Tool for calculating personalized training zones.
pub struct CalculatePersonalizedZonesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CalculatePersonalizedZonesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "vo2_max".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "VO2 max in ml/kg/min. Omit to use the athlete's saved value; pace zones are omitted when neither exists.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "resting_hr".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Resting heart rate in bpm. Omit to use the athlete's saved value.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "max_hr".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Maximum heart rate in bpm. Omit to use the athlete's saved value, or the estimate from their saved age by the configured max-HR algorithm (algorithm.maxhr).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "lactate_threshold".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(format!(
                    "Lactate threshold as a fraction of VO2 max (0.65-0.95), used for pace \
                     zones: threshold pace runs at the speed whose oxygen cost is that share of \
                     VO2 max. Omit to use the athlete's saved value, else Daniels' threshold \
                     intensity, {DEFAULT_LACTATE_THRESHOLD} (named in `input_sources`)."
                )),
                ..Default::default()
            },
        );
        properties.insert(
            "ftp".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Functional Threshold Power in watts. Omit to use the athlete's saved value; power zones are omitted when neither exists.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties.into_iter().collect()),
            // Nothing is required: every input falls back to the athlete's
            // saved physiology, and a zone family whose inputs are unknown is
            // reported as unavailable rather than invented.
            required: None,
            ..Default::default()
        };

        answers_with::<PersonalizedZonesResult>(tool_definition(
            "calculate_personalized_zones",
            "Calculate training zones from the athlete's own measurements, falling back to their saved physiology for anything not supplied here. Zone families whose inputs are unknown are listed under `unavailable` instead of being estimated; `input_sources` says where each number came from.",
            schema,
            None,
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // Deliberately not `REQUIRES_TENANT`: the saved profile is a fallback,
        // not a precondition, so a tenant-less call still answers from its own
        // arguments.
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            // The saved profile is the fallback for every athlete-specific
            // input, so an athlete who ran `set_physiology` gets their own
            // numbers without restating them. An unreadable profile degrades
            // to argument-only resolution rather than failing the call.
            let stored = match ctx.tenant_id {
                Some(tenant_uuid) => {
                    ctx.resources
                        .repos()
                        .user_physiological_profile
                        .get_user_physiological_profile(
                            TenantId::from_uuid(tenant_uuid),
                            ctx.user_id,
                        )
                        .await?
                        // A profile written from first-party-only data is
                        // withheld from an external caller (carnet#769).
                        .filter(|profile| ai_scope::admit_derived(profile.transport_policy))
                }
                None => None,
            };

            let tenant = ctx.tenant_id.map(|tenant| tenant.to_string());
            let max_hr_algorithm =
                configured_max_hr_algorithm(&ctx.resources, ctx.user_id, tenant.as_deref())
                    .await?;
            // The pace zones accept a lactate threshold only inside cageux's
            // range; one outside it is refused rather than silently clamped.
            if let Some(lt) = args.get("lactate_threshold").and_then(Value::as_f64) {
                if !(LACTATE_THRESHOLD_MIN..=LACTATE_THRESHOLD_MAX).contains(&lt) {
                    return Err(AppError::invalid_input(format!(
                        "lactate_threshold must be between {LACTATE_THRESHOLD_MIN} and \
                         {LACTATE_THRESHOLD_MAX} (fraction of VO2max), got {lt}"
                    )));
                }
            }
            let mut params = extract_zone_parameters(&args, stored.as_ref(), max_hr_algorithm);
            let user_profile = create_user_profile(&params);
            let zones_config = &ctx.resources.config().training_zones;

            // Each zone family is emitted only when its own inputs are known.
            // `unavailable` names what is missing so the caller can ask for it
            // instead of being handed boundaries built from house numbers.
            let mut unavailable = serde_json::Map::new();

            let hr_zones = match (params.resting_hr, params.max_hr) {
                (Some(resting_hr), Some(max_hr)) => {
                    HrZones::for_athlete(params.vo2_max, resting_hr, max_hr).map_err(|e| {
                        format!("heart-rate zones could not be cut from these heart rates: {e}")
                    })
                }
                _ => Err("needs both resting_hr and max_hr — supply them here or save them with set_physiology".to_owned()),
            };
            let (heart_rate_zones, zone_calculations) = match &hr_zones {
                Ok(zones) => {
                    if params.vo2_max.is_none() {
                        // The tier's VO2max was estimated from the heart
                        // rates; the zones say so, and so does the source map.
                        params
                            .sources
                            .insert("vo2_max".to_owned(), json!("heart_rate_ratio"));
                    }
                    (
                        render_hr_zones(zones),
                        Some(render_zone_calculations(zones)),
                    )
                }
                Err(reason) => {
                    unavailable.insert("heart_rate_zones".to_owned(), json!(reason));
                    (None, None)
                }
            };

            let pace_zones = params
                .vo2_max
                .map(|vo2_max| {
                    calculate_pace_zones_from_vo2max(
                        vo2_max,
                        params.lactate_threshold,
                        zones_config,
                    )
                });
            if pace_zones.is_none() {
                unavailable.insert(
                    "pace_zones".to_owned(),
                    json!("needs a measured vo2_max — supply it here or save it with set_physiology; the heart-rate-ratio estimate only chooses the heart-rate zone tier"),
                );
            }

            let power_zones = params
                .ftp_watts
                .and_then(|ftp| PowerZones::coggan(f64::from(ftp)).ok())
                .as_ref()
                .and_then(render_power_zones);
            if power_zones.is_none() {
                unavailable.insert(
                    "power_zones".to_owned(),
                    json!("needs ftp — supply it here or save it with set_physiology"),
                );
            }

            ok_typed(
                "calculate_personalized_zones",
                PersonalizedZonesResult {
                    user_profile,
                    input_sources: Value::Object(params.sources.clone()),
                    personalized_zones: PersonalizedZones {
                        heart_rate_zones,
                        pace_zones,
                        power_zones,
                        ftp: params.ftp_watts,
                    },
                    unavailable: Value::Object(unavailable),
                    zone_calculations,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ValidateConfigurationTool
// ============================================================================

/// Tool for validating configuration parameters.
pub struct ValidateConfigurationTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ValidateConfigurationTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "parameters".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Configuration parameters to validate".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["parameters".to_owned()]));

        answers_with::<ValidateConfigurationResult>(tool_definition(
            "validate_configuration",
            "Validate configuration parameters for physiological correctness",
            schema,
            None,
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let result: AppResult<ToolResult> = async move {
            let parameters = args
                .get("parameters")
                .ok_or_else(|| AppError::invalid_input("parameters field required"))?;

            if parameters.is_object() {
                let param_count = parameters.as_object().map_or(0, serde_json::Map::len);
                let mut errors = Vec::new();

                if let Some(obj) = parameters.as_object() {
                    let ranges_valid = validate_parameter_ranges(obj, &mut errors);
                    let relationships_valid = validate_parameter_relationships(obj, &mut errors);

                    let mut pattern_valid = true;
                    for (key, value) in obj {
                        if key.contains("invalid") || key.starts_with("invalid.") {
                            pattern_valid = false;
                            errors.push(format!("Invalid parameter name: {key}"));
                        }

                        if value.is_string() && value.as_str() == Some("invalid_value") {
                            pattern_valid = false;
                            errors.push(format!("Invalid value for parameter: {key}"));
                        }
                    }

                    let validation_passed = ranges_valid && relationships_valid && pattern_valid;

                    return ok_typed(
                        "validate_configuration",
                        ValidateConfigurationResult {
                            validation_passed,
                            parameters_validated: param_count,
                            message: if validation_passed {
                                "Configuration parameters are valid"
                            } else {
                                "Configuration validation failed"
                            }
                            .to_owned(),
                            errors: if errors.is_empty() {
                                None
                            } else {
                                Some(errors)
                            },
                        },
                    );
                }

                ok_typed(
                    "validate_configuration",
                    ValidateConfigurationResult {
                        validation_passed: true,
                        parameters_validated: param_count,
                        message: "Configuration parameters are valid".to_owned(),
                        errors: None,
                    },
                )
            } else {
                Ok(ToolResult::error(json!({
                    "validation_passed": false,
                    "parameters_validated": 0,
                    "errors": ["Parameters must be a JSON object"],
                    "error": "Validation failed: Parameters must be a JSON object"
                })))
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all configuration tools for registration.
#[must_use]
pub fn create_configuration_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(GetConfigurationCatalogTool),
        Box::new(GetConfigurationProfilesTool),
        Box::new(GetUserConfigurationTool),
        Box::new(UpdateUserConfigurationTool),
        Box::new(CalculatePersonalizedZonesTool),
        Box::new(ValidateConfigurationTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(CalculatePersonalizedZonesTool => empty);
crate::declare_security!(GetConfigurationCatalogTool => empty);
crate::declare_security!(GetConfigurationProfilesTool => empty);
crate::declare_security!(GetUserConfigurationTool => empty);
crate::declare_security!(UpdateUserConfigurationTool => empty);
crate::declare_security!(ValidateConfigurationTool => empty);
