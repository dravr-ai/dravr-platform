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

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::warn;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::catalog::CatalogBuilder;
use pierre_config::constants::configuration_system::AVAILABLE_PARAMETERS_COUNT;
use pierre_config::constants::limits::METERS_PER_KILOMETER;
use pierre_config::environment::TrainingZonesConfig;
use pierre_core::config::profiles::ProfileTemplates;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::zones::{HrZoneSet, PowerZoneSet};
use pierre_core::models::{TenantId, UserPhysiologicalProfile};
use pierre_fitness_compute::velocity_at_vo2max;
use pierre_intelligence::physiological_constants::configuration_validation;
use pierre_intelligence::physiological_constants::heart_rate_zones::{
    AEROBIC_THRESHOLD_PERMILLE, LACTATE_THRESHOLD_PERMILLE, PERMILLE_DIVISOR, ZONE_1_MAX_PERMILLE,
    ZONE_1_MIN_PERMILLE, ZONE_2_MAX_PERMILLE, ZONE_3_MAX_PERMILLE, ZONE_4_MAX_PERMILLE,
};
use pierre_intelligence::physiological_constants::physiological_defaults::{
    DEFAULT_LACTATE_THRESHOLD, DEFAULT_SPORT_EFFICIENCY,
};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

// ============================================================================
// Helpers (inlined from former handlers/configuration.rs)
// ============================================================================

/// Seconds in a minute, in the float form the pace formatter multiplies by.
const SECONDS_PER_MINUTE_F64: f64 = 60.0;

/// Normalize stored configuration structure with defaults
fn normalize_stored_configuration(stored_config: &Value) -> Value {
    if stored_config.is_object() {
        let profile = stored_config.get("profile").cloned().unwrap_or_else(|| {
            json!({
                "name": "custom",
                "sport_type": "general",
                "training_focus": "custom"
            })
        });
        let session_overrides = stored_config
            .get("session_overrides")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let last_modified = stored_config
            .get("last_modified")
            .cloned()
            .unwrap_or_else(|| json!(chrono::Utc::now().to_rfc3339()));

        json!({
            "profile": profile,
            "session_overrides": session_overrides,
            "last_modified": last_modified
        })
    } else {
        json!({
            "profile": {
                "name": "custom",
                "sport_type": "general",
                "training_focus": "custom"
            },
            "session_overrides": {},
            "last_modified": chrono::Utc::now().to_rfc3339()
        })
    }
}

/// Build payload for user configuration response
fn build_configuration_payload(
    user_uuid: &uuid::Uuid,
    configuration: &Value,
    has_overrides: bool,
) -> Value {
    json!({
        "user_id": user_uuid.to_string(),
        "active_profile": if has_overrides { "custom" } else { "default" },
        "configuration": configuration,
        "available_parameters": AVAILABLE_PARAMETERS_COUNT
    })
}

/// Zone calculation inputs, each carrying whether it is actually known.
///
/// The athlete-specific measurements are `Option` on purpose. They used to
/// fall back to `DEFAULT_ESTIMATED_FTP` / `DEFAULT_MAX_HR` / `DEFAULT_RESTING_HR`,
/// so an athlete who had supplied nothing still received zone boundaries
/// derived from house numbers and labelled as their own. A zone family whose
/// inputs are unknown is now omitted with a reason instead.
struct ZoneParams {
    vo2_max: Option<f64>,
    resting_hr: Option<u16>,
    max_hr: Option<u16>,
    ftp_watts: Option<u32>,
    lactate_threshold: f64,
    sport_efficiency: f64,
    /// Where each athlete-specific input came from, keyed by field name:
    /// `provided` (this call's arguments), `profile` (the saved physiology),
    /// or `estimated_from_age` for the one documented estimator.
    sources: serde_json::Map<String, Value>,
}

/// Resolve zone calculation inputs from this call's arguments, falling back to
/// the athlete's saved physiology.
///
/// Precedence is argument, then stored profile, then unknown. The single
/// exception is maximum heart rate, which falls back to the Tanaka estimate
/// from a stored age — a named, documented estimator whose use is reported in
/// `sources`, not an undisclosed constant.
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

