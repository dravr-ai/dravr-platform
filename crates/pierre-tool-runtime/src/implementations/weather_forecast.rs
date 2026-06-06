// ABOUTME: MCP tool exposing Open-Meteo forecast weather for a planned activity location + date
// ABOUTME: Lets the coach ground recommendations in real temperature instead of guessing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde_json::{json, Value};
use tracing::{info, warn};

use pierre_intelligence::location::{ForwardGeocodeResult, LocationService};
use pierre_weather::{OpenMeteoForecastProvider, WeatherProvider, WeatherQuery};

use crate::context::ToolExecutionContext;
use crate::traits::{McpTool, ToolCapabilities};
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_mcp_schema::{JsonSchema, PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

/// Default hour-of-day (UTC) used when the caller omits `hour`. Noon is a
/// reasonable proxy for a daytime training session and avoids the overnight
/// temperature trough skewing the recommendation.
const DEFAULT_HOUR_UTC: u32 = 12;

fn forecast_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        // Hits the Open-Meteo forecast API (external infrastructure).
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Tool returning the weather forecast for a location and date.
///
/// The coach calls this when a user asks for a session "based on the
/// temperature" or for a future/today activity at a place. The platform's
/// other weather path (`get_weather_for_activity`) only resolves weather for
/// an already-recorded activity; this one forecasts a planned one.
pub struct GetWeatherForecastTool;

#[async_trait]
impl McpTool for GetWeatherForecastTool {
    fn name(&self) -> &'static str {
        "get_weather_forecast"
    }

    fn description(&self) -> &'static str {
        "Get the weather forecast (temperature in °C, conditions, humidity, wind) for a \
         location and date, grounded in the Open-Meteo forecast API. Use this whenever the \
         user asks you to base a session on the temperature or weather, or proposes an \
         activity today or on an upcoming date. Pass either a place name via `place` \
         (preferred — e.g. \"Prévost, QC\") or explicit `latitude`/`longitude`. Optionally \
         pass `date` (YYYY-MM-DD, defaults to today UTC) and `hour` (0-23 UTC, defaults to \
         12). Forecast coverage is up to ~16 days ahead. Never invent the temperature — \
         call this tool to obtain it."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();

        properties.insert(
            "place".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Place name to resolve via Nominatim forward geocoding. Prefer this over \
                     latitude/longitude — pass natural strings like 'Prévost, QC'. When set, \
                     latitude/longitude are ignored."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "latitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Latitude in decimal degrees. Only used when `place` is NOT set.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "longitude".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Longitude in decimal degrees. Only used when `place` is NOT set.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "date".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Target date as YYYY-MM-DD (UTC). Defaults to today. Must be within ~16 \
                     days for forecast coverage."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "hour".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Hour of day 0-23 (UTC). Defaults to 12 (midday).".to_owned()),
                ..Default::default()
            },
        );

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            // `place` XOR (`latitude`+`longitude`) is enforced at runtime in
            // execute(); the schema presents all params as optional.
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        // Auth-gated like the sibling discover_routes tool: keeps it out of the
        // unauthenticated public-discovery set (PUBLIC_DISCOVERY_TOOLS), which
        // is an explicit allowlist. It's a coach feature used in authenticated
        // chat, so requiring auth is correct and avoids public-discovery drift.
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(forecast_annotations())
    }

    async fn execute(&self, args: Value, _context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let resolved = match resolve_location(&args).await {
            Ok(center) => center,
            Err(err) => return Ok(ToolResult::error(err)),
        };

        let timestamp = match resolve_timestamp(&args) {
            Ok(ts) => ts,
            Err(err) => return Ok(ToolResult::error(err)),
        };

        info!(
            latitude = resolved.latitude,
            longitude = resolved.longitude,
            %timestamp,
            place = %resolved.display_name.as_deref().unwrap_or("(raw coordinates)"),
            "get_weather_forecast: querying Open-Meteo forecast"
        );

        let provider = OpenMeteoForecastProvider::new();
        let sample = match provider
            .weather_at(WeatherQuery {
                latitude: resolved.latitude,
                longitude: resolved.longitude,
                timestamp,
            })
            .await
        {
            Ok(sample) => sample,
            Err(e) => {
                warn!(error = %e, "get_weather_forecast: forecast lookup failed");
                return Ok(ToolResult::error(json!({
                    "error": format!("weather forecast unavailable: {e}. The date may be outside forecast coverage (~16 days)."),
                })));
            }
        };

        let mut response = json!({
            "latitude": resolved.latitude,
            "longitude": resolved.longitude,
            "timestamp": timestamp.to_rfc3339(),
            "temperature_celsius": sample.temperature_celsius,
            "conditions": sample.conditions,
            "humidity_percentage": sample.humidity_percentage,
            "wind_speed_kmh": sample.wind_speed_kmh,
        });
        if let Some(name) = resolved.display_name {
            if let Some(obj) = response.as_object_mut() {
                obj.insert("place".to_owned(), Value::String(name));
            }
        }

        Ok(ToolResult::ok(response))
    }
}

