// ABOUTME: Nutrition tools for meal planning and nutrient tracking.
// ABOUTME: Implements calculate_daily_nutrition, get_nutrient_timing, search_food, get_food_details, analyze_meal_nutrition.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Nutrition Tools
//!
//! This module provides tools for nutrition management with direct business logic:
//! - `CalculateDailyNutritionTool` - Calculate daily calorie and macronutrient needs
//! - `GetNutrientTimingTool` - Optimal nutrient timing recommendations
//! - `SearchFoodTool` - Search USDA food database
//! - `GetFoodDetailsTool` - Get detailed food information
//! - `AnalyzeMealNutritionTool` - Analyze meal nutritional content
//!
//! All tools use direct intelligence module access.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, PropertySchema};
use crate::protocols::universal::handlers;
use crate::tools::context::ToolExecutionContext;
use crate::tools::dispatch::dispatch_handler;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

// ============================================================================
// CalculateDailyNutritionTool
// ============================================================================

/// Tool for calculating daily calorie and macronutrient needs.
pub struct CalculateDailyNutritionTool;

#[async_trait]
impl McpTool for CalculateDailyNutritionTool {
    fn name(&self) -> &'static str {
        "calculate_daily_nutrition"
    }

    fn description(&self) -> &'static str {
        "Calculate daily calorie and macronutrient needs based on biometrics and goals"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "weight_kg".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Body weight in kilograms".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "height_cm".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Height in centimeters".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "age".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Age in years".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "gender".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Gender: male or female".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_level".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Activity level: sedentary, lightly_active, moderately_active, very_active, extra_active".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "training_goal".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Training goal: maintenance, weight_loss, muscle_gain, endurance_performance"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![
                "weight_kg".to_owned(),
                "height_cm".to_owned(),
                "age".to_owned(),
                "gender".to_owned(),
                "activity_level".to_owned(),
                "training_goal".to_owned(),
            ]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "calculate_daily_nutrition",
            handlers::handle_calculate_daily_nutrition,
        )
        .await
    }
}

// ============================================================================
// GetNutrientTimingTool
// ============================================================================

/// Tool for optimal nutrient timing recommendations.
pub struct GetNutrientTimingTool;

#[async_trait]
impl McpTool for GetNutrientTimingTool {
    fn name(&self) -> &'static str {
        "get_nutrient_timing"
    }

    fn description(&self) -> &'static str {
        "Get optimal nutrient timing recommendations around workouts"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "workout_intensity".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Workout intensity: low, moderate, high".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "weight_kg".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Body weight in kilograms".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "daily_protein_g".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Daily protein target in grams".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![
                "workout_intensity".to_owned(),
                "weight_kg".to_owned(),
                "daily_protein_g".to_owned(),
            ]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "get_nutrient_timing",
            handlers::handle_get_nutrient_timing,
        )
        .await
    }
}

// ============================================================================
// SearchFoodTool
// ============================================================================

/// Tool for searching the USDA food database.
pub struct SearchFoodTool;

#[async_trait]
impl McpTool for SearchFoodTool {
    fn name(&self) -> &'static str {
        "search_food"
    }

    fn description(&self) -> &'static str {
        "Search USDA FoodData Central database for foods. Returns up to 10 results by default. Check the `has_more` field before requesting additional pages."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "query".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Search query for food items".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "page_size".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Number of results per page (default: 10, max: 50)".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "page_number".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Page number (1-indexed, default: 1). Only use if previous response had has_more=true".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(context, args, "search_food", handlers::handle_search_food).await
    }
}

// ============================================================================
// GetFoodDetailsTool
// ============================================================================

/// Tool for getting detailed food information.
pub struct GetFoodDetailsTool;

#[async_trait]
impl McpTool for GetFoodDetailsTool {
    fn name(&self) -> &'static str {
        "get_food_details"
    }

    fn description(&self) -> &'static str {
        "Get detailed nutritional information for a specific food item"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "fdc_id".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("USDA FoodData Central ID of the food item".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["fdc_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "get_food_details",
            handlers::handle_get_food_details,
        )
        .await
    }
}

// ============================================================================
// AnalyzeMealNutritionTool
// ============================================================================

/// Tool for analyzing meal nutritional content.
pub struct AnalyzeMealNutritionTool;

#[async_trait]
impl McpTool for AnalyzeMealNutritionTool {
    fn name(&self) -> &'static str {
        "analyze_meal_nutrition"
    }

    fn description(&self) -> &'static str {
        "Analyze nutritional content of a meal from its ingredients"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "ingredients".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some(
                    "Array of ingredients with fdc_id and amount_g fields".to_owned(),
                ),
                items: Some(Box::new(PropertySchema {
                    property_type: "object".to_owned(),
                    properties: Some(HashMap::from([
                        (
                            "fdc_id".to_owned(),
                            PropertySchema {
                                property_type: "number".to_owned(),
                                description: Some("USDA FoodData Central food ID".to_owned()),
                                ..Default::default()
                            },
                        ),
                        (
                            "amount_g".to_owned(),
                            PropertySchema {
                                property_type: "number".to_owned(),
                                description: Some("Amount in grams".to_owned()),
                                ..Default::default()
                            },
                        ),
                    ])),
                    required: Some(vec!["fdc_id".to_owned(), "amount_g".to_owned()]),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["ingredients".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "analyze_meal_nutrition",
            handlers::handle_analyze_meal_nutrition,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all nutrition tools for registration
#[must_use]
pub fn create_nutrition_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(CalculateDailyNutritionTool),
        Box::new(GetNutrientTimingTool),
        Box::new(SearchFoodTool),
        Box::new(GetFoodDetailsTool),
        Box::new(AnalyzeMealNutritionTool),
    ]
}
