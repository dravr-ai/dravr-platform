// ABOUTME: MCP tool schema definitions for nutrition, sleep, and recovery tools
// ABOUTME: Covers daily nutrition, nutrient timing, food search, sleep analysis, and recovery scoring
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use crate::constants::json_fields::FORMAT;

use super::{format_property, JsonSchema, PropertySchema, ToolSchema};

/// Create nutrition, sleep, and recovery tool schemas
pub(super) fn create_nutrition_tools() -> Vec<ToolSchema> {
    vec![
        // Nutrition Tools
        create_calculate_daily_nutrition_tool(),
        create_get_nutrient_timing_tool(),
        create_search_food_tool(),
        create_get_food_details_tool(),
        create_analyze_meal_nutrition_tool(),
        // Sleep & Recovery Tools
        create_analyze_sleep_quality_tool(),
        create_calculate_recovery_score_tool(),
        create_suggest_rest_day_tool(),
        create_track_sleep_trends_tool(),
        create_optimize_sleep_schedule_tool(),
    ]
}

/// Create the `calculate_daily_nutrition` tool schema
fn create_calculate_daily_nutrition_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "weight_kg".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Body weight in kilograms".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "height_cm".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Height in centimeters".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "age".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Age in years (max 150)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "gender".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Gender: 'male' or 'female'".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "activity_level".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Activity level: 'sedentary', 'lightly_active', 'moderately_active', 'very_active', or 'extra_active'".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "training_goal".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Training goal: 'maintenance', 'weight_loss', 'muscle_gain', or 'endurance_performance'".into(),
            ),
            ..Default::default()
        },
    );

    ToolSchema {
        name: "calculate_daily_nutrition".to_owned(),
        description: "Calculate daily calorie and macronutrient needs using Mifflin-St Jeor BMR formula. Returns BMR, TDEE, and macros (protein, carbs, fat) adjusted for training goal.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![
                "weight_kg".to_owned(),
                "height_cm".to_owned(),
                "age".to_owned(),
                "gender".to_owned(),
                "activity_level".to_owned(),
                "training_goal".to_owned(),
            ]),
        },
        annotations: None,
    }
}

/// Create the `get_nutrient_timing` tool schema
///
/// Supports cross-provider integration: if `activity_provider` is specified,
/// workout intensity is auto-inferred from recent training load.
fn create_get_nutrient_timing_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "weight_kg".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Body weight in kilograms".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "daily_protein_g".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Daily protein target in grams".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "workout_intensity".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Workout intensity: 'low', 'moderate', or 'high'. Optional if activity_provider specified (auto-inferred from recent training load).".into()
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "activity_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Fitness provider for activity data (e.g., 'strava', 'garmin'). If provided, workout intensity is auto-inferred from recent training load.".into()
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "days_back".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some(
                "Number of days of activity history to analyze for intensity inference (default: 7).".into()
            ),
            ..Default::default()
        },
    );

    ToolSchema {
        name: "get_nutrient_timing".to_owned(),
        description: "Get optimal pre-workout and post-workout nutrition recommendations following ISSN (International Society of Sports Nutrition) guidelines. Returns timing windows, macros, and hydration targets. Supports cross-provider integration for automatic workout intensity inference.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            // weight_kg and daily_protein_g always required
            // workout_intensity OR activity_provider required (validated in handler)
            required: Some(vec![
                "weight_kg".to_owned(),
                "daily_protein_g".to_owned(),
            ]),
        },
        annotations: None,
    }
}

/// Create the `search_food` tool schema
fn create_search_food_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "query".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Food name or description to search for".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "page_size".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of results per page (default: 10, max: 200)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "page_number".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Page number to retrieve (1-indexed, default: 1)".into()),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "search_food".to_owned(),
        description: "Search USDA FoodData Central database for foods by name or description. Returns food ID, name, brand, and category for each match.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `get_food_details` tool schema
