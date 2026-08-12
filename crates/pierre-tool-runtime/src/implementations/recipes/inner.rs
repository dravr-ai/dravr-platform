// ABOUTME: Inlined recipe tool implementations (moved from protocols/universal/handlers/recipes.rs).
// ABOUTME: Refactored to take ToolExecutionContext + raw args and return ToolResult directly.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::models::TenantId;
use serde::Deserialize;
use serde_json::{json, Value};

use pierre_intelligence::config::intelligence::MealTdeeProportionsConfig;
use pierre_intelligence::recipes::{
    convert_to_grams, DietaryRestriction, IngredientUnit, MacroTargets, MacroTargetsExt,
    MealTiming, Recipe, RecipeConstraints, RecipeIngredient, SkillLevel,
};

use crate::context::ToolExecutionContext;
use pierre_core::errors::{AppError, AppResult};
use pierre_external::{UsdaClient, UsdaClientConfig};
use pierre_formatters::{format_output, OutputFormat};
use pierre_tools_core::ToolResult;

/// TDEE context for calorie calculation.
struct TdeeContext<'a> {
    tdee_based: bool,
    tdee: Option<f64>,
    proportions: &'a MealTdeeProportionsConfig,
}

#[derive(Debug, Deserialize)]
struct SaveRecipeParams {
    name: String,
    description: Option<String>,
    servings: u8,
    prep_time_mins: Option<u16>,
    cook_time_mins: Option<u16>,
    instructions: Vec<String>,
    ingredients: Vec<IngredientInput>,
    tags: Option<Vec<String>>,
    meal_timing: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IngredientInput {
    name: String,
    amount: f64,
    unit: String,
    preparation: Option<String>,
}

/// Pull `format` from args, defaulting to JSON.
fn parse_output_format(args: &Value) -> OutputFormat {
    args.get("format")
        .and_then(Value::as_str)
        .map_or(OutputFormat::Json, OutputFormat::from_str_param)
}

/// Apply TOON formatting to a payload, mirroring `apply_format_to_response`.
fn apply_format(payload: Value, data_key: &str, format: OutputFormat) -> Value {
    match format {
        OutputFormat::Json => payload,
        OutputFormat::Toon => match format_output(&payload, OutputFormat::Toon) {
            Ok(formatted) => json!({
                format!("{data_key}_toon"): formatted.data,
                "format": "toon",
            }),
            Err(e) => json!({
                data_key: payload,
                "format": "json",
                "format_fallback": true,
                "format_error": e.to_string(),
            }),
        },
    }
}

// ---------------------------------------------------------------------------
// get_recipe_constraints
// ---------------------------------------------------------------------------

pub fn handle_get_recipe_constraints(ctx: &ToolExecutionContext, args: &Value) -> ToolResult {
    let cageux_config = ctx.cageux_config();

    let meal_timing = args
        .get("meal_timing")
        .and_then(Value::as_str)
        .map_or(MealTiming::General, parse_meal_timing);

    let tdee = args.get("tdee").and_then(Value::as_f64);
    let tdee_proportions = &cageux_config.nutrition.meal_tdee_proportions;

    let (calories, tdee_based) = args.get("calories").and_then(Value::as_f64).map_or_else(
        || {
            (
                tdee_proportions.calories_for_timing(meal_timing, tdee),
                tdee.is_some(),
            )
        },
        |explicit_cals| (explicit_cals, false),
    );

    let macro_targets =
        MacroTargets::from_calories_and_timing(calories, meal_timing, &cageux_config.nutrition);
    let (protein_pct, carbs_pct, fat_pct) = meal_timing.macro_distribution();

    let tdee_ctx = TdeeContext {
        tdee_based,
        tdee,
        proportions: tdee_proportions,
    };

    let prompt_hint = build_recipe_prompt_hint(
        meal_timing,
        calories,
        &macro_targets,
        protein_pct,
        carbs_pct,
        fat_pct,
        &tdee_ctx,
    );

    let constraints = build_recipe_constraints(macro_targets, meal_timing, &prompt_hint, args);

    let result =
        build_constraints_response(&constraints, calories, meal_timing, &prompt_hint, &tdee_ctx);

    ToolResult::ok(result)
}

fn build_recipe_prompt_hint(
    timing: MealTiming,
    calories: f64,
    macros: &MacroTargets,
    protein_pct: u8,
    carbs_pct: u8,
    fat_pct: u8,
    tdee_ctx: &TdeeContext<'_>,
) -> String {
    let tdee_info = if tdee_ctx.tdee_based {
        let proportion = tdee_ctx.proportions.proportion_for_timing(timing);
        format!(
            " (Based on TDEE of {:.0} kcal, {:.1}% of daily calories)",
            tdee_ctx.tdee.unwrap_or(0.0),
            proportion * 100.0
        )
    } else {
        String::new()
    };

    format!(
        "Create a {} recipe (~{:.0} kcal){} with approximately {:.0}g protein, {:.0}g carbs, {:.0}g fat. \
         Macro distribution: {}% protein, {}% carbs, {}% fat.",
        timing.description(),
        calories,
        tdee_info,
        macros.protein_g.unwrap_or(0.0),
        macros.carbs_g.unwrap_or(0.0),
        macros.fat_g.unwrap_or(0.0),
        protein_pct,
        carbs_pct,
        fat_pct
    )
}

fn build_recipe_constraints(
    macro_targets: MacroTargets,
    meal_timing: MealTiming,
    prompt_hint: &str,
    args: &Value,
) -> RecipeConstraints {
    RecipeConstraints {
        macro_targets,
        dietary_restrictions: parse_dietary_restrictions(
            args.get("dietary_restrictions").and_then(Value::as_array),
        ),
        cuisine_preferences: Vec::new(),
        excluded_ingredients: Vec::new(),
        max_prep_time_mins: parse_time_mins(args, "max_prep_time_mins"),
        max_cook_time_mins: parse_time_mins(args, "max_cook_time_mins"),
        skill_level: SkillLevel::default(),
        meal_timing,
        prompt_hint: Some(prompt_hint.to_owned()),
    }
}

fn parse_time_mins(params: &Value, key: &str) -> Option<u16> {
    params.get(key).and_then(Value::as_u64).map(|v| {
        #[allow(clippy::cast_possible_truncation)]
        let capped = v.min(480) as u16;
        capped
    })
}

fn build_constraints_response(
    constraints: &RecipeConstraints,
    calories: f64,
    meal_timing: MealTiming,
    prompt_hint: &str,
    tdee_ctx: &TdeeContext<'_>,
) -> Value {
    let mut result = json!({
        "calories": calories,
        "protein_g": constraints.macro_targets.protein_g,
        "carbs_g": constraints.macro_targets.carbs_g,
        "fat_g": constraints.macro_targets.fat_g,
        "meal_timing": format!("{meal_timing:?}").to_lowercase(),
        "meal_timing_description": meal_timing.description(),
        "prompt_hint": prompt_hint,
        "max_prep_time_mins": constraints.max_prep_time_mins,
        "max_cook_time_mins": constraints.max_cook_time_mins,
        "tdee_based": tdee_ctx.tdee_based,
    });

    if let Some(user_tdee) = tdee_ctx.tdee {
        result["tdee"] = json!(user_tdee);
        result["tdee_proportion"] = json!(tdee_ctx.proportions.proportion_for_timing(meal_timing));
    }

    result
}

// ---------------------------------------------------------------------------
// validate_recipe
// ---------------------------------------------------------------------------

pub async fn handle_validate_recipe(
    ctx: &ToolExecutionContext,
    args: Value,
) -> AppResult<ToolResult> {
    let servings_val = args
        .get("servings")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            AppError::invalid_input("validate_recipe: Missing required parameter: servings")
        })?;
    if servings_val == 0 {
        return Err(AppError::invalid_input(
            "validate_recipe: servings must be at least 1",
        ));
    }
    #[allow(clippy::cast_possible_truncation)]
    let servings = servings_val.min(255) as u8;

    let ingredients_json = args
        .get("ingredients")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::invalid_input(
                "validate_recipe: Missing required parameter: ingredients (must be array)",
            )
        })?;

    let api_key = ctx
        .resources
        .config()
        .usda_api_key
        .clone()
        .unwrap_or_default();

    if api_key.is_empty() {
        return Ok(ToolResult::error(json!({
            "error": "USDA API key not configured",
        })));
    }

    let usda_config = UsdaClientConfig {
        api_key,
        ..UsdaClientConfig::default()
    };
    let client = UsdaClient::new(usda_config);

    let mut total_calories = 0.0;
    let mut total_protein = 0.0;
    let mut total_carbs = 0.0;
    let mut total_fat = 0.0;
    let mut total_fiber = 0.0;
    let mut total_sodium = 0.0;
    let mut total_sugar = 0.0;
    let mut warnings: Vec<String> = Vec::new();
    let mut validated_ingredients: Vec<Value> = Vec::new();
    let mut usda_matched_count: u32 = 0;

    for ingredient_value in ingredients_json {
        let name = ingredient_value
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::invalid_input("validate_recipe: Each ingredient must have 'name'")
            })?;

        let amount = ingredient_value
            .get("amount")
            .and_then(Value::as_f64)
            .ok_or_else(|| {
                AppError::invalid_input("validate_recipe: Each ingredient must have 'amount'")
            })?;

        let unit_str = ingredient_value
            .get("unit")
            .and_then(Value::as_str)
            .unwrap_or("grams");

        let unit = parse_ingredient_unit(unit_str);
        let grams = match convert_to_grams(name, amount, unit) {
            Ok(g) => g,
            Err(e) => {
                warnings.push(format!("Could not convert {name}: {e}"));
                if unit.is_volume() {
                    amount * 100.0
                } else if unit.is_count() {
                    amount * 50.0
                } else {
                    amount
                }
            }
        };

        match client.search_foods(name, 1, 1).await {
            Ok(result) if !result.foods.is_empty() => {
                let food = &result.foods[0];
                match client.get_food_details(food.fdc_id).await {
                    Ok(details) => {
                        let multiplier = grams / 100.0;
                        for nutrient in &details.food_nutrients {
                            match nutrient.nutrient_name.as_str() {
                                "Energy" => {
                                    total_calories =
                                        nutrient.amount.mul_add(multiplier, total_calories);
                                }
                                "Protein" => {
                                    total_protein =
                                        nutrient.amount.mul_add(multiplier, total_protein);
                                }
                                "Carbohydrate, by difference" => {
                                    total_carbs = nutrient.amount.mul_add(multiplier, total_carbs);
                                }
                                "Total lipid (fat)" => {
                                    total_fat = nutrient.amount.mul_add(multiplier, total_fat);
                                }
                                "Fiber, total dietary" => {
                                    total_fiber = nutrient.amount.mul_add(multiplier, total_fiber);
                                }
                                "Sodium, Na" => {
                                    total_sodium =
                                        nutrient.amount.mul_add(multiplier, total_sodium);
                                }
                                "Sugars, total including NLEA" => {
                                    total_sugar = nutrient.amount.mul_add(multiplier, total_sugar);
                                }
                                _ => {}
                            }
                        }
                        validated_ingredients.push(json!({
                            "name": name,
                            "amount": amount,
                            "unit": unit_str,
                            "grams": grams,
                            "fdc_id": food.fdc_id,
                            "usda_match": food.description,
                        }));
                        usda_matched_count += 1;
                    }
                    Err(e) => {
                        warnings.push(format!("USDA lookup failed for {name}: {e}"));
                        validated_ingredients.push(json!({
                            "name": name,
                            "amount": amount,
                            "unit": unit_str,
                            "grams": grams,
                            "usda_match": null,
                        }));
                    }
                }
            }
            Ok(_) => {
                warnings.push(format!("No USDA match found for: {name}"));
                validated_ingredients.push(json!({
                    "name": name,
                    "amount": amount,
                    "unit": unit_str,
                    "grams": grams,
                    "usda_match": null,
                }));
            }
            Err(e) => {
                warnings.push(format!("USDA search failed for {name}: {e}"));
                validated_ingredients.push(json!({
                    "name": name,
                    "amount": amount,
                    "unit": unit_str,
                    "grams": grams,
                    "usda_match": null,
                }));
            }
        }
    }

    let servings_f64 = f64::from(servings);
    let nutrition_per_serving = json!({
        "calories": (total_calories / servings_f64).round(),
        "protein_g": (total_protein / servings_f64 * 10.0).round() / 10.0,
        "carbs_g": (total_carbs / servings_f64 * 10.0).round() / 10.0,
        "fat_g": (total_fat / servings_f64 * 10.0).round() / 10.0,
        "fiber_g": (total_fiber / servings_f64 * 10.0).round() / 10.0,
        "sodium_mg": (total_sodium / servings_f64).round(),
        "sugar_g": (total_sugar / servings_f64 * 10.0).round() / 10.0,
    });

    #[allow(clippy::cast_precision_loss)]
    let total_ingredients = validated_ingredients.len() as f64;
    let validation_completeness = if total_ingredients > 0.0 {
        (f64::from(usda_matched_count) / total_ingredients * 100.0).round() / 100.0
    } else {
        0.0
    };

    Ok(ToolResult::ok(json!({
        "validated": true,
        "servings": servings,
        "nutrition_per_serving": nutrition_per_serving,
        "ingredients": validated_ingredients,
        "warnings": warnings,
        "validated_at": Utc::now().to_rfc3339(),
        "validation_completeness": validation_completeness,
        "usda_matched_count": usda_matched_count,
        "total_ingredients": validated_ingredients.len(),
    })))
}

