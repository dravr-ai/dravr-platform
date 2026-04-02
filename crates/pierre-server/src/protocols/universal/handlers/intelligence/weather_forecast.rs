// ABOUTME: Handler for get_weather_forecast tool providing location-based weather forecasts
// ABOUTME: Geocodes locations, validates dates, and returns outdoor activity suitability ratings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;
use std::pin::Pin;

use chrono::{NaiveDate, NaiveTime, Utc};
use serde_json::json;
use tracing::info;

use crate::intelligence::weather::{WeatherDifficulty, WeatherService};
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;

/// Maximum number of days ahead that `OpenWeatherMap` free tier supports
const FORECAST_MAX_DAYS: i64 = 5;
/// Default forecast hour when no specific time is requested (2:00 PM)
const DEFAULT_FORECAST_HOUR: u32 = 14;

/// Handle `get_weather_forecast` tool — get weather forecast for a location and date
#[must_use]
pub fn handle_get_weather_forecast(
    _executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let Some(location) = request.parameters.get("location").and_then(|v| v.as_str()) else {
            return Ok(UniversalResponse {
                success: false,
                result: Some(json!({ "error": "location is required" })),
                error: Some("Missing required parameter: location".to_owned()),
                metadata: None,
            });
        };

        let today = Utc::now().date_naive();

        let forecast_date = if let Some(date_str) =
            request.parameters.get("date").and_then(|v| v.as_str())
        {
            match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => {
                    return Ok(UniversalResponse {
                        success: false,
                        result: Some(json!({
                            "error": format!("Invalid date format: '{date_str}'. Use YYYY-MM-DD format."),
                            "example": "2026-04-05"
                        })),
                        error: Some(format!("Invalid date format: {date_str}")),
                        metadata: None,
                    });
                }
            }
        } else {
            today
        };

        // Validate date is not too far in the future
        let days_ahead = (forecast_date - today).num_days();
        if days_ahead > FORECAST_MAX_DAYS {
            return Ok(UniversalResponse {
                success: true,
                result: Some(json!({
                    "location": location,
                    "date": forecast_date.to_string(),
                    "forecast_available": false,
                    "message": format!(
                        "Weather forecast is only available up to {} days ahead. Requested date is {} days away.",
                        FORECAST_MAX_DAYS, days_ahead
                    )
                })),
                error: None,
                metadata: None,
            });
        }

        // Allow past dates up to 1 day back for timezone edge cases
        if days_ahead < -1 {
            return Ok(UniversalResponse {
                success: false,
                result: Some(json!({
                    "error": format!("Cannot get forecast for past date: {forecast_date}"),
                    "hint": "Use analyze_weather_impact with an activity_id for past weather analysis"
                })),
                error: Some(format!("Past date: {forecast_date}")),
                metadata: None,
            });
        }

        let mut weather_service = WeatherService::with_default_config();

        // Geocode the location
        let (lat, lon) = match weather_service.geocode_location(location).await {
            Ok(coords) => coords,
            Err(e) => {
                return Ok(UniversalResponse {
                    success: false,
                    result: Some(json!({
                        "error": format!("Could not find location '{location}': {e}"),
                        "hint": "Try a city name like 'Montreal' or coordinates like '45.5,-73.6'"
                    })),
                    error: Some(format!("Geocoding failed: {e}")),
                    metadata: None,
                });
            }
        };

        // Build timestamp at default forecast hour (14:00 UTC)
        let Some(forecast_time) = NaiveTime::from_hms_opt(DEFAULT_FORECAST_HOUR, 0, 0) else {
            return Ok(UniversalResponse {
                success: false,
                result: Some(json!({ "error": "Internal error: invalid forecast hour" })),
                error: Some("Invalid forecast hour constant".to_owned()),
                metadata: None,
            });
        };
        let forecast_datetime = forecast_date.and_time(forecast_time).and_utc();

        // Get weather data
        let weather = match weather_service
            .get_weather_at_time(lat, lon, forecast_datetime)
            .await
        {
            Ok(w) => w,
            Err(e) => {
                return Ok(UniversalResponse {
                    success: false,
                    result: Some(json!({
                        "error": format!("Weather data unavailable: {e}"),
                        "location": location,
                        "date": forecast_date.to_string()
                    })),
                    error: Some(format!("Weather API error: {e}")),
                    metadata: None,
                });
            }
        };

        // Analyze conditions for outdoor activity suitability
        let impact = weather_service.analyze_weather_impact(&weather);

        let activity_rating = match impact.difficulty_level {
            WeatherDifficulty::Ideal => "good",
            WeatherDifficulty::Challenging => "fair",
            WeatherDifficulty::Difficult | WeatherDifficulty::Extreme => "poor",
        };

        info!(
            "Weather forecast for '{}' on {}: {}°C, {} conditions",
            location, forecast_date, weather.temperature_celsius, activity_rating
        );

        Ok(UniversalResponse {
            success: true,
            result: Some(json!({
                "location": location,
                "coordinates": { "lat": lat, "lon": lon },
                "date": forecast_date.to_string(),
                "forecast_available": true,
                "weather": {
                    "temperature_celsius": weather.temperature_celsius,
                    "humidity_percentage": weather.humidity_percentage,
                    "wind_speed_kmh": weather.wind_speed_kmh,
                    "conditions": weather.conditions
                },
                "outdoor_activity_rating": activity_rating,
                "impact_factors": impact.impact_factors,
                "performance_adjustment_percent": impact.performance_adjustment
            })),
            error: None,
            metadata: None,
        })
    })
}
