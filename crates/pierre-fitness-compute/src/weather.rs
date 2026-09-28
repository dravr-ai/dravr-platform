// ABOUTME: Weather impact analytics — scores a dravr-meteo sample against the physiological thresholds
// ABOUTME: Vendor abstraction, vendor selection and the cache decorator all live in dravr-meteo
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Weather analytics.
//!
//! Vendor logic (HTTP calls, parsing, caching) and vendor selection
//! (`dravr_meteo::provider_from_env`, reading `WEATHER_PROVIDER` /
//! `OPENWEATHER_API_KEY`) live in the `dravr-meteo` crate. This module
//! holds the platform's own half: [`analyze_weather_impact`] scores a sample
//! against the cageux physiological thresholds (heat stress, wind drag,
//! precipitation).

use pierre_weather::WeatherSample;
use serde::{Deserialize, Serialize};

use dravr_cageux::physiological_constants::{
    weather_impact_factors::{
        COLD_DIFFICULTY, EXTREME_COLD_DIFFICULTY, EXTREME_HOT_DIFFICULTY, HIGH_HUMIDITY_DIFFICULTY,
        MODERATE_WIND_DIFFICULTY, RAIN_DIFFICULTY, SNOW_DIFFICULTY, STRONG_WIND_DIFFICULTY,
        WARM_DIFFICULTY,
    },
    weather_thresholds::{
        COLD_THRESHOLD_CELSIUS, EXTREME_COLD_CELSIUS, EXTREME_HOT_THRESHOLD_CELSIUS,
        HIGH_HUMIDITY_THRESHOLD, HOT_THRESHOLD_CELSIUS, HUMIDITY_IMPACT_TEMP_THRESHOLD,
        MODERATE_WIND_THRESHOLD, STRONG_WIND_THRESHOLD,
    },
};

/// Analyze weather impact on performance using physiological thresholds.
///
/// Pure function over a `WeatherSample` — no I/O, deterministic from
/// the input. Used by the `analyze_weather_impact` MCP tool and by the
/// backfill orchestrator's annotation pass.
#[must_use]
pub fn analyze_weather_impact(weather: &WeatherSample) -> WeatherImpact {
    let mut impact_factors = Vec::new();
    let mut overall_difficulty = 0.0_f64;

    match weather.temperature_celsius {
        t if t < EXTREME_COLD_CELSIUS => {
            impact_factors.push("Extremely cold conditions increase energy expenditure".into());
            overall_difficulty += EXTREME_COLD_DIFFICULTY;
        }
        t if t < COLD_THRESHOLD_CELSIUS => {
            impact_factors.push("Cold conditions may affect performance".into());
            overall_difficulty += COLD_DIFFICULTY;
        }
        t if t > EXTREME_HOT_THRESHOLD_CELSIUS => {
            impact_factors.push("Hot conditions increase heat stress".into());
            overall_difficulty += EXTREME_HOT_DIFFICULTY;
        }
        t if t > HOT_THRESHOLD_CELSIUS => {
            impact_factors.push("Warm conditions may increase perceived effort".into());
            overall_difficulty += WARM_DIFFICULTY;
        }
        _ => {
            impact_factors.push("Ideal temperature conditions".into());
        }
    }

    if let Some(wind_speed) = weather.wind_speed_kmh {
        match wind_speed {
            w if w > STRONG_WIND_THRESHOLD => {
                impact_factors.push("Strong winds significantly impact performance".into());
                overall_difficulty += STRONG_WIND_DIFFICULTY;
            }
            w if w > MODERATE_WIND_THRESHOLD => {
                impact_factors.push("Moderate winds may affect pace".into());
                overall_difficulty += MODERATE_WIND_DIFFICULTY;
            }
            _ => {}
        }
    }

    if weather.conditions.contains("rain") {
        impact_factors.push("Wet conditions require extra caution and mental focus".into());
        overall_difficulty += RAIN_DIFFICULTY;
    } else if weather.conditions.contains("snow") {
        impact_factors.push("Snow conditions significantly increase difficulty".into());
        overall_difficulty += SNOW_DIFFICULTY;
    }

    if let Some(humidity) = weather.humidity_percentage {
        if humidity > HIGH_HUMIDITY_THRESHOLD
            && weather.temperature_celsius > HUMIDITY_IMPACT_TEMP_THRESHOLD
        {
            impact_factors.push("High humidity makes cooling less efficient".into());
            overall_difficulty += HIGH_HUMIDITY_DIFFICULTY;
        }
    }

    let difficulty_level = match overall_difficulty {
        d if d < 1.0 => WeatherDifficulty::Ideal,
        d if d < 2.5 => WeatherDifficulty::Challenging,
        d if d < 5.0 => WeatherDifficulty::Difficult,
        _ => WeatherDifficulty::Extreme,
    };

    WeatherImpact {
        difficulty_level,
        impact_factors,
        performance_adjustment: clamp_f64_to_f32(-overall_difficulty * 2.0),
    }
}

/// Clamp an `f64` into the representable `f32` range without `as` casts.
#[inline]
#[allow(clippy::cast_possible_truncation)] // Safe: clamped to f32 range above
fn clamp_f64_to_f32(value: f64) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    if value > f64::from(f32::MAX) {
        return f32::MAX;
    }
    if value < f64::from(f32::MIN) {
        return f32::MIN;
    }
    value as f32
}

/// Weather impact analysis result
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WeatherImpact {
    /// Classified difficulty level based on weather conditions
    pub difficulty_level: WeatherDifficulty,
    /// List of specific factors affecting performance
    pub impact_factors: Vec<String>,
    /// Percentage adjustment to expected performance (negative = slower)
    pub performance_adjustment: f32,
}

/// Weather difficulty classification
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WeatherDifficulty {
    /// Perfect conditions for optimal performance
    Ideal,
    /// Moderate challenges requiring minor adjustments
    Challenging,
    /// Significant obstacles affecting performance
    Difficult,
    /// Dangerous or extreme conditions
    Extreme,
}