// ---------------------------------------------------------------------------
// save_recipe
// ---------------------------------------------------------------------------

pub async fn handle_save_recipe(ctx: &ToolExecutionContext, args: Value) -> AppResult<ToolResult> {
    let user_id = ctx.user_id;
    let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

    let params: SaveRecipeParams = serde_json::from_value(args).map_err(|e| {
        AppError::invalid_input(format!("save_recipe: Invalid recipe parameters: {e}"))
    })?;

    let meal_timing = params
        .meal_timing
        .as_deref()
        .map_or(MealTiming::General, parse_meal_timing);

    let mut recipe = Recipe::new(user_id, &params.name, params.servings)
        .with_meal_timing(meal_timing)
        .with_instructions(params.instructions);

    if let Some(desc) = params.description {
        recipe = recipe.with_description(desc);
    }

    if let Some(prep) = params.prep_time_mins {
        recipe = recipe.with_prep_time(prep);
    }

    if let Some(cook) = params.cook_time_mins {
        recipe = recipe.with_cook_time(cook);
    }

    if let Some(tags) = params.tags {
        for tag in tags {
            recipe = recipe.with_tag(tag);
        }
    }

    let mut ingredients = Vec::new();
    for ing in params.ingredients {
        let unit = parse_ingredient_unit(&ing.unit);
        let grams = convert_to_grams(&ing.name, ing.amount, unit).unwrap_or(ing.amount);
        let mut ingredient = RecipeIngredient::new(&ing.name, ing.amount, unit, grams);
        if let Some(prep) = ing.preparation {
            ingredient = ingredient.with_preparation(prep);
        }
        ingredients.push(ingredient);
    }
    recipe = recipe.with_ingredients(ingredients);

    let repo = ctx.resources.repos().recipes.as_ref();
    let recipe_id = repo
        .create(user_id, tenant_id, &recipe)
        .await
        .map_err(|e| AppError::internal(format!("save_recipe: Failed to save recipe: {e}")))?;

    Ok(ToolResult::ok(json!({
        "recipe_id": recipe_id,
        "name": params.name,
        "servings": params.servings,
        "meal_timing": format!("{meal_timing:?}").to_lowercase(),
        "created_at": Utc::now().to_rfc3339(),
    })))
}

