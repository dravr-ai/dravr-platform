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

use crate::annotations::{annotations, Effect, Reach};
use crate::read_subject::DataClass;
use crate::tool_data::ToolData;
use dravr_barrage::Stream;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use secrecy::ExposeSecret;
use serde::Serialize;
use serde_json::{json, Value};

use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::implementations::nutrition_args::{
    parse_nutrition_params, parse_planned_session, parse_workout_intensity,
};
use crate::implementations::nutrition_gate::{gate_for_call, NutritionAnswer};
use crate::implementations::usda_shared::{check_ingredient_count, shared_usda_client};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_cageux::nutrition_calculator::{
    calculate_daily_nutrition_needs, calculate_nutrient_timing, TrainingDayLoad,
};
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_config::nutrition_params::{athlete_protein_g_per_kg, PROTEIN_ATHLETE_G_PER_KG_KEY};
use pierre_core::errors::{AppError, AppResult};
use pierre_external::{FoodSearchResult, UsdaClient};
use pierre_mcp_schema::PropertySchema;
use pierre_runtime_context::ConfigLookupScope;
use pierre_tools_core::ToolResult;

/// The admin's athlete protein target for this call's athlete, when one is
/// set: the per-user row, then the tenant's, then the fleet's.
///
/// Only an explicit override is read. With none, the kernel's configured
/// `protein_athlete_g_per_kg` stands, so the catalogue's displayed default
/// never shadows it; with no admin config wired there is nothing to override.
///
/// # Errors
///
/// Returns the repository error when the override cannot be read, and a
/// config error when a stored override is not a target inside 1.2-2.0 g/kg/day.
async fn athlete_protein_override(context: &ToolExecutionContext) -> AppResult<Option<f64>> {
    let Some(config) = context.resources.admin_config() else {
        return Ok(None);
    };
    let user = context.user_id.to_string();
    let tenant = context.tenant_id.map(|tenant| tenant.to_string());
    let scope = ConfigLookupScope {
        user_id: Some(&user),
        tenant_id: tenant.as_deref(),
    };
    config
        .get_override_value(PROTEIN_ATHLETE_G_PER_KG_KEY, scope)
        .await?
        .map(|value| athlete_protein_g_per_kg(&value))
        .transpose()
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

    if api_key.expose_secret().is_empty() {
        return Err(ToolResult::error(json!({
            "error": "USDA API key not configured. Set USDA_API_KEY environment variable."
        })));
    }

    Ok(shared_usda_client(api_key))
}

/// Most foods `search_food` returns in one page: the figure its schema
/// promises the model.
const MAX_FOOD_SEARCH_PAGE_SIZE: u64 = 50;

/// What `calculate_daily_nutrition` says instead of targets when a clinician
/// sets them. Foods and patterns only (Thomas, Erdman & Burke 2016: fuel for
/// the work of the day, protein spread across it) — no amount.
const DAILY_NUTRITION_GUIDANCE: &[&str] = &[
    "Build each meal around a protein source, carbohydrate-rich foods and vegetables or fruit.",
    "Eat more carbohydrate on long or hard training days and less on rest days.",
    "Spread protein across the day's meals rather than loading it into one.",
    "Daily energy and macronutrient targets come from your clinician; bring them to any nutrition conversation.",
];

/// [`DAILY_NUTRITION_GUIDANCE`] for a model the athlete keeps their health
/// records from: the same foods and patterns, and no word naming their
/// clinician (carnet#1544).
const DAILY_NUTRITION_GUIDANCE_UNNAMED: &[&str] = &[
    "Build each meal around a protein source, carbohydrate-rich foods and vegetables or fruit.",
    "Eat more carbohydrate on long or hard training days and less on rest days.",
    "Spread protein across the day's meals rather than loading it into one.",
    "Daily energy and macronutrient targets are withheld here; bring any you have been given to the conversation.",
];

