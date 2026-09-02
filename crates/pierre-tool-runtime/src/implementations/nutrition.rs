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
//! All tools use direct intelligence module and USDA API access.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::implementations::usda_shared::{shared_usda_client, MAX_USDA_INGREDIENTS};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_external::UsdaClient;
use pierre_intelligence::{
    calculate_daily_nutrition_needs, calculate_nutrient_timing, ActivityLevel,
    DailyNutritionParams, Gender, TrainingGoal, WorkoutIntensity,
};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

// ============================================================================
// Helper functions
// ============================================================================

fn parse_gender(s: &str) -> AppResult<Gender> {
    match s.to_lowercase().as_str() {
        "male" => Ok(Gender::Male),
        "female" => Ok(Gender::Female),
        _ => Err(AppError::invalid_input("Gender must be 'male' or 'female'")),
    }
}

fn parse_activity_level(s: &str) -> AppResult<ActivityLevel> {
    match s.to_lowercase().as_str() {
        "sedentary" => Ok(ActivityLevel::Sedentary),
        "lightly_active" => Ok(ActivityLevel::LightlyActive),
        "moderately_active" => Ok(ActivityLevel::ModeratelyActive),
        "very_active" => Ok(ActivityLevel::VeryActive),
        "extra_active" => Ok(ActivityLevel::ExtraActive),
        _ => Err(AppError::invalid_input(
            "Invalid activity_level. Must be one of: sedentary, lightly_active, \
             moderately_active, very_active, extra_active",
        )),
    }
}

fn parse_training_goal(s: &str) -> AppResult<TrainingGoal> {
    match s.to_lowercase().as_str() {
        "maintenance" => Ok(TrainingGoal::Maintenance),
        "weight_loss" => Ok(TrainingGoal::WeightLoss),
        "muscle_gain" => Ok(TrainingGoal::MuscleGain),
        "endurance_performance" => Ok(TrainingGoal::EndurancePerformance),
        _ => Err(AppError::invalid_input(
            "Invalid training_goal. Must be one of: maintenance, weight_loss, \
             muscle_gain, endurance_performance",
        )),
    }
}

fn parse_workout_intensity(s: &str) -> AppResult<WorkoutIntensity> {
    match s.to_lowercase().as_str() {
        "low" => Ok(WorkoutIntensity::Low),
        "moderate" => Ok(WorkoutIntensity::Moderate),
        "high" => Ok(WorkoutIntensity::High),
        _ => Err(AppError::invalid_input(
            "Invalid workout_intensity. Must be one of: low, moderate, high",
        )),
    }
}

fn parse_nutrition_params(args: &Value) -> AppResult<DailyNutritionParams> {
    let weight_kg = args
        .get("weight_kg")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: weight_kg")
        })?;

    let height_cm = args
        .get("height_cm")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: height_cm")
        })?;

    let age_u64 = args
        .get("age")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::invalid_input("Missing or invalid required parameter: age"))?;

    #[allow(clippy::cast_possible_truncation)]
    let age = if age_u64 <= 150 {
        age_u64 as u32
    } else {
        return Err(AppError::invalid_input(
            "Age must be between 0 and 150 years",
        ));
    };

    let gender_str = args
        .get("gender")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing or invalid required parameter: gender"))?;
    let gender = parse_gender(gender_str)?;

    let activity_level_str = args
        .get("activity_level")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: activity_level")
        })?;
    let activity_level = parse_activity_level(activity_level_str)?;

    let training_goal_str = args
        .get("training_goal")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::invalid_input("Missing or invalid required parameter: training_goal")
        })?;
    let training_goal = parse_training_goal(training_goal_str)?;

    Ok(DailyNutritionParams {
        weight_kg,
        height_cm,
        age,
        gender,
        activity_level,
        training_goal,
    })
}