// ---------------------------------------------------------------------------
// list_recipes
// ---------------------------------------------------------------------------

pub async fn handle_list_recipes(ctx: &ToolExecutionContext, args: Value) -> AppResult<ToolResult> {
    let output_format = parse_output_format(&args);
    let user_id = ctx.user_id;
    let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

    let meal_timing = args
        .get("meal_timing")
        .and_then(Value::as_str)
        .map(parse_meal_timing);

    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(20_u32, |v| {
            #[allow(clippy::cast_possible_truncation)]
            let capped = v.min(100) as u32;
            capped
        });

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let offset = args.get("offset").and_then(|v| {
        v.as_u64()
            .map(|n| n.min(u64::from(u32::MAX)) as u32)
            .or_else(|| v.as_f64().map(|f| f as u32))
    });

    let repo = ctx.resources.repos().recipes.as_ref();
    let recipes = repo
        .list(user_id, tenant_id, meal_timing, Some(limit), offset)
        .await
        .map_err(|e| AppError::internal(format!("list_recipes: Failed to list recipes: {e}")))?;

    let recipe_summaries: Vec<Value> = recipes
        .iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "name": r.name,
                "servings": r.servings,
                "meal_timing": format!("{:?}", r.meal_timing).to_lowercase(),
                "total_time_mins": r.total_time_mins(),
                "tags": r.tags,
                "has_nutrition": r.nutrition.is_some(),
                "calories_per_serving": r.nutrition.as_ref().map(|n| n.calories.round()),
                "updated_at": r.updated_at.to_rfc3339(),
            })
        })
        .collect();

    let returned_count = recipe_summaries.len();
    #[allow(clippy::cast_possible_truncation)]
    let has_more = returned_count == limit as usize;
    let offset_val = offset.unwrap_or(0);

    let payload = json!({
        "recipes": recipe_summaries,
        "count": returned_count,
        "offset": offset_val,
        "limit": limit,
        "has_more": has_more,
    });

    Ok(ToolResult::ok(apply_format(
        payload,
        "recipes",
        output_format,
    )))
}