/// What `get_nutrient_timing` says instead of amounts when a clinician sets
/// them. Timing and food choice only (Kerksick et al. 2017; Aragon & Schoenfeld
/// 2013: the window is flexible and total daily intake matters most).
const NUTRIENT_TIMING_GUIDANCE: &[&str] = &[
    "Before a session over an hour: an easily digested, carbohydrate-based meal 1-4 hours beforehand, or a small snack in the last hour (banana, oatmeal, toast).",
    "After the session: a meal with protein and carbohydrate within about 2 hours; total daily intake matters more than the exact minute.",
    "Spread protein across 3-4 meals through the day.",
    "How much of each to take is for your clinician to set.",
];

/// [`NUTRIENT_TIMING_GUIDANCE`] for a model the athlete keeps their health
/// records from: the same timing and foods, and no word naming their
/// clinician (carnet#1544).
const NUTRIENT_TIMING_GUIDANCE_UNNAMED: &[&str] = &[
    "Before a session over an hour: an easily digested, carbohydrate-based meal 1-4 hours beforehand, or a small snack in the last hour (banana, oatmeal, toast).",
    "After the session: a meal with protein and carbohydrate within about 2 hours; total daily intake matters more than the exact minute.",
    "Spread protein across 3-4 meals through the day.",
    "How much of each to take is withheld here; ask what you have been advised.",
];

/// What `calculate_daily_nutrition` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DailyNutritionResult {
    /// Basal metabolic rate, kcal/day.
    pub bmr: f64,
    /// Total daily energy expenditure, kcal/day.
    pub tdee: f64,
    /// Daily protein target, grams.
    pub protein_g: f64,
    /// Daily carbohydrate, grams: the middle of `carbs_g_range`, the figure
    /// the fat and percentages are worked from.
    pub carbs_g: f64,
    /// The day's carbohydrate range, grams (low, high): the 2016 joint ACSM
    /// table row the day's training falls in.
    pub carbs_g_range: (f64, f64),
    /// The table row the day's training was read as.
    pub carbohydrate_load: TrainingDayLoad,
    /// Daily fat target, grams.
    pub fat_g: f64,
    /// Energy the protein, carbohydrate and fat add up to, kcal/day: equal to
    /// `tdee` unless the day's training calls for more carbohydrate than it
    /// leaves room for (above) or a light day leaves more than fat's ceiling
    /// (below).
    pub energy_from_macros_kcal: f64,
    /// Protein share of energy, percent.
    pub protein_percent: f64,
    /// Carbohydrate share of energy, percent.
    pub carbs_percent: f64,
    /// Fat share of energy, percent.
    pub fat_percent: f64,
    /// The training goal the split was computed for.
    pub goal: String,
}

/// The pre-workout half of a nutrient-timing plan.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreWorkoutTiming {
    /// How long before the session the athlete eats, hours: the gap they gave.
    pub timing_hours_before: f64,
    /// Carbohydrate to take, grams; absent for a session of an hour or less,
    /// or an easy one, which takes no set amount.
    pub carbs_g: Option<f64>,
    /// What to eat, in the athlete's terms.
    pub recommendations: Vec<String>,
}

/// The post-workout half of a nutrient-timing plan.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PostWorkoutTiming {
    /// How long after the session to eat, hours.
    pub timing_hours_after: f64,
    /// Protein to take, grams.
    pub protein_g: f64,
    /// Carbohydrate to take, grams.
    pub carbs_g: f64,
    /// What to eat, in the athlete's terms.
    pub recommendations: Vec<String>,
}

/// How the day's protein is spread across meals.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProteinDistribution {
    /// Meals the target is split across.
    pub meals_per_day: u8,
    /// Protein per meal, grams.
    pub protein_per_meal_g: f64,
    /// The strategy behind the split.
    pub strategy: String,
}

/// What `get_nutrient_timing` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NutrientTimingResult {
    /// What to take before the session.
    pub pre_workout: PreWorkoutTiming,
    /// What to take after it.
    pub post_workout: PostWorkoutTiming,
    /// How to spread protein across the day.
    pub daily_protein_distribution: ProteinDistribution,
    /// Where the workout intensity came from — stated so the athlete can tell
    /// a figure they gave from one the server inferred.
    pub intensity_source: String,
}