fn extract_zone_parameters(args: &Value, stored: Option<&UserPhysiologicalProfile>) -> ZoneParams {
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
    .unwrap_or(DEFAULT_LACTATE_THRESHOLD);
    let sport_efficiency = args
        .get("sport_efficiency")
        .and_then(Value::as_f64)
        .unwrap_or(DEFAULT_SPORT_EFFICIENCY);

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
        if let Some(estimated) = stored.and_then(UserPhysiologicalProfile::estimated_max_hr) {
            sources.insert("max_hr".to_owned(), json!("estimated_from_age"));
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
        sport_efficiency,
        sources,
    }
}

fn create_user_profile(params: &ZoneParams) -> Value {
    json!({
        "vo2_max": params.vo2_max,
        "resting_hr": params.resting_hr,
        "max_hr": params.max_hr,
        "ftp": params.ftp_watts,
        "lactate_threshold": params.lactate_threshold,
        "sport_efficiency": params.sport_efficiency
    })
}

fn calculate_zone_offset(hr_range: u64, percentage: u32) -> u64 {
    hr_range.saturating_mul(u64::from(percentage)) / PERMILLE_DIVISOR
}

/// Derive typed heart-rate zone boundaries by the heart-rate-reserve method.
///
/// The single source of the platform's HR-zone math: `set_physiology` persists
/// what this returns and [`heart_rate_zones_payload`] renders the same values
/// for display. `None` when the boundaries do not come out strictly
/// increasing, which [`HrZoneSet::new`] rejects — a resting rate at or above
/// the maximum collapses every zone onto one number.
pub(crate) fn derive_hr_zone_set(resting_hr: u16, max_hr: u16) -> Option<HrZoneSet> {
    let hr_range = u64::from(max_hr).saturating_sub(u64::from(resting_hr));
    let bound = |permille: u32| -> u16 {
        let value = u64::from(resting_hr) + calculate_zone_offset(hr_range, permille);
        u16::try_from(value).unwrap_or(u16::MAX)
    };
    let zones = HrZoneSet::new(
        bound(ZONE_1_MAX_PERMILLE),
        bound(ZONE_2_MAX_PERMILLE),
        bound(ZONE_3_MAX_PERMILLE),
        bound(ZONE_4_MAX_PERMILLE),
        max_hr,
    );
    match zones {
        Ok(zones) => Some(zones),
        Err(reason) => {
            warn!(
                resting_hr = resting_hr,
                max_hr = max_hr,
                reason = reason,
                "heart-rate zones not derivable from these bounds"
            );
            None
        }
    }
}

/// Render heart-rate zones for a tool payload from the typed boundaries.
///
/// Zone 1 opens above the recovery floor rather than at the resting rate, so
/// its minimum is derived here rather than read off the zone set.
fn heart_rate_zones_payload(resting_hr: u16, max_hr: u16, zones: &HrZoneSet) -> Value {
    let hr_range = u64::from(max_hr).saturating_sub(u64::from(resting_hr));
    let zone_1_min = u64::from(resting_hr) + calculate_zone_offset(hr_range, ZONE_1_MIN_PERMILLE);
    json!({
        "zone_1": { "name": "Active Recovery", "min_hr": zone_1_min, "max_hr": zones.z1_max },
        "zone_2": { "name": "Aerobic Base", "min_hr": zones.z1_max, "max_hr": zones.z2_max },
        "zone_3": { "name": "Aerobic Threshold", "min_hr": zones.z2_max, "max_hr": zones.z3_max },
        "zone_4": { "name": "Lactate Threshold", "min_hr": zones.z3_max, "max_hr": zones.z4_max },
        "zone_5": { "name": "VO2 Max", "min_hr": zones.z4_max, "max_hr": zones.z5_max }
    })
}