// ---------------------------------------------------------------------------
// get_recipe
// ---------------------------------------------------------------------------

pub async fn handle_get_recipe(ctx: &ToolExecutionContext, args: Value) -> AppResult<ToolResult> {
    let output_format = parse_output_format(&args);
    let user_id = ctx.user_id;
    let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

    let recipe_id = args
        .get("recipe_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::invalid_input("get_recipe: Missing required parameter: recipe_id")
        })?;

    let repo = ctx.resources.repos().recipes.as_ref();
    let recipe = repo
        .get_by_id(recipe_id, user_id, tenant_id)
        .await
        .map_err(|e| AppError::internal(format!("get_recipe: Failed to get recipe: {e}")))?;

    match recipe {
        Some(r) => {
            let payload = json!({
                "id": r.id.to_string(),
                "name": r.name,
                "description": r.description,
                "servings": r.servings,
                "prep_time_mins": r.prep_time_mins,
                "cook_time_mins": r.cook_time_mins,
                "total_time_mins": r.total_time_mins(),
                "meal_timing": format!("{:?}", r.meal_timing).to_lowercase(),
                "ingredients": r.ingredients.iter().map(|i| json!({
                    "name": i.name,
                    "amount": i.amount,
                    "unit": format!("{:?}", i.unit).to_lowercase(),
                    "grams": i.grams,
                    "preparation": i.preparation,
                    "fdc_id": i.fdc_id,
                })).collect::<Vec<_>>(),
                "instructions": r.instructions,
                "tags": r.tags,
                "nutrition_per_serving": r.nutrition.map(|n| json!({
                    "calories": n.calories.round(),
                    "protein_g": (n.protein_g * 10.0).round() / 10.0,
                    "carbs_g": (n.carbs_g * 10.0).round() / 10.0,
                    "fat_g": (n.fat_g * 10.0).round() / 10.0,
                    "fiber_g": n.fiber_g.map(|v| (v * 10.0).round() / 10.0),
                    "sodium_mg": n.sodium_mg.map(f64::round),
                    "sugar_g": n.sugar_g.map(|v| (v * 10.0).round() / 10.0),
                    "validated_at": n.validated_at.to_rfc3339(),
                })),
                "created_at": r.created_at.to_rfc3339(),
                "updated_at": r.updated_at.to_rfc3339(),
            });
            Ok(ToolResult::ok(apply_format(
                payload,
                "recipe",
                output_format,
            )))
        }
        None => Ok(ToolResult::error(json!({
            "error": format!("Recipe not found: {recipe_id}"),
        }))),
    }
}