/// What `search_food` answers with.
///
/// `foods` carries USDA's own `FoodSearchResult` rather than a projection, so
/// the declared schema is the vendor's shape. That is a deliberate call: it
/// keeps one source of truth, at the cost that a change at USDA's end changes
/// this published contract without anyone here agreeing to it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchFoodResult {
    /// The matches on this page.
    pub foods: Vec<FoodSearchResult>,
    /// How many came back on this page.
    pub returned_count: usize,
    /// How many matched in total.
    pub total_hits: u32,
    /// Which page this is, 1-based.
    pub page_number: u32,
    /// How many per page were asked for.
    pub page_size: u32,
    /// How many pages the match set spans.
    pub total_pages: u32,
    /// Whether another page follows.
    pub has_more: bool,
}

/// One nutrient on a food, as `get_food_details` reports it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FoodNutrientEntry {
    /// USDA nutrient identifier.
    pub nutrient_id: u32,
    /// Nutrient name.
    pub name: String,
    /// How much of it, in `unit`.
    pub amount: f64,
    /// The unit `amount` is measured in.
    pub unit: String,
}

/// What `get_food_details` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FoodDetailsResult {
    /// USDA food identifier.
    pub fdc_id: u64,
    /// Food description.
    pub description: String,
    /// Which USDA dataset it came from.
    pub data_type: String,
    /// Its nutrients.
    pub nutrients: Vec<FoodNutrientEntry>,
    /// Serving size, when USDA states one.
    pub serving_size: Option<f64>,
    /// The unit the serving size is in.
    pub serving_size_unit: Option<String>,
}

/// One food in an analysed meal.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MealFoodEntry {
    /// USDA food identifier.
    pub fdc_id: u64,
    /// Food description.
    pub description: String,
    /// How much of it, grams.
    pub grams: f64,
}

/// What `analyze_meal_nutrition` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AnalyzeMealNutritionResult {
    /// Meal energy, kcal, rounded.
    pub total_calories: f64,
    /// Meal protein, grams, rounded.
    pub total_protein_g: f64,
    /// Meal carbohydrate, grams, rounded.
    pub total_carbs_g: f64,
    /// Meal fat, grams, rounded.
    pub total_fat_g: f64,
    /// The foods it was computed from.
    pub foods: Vec<MealFoodEntry>,
}

// ============================================================================
// CalculateDailyNutritionTool
// ============================================================================