/// Resolved location for a forecast query.
struct ResolvedLocation {
    latitude: f64,
    longitude: f64,
    display_name: Option<String>,
}

/// Resolve latitude/longitude from `place` (forward-geocoded) or explicit
/// coordinates, mirroring the `discover_routes` resolution contract.
async fn resolve_location(args: &Value) -> Result<ResolvedLocation, Value> {
    if let Some(place) = args
        .get("place")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let mut location_service = LocationService::new();
        return match location_service.forward_geocode(place).await {
            Ok(ForwardGeocodeResult {
                latitude,
                longitude,
                display_name,
            }) => Ok(ResolvedLocation {
                latitude,
                longitude,
                display_name: Some(display_name),
            }),
            Err(e) if e.code == ErrorCode::ResourceNotFound => Err(json!({
                "error": format!("no place matched '{place}'. Try a more specific name or pass latitude/longitude directly."),
            })),
            Err(e) => Err(json!({
                "error": format!("forward geocoding failed for '{place}': {e}. You can retry with explicit latitude/longitude."),
            })),
        };
    }

    let latitude = match args.get("latitude").and_then(Value::as_f64) {
        Some(lat) if (-90.0..=90.0).contains(&lat) => lat,
        _ => {
            return Err(json!({
                "error": "provide a `place` name, or both `latitude` (-90..90) and `longitude` (-180..180).",
            }))
        }
    };
    let longitude = match args.get("longitude").and_then(Value::as_f64) {
        Some(lon) if (-180.0..=180.0).contains(&lon) => lon,
        _ => {
            return Err(json!({
                "error": "provide a `place` name, or both `latitude` (-90..90) and `longitude` (-180..180).",
            }))
        }
    };

    Ok(ResolvedLocation {
        latitude,
        longitude,
        display_name: None,
    })
}

/// Resolve the target instant from `date` (YYYY-MM-DD, default today UTC) and
/// `hour` (0-23, default midday). Returns an LLM-facing error on a malformed
/// date or out-of-range hour.
fn resolve_timestamp(args: &Value) -> Result<DateTime<Utc>, Value> {
    let date = match args.get("date").and_then(Value::as_str).map(str::trim) {
        Some(s) if !s.is_empty() => match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            Ok(d) => d,
            Err(e) => {
                return Err(json!({
                    "error": format!("invalid `date` '{s}': {e}. Use YYYY-MM-DD."),
                }))
            }
        },
        _ => Utc::now().date_naive(),
    };

    let hour = match args.get("hour").and_then(Value::as_u64) {
        Some(h) if h <= 23 => u32::try_from(h).unwrap_or(DEFAULT_HOUR_UTC),
        Some(_) => {
            return Err(json!({
                "error": "`hour` must be between 0 and 23.",
            }))
        }
        None => DEFAULT_HOUR_UTC,
    };

    let naive = date
        .and_hms_opt(hour, 0, 0)
        .ok_or_else(|| json!({ "error": "could not build a valid timestamp from date/hour." }))?;

    Ok(Utc.from_utc_datetime(&naive))
}

/// Create weather-forecast tools for registration.
#[must_use]
pub fn create_weather_forecast_tools() -> Vec<Box<dyn McpTool>> {
    vec![Box::new(GetWeatherForecastTool)]
}