// ---------------------------------------------------------------------------
// delete_recipe
// ---------------------------------------------------------------------------

pub async fn handle_delete_recipe(
    ctx: &ToolExecutionContext,
    args: Value,
) -> AppResult<ToolResult> {
    let user_id = ctx.user_id;
    let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

    let recipe_id = args
        .get("recipe_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::invalid_input("delete_recipe: Missing required parameter: recipe_id")
        })?;

    let repo = ctx.resources.repos().recipes.as_ref();
    let deleted = repo
        .delete(recipe_id, user_id, tenant_id)
        .await
        .map_err(|e| AppError::internal(format!("delete_recipe: Failed to delete recipe: {e}")))?;

    if deleted {
        Ok(ToolResult::ok(json!({
            "deleted": true,
            "recipe_id": recipe_id,
        })))
    } else {
        Ok(ToolResult::error(json!({
            "error": format!("Recipe not found: {recipe_id}"),
        })))
    }
}

// ---------------------------------------------------------------------------
// search_recipes
// ---------------------------------------------------------------------------

pub async fn handle_search_recipes(
    ctx: &ToolExecutionContext,
    args: Value,
) -> AppResult<ToolResult> {
    let output_format = parse_output_format(&args);
    let user_id = ctx.user_id;
    let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

    let query = args.get("query").and_then(Value::as_str).ok_or_else(|| {
        AppError::invalid_input("search_recipes: Missing required parameter: query")
    })?;

    #[allow(clippy::cast_possible_truncation)]
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(10_u32, |v| v.min(100) as u32);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let offset = args.get("offset").and_then(|v| {
        v.as_u64()
            .map(|n| n.min(u64::from(u32::MAX)) as u32)
            .or_else(|| v.as_f64().map(|f| f as u32))
    });

    let repo = ctx.resources.repos().recipes.as_ref();
    let recipes = repo
        .search(user_id, tenant_id, query, Some(limit), offset)
        .await
        .map_err(|e| {
            AppError::internal(format!("search_recipes: Failed to search recipes: {e}"))
        })?;

    let results: Vec<Value> = recipes
        .iter()
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "name": r.name,
                "description": r.description,
                "servings": r.servings,
                "meal_timing": format!("{:?}", r.meal_timing).to_lowercase(),
                "tags": r.tags,
                "calories_per_serving": r.nutrition.as_ref().map(|n| n.calories.round()),
            })
        })
        .collect();

    let returned_count = results.len();
    #[allow(clippy::cast_possible_truncation)]
    let has_more = returned_count == limit as usize;
    let offset_val = offset.unwrap_or(0);

    let payload = json!({
        "query": query,
        "results": results,
        "count": returned_count,
        "offset": offset_val,
        "limit": limit,
        "has_more": has_more,
    });

    Ok(ToolResult::ok(apply_format(
        payload,
        "results",
        output_format,
    )))
}

