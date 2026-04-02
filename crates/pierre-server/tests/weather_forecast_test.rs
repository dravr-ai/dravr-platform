// ABOUTME: Tests for get_weather_forecast tool including geocoding, date parsing, and tool registration
// ABOUTME: Validates tool metadata, coordinate parsing, date validation, and forecast range limits
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use pierre_mcp_server::config::fitness::WeatherApiConfig;
use pierre_mcp_server::intelligence::weather::WeatherService;
use pierre_mcp_server::tools::implementations::analytics::GetWeatherForecastTool;
use pierre_mcp_server::tools::traits::{McpTool, ToolCapabilities};
use pierre_mcp_server::tools::{AuthMethod, ToolExecutionContext};
use uuid::Uuid;

// ============================================================================
// Helper to create a minimal ToolExecutionContext for tests
// ============================================================================

async fn create_test_context() -> ToolExecutionContext {
    let resources = common::create_test_server_resources().await.unwrap();
    ToolExecutionContext::new(
        Uuid::new_v4(),
        None,
        Arc::clone(&resources),
        AuthMethod::JwtBearer,
    )
}

// ============================================================================
// Tool metadata tests
// ============================================================================

#[test]
fn test_get_weather_forecast_tool_name() {
    let tool = GetWeatherForecastTool;
    assert_eq!(tool.name(), "get_weather_forecast");
}

#[test]
fn test_get_weather_forecast_tool_description() {
    let tool = GetWeatherForecastTool;
    assert!(!tool.description().is_empty());
    assert!(
        tool.description().to_lowercase().contains("forecast"),
        "Description should mention 'forecast'"
    );
}

#[test]
fn test_get_weather_forecast_tool_schema() {
    let tool = GetWeatherForecastTool;
    let schema = tool.input_schema();

    assert_eq!(schema.schema_type, "object");

    let properties = schema.properties.as_ref().expect("Should have properties");
    assert!(
        properties.contains_key("location"),
        "Should have location property"
    );
    assert!(properties.contains_key("date"), "Should have date property");

    let required = schema
        .required
        .as_ref()
        .expect("Should have required fields");
    assert!(
        required.contains(&"location".to_owned()),
        "location should be required"
    );
    assert!(
        !required.contains(&"date".to_owned()),
        "date should be optional"
    );
}

#[test]
fn test_get_weather_forecast_tool_capabilities() {
    let tool = GetWeatherForecastTool;
    let caps = tool.capabilities();

    assert!(
        caps.contains(ToolCapabilities::READS_DATA),
        "Should have READS_DATA capability"
    );
    assert!(
        !caps.contains(ToolCapabilities::WRITES_DATA),
        "Should not have WRITES_DATA capability"
    );
}

// ============================================================================
// Geocoding coordinate parsing tests
// ============================================================================

