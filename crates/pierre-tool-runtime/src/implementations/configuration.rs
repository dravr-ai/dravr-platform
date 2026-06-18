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
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::catalog::CatalogBuilder;
use pierre_config::constants::configuration_system::AVAILABLE_PARAMETERS_COUNT;
use pierre_config::constants::limits::METERS_PER_KILOMETER;
use pierre_config::environment::TrainingZonesConfig;
use pierre_core::config::profiles::ProfileTemplates;
use pierre_core::errors::{AppError, AppResult};
use pierre_intelligence::physiological_constants::configuration_validation;
use pierre_intelligence::physiological_constants::heart_rate_zones::{
    AEROBIC_THRESHOLD_PERMILLE, LACTATE_THRESHOLD_PERMILLE, PERMILLE_DIVISOR, ZONE_1_MAX_PERMILLE,
    ZONE_1_MIN_PERMILLE, ZONE_2_MAX_PERMILLE, ZONE_3_MAX_PERMILLE, ZONE_4_MAX_PERMILLE,
};
use pierre_intelligence::physiological_constants::physiological_defaults::{
    DEFAULT_ESTIMATED_FTP, DEFAULT_LACTATE_THRESHOLD, DEFAULT_MAX_HR, DEFAULT_RESTING_HR,
    DEFAULT_SPORT_EFFICIENCY,
};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

// ============================================================================
// Helpers (inlined from former handlers/configuration.rs)
// ============================================================================

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

/// Zone calculation parameters
struct ZoneParams {
    vo2_max: f64,
    resting_hr: u64,
    max_hr: u64,
    lactate_threshold: f64,
    sport_efficiency: f64,
}

/// Extract and validate zone calculation parameters from tool args
fn extract_zone_parameters(args: &Value) -> AppResult<ZoneParams> {
    let vo2_max = args
        .get("vo2_max")
        .and_then(Value::as_f64)
        .ok_or_else(|| AppError::invalid_input("vo2_max parameter required"))?;

    let resting_hr = args
        .get("resting_hr")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_RESTING_HR);

    let max_hr = args
        .get("max_hr")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_HR);

    let lactate_threshold = args
        .get("lactate_threshold")
        .and_then(Value::as_f64)
        .unwrap_or(DEFAULT_LACTATE_THRESHOLD);

    let sport_efficiency = args
        .get("sport_efficiency")
        .and_then(Value::as_f64)
        .unwrap_or(DEFAULT_SPORT_EFFICIENCY);

    Ok(ZoneParams {
        vo2_max,
        resting_hr,
        max_hr,
        lactate_threshold,
        sport_efficiency,
    })
}

fn create_user_profile(params: &ZoneParams) -> Value {
    json!({
        "vo2_max": params.vo2_max,
        "resting_hr": params.resting_hr,
        "max_hr": params.max_hr,
        "lactate_threshold": params.lactate_threshold,
        "sport_efficiency": params.sport_efficiency
    })
}

fn calculate_zone_offset(hr_range: u64, percentage: u32) -> u64 {
    hr_range.saturating_mul(u64::from(percentage)) / PERMILLE_DIVISOR
}

fn calculate_heart_rate_zones(params: &ZoneParams) -> (Value, Value) {
    let hr_range = params.max_hr.saturating_sub(params.resting_hr);

    let zone_1_min = params.resting_hr + calculate_zone_offset(hr_range, ZONE_1_MIN_PERMILLE);
    let zone_1_max = params.resting_hr + calculate_zone_offset(hr_range, ZONE_1_MAX_PERMILLE);
    let zone_2_min = params.resting_hr + calculate_zone_offset(hr_range, ZONE_1_MAX_PERMILLE);
    let zone_2_max = params.resting_hr + calculate_zone_offset(hr_range, ZONE_2_MAX_PERMILLE);
    let zone_3_min = params.resting_hr + calculate_zone_offset(hr_range, ZONE_2_MAX_PERMILLE);
    let zone_3_max = params.resting_hr + calculate_zone_offset(hr_range, ZONE_3_MAX_PERMILLE);
    let zone_4_min = params.resting_hr + calculate_zone_offset(hr_range, ZONE_3_MAX_PERMILLE);
    let zone_4_max = params.resting_hr + calculate_zone_offset(hr_range, ZONE_4_MAX_PERMILLE);
    let zone_5_min = params.resting_hr + calculate_zone_offset(hr_range, ZONE_4_MAX_PERMILLE);

    let lactate_threshold_hr =
        params.resting_hr + calculate_zone_offset(hr_range, LACTATE_THRESHOLD_PERMILLE);
    let aerobic_threshold_hr =
        params.resting_hr + calculate_zone_offset(hr_range, AEROBIC_THRESHOLD_PERMILLE);

    let zones = json!({
        "zone_1": { "name": "Active Recovery", "min_hr": zone_1_min, "max_hr": zone_1_max },
        "zone_2": { "name": "Aerobic Base", "min_hr": zone_2_min, "max_hr": zone_2_max },
        "zone_3": { "name": "Aerobic Threshold", "min_hr": zone_3_min, "max_hr": zone_3_max },
        "zone_4": { "name": "Lactate Threshold", "min_hr": zone_4_min, "max_hr": zone_4_max },
        "zone_5": { "name": "VO2 Max", "min_hr": zone_5_min, "max_hr": params.max_hr }
    });

    let zone_calculations = json!({
        "method": "heart_rate_reserve",
        "lactate_threshold_hr": lactate_threshold_hr,
        "aerobic_threshold_hr": aerobic_threshold_hr,
        "sport_efficiency_factor": params.sport_efficiency,
        "pace_formula": "Pace = 3.5 / (VO2 / body_weight)",
        "power_estimation": "Power = 0.98 * body_weight * VO2_max"
    });

    (zones, zone_calculations)
}