// ---------------------------------------------------------------------------
// Shared parsing helpers
// ---------------------------------------------------------------------------

fn parse_meal_timing(s: &str) -> MealTiming {
    match s.to_lowercase().as_str() {
        "pre_training" => MealTiming::PreTraining,
        "post_training" => MealTiming::PostTraining,
        "rest_day" => MealTiming::RestDay,
        _ => MealTiming::General,
    }
}

fn parse_ingredient_unit(s: &str) -> IngredientUnit {
    match s.to_lowercase().as_str() {
        "milliliters" | "ml" => IngredientUnit::Milliliters,
        "cups" | "cup" => IngredientUnit::Cups,
        "tablespoons" | "tbsp" => IngredientUnit::Tablespoons,
        "teaspoons" | "tsp" => IngredientUnit::Teaspoons,
        "pieces" | "piece" | "pc" => IngredientUnit::Pieces,
        "ounces" | "oz" => IngredientUnit::Ounces,
        "pounds" | "lb" => IngredientUnit::Pounds,
        "kilograms" | "kg" => IngredientUnit::Kilograms,
        _ => IngredientUnit::Grams,
    }
}

fn parse_dietary_restrictions(arr: Option<&Vec<Value>>) -> Vec<DietaryRestriction> {
    let Some(values) = arr else {
        return Vec::new();
    };

    values
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|s| match s.to_lowercase().as_str() {
            "gluten_free" => Some(DietaryRestriction::GlutenFree),
            "dairy_free" => Some(DietaryRestriction::DairyFree),
            "vegan" => Some(DietaryRestriction::Vegan),
            "vegetarian" => Some(DietaryRestriction::Vegetarian),
            "nut_free" => Some(DietaryRestriction::NutFree),
            "low_sodium" => Some(DietaryRestriction::LowSodium),
            "low_sugar" => Some(DietaryRestriction::LowSugar),
            "keto" => Some(DietaryRestriction::Keto),
            "paleo" => Some(DietaryRestriction::Paleo),
            _ => None,
        })
        .collect()
}
