// ABOUTME: analyze_weather_impact: how the weather during an activity affected its performance
// ABOUTME: Reads the activity from its provider, fetches its weather and grades the impact
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::capabilities::PROVIDER_READ;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, task_capable, tool_definition, tool_result_to_response,
};
use crate::implementations::analytics::output::{
    ImperialWeather, MetricWeather, WeatherImpactAssessment, WeatherImpactResult, WeatherReading,
};
use crate::protocol::provider_helpers::{by_id_reader, NO_PROVIDER_CONNECTED_TEXT};
use crate::protocol::types::META_NO_PROVIDER_CONNECTED;
use crate::runtime::ToolRuntime;
use dravr_meteo::{provider_from_env, WeatherQuery};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_config::environment::default_provider;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_fitness_compute::weather::analyze_weather_impact;
use pierre_fitness_compute::weather_cache_adapter::WeatherCacheRepoAdapter;
use pierre_mcp_schema::PropertySchema;
use pierre_providers::activity_source::resolve_activity_source;
use pierre_providers::backend_resolver;
use pierre_tools_core::ToolResult;

use super::create_provider;

// ============================================================================
// AnalyzeWeatherImpactTool - Analyze weather conditions for an activity
// ============================================================================

/// Conversion factor from Celsius to Fahrenheit: F = C * 1.8 + 32
const CELSIUS_TO_FAHRENHEIT_FACTOR: f64 = 1.8;
/// Offset added after scaling for Celsius to Fahrenheit conversion
const FAHRENHEIT_OFFSET: f64 = 32.0;
/// Conversion factor from km/h to mph
const KMH_TO_MPH_FACTOR: f32 = 0.621_371;