/// Tool for calculating daily calorie and macronutrient needs.
#[derive(Debug)]
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
                description: Some("male or female".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "activity_level".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "sedentary, lightly_active, moderately_active, very_active or extra_active"
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
                    "maintenance, weight_loss, muscle_gain or endurance_performance".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "training_minutes".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some(
                    "Training minutes that day, planned or done; omit both for a rest day"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "training_intensity".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "easy (HR zones 1-2), moderate (3), hard (4) or very_hard (5+)".to_owned(),
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
        answers_with::<NutritionAnswer<DailyNutritionResult>>(tool_definition(
            "calculate_daily_nutrition",
            "Calculate daily calorie and macronutrient needs from biometrics and goals, \
             carbohydrate from the day's training. For an athlete with a medical/PAR-Q flag on \
             file it returns qualitative `guidance` and `figures_withheld` instead of any \
             amount: their clinician sets those.",
            schema,
            annotations(
                "Calculate daily nutrition",
                Effect::ReadOnly,
                Reach::Internal,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
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

            if let Some(withheld) = gate_for_call(&context).await? {
                let guidance = if withheld.names_clinician() {
                    DAILY_NUTRITION_GUIDANCE
                } else {
                    DAILY_NUTRITION_GUIDANCE_UNNAMED
                };
                return ok_typed(
                    "calculate_daily_nutrition",
                    NutritionAnswer::<DailyNutritionResult>::withheld(withheld, guidance),
                );
            }

            let cageux_config = context.cageux_config();
            let nutrition_config = &cageux_config.nutrition;
            // The kernel's macronutrient factors, with the admin's athlete
            // protein target in place of the compiled one when an admin set it.
            let mut macronutrients = nutrition_config.macronutrients.clone();
            if let Some(g_per_kg) = athlete_protein_override(&context).await? {
                macronutrients.protein_athlete_g_per_kg = g_per_kg;
            }

            match calculate_daily_nutrition_needs(
                &params,
                &nutrition_config.bmr,
                &nutrition_config.activity_factors,
                &macronutrients,
            ) {
                Ok(nutrition) => ok_typed(
                    "calculate_daily_nutrition",
                    NutritionAnswer::Figures(DailyNutritionResult {
                        bmr: nutrition.bmr,
                        tdee: nutrition.tdee,
                        protein_g: nutrition.protein_g,
                        carbs_g: nutrition.carbs_g,
                        carbs_g_range: nutrition.carbs_g_range,
                        carbohydrate_load: nutrition.carbohydrate_load,
                        fat_g: nutrition.fat_g,
                        energy_from_macros_kcal: nutrition.energy_from_macros_kcal,
                        protein_percent: nutrition.macro_percentages.protein_percent,
                        carbs_percent: nutrition.macro_percentages.carbs_percent,
                        fat_percent: nutrition.macro_percentages.fat_percent,
                        goal: format!("{:?}", params.training_goal),
                    }),
                ),
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
#[derive(Debug)]
pub struct GetNutrientTimingTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetNutrientTimingTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "workout_intensity".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("low, moderate or high".to_owned()),
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
        properties.insert(
            "session_minutes".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Session length, minutes".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "hours_before".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Hours from eating to the session start, 0-24".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec![
                "workout_intensity".to_owned(),
                "weight_kg".to_owned(),
                "daily_protein_g".to_owned(),
                "session_minutes".to_owned(),
                "hours_before".to_owned(),
            ]),
        );
        answers_with::<NutritionAnswer<NutrientTimingResult>>(tool_definition(
            "get_nutrient_timing",
            "Get nutrient timing around one session, from its length and how long before it the \
             athlete eats. For an athlete with a medical/PAR-Q flag on file it returns \
             qualitative `guidance` and `figures_withheld` instead of any amount: their \
             clinician sets those.",
            schema,
            annotations("Get nutrient timing", Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
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
            let session = parse_planned_session(&args)?;

            if let Some(withheld) = gate_for_call(&context).await? {
                let guidance = if withheld.names_clinician() {
                    NUTRIENT_TIMING_GUIDANCE
                } else {
                    NUTRIENT_TIMING_GUIDANCE_UNNAMED
                };
                return ok_typed(
                    "get_nutrient_timing",
                    NutritionAnswer::<NutrientTimingResult>::withheld(withheld, guidance),
                );
            }

            let cageux_config = context.cageux_config();
            let config = &cageux_config.nutrition;

            match calculate_nutrient_timing(
                weight_kg,
                daily_protein_g,
                workout_intensity,
                session,
                &config.nutrient_timing,
            ) {
                Ok(timing) => ok_typed(
                    "get_nutrient_timing",
                    NutritionAnswer::Figures(NutrientTimingResult {
                        pre_workout: PreWorkoutTiming {
                            timing_hours_before: timing.pre_workout.timing_hours_before,
                            carbs_g: timing.pre_workout.carbs_g,
                            recommendations: timing.pre_workout.recommendations,
                        },
                        post_workout: PostWorkoutTiming {
                            timing_hours_after: timing.post_workout.timing_hours_after,
                            protein_g: timing.post_workout.protein_g,
                            carbs_g: timing.post_workout.carbs_g,
                            recommendations: timing.post_workout.recommendations,
                        },
                        daily_protein_distribution: ProteinDistribution {
                            meals_per_day: timing.daily_protein_distribution.meals_per_day,
                            protein_per_meal_g: timing
                                .daily_protein_distribution
                                .protein_per_meal_g,
                            strategy: timing.daily_protein_distribution.strategy,
                        },
                        intensity_source: intensity_source.to_owned(),
                    }),
                ),
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
#[derive(Debug)]
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
                description: Some(format!(
                    "Number of results per page (default: 10, max: {MAX_FOOD_SEARCH_PAGE_SIZE})"
                )),
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
        answers_with::<SearchFoodResult>(tool_definition(
            "search_food",
            "Search USDA FoodData Central for foods.",
            schema,
            annotations("Search food", Effect::ReadOnly, Reach::OpenWorld),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            // Input is validated before the client is built, so a blank query
            // is refused as invalid whether or not a USDA key is configured.
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .filter(|q| !q.trim().is_empty())
                .ok_or_else(|| {
                    AppError::invalid_input("Missing or invalid required parameter: query")
                })?;

            let client = match build_usda_client(&context) {
                Ok(c) => c,
                Err(err_result) => return Ok(err_result),
            };

            let page_size_u64 = args.get("page_size").and_then(Value::as_u64).unwrap_or(10);

            // The schema promises at most 50 results a page, and the model
            // reads every one it is handed: USDA's own ceiling of 200 is not
            // the tool's.
            #[allow(clippy::cast_possible_truncation)]
            let page_size = if (1..=MAX_FOOD_SEARCH_PAGE_SIZE).contains(&page_size_u64) {
                page_size_u64 as u32
            } else {
                return Ok(ToolResult::error(json!({
                    "error": format!("Page size must be between 1 and {MAX_FOOD_SEARCH_PAGE_SIZE}")
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
                    ok_typed(
                        "search_food",
                        SearchFoodResult {
                            foods: paginated.foods,
                            returned_count: count,
                            total_hits: paginated.total_hits,
                            page_number: paginated.current_page,
                            page_size,
                            total_pages: paginated.total_pages,
                            has_more,
                        },
                    )
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
#[derive(Debug)]
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
        answers_with::<FoodDetailsResult>(tool_definition(
            "get_food_details",
            "Get detailed nutritional information for a specific food item",
            schema,
            annotations("Get food details", Effect::ReadOnly, Reach::OpenWorld),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
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
                Ok(food) => ok_typed(
                    "get_food_details",
                    FoodDetailsResult {
                        fdc_id: food.fdc_id,
                        description: food.description,
                        data_type: food.data_type,
                        nutrients: food
                            .food_nutrients
                            .iter()
                            .map(|n| FoodNutrientEntry {
                                nutrient_id: n.nutrient_id,
                                name: n.nutrient_name.clone(),
                                amount: n.amount,
                                unit: n.unit_name.clone(),
                            })
                            .collect(),
                        serving_size: food.serving_size,
                        serving_size_unit: food.serving_size_unit,
                    },
                ),
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
#[derive(Debug)]
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
                    properties: Some(BTreeMap::from([
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
        answers_with::<AnalyzeMealNutritionResult>(tool_definition(
            "analyze_meal_nutrition",
            "Analyze nutritional content of a meal from its ingredients",
            schema,
            annotations("Analyze meal nutrition", Effect::ReadOnly, Reach::OpenWorld),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
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
            check_ingredient_count(ingredients)?;

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

                food_details_list.push(MealFoodEntry {
                    fdc_id,
                    description: food.description,
                    grams,
                });
            }

            ok_typed(
                "analyze_meal_nutrition",
                AnalyzeMealNutritionResult {
                    total_calories: total_calories.round(),
                    total_protein_g: total_protein.round(),
                    total_carbs_g: total_carbs.round(),
                    total_fat_g: total_fat.round(),
                    foods: food_details_list,
                },
            )
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
// Food references and pure calculators.
crate::declare_security!(
    SearchFoodTool => empty; food_database,
    reads: ToolData::of(DataClass::NoSubject, &[]),
);
crate::declare_security!(
    GetFoodDetailsTool => empty; food_database,
    reads: ToolData::of(DataClass::NoSubject, &[]),
);
crate::declare_security!(
    GetNutrientTimingTool => empty; first_party_only,
    reads: ToolData::of(DataClass::Wellness, &[Stream::Body, Stream::Profile]),
);
crate::declare_security!(
    AnalyzeMealNutritionTool => empty; food_database,
    reads: ToolData::of(DataClass::NoSubject, &[]),
);
// Nutrition targets, which the athlete's medical flag governs; the flag
// itself is not in the output (owl Q8).
crate::declare_security!(
    CalculateDailyNutritionTool => empty; first_party_only,
    reads: ToolData::of(DataClass::Wellness, &[Stream::Body, Stream::Profile]),
);