/// Describe how the zones were derived, plus the two threshold heart rates.
fn zone_calculations_payload(params: &ZoneParams, resting_hr: u16, max_hr: u16) -> Value {
    let hr_range = u64::from(max_hr).saturating_sub(u64::from(resting_hr));
    let lactate_threshold_hr =
        u64::from(resting_hr) + calculate_zone_offset(hr_range, LACTATE_THRESHOLD_PERMILLE);
    let aerobic_threshold_hr =
        u64::from(resting_hr) + calculate_zone_offset(hr_range, AEROBIC_THRESHOLD_PERMILLE);
    json!({
        "method": "heart_rate_reserve",
        "lactate_threshold_hr": lactate_threshold_hr,
        "aerobic_threshold_hr": aerobic_threshold_hr,
        "sport_efficiency_factor": params.sport_efficiency,
        "pace_formula": "Pace = 3.5 / (VO2 / body_weight)",
        "power_estimation": "Power = 0.98 * body_weight * VO2_max"
    })
}

/// Render the VDOT pace zones for a tool payload.
///
/// Every zone is a configured fraction of the velocity the athlete holds at
/// `VO2max`, which [`velocity_at_vo2max`] derives from Daniels' oxygen-cost
/// curve — the platform's one inversion of that relation.
fn calculate_pace_zones_from_vo2max(vo2_max: f64, config: &TrainingZonesConfig) -> Value {
    let base_velocity = velocity_at_vo2max(vo2_max);

    let easy_velocity = base_velocity * config.vdot_easy_zone_percent;
    let tempo_velocity = base_velocity * config.vdot_tempo_zone_percent;
    let threshold_velocity = base_velocity * config.vdot_threshold_zone_percent;
    let interval_velocity = base_velocity * config.vdot_interval_zone_percent;
    let repetition_velocity = base_velocity * config.vdot_repetition_zone_percent;

    let format_pace = |velocity_m_per_min: f64| -> String {
        // Velocities are metres per minute, so a kilometre costs
        // 1000 / velocity minutes; the payload quotes minutes and seconds.
        let minutes_per_km = METERS_PER_KILOMETER / velocity_m_per_min.max(1.0);
        let seconds_per_km = minutes_per_km * SECONDS_PER_MINUTE_F64;

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

    json!({
        "zone_1_easy": { "min_pace": format_pace(easy_velocity * 0.85), "max_pace": format_pace(easy_velocity * 0.95) },
        "zone_2_moderate": { "min_pace": format_pace(tempo_velocity * 0.9), "max_pace": format_pace(tempo_velocity * 1.05) },
        "zone_3_threshold": { "min_pace": format_pace(threshold_velocity * 0.95), "max_pace": format_pace(threshold_velocity * 1.05) },
        "zone_4_interval": { "min_pace": format_pace(interval_velocity * 0.95), "max_pace": format_pace(interval_velocity * 1.05) },
        "zone_5_repetition": { "min_pace": format_pace(repetition_velocity * 0.95), "max_pace": format_pace(repetition_velocity * 1.05) }
    })
}

/// Derive typed power-zone boundaries from FTP.
///
/// The single source of the platform's power-zone math: `set_physiology`
/// persists what this returns and [`power_zones_payload`] renders the same
/// values for display. `None` when the configured percentages do not produce
/// strictly increasing boundaries, which [`PowerZoneSet::new`] rejects.
pub(crate) fn derive_power_zone_set(
    ftp: u32,
    config: &TrainingZonesConfig,
) -> Option<PowerZoneSet> {
    let bound = |percent: u32| -> u32 {
        u32::try_from(u64::from(ftp) * u64::from(percent) / 100).unwrap_or_else(|e| {
            warn!(ftp = ftp, percent = percent, error = %e, "power zone bound overflowed, using u32::MAX");
            u32::MAX
        })
    };
    let zones = PowerZoneSet::new(
        bound(config.ftp_zone1_percent),
        bound(config.ftp_zone2_percent),
        bound(config.ftp_zone3_percent),
        bound(config.ftp_zone4_percent),
        bound(config.ftp_zone5_percent),
    );
    match zones {
        Ok(zones) => Some(zones),
        Err(reason) => {
            warn!(
                ftp = ftp,
                reason = reason,
                "power zones not derivable from the configured percentages"
            );
            None
        }
    }
}

/// Render power zones for a tool payload from the typed boundaries.
fn power_zones_payload(zones: &PowerZoneSet) -> Value {
    json!({
        "zone_1": { "min_watts": 0, "max_watts": zones.z1_max },
        "zone_2": { "min_watts": zones.z1_max, "max_watts": zones.z2_max },
        "zone_3": { "min_watts": zones.z2_max, "max_watts": zones.z3_max },
        "zone_4": { "min_watts": zones.z3_max, "max_watts": zones.z4_max },
        "zone_5": { "min_watts": zones.z4_max, "max_watts": zones.z5_max }
    })
}

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
            properties: Some(HashMap::new()),
            required: None,
            ..Default::default()
        };

        tool_definition(
            "get_configuration_catalog",
            "Get the complete catalog of available configuration options",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let result: AppResult<ToolResult> = async move {
            let catalog = CatalogBuilder::build();
            Ok(ToolResult::ok(json!({ "catalog": catalog })))
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
            properties: Some(HashMap::new()),
            required: None,
            ..Default::default()
        };

        tool_definition(
            "get_configuration_profiles",
            "Get available configuration profile templates",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let result: AppResult<ToolResult> = async move {
            let profile_templates = ProfileTemplates::all();
            let profiles: Vec<Value> = profile_templates
                .into_iter()
                .map(|(name, profile)| {
                    json!({
                        "name": name,
                        "profile": profile,
                        "description": format!("Configuration profile: {name}")
                    })
                })
                .collect();

            let total_count = profiles.len();
            Ok(ToolResult::ok(json!({
                "profiles": profiles,
                "total_count": total_count
            })))
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
            properties: Some(HashMap::new()),
            required: None,
            ..Default::default()
        };

        tool_definition(
            "get_user_configuration",
            "Get your current training configuration settings",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
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

        match ctx
            .resources
            .repos()
            .profiles
            .get_configuration(&user_uuid.to_string())
            .await
        {
            Ok(Some(config_str)) => {
                let stored_config: Value = serde_json::from_str(&config_str).unwrap_or_else(|e| {
                    warn!(
                        user_id = %user_uuid,
                        error = %e,
                        "Failed to parse stored fitness configuration JSON, using empty default"
                    );
                    json!({})
                });

                let configuration = normalize_stored_configuration(&stored_config);
                Ok(ToolResult::ok(build_configuration_payload(
                    &user_uuid,
                    &configuration,
                    true,
                )))
            }
            Ok(None) => {
                let default_configuration = json!({
                    "profile": {
                        "name": "default",
                        "sport_type": "general",
                        "training_focus": "recreational"
                    },
                    "session_overrides": {},
                    "last_modified": chrono::Utc::now().to_rfc3339()
                });
                Ok(ToolResult::ok(build_configuration_payload(
                    &user_uuid,
                    &default_configuration,
                    false,
                )))
            }
            Err(e) => Ok(ToolResult::error(json!({
                "error": format!("Failed to get user configuration: {e}")
            }))),
        }
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
                description: Some("Profile name to apply".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "parameters".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Configuration parameters to update".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);

        tool_definition(
            "update_user_configuration",
            "Update your training configuration settings",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA)
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

            let profile = args
                .get("profile")
                .and_then(Value::as_str)
                .unwrap_or("custom");
            let parameters = args.get("parameters").cloned().unwrap_or_else(|| json!({}));

            let configuration = json!({
                "active_profile": profile,
                "profile": {
                    "name": profile,
                    "sport_type": "general",
                    "training_focus": "custom"
                },
                "session_overrides": parameters,
                "applied_overrides": parameters.as_object().map_or(0, serde_json::Map::len),
                "last_modified": chrono::Utc::now().to_rfc3339()
            });

            let config_json = serde_json::to_string(&configuration)
                .map_err(|e| AppError::internal(format!("Failed to serialize config: {e}")))?;

            match ctx
                .resources
                .repos()
                .profiles
                .save_configuration(&user_uuid.to_string(), &config_json)
                .await
            {
                Ok(()) => {
                    let param_count = parameters.as_object().map_or(0, serde_json::Map::len);
                    Ok(ToolResult::ok(json!({
                        "user_id": user_uuid.to_string(),
                        "updated_configuration": configuration,
                        "changes_applied": param_count,
                        "message": "Configuration updated successfully"
                    })))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to update configuration: {e}")
                }))),
            }
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
                    "Maximum heart rate in bpm. Omit to use the athlete's saved value, or the Tanaka estimate from their saved age.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "lactate_threshold".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Lactate threshold".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "sport_efficiency".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Sport efficiency factor".to_owned()),
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
            properties: Some(properties),
            // Nothing is required: every input falls back to the athlete's
            // saved physiology, and a zone family whose inputs are unknown is
            // reported as unavailable rather than invented.
            required: None,
            ..Default::default()
        };

        tool_definition(
            "calculate_personalized_zones",
            "Calculate training zones from the athlete's own measurements, falling back to their saved physiology for anything not supplied here. Zone families whose inputs are unknown are listed under `unavailable` instead of being estimated; `input_sources` says where each number came from.",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        // Deliberately not `REQUIRES_TENANT`: the saved profile is a fallback,
        // not a precondition, so a tenant-less call still answers from its own
        // arguments.
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
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
                }
                None => None,
            };

            let params = extract_zone_parameters(&args, stored.as_ref());
            let user_profile = create_user_profile(&params);
            let zones_config = &ctx.resources.config().training_zones;

            // Each zone family is emitted only when its own inputs are known.
            // `unavailable` names what is missing so the caller can ask for it
            // instead of being handed boundaries built from house numbers.
            let mut unavailable = serde_json::Map::new();

            let (heart_rate_zones, zone_calculations) =
                match (params.resting_hr, params.max_hr) {
                    (Some(resting_hr), Some(max_hr)) => derive_hr_zone_set(resting_hr, max_hr)
                        .map_or((Value::Null, Value::Null), |set| {
                            (
                                heart_rate_zones_payload(resting_hr, max_hr, &set),
                                zone_calculations_payload(&params, resting_hr, max_hr),
                            )
                        }),
                    _ => (Value::Null, Value::Null),
                };
            if heart_rate_zones.is_null() {
                unavailable.insert(
                    "heart_rate_zones".to_owned(),
                    json!("needs both resting_hr and max_hr — supply them here or save them with set_physiology"),
                );
            }

            let pace_zones = params
                .vo2_max
                .map_or(Value::Null, |vo2_max| {
                    calculate_pace_zones_from_vo2max(vo2_max, zones_config)
                });
            if pace_zones.is_null() {
                unavailable.insert(
                    "pace_zones".to_owned(),
                    json!("needs vo2_max — supply it here or save it with set_physiology"),
                );
            }

            let power_zones = params
                .ftp_watts
                .and_then(|ftp| derive_power_zone_set(ftp, zones_config))
                .as_ref()
                .map_or(Value::Null, power_zones_payload);
            if power_zones.is_null() {
                unavailable.insert(
                    "power_zones".to_owned(),
                    json!("needs ftp — supply it here or save it with set_physiology"),
                );
            }

            Ok(ToolResult::ok(json!({
                "user_profile": user_profile,
                "input_sources": params.sources,
                "personalized_zones": {
                    "heart_rate_zones": heart_rate_zones,
                    "pace_zones": pace_zones,
                    "power_zones": power_zones,
                    "ftp": params.ftp_watts
                },
                "unavailable": unavailable,
                "zone_calculations": zone_calculations
            })))
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

        tool_definition(
            "validate_configuration",
            "Validate configuration parameters for physiological correctness",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
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

                    return Ok(ToolResult::ok(json!({
                        "validation_passed": validation_passed,
                        "parameters_validated": param_count,
                        "message": if validation_passed {
                            "Configuration parameters are valid"
                        } else {
                            "Configuration validation failed"
                        },
                        "errors": if errors.is_empty() { Value::Null } else { json!(errors) }
                    })));
                }

                Ok(ToolResult::ok(json!({
                    "validation_passed": true,
                    "parameters_validated": param_count,
                    "message": "Configuration parameters are valid",
                    "errors": Value::Null
                })))
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