/// Resolve the process-wide `UsdaClient`, returning an error `ToolResult` if
/// the API key is missing. Shared so its 24h caches and rate limiter span
/// calls instead of dying with each one.
fn build_usda_client(context: &ToolExecutionContext) -> Result<Arc<UsdaClient>, ToolResult> {
    let api_key = context
        .resources
        .config()
        .usda_api_key
        .clone()
        .unwrap_or_default();

    if api_key.is_empty() {
        return Err(ToolResult::error(json!({
            "error": "USDA API key not configured. Set USDA_API_KEY environment variable."
        })));
    }

    Ok(shared_usda_client(api_key))
}

// ============================================================================
// CalculateDailyNutritionTool
// ============================================================================

/// Tool for calculating daily calorie and macronutrient needs.
pub struct CalculateDailyNutritionTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for CalculateDailyNutritionTool {
    fn definition(&self) -> Tool {
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
                    "Activity level: sedentary, lightly_active, moderately_active, very_active, extra_active"
                        .to_owned(),
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
        let schema = object_schema(
            properties,
            Some(vec![
                "weight_kg".to_owned(),
                "height_cm".to_owned(),
                "age".to_owned(),
                "gender".to_owned(),
                "activity_level".to_owned(),
                "training_goal".to_owned(),
            ]),
        );
        tool_definition(
            "calculate_daily_nutrition",
            "Calculate daily calorie and macronutrient needs based on biometrics and goals",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let params = parse_nutrition_params(&args)?;

            let cageux_config = context.cageux_config();
            let nutrition_config = &cageux_config.nutrition;

            match calculate_daily_nutrition_needs(
                &params,
                &nutrition_config.bmr,
                &nutrition_config.activity_factors,
                &nutrition_config.macronutrients,
            ) {
                Ok(nutrition) => Ok(ToolResult::ok(json!({
                    "bmr": nutrition.bmr,
                    "tdee": nutrition.tdee,
                    "protein_g": nutrition.protein_g,
                    "carbs_g": nutrition.carbs_g,
                    "fat_g": nutrition.fat_g,
                    "protein_percent": nutrition.macro_percentages.protein_percent,
                    "carbs_percent": nutrition.macro_percentages.carbs_percent,
                    "fat_percent": nutrition.macro_percentages.fat_percent,
                    "goal": format!("{:?}", params.training_goal),
                }))),
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Calculation error: {e}")
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetNutrientTimingTool
// ============================================================================

/// Tool for optimal nutrient timing recommendations.
pub struct GetNutrientTimingTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetNutrientTimingTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(
            properties,
            Some(vec![
                "workout_intensity".to_owned(),
                "weight_kg".to_owned(),
                "daily_protein_g".to_owned(),
            ]),
        );
        tool_definition(
            "get_nutrient_timing",
            "Get optimal nutrient timing recommendations around workouts",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let weight_kg = args
                .get("weight_kg")
                .and_then(Value::as_f64)
                .ok_or_else(|| {
                    AppError::invalid_input("Missing or invalid required parameter: weight_kg")
                })?;

            let daily_protein_g = args
                .get("daily_protein_g")
                .and_then(Value::as_f64)
                .ok_or_else(|| {
                    AppError::invalid_input(
                        "Missing or invalid required parameter: daily_protein_g",
                    )
                })?;

            // `workout_intensity` is the schema's only intensity input. An earlier
            // branch inferred intensity from a 50-activity provider fetch behind
            // two args (`activity_provider`, `days_back`) the schema never
            // declared, so no conforming client could reach it — deleted per the
            // consume-what-you-declare rule. A declared, provider-resolved
            // variant is the way to bring inference back.
            let intensity_str = args
                .get("workout_intensity")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::invalid_input(
                        "Missing or invalid required parameter: workout_intensity",
                    )
                })?;
            let (workout_intensity, intensity_source) =
                (parse_workout_intensity(intensity_str)?, "explicit");

            let cageux_config = context.cageux_config();
            let config = &cageux_config.nutrition;

            match calculate_nutrient_timing(
                weight_kg,
                daily_protein_g,
                workout_intensity,
                &config.nutrient_timing,
            ) {
                Ok(timing) => Ok(ToolResult::ok(json!({
                    "pre_workout": {
                        "timing_hours_before": timing.pre_workout.timing_hours_before,
                        "carbs_g": timing.pre_workout.carbs_g,
                        "recommendations": timing.pre_workout.recommendations,
                    },
                    "post_workout": {
                        "timing_hours_after": timing.post_workout.timing_hours_after,
                        "protein_g": timing.post_workout.protein_g,
                        "carbs_g": timing.post_workout.carbs_g,
                        "recommendations": timing.post_workout.recommendations,
                    },
                    "daily_protein_distribution": {
                        "meals_per_day": timing.daily_protein_distribution.meals_per_day,
                        "protein_per_meal_g": timing.daily_protein_distribution.protein_per_meal_g,
                        "strategy": timing.daily_protein_distribution.strategy,
                    },
                    "intensity_source": intensity_source,
                }))),
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Calculation error: {e}")
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SearchFoodTool
// ============================================================================

/// Tool for searching the USDA food database.
pub struct SearchFoodTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SearchFoodTool {
    fn definition(&self) -> Tool {
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
                description: Some(
                    "Page number (1-indexed, default: 1). Only use if previous response had has_more=true"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["query".to_owned()]));
        tool_definition(
            "search_food",
            "Search USDA FoodData Central database for foods. Returns up to 10 results by default. Check the `has_more` field before requesting additional pages.",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let client = match build_usda_client(&context) {
                Ok(c) => c,
                Err(err_result) => return Ok(err_result),
            };

            let query = args.get("query").and_then(Value::as_str).ok_or_else(|| {
                AppError::invalid_input("Missing or invalid required parameter: query")
            })?;

            let page_size_u64 = args.get("page_size").and_then(Value::as_u64).unwrap_or(10);

            #[allow(clippy::cast_possible_truncation)]
            let page_size = if (1..=200).contains(&page_size_u64) {
                page_size_u64 as u32
            } else {
                return Ok(ToolResult::error(json!({
                    "error": "Page size must be between 1 and 200"
                })));
            };

            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let page_number = args
                .get("page_number")
                .and_then(|v| {
                    v.as_u64()
                        .map(|n| n.min(u64::from(u32::MAX)) as u32)
                        .or_else(|| v.as_f64().map(|f| f as u32))
                })
                .unwrap_or(1)
                .max(1);

            match client.search_foods(query, page_size, page_number).await {
                Ok(paginated) => {
                    let count = paginated.foods.len();
                    let has_more = paginated.current_page < paginated.total_pages;
                    Ok(ToolResult::ok(json!({
                        "foods": paginated.foods,
                        "returned_count": count,
                        "total_hits": paginated.total_hits,
                        "page_number": paginated.current_page,
                        "page_size": page_size,
                        "total_pages": paginated.total_pages,
                        "has_more": has_more,
                    })))
                }
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Search error: {e}")
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetFoodDetailsTool
// ============================================================================

/// Tool for getting detailed food information.
pub struct GetFoodDetailsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetFoodDetailsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "fdc_id".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("USDA FoodData Central ID of the food item".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["fdc_id".to_owned()]));
        tool_definition(
            "get_food_details",
            "Get detailed nutritional information for a specific food item",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let client = match build_usda_client(&context) {
                Ok(c) => c,
                Err(err_result) => return Ok(err_result),
            };

            let fdc_id = args.get("fdc_id").and_then(Value::as_u64).ok_or_else(|| {
                AppError::invalid_input("Missing or invalid required parameter: fdc_id")
            })?;

            match client.get_food_details(fdc_id).await {
                Ok(food) => Ok(ToolResult::ok(json!({
                    "fdc_id": food.fdc_id,
                    "description": food.description,
                    "data_type": food.data_type,
                    "nutrients": food.food_nutrients.iter().map(|n| json!({
                        "nutrient_id": n.nutrient_id,
                        "name": n.nutrient_name,
                        "amount": n.amount,
                        "unit": n.unit_name,
                    })).collect::<Vec<_>>(),
                    "serving_size": food.serving_size,
                    "serving_size_unit": food.serving_size_unit,
                }))),
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Food not found: {e}")
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// AnalyzeMealNutritionTool
// ============================================================================

/// Tool for analyzing meal nutritional content.
pub struct AnalyzeMealNutritionTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for AnalyzeMealNutritionTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(properties, Some(vec!["ingredients".to_owned()]));
        tool_definition(
            "analyze_meal_nutrition",
            "Analyze nutritional content of a meal from its ingredients",
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
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let client = match build_usda_client(&context) {
                Ok(c) => c,
                Err(err_result) => return Ok(err_result),
            };

            let ingredients = args
                .get("ingredients")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    AppError::invalid_input(
                        "Missing or invalid required parameter: ingredients (must be array)",
                    )
                })?;

            // One USDA call per ingredient, so the array length is the
            // fan-out — bound it by a constant, not by caller input.
            if ingredients.len() > MAX_USDA_INGREDIENTS {
                return Err(AppError::invalid_input(format!(
                    "too many ingredients ({}; max {MAX_USDA_INGREDIENTS})",
                    ingredients.len()
                )));
            }

            let mut meal_items: Vec<(u64, f64)> = Vec::new();
            for item in ingredients {
                let fdc_id = item
                    .get("fdc_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AppError::invalid_input("Each ingredient must have fdc_id"))?;
                let amount_g = item
                    .get("amount_g")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| AppError::invalid_input("Each ingredient must have amount_g"))?;
                meal_items.push((fdc_id, amount_g));
            }

            let mut total_calories = 0.0_f64;
            let mut total_protein = 0.0_f64;
            let mut total_carbs = 0.0_f64;
            let mut total_fat = 0.0_f64;
            let mut food_details_list = Vec::new();

            for (fdc_id, grams) in meal_items {
                let food = match client.get_food_details(fdc_id).await {
                    Ok(f) => f,
                    Err(e) => {
                        return Ok(ToolResult::error(json!({
                            "error": format!("USDA API request failed for fdc_id {fdc_id}: {e}")
                        })));
                    }
                };

                // USDA nutrient amounts are per 100 g
                let multiplier = grams / 100.0;
                for nutrient in &food.food_nutrients {
                    match nutrient.nutrient_name.as_str() {
                        "Energy" => {
                            total_calories = nutrient.amount.mul_add(multiplier, total_calories);
                        }
                        "Protein" => {
                            total_protein = nutrient.amount.mul_add(multiplier, total_protein);
                        }
                        "Carbohydrate, by difference" => {
                            total_carbs = nutrient.amount.mul_add(multiplier, total_carbs);
                        }
                        "Total lipid (fat)" => {
                            total_fat = nutrient.amount.mul_add(multiplier, total_fat);
                        }
                        _ => {}
                    }
                }

                food_details_list.push(json!({
                    "fdc_id": fdc_id,
                    "description": food.description,
                    "grams": grams,
                }));
            }

            Ok(ToolResult::ok(json!({
                "total_calories": total_calories.round(),
                "total_protein_g": total_protein.round(),
                "total_carbs_g": total_carbs.round(),
                "total_fat_g": total_fat.round(),
                "foods": food_details_list,
            })))
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all nutrition tools for registration
#[must_use]
pub fn create_nutrition_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(CalculateDailyNutritionTool),
        Box::new(GetNutrientTimingTool),
        Box::new(SearchFoodTool),
        Box::new(GetFoodDetailsTool),
        Box::new(AnalyzeMealNutritionTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(SearchFoodTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetFoodDetailsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(GetNutrientTimingTool => empty);
crate::declare_security!(AnalyzeMealNutritionTool => empty);
crate::declare_security!(CalculateDailyNutritionTool => empty);