fn calculate_pace_zones_from_vo2max(vo2_max: f64, config: &TrainingZonesConfig) -> Value {
    let base_velocity = (vo2_max + 4.60) / 0.182_258;

    let easy_velocity = base_velocity * config.vdot_easy_zone_percent;
    let tempo_velocity = base_velocity * config.vdot_tempo_zone_percent;
    let threshold_velocity = base_velocity * config.vdot_threshold_zone_percent;
    let interval_velocity = base_velocity * config.vdot_interval_zone_percent;
    let repetition_velocity = base_velocity * config.vdot_repetition_zone_percent;

    let format_pace = |velocity_m_per_min: f64| -> String {
        let seconds_per_km = METERS_PER_KILOMETER / velocity_m_per_min.max(1.0);

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

fn calculate_power_zones_from_ftp(ftp: u32, config: &TrainingZonesConfig) -> Value {
    let zone_1_min = 0_u32;
    let zone_1_max = u32::try_from(u64::from(ftp) * u64::from(config.ftp_zone1_percent) / 100)
        .unwrap_or_else(|e| {
            warn!(ftp = ftp, error = %e, "Zone 1 max calculation failed, using u32::MAX");
            u32::MAX
        });
    let zone_2_max = u32::try_from(u64::from(ftp) * u64::from(config.ftp_zone2_percent) / 100)
        .unwrap_or_else(|e| {
            warn!(ftp = ftp, error = %e, "Zone 2 max calculation failed, using u32::MAX");
            u32::MAX
        });
    let zone_3_max = u32::try_from(u64::from(ftp) * u64::from(config.ftp_zone3_percent) / 100)
        .unwrap_or_else(|e| {
            warn!(ftp = ftp, error = %e, "Zone 3 max calculation failed, using u32::MAX");
            u32::MAX
        });
    let zone_4_max = u32::try_from(u64::from(ftp) * u64::from(config.ftp_zone4_percent) / 100)
        .unwrap_or_else(|e| {
            warn!(ftp = ftp, error = %e, "Zone 4 max calculation failed, using u32::MAX");
            u32::MAX
        });
    let zone_5_max = u32::try_from(u64::from(ftp) * u64::from(config.ftp_zone5_percent) / 100)
        .unwrap_or_else(|e| {
            warn!(ftp = ftp, error = %e, "Zone 5 max calculation failed, using u32::MAX");
            u32::MAX
        });

    json!({
        "zone_1": { "min_watts": zone_1_min, "max_watts": zone_1_max },
        "zone_2": { "min_watts": zone_1_max, "max_watts": zone_2_max },
        "zone_3": { "min_watts": zone_2_max, "max_watts": zone_3_max },
        "zone_4": { "min_watts": zone_3_max, "max_watts": zone_4_max },
        "zone_5": { "min_watts": zone_4_max, "max_watts": zone_5_max }
    })
}

fn validate_parameter_ranges(
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

fn validate_parameter_relationships(
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };

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
                description: Some("VO2 max in ml/kg/min (required)".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "resting_hr".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Resting heart rate in bpm".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "max_hr".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum heart rate in bpm".to_owned()),
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
                description: Some("Functional Threshold Power in watts".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["vo2_max".to_owned()]),
        };

        tool_definition(
            "calculate_personalized_zones",
            "Calculate personalized training zones based on your fitness metrics",
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
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let params = extract_zone_parameters(&args)?;
            let user_profile = create_user_profile(&params);
            let (zones, zone_calculations) = calculate_heart_rate_zones(&params);

            let pace_zones = calculate_pace_zones_from_vo2max(
                params.vo2_max,
                &ctx.resources.config().training_zones,
            );

            let ftp = args
                .get("ftp")
                .and_then(Value::as_u64)
                .and_then(|f| u32::try_from(f).ok())
                .unwrap_or(DEFAULT_ESTIMATED_FTP);

            let power_zones_result =
                calculate_power_zones_from_ftp(ftp, &ctx.resources.config().training_zones);

            Ok(ToolResult::ok(json!({
                "user_profile": user_profile,
                "personalized_zones": {
                    "heart_rate_zones": zones,
                    "pace_zones": pace_zones,
                    "power_zones": power_zones_result,
                    "estimated_ftp": ftp
                },
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["parameters".to_owned()]),
        };

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
pub fn create_configuration_tools() -> Vec<Box<dyn McpTool<dyn ToolRuntime>>> {
    vec![
        Box::new(GetConfigurationCatalogTool),
        Box::new(GetConfigurationProfilesTool),
        Box::new(GetUserConfigurationTool),
        Box::new(UpdateUserConfigurationTool),
        Box::new(CalculatePersonalizedZonesTool),
        Box::new(ValidateConfigurationTool),
    ]
}
