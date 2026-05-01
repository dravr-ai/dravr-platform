// ABOUTME: Unit tests for intelligence weather impact analytics
// ABOUTME: Validates analyze_weather_impact behavior across thresholds (cold/ideal/hot+humid)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_meteo::WeatherSample;
use pierre_mcp_server::intelligence::weather::{analyze_weather_impact, WeatherDifficulty};

#[test]
fn test_analyze_weather_impact_cold() {
    let cold_weather = WeatherSample {
        temperature_celsius: -10.0,
        humidity_percentage: Some(50.0),
        wind_speed_kmh: Some(10.0),
        conditions: "snow".into(),
    };

    let impact = analyze_weather_impact(&cold_weather);
    assert!(matches!(
        impact.difficulty_level,
        WeatherDifficulty::Difficult | WeatherDifficulty::Extreme
    ));
    assert!(!impact.impact_factors.is_empty());
    assert!(impact.performance_adjustment < 0.0);
}

#[test]
fn test_analyze_weather_impact_ideal() {
    let ideal_weather = WeatherSample {
        temperature_celsius: 15.0,
        humidity_percentage: Some(50.0),
        wind_speed_kmh: Some(5.0),
        conditions: "sunny".into(),
    };

    let impact = analyze_weather_impact(&ideal_weather);
    assert!(matches!(impact.difficulty_level, WeatherDifficulty::Ideal));
}

#[test]
fn test_analyze_weather_impact_hot_humid() {
    let hot_humid_weather = WeatherSample {
        temperature_celsius: 32.0,
        humidity_percentage: Some(85.0),
        wind_speed_kmh: Some(2.0),
        conditions: "sunny".into(),
    };

    let impact = analyze_weather_impact(&hot_humid_weather);
    assert!(matches!(
        impact.difficulty_level,
        WeatherDifficulty::Challenging | WeatherDifficulty::Difficult
    ));
    assert!(impact.performance_adjustment < 0.0);
}