/// Tool for analyzing how weather conditions affected activity performance.
#[derive(Debug)]
pub struct AnalyzeWeatherImpactTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeWeatherImpactTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "activity_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the activity to analyze".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Fitness provider. Defaults to configured provider.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "units".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Temperature and distance units: 'metric' (default) or 'imperial'.".to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["activity_id".to_owned()]));
        answers_with::<WeatherImpactResult>(task_capable(tool_definition(
            "analyze_weather_impact",
            "Analyze how weather (temperature, humidity, wind, precipitation) affected an \
             activity's performance.",
            schema,
            annotations("Analyze weather impact", Effect::ReadOnly, Reach::OpenWorld),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        PROVIDER_READ
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
        let Some(activity_id) = args.get("activity_id").and_then(Value::as_str) else {
            return Ok(ToolResult::error(json!({
                "error": "activity_id is required"
            })));
        };

        let provider_name = if let Some(p) = args
            .get("provider")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            p.to_owned()
        } else if let Some(env_p) = default_provider() {
            env_p
        } else {
            let tenant = context.tenant_id.map(TenantId::from_uuid);
            match resolve_activity_source(
                context.resources.repos().provider_connections.as_ref(),
                context.resources.provider_registry(),
                context.user_id,
                tenant,
            )
            .await
            {
                Ok(Some(conn)) => conn.provider,
                Ok(None) | Err(_) => {
                    return Ok(ToolResult::error(json!({
                        "error": NO_PROVIDER_CONNECTED_TEXT,
                        META_NO_PROVIDER_CONNECTED: true
                    })));
                }
            }
        };

        let units = args
            .get("units")
            .and_then(Value::as_str)
            .unwrap_or("metric");

        let provider = match create_provider(&context, &provider_name).await {
            Ok(p) => p,
            Err(result) => return Ok(result),
        };

        // The copy standing for the workout answers: an export or upload the
        // athlete brought in carries the start position a date-only capture
        // of the same ride lacks.
        let tenant_id = context.tenant_id.map(|id| id.to_string());
        let reader = match by_id_reader(
            &context.resources,
            context.user_id,
            &provider_name,
            tenant_id.as_deref(),
            activity_id,
        )
        .await
        {
            Ok(reader) => reader,
            Err(e) => {
                warn!(provider = %provider_name, error = %e, "activity reader failed");
                return Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch activity: {}", e.tool_message()),
                    "activity_id": activity_id,
                    "provider": backend_resolver::user_facing_name(&provider_name)
                })));
            }
        };
        let target = reader.as_ref().map_or_else(|| provider.as_ref(), |(_, p)| p.as_ref());
        let activity = match target.get_activity(activity_id).await {
            Ok(a) => a,
            Err(e) => {
                warn!(provider = %provider_name, error = %e, "activity read failed");
                return Ok(ToolResult::error(json!({
                    "error": format!("Failed to fetch activity: {}", e.tool_message()),
                    "activity_id": activity_id,
                    "provider": backend_resolver::user_facing_name(&provider_name)
                })));
            }
        };

        // GPS location is needed for weather lookup
        let (Some(latitude), Some(longitude)) =
            (activity.start_latitude(), activity.start_longitude())
        else {
            return ok_typed(
                "analyze_weather_impact",
                WeatherImpactResult {
                    activity_id: activity_id.to_owned(),
                    activity_name: display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS),
                    weather: None,
                    impact: None,
                    note: Some(
                        "Activity has no GPS location data — weather analysis requires start coordinates"
                            .to_owned(),
                    ),
                    units: units.to_owned(),
                },
            );
        };

        let cache_repo = context.resources.repos().weather_cache.clone();
        let cache_store = Arc::new(WeatherCacheRepoAdapter::new(cache_repo));
        let provider = match provider_from_env(cache_store) {
            Ok(provider) => provider,
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Weather is misconfigured on this server: {e}"),
                    "activity_id": activity_id,
                    "provider": backend_resolver::user_facing_name(&provider_name)
                })));
            }
        };

        let weather = match provider
            .weather_at(WeatherQuery {
                latitude,
                longitude,
                timestamp: activity.start_date(),
            })
            .await
        {
            Ok(sample) => sample,
            Err(dravr_meteo::WeatherError::Disabled) => {
                return ok_typed(
                    "analyze_weather_impact",
                    WeatherImpactResult {
                        activity_id: activity_id.to_owned(),
                        activity_name: display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS),
                        weather: None,
                        impact: None,
                        note: Some("Weather provider is disabled by configuration".to_owned()),
                        units: units.to_owned(),
                    },
                );
            }
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Weather lookup failed: {e}"),
                    "activity_id": activity_id,
                    "provider": provider_name
                })));
            }
        };

        let impact = analyze_weather_impact(&weather);

        let weather_json = if units == "imperial" {
            WeatherReading::Imperial(ImperialWeather {
                temperature_fahrenheit: f64::from(weather.temperature_celsius)
                    .mul_add(CELSIUS_TO_FAHRENHEIT_FACTOR, FAHRENHEIT_OFFSET)
                    .round(),
                humidity_percentage: weather.humidity_percentage,
                wind_speed_mph: weather
                    .wind_speed_kmh
                    .map(|w| f64::from((w * KMH_TO_MPH_FACTOR * 10.0).round() / 10.0)),
                conditions: weather.conditions,
            })
        } else {
            WeatherReading::Metric(MetricWeather {
                temperature_celsius: weather.temperature_celsius,
                humidity_percentage: weather.humidity_percentage,
                wind_speed_kmh: weather.wind_speed_kmh,
                conditions: weather.conditions,
            })
        };

        info!(
            "Weather impact analysis for activity {}: {:?}",
            activity_id, impact.difficulty_level
        );

        ok_typed(
            "analyze_weather_impact",
            WeatherImpactResult {
                activity_id: activity_id.to_owned(),
                activity_name: display_line(activity.name(), ACTIVITY_NAME_MAX_CHARS),
                weather: Some(weather_json),
                impact: Some(WeatherImpactAssessment {
                    difficulty_level: impact.difficulty_level.clone(),
                    impact_factors: impact.impact_factors.clone(),
                    performance_adjustment: impact.performance_adjustment,
                }),
                note: None,
                units: units.to_owned(),
            },
        )
        }
        .await;
        tool_result_to_response(result)
    }
}