#[tokio::test]
async fn test_geocode_location_lat_lon_format() {
    let config = WeatherApiConfig::default();
    let mut service = WeatherService::new(config, None);

    let result = service.geocode_location("45.5,-72.1").await;
    assert!(result.is_ok(), "Should parse lat,lon format");
    let (lat, lon) = result.unwrap();
    assert!((lat - 45.5).abs() < f64::EPSILON);
    assert!((lon - (-72.1)).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_geocode_location_lat_lon_with_spaces() {
    let config = WeatherApiConfig::default();
    let mut service = WeatherService::new(config, None);

    let result = service.geocode_location("45.5 , -72.1").await;
    assert!(result.is_ok(), "Should parse lat,lon with spaces");
    let (lat, lon) = result.unwrap();
    assert!((lat - 45.5).abs() < f64::EPSILON);
    assert!((lon - (-72.1)).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_geocode_location_lat_lon_negative_coords() {
    let config = WeatherApiConfig::default();
    let mut service = WeatherService::new(config, None);

    let result = service.geocode_location("-33.8688,151.2093").await;
    assert!(result.is_ok(), "Should parse negative latitude");
    let (lat, lon) = result.unwrap();
    assert!((lat - (-33.8688)).abs() < 0.0001);
    assert!((lon - 151.2093).abs() < 0.0001);
}

#[tokio::test]
async fn test_geocode_location_lat_lon_out_of_range() {
    let config = WeatherApiConfig::default();
    let mut service = WeatherService::new(config, None);

    // Latitude > 90 is invalid, should not parse as coordinates
    // (will attempt geocoding which fails without API key)
    let result = service.geocode_location("91.0,0.0").await;
    assert!(result.is_err(), "Latitude > 90 should fail");
}

#[tokio::test]
async fn test_geocode_location_city_name_without_api_key() {
    let config = WeatherApiConfig::default();
    let mut service = WeatherService::new(config, None);

    // City name geocoding requires API key, should fail gracefully
    let result = service.geocode_location("Bromont").await;
    assert!(result.is_err(), "Should fail without API key");
}

#[tokio::test]
async fn test_geocode_location_city_with_region() {
    let config = WeatherApiConfig::default();
    let mut service = WeatherService::new(config, None);

    // "Montreal, QC" is not a lat,lon, so needs API key
    let result = service.geocode_location("Montreal, QC").await;
    assert!(result.is_err(), "Should fail for city name without API key");
}

// ============================================================================
// Date parsing and validation tests (via tool execution)
// ============================================================================

#[tokio::test]
async fn test_weather_forecast_invalid_date_format() {
    common::init_server_config();
    let tool = GetWeatherForecastTool;
    let ctx = create_test_context().await;

    let args = serde_json::json!({
        "location": "45.5,-72.1",
        "date": "April 5 2026"
    });

    let result = tool.execute(args, &ctx).await.unwrap();
    let output = result.content.to_string();
    assert!(
        output.contains("Invalid date format"),
        "Should report invalid date format, got: {output}"
    );
}

#[tokio::test]
async fn test_weather_forecast_date_too_far_ahead() {
    common::init_server_config();
    let tool = GetWeatherForecastTool;
    let ctx = create_test_context().await;

    // Date 30 days from now is beyond the 5-day limit
    let far_date = (chrono::Utc::now() + chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();

    let args = serde_json::json!({
        "location": "45.5,-72.1",
        "date": far_date
    });

    let result = tool.execute(args, &ctx).await.unwrap();
    let output = result.content.to_string();
    assert!(
        output.contains("forecast_available") && output.contains("false"),
        "Should report forecast not available, got: {output}"
    );
}

#[tokio::test]
async fn test_weather_forecast_past_date() {
    common::init_server_config();
    let tool = GetWeatherForecastTool;
    let ctx = create_test_context().await;

    let args = serde_json::json!({
        "location": "45.5,-72.1",
        "date": "2020-01-01"
    });

    let result = tool.execute(args, &ctx).await.unwrap();
    let output = result.content.to_string();
    assert!(
        output.contains("past date") || output.contains("Cannot get forecast"),
        "Should reject past dates, got: {output}"
    );
}

#[tokio::test]
async fn test_weather_forecast_missing_location() {
    common::init_server_config();
    let tool = GetWeatherForecastTool;
    let ctx = create_test_context().await;

    let args = serde_json::json!({
        "date": "2026-04-05"
    });

    let result = tool.execute(args, &ctx).await.unwrap();
    let output = result.content.to_string();
    assert!(
        output.contains("location is required"),
        "Should require location, got: {output}"
    );
}

// ============================================================================
// Tool registration tests
// ============================================================================

#[test]
fn test_weather_forecast_tool_id_roundtrip() {
    use pierre_mcp_server::protocols::universal::tool_registry::ToolId;

    let tool_id = ToolId::from_name("get_weather_forecast");
    assert!(
        tool_id.is_some(),
        "get_weather_forecast should resolve to a ToolId"
    );
    assert_eq!(
        tool_id.unwrap().name(),
        "get_weather_forecast",
        "ToolId should round-trip through name()"
    );
}

#[test]
fn test_weather_forecast_tool_description_in_registry() {
    use pierre_mcp_server::protocols::universal::tool_registry::ToolId;

    let tool_id = ToolId::from_name("get_weather_forecast").unwrap();
    let description = tool_id.description();
    assert!(
        !description.is_empty(),
        "ToolId description should not be empty"
    );
}