fn create_get_food_details_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "fdc_id".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some(
                "USDA FoodData Central ID for the food (from search_food results)".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "get_food_details".to_owned(),
        description: "Get detailed nutritional information for a specific food from USDA FoodData Central. Returns complete nutrient breakdown including calories, macros, vitamins, and minerals per 100g serving.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["fdc_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `analyze_meal_nutrition` tool schema
fn create_analyze_meal_nutrition_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "foods".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Array of food items with 'fdc_id' (number) and 'grams' (number) for each food"
                    .into(),
            ),
            items: Some(Box::new(PropertySchema {
                property_type: "object".into(),
                properties: Some(HashMap::from([
                    (
                        "fdc_id".to_owned(),
                        PropertySchema {
                            property_type: "number".into(),
                            description: Some("USDA FoodData Central food ID".into()),
                            ..Default::default()
                        },
                    ),
                    (
                        "grams".to_owned(),
                        PropertySchema {
                            property_type: "number".into(),
                            description: Some("Portion size in grams".into()),
                            ..Default::default()
                        },
                    ),
                ])),
                required: Some(vec!["fdc_id".to_owned(), "grams".to_owned()]),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "analyze_meal_nutrition".to_owned(),
        description: "Analyze total calories and macronutrients for a meal composed of multiple foods. Each food requires USDA FoodData Central ID and portion size in grams. Returns aggregated nutrition totals.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["foods".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `analyze_sleep_quality` tool schema
fn create_analyze_sleep_quality_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "sleep_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Provider to fetch sleep data from: 'whoop', 'fitbit', 'garmin', or 'terra'. Auto-fetches most recent night's data.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "sleep_data".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some(
                "Manual sleep data object (used if sleep_provider not specified) with: date (string), duration_hours (number), efficiency_percent (number), deep_sleep_hours (number), rem_sleep_hours (number), light_sleep_hours (number), awakenings (number), hrv_rmssd_ms (number, optional)".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "recent_hrv_values".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Optional array of recent HRV RMSSD values (numbers) for trend analysis".into(),
            ),
            items: Some(Box::new(PropertySchema {
                property_type: "number".into(),
                description: Some("HRV RMSSD value in milliseconds".into()),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(
        "baseline_hrv".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Optional baseline HRV RMSSD value for comparison".into()),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "analyze_sleep_quality".to_owned(),
        description: "Analyze sleep quality using NSF/AASM guidelines. Supports two modes: (1) Provider mode - specify 'sleep_provider' to auto-fetch from connected provider (whoop, fitbit, garmin, terra), (2) Manual mode - provide 'sleep_data' JSON. Returns overall score (0-100), stage breakdown, efficiency rating, and HRV trends. Provides recommendations for sleep optimization.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None, // Either sleep_provider or sleep_data is required
        },
        annotations: None,
    }
}

/// Create the `calculate_recovery_score` tool schema
fn create_calculate_recovery_score_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "activity_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Provider for activity/training data: 'strava', 'garmin', 'fitbit', 'whoop', or 'terra'. Auto-selects best connected provider if not specified.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "sleep_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Provider for sleep/HRV data: 'whoop', 'fitbit', 'garmin', or 'terra'. Auto-fetches most recent sleep data. Auto-selects if not specified.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "sleep_data".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some(
                "Manual sleep data (used if sleep_provider not specified) with: date (string), duration_hours (number), efficiency_percent (number), deep_sleep_hours (number), rem_sleep_hours (number), hrv_rmssd_ms (number, optional)".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "user_config".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some(
                "Optional user configuration with: ftp (number), lthr (number), max_hr (number), resting_hr (number), weight_kg (number)".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "recent_hrv_values".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Optional array of recent HRV RMSSD values for trend analysis".into(),
            ),
            items: Some(Box::new(PropertySchema {
                property_type: "number".into(),
                description: Some("HRV RMSSD value in milliseconds".into()),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(
        "baseline_hrv".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Optional baseline HRV RMSSD value for comparison".into()),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "calculate_recovery_score".to_owned(),
        description: "Calculate comprehensive recovery score combining Training Stress Balance (TSB), sleep quality, HRV metrics, and WHOOP daily cycle strain. Use this tool when users ask about recovery, daily strain, WHOOP cycles, or training readiness. Supports cross-provider integration: use 'activity_provider' for training data (e.g., Strava) and 'sleep_provider' for sleep/HRV/strain data (e.g., WHOOP). Auto-selects connected providers if not specified. FALLBACK MODE: If no sleep data is available, provides TSB-only recovery assessment based on training load alone with clear limitations noted. Returns overall score (0-100), recovery category, training readiness, daily strain, data_completeness indicator, and providers used.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None, // Auto-selects providers, TSB-only fallback if no sleep data
        },
        annotations: None,
    }
}

/// Create the `suggest_rest_day` tool schema
fn create_suggest_rest_day_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "activity_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Provider for activity/training data: 'strava', 'garmin', 'fitbit', 'whoop', or 'terra'. Auto-selects if not specified.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "sleep_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Provider for sleep/HRV data: 'whoop', 'fitbit', 'garmin', or 'terra'. Auto-selects if not specified.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "sleep_data".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some("Manual sleep data (used if sleep_provider not specified)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "user_config".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some(
                "Optional user configuration with: ftp, lthr, max_hr, resting_hr, weight_kg".into(),
            ),
            ..Default::default()
        },
    );

    ToolSchema {
        name: "suggest_rest_day".to_owned(),
        description: "AI-powered rest day recommendation based on training load analysis, recovery metrics, and fatigue indicators. Supports cross-provider integration for comprehensive analysis. Auto-selects connected providers if not specified. FALLBACK MODE: If no sleep data is available, provides TSB-only recommendation based on training load alone with clear limitations noted. Returns whether rest is recommended, confidence level, reasoning, data_completeness indicator, and recovery insights.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None, // Auto-selects providers, TSB-only fallback if no sleep data
        },
        annotations: None,
    }
}

/// Create the `track_sleep_trends` tool schema
fn create_track_sleep_trends_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "sleep_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Provider to fetch sleep history from: 'whoop', 'fitbit', 'garmin', or 'terra'. Auto-selects if not specified.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "days".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some(
                "Number of days of sleep history to analyze (default: 14). Minimum 7 days required for trend analysis.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "sleep_history".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Manual sleep history array (used if sleep_provider not specified). Each item needs: date (string), duration_hours (number), efficiency_percent (number, optional), deep_sleep_hours (number, optional), rem_sleep_hours (number, optional). Minimum 7 days required.".into(),
            ),
            items: Some(Box::new(PropertySchema {
                property_type: "object".into(),
                properties: Some(HashMap::from([
                    ("date".to_owned(), PropertySchema {
                        property_type: "string".into(),
                        description: Some("Date of sleep record".into()),
                        ..Default::default()
                    }),
                    ("duration_hours".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Total sleep duration in hours".into()),
                        ..Default::default()
                    }),
                    ("efficiency_percent".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Sleep efficiency percentage".into()),
                        ..Default::default()
                    }),
                    ("deep_sleep_hours".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Deep sleep duration in hours".into()),
                        ..Default::default()
                    }),
                    ("rem_sleep_hours".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("REM sleep duration in hours".into()),
                        ..Default::default()
                    }),
                ])),
                required: Some(vec!["date".to_owned(), "duration_hours".to_owned()]),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "track_sleep_trends".to_owned(),
        description: "Track sleep patterns over time and identify trends. Supports two modes: (1) Provider mode - specify 'sleep_provider' and 'days' to auto-fetch history, (2) Manual mode - provide 'sleep_history' array. Requires at least 7 days of data. Returns average metrics, trend direction (improving/stable/declining), consistency analysis, and recommendations.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None, // Either sleep_provider or sleep_history is required
        },
        annotations: None,
    }
}

/// Create the `optimize_sleep_schedule` tool schema
fn create_optimize_sleep_schedule_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "activity_provider".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Provider for activity/training data: 'strava', 'garmin', 'fitbit', 'whoop', or 'terra'. Auto-selects if not specified.".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "user_config".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some(
                "Optional user configuration with: ftp (number), lthr (number), max_hr (number), resting_hr (number), weight_kg (number)".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "upcoming_workout_intensity".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Intensity of upcoming workout: 'low', 'moderate', or 'high' (default: 'moderate')"
                    .into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "typical_wake_time".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Your typical wake time in 'HH:MM' format (default: '06:00')".into()),
            ..Default::default()
        },
    );

    ToolSchema {
        name: "optimize_sleep_schedule".to_owned(),
        description: "Generate personalized sleep schedule recommendations based on training load, recovery needs, and upcoming workouts. Supports any connected activity provider. Auto-selects provider if not specified. Returns recommended sleep duration, optimal bedtime window, and sleep quality tips tailored to current training phase.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: None, // Auto-selects provider
        },
        annotations: None,
    }
}
