// ABOUTME: Integration tests for nutrition MCP tool handlers
// ABOUTME: Tests calculate_daily_nutrition, get_nutrient_timing, search_food, get_food_details, analyze_meal_nutrition
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Nutrition Tool Handler Integration Tests
//!
//! Tests the 5 nutrition MCP tools via the `UniversalToolExecutor`:
//! - `calculate_daily_nutrition`: BMR, TDEE, and macro calculation
//! - `get_nutrient_timing`: Pre/post workout nutrition recommendations
//! - `search_food`: USDA `FoodData` Central search (requires API key)
//! - `get_food_details`: USDA food details lookup (requires API key)
//! - `analyze_meal_nutrition`: Multi-food meal analysis (requires API key)
//!
//! Everything here is offline and independent of the process environment.
//! The USDA-backed tools are covered for input validation and for the refusal
//! they give when no key is configured; the USDA key each executor carries is
//! set explicitly in its config, never inherited from `USDA_API_KEY`. The
//! calls that reach USDA live in the `live-e2e` target `usda_live_test`
//! (carnet#805).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_config::environment::ServerConfig;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::json;
use uuid::Uuid;

mod common;

// ============================================================================
// Test Setup
// ============================================================================

/// USDA key for the offline executor. Never sent: the USDA tools resolve the
/// key first, and every USDA-tool test in this file fails input validation
/// before a request would be made, so the key only gets them past the
/// "key not configured" refusal to the validation under test.
const OFFLINE_USDA_KEY: &str = "offline-test-key-never-sent";

/// Executor whose USDA key is `usda_api_key`, set explicitly so no test here
/// depends on whether the process exports `USDA_API_KEY`.
async fn create_executor_with_usda_key(
    usda_api_key: Option<String>,
) -> Result<UniversalToolExecutor> {
    common::init_server_config();
    common::init_test_http_clients();

    let config = ServerConfig {
        usda_api_key,
        ..common::test_server_config()
    };
    let resources = common::create_test_server_resources_with_config(config).await?;
    Ok(UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()))
}

/// Create test executor for nutrition tool tests: a USDA key is configured,
/// so the USDA tools reach their input validation.
async fn create_nutrition_test_executor() -> Result<UniversalToolExecutor> {
    create_executor_with_usda_key(Some(OFFLINE_USDA_KEY.to_owned())).await
}

/// Create a test executor with no USDA key configured, so the "key not
/// configured" refusal is exercised.
async fn create_nutrition_test_executor_without_usda_key() -> Result<UniversalToolExecutor> {
    create_executor_with_usda_key(None).await
}

/// Create a test request with given parameters
fn create_test_request(tool_name: &str, parameters: serde_json::Value) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool_name.to_owned(),
        parameters,
        user_id: Uuid::new_v4().to_string(),
        protocol: "test".to_owned(),
        tenant_id: None,
    }
}

// ============================================================================
// Tool Registration Tests
// ============================================================================

#[tokio::test]
async fn test_nutrition_tools_registered() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let tool_names: Vec<String> = executor
        .resources
        .tool_registry()
        .tool_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();

    let expected_tools = vec![
        "calculate_daily_nutrition",
        "get_nutrient_timing",
        "search_food",
        "get_food_details",
        "analyze_meal_nutrition",
    ];

    for expected_tool in expected_tools {
        assert!(
            tool_names.contains(&expected_tool.to_owned()),
            "Missing nutrition tool: {expected_tool}"
        );
    }

    Ok(())
}

// ============================================================================
// calculate_daily_nutrition Tests
// ============================================================================

#[tokio::test]
async fn test_calculate_daily_nutrition_male_maintenance() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "weight_kg": 75.0,
            "height_cm": 180.0,
            "age": 30,
            "gender": "male",
            "activity_level": "moderately_active",
            "training_goal": "maintenance"
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success, "Tool should succeed");
    assert!(response.result.is_some(), "Result should be present");

    let result = response.result.unwrap();

    // Verify BMR and TDEE are calculated
    assert!(
        result["bmr"].as_f64().unwrap() > 1500.0,
        "BMR should be > 1500"
    );
    assert!(
        result["tdee"].as_f64().unwrap() > result["bmr"].as_f64().unwrap(),
        "TDEE > BMR"
    );

    // Verify macros are present
    assert!(
        result["protein_g"].as_f64().is_some(),
        "Protein should be calculated"
    );
    assert!(
        result["carbs_g"].as_f64().is_some(),
        "Carbs should be calculated"
    );
    assert!(
        result["fat_g"].as_f64().is_some(),
        "Fat should be calculated"
    );

    // Verify macro percentages sum to 100%
    let protein_pct = result["protein_percent"].as_f64().unwrap();
    let carbs_pct = result["carbs_percent"].as_f64().unwrap();
    let fat_pct = result["fat_percent"].as_f64().unwrap();
    let total = protein_pct + carbs_pct + fat_pct;

    assert!(
        (total - 100.0).abs() < 1.0,
        "Macro percentages should sum to ~100%, got {total}"
    );

    Ok(())
}

#[tokio::test]
async fn test_calculate_daily_nutrition_female_weight_loss() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "weight_kg": 60.0,
            "height_cm": 165.0,
            "age": 28,
            "gender": "female",
            "activity_level": "lightly_active",
            "training_goal": "weight_loss"
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success, "Tool should succeed");
    let result = response.result.unwrap();

    // Female BMR should be lower than male equivalent
    assert!(
        result["bmr"].as_f64().unwrap() > 1200.0,
        "BMR should be > 1200"
    );
    assert!(
        result["bmr"].as_f64().unwrap() < 1600.0,
        "BMR should be < 1600"
    );

    // Weight loss should have elevated protein
    let protein_pct = result["protein_percent"].as_f64().unwrap();
    assert!(
        protein_pct >= 18.0,
        "Weight loss should have elevated protein %"
    );

    Ok(())
}

#[tokio::test]
async fn test_calculate_daily_nutrition_endurance_athlete() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "weight_kg": 70.0,
            "height_cm": 175.0,
            "age": 25,
            "gender": "male",
            "activity_level": "very_active",
            "training_goal": "endurance_performance"
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success, "Tool should succeed");
    let result = response.result.unwrap();

    // Endurance athletes need high carbs
    let carbs = result["carbs_g"].as_f64().unwrap();
    assert!(
        carbs >= 400.0,
        "Endurance athlete should have high carbs: {carbs}g"
    );

    // High TDEE for very active
    let tdee = result["tdee"].as_f64().unwrap();
    assert!(tdee >= 2500.0, "Very active TDEE should be >= 2500: {tdee}");

    Ok(())
}

#[tokio::test]
async fn test_calculate_daily_nutrition_missing_parameters() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    // Missing weight_kg
    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "height_cm": 180.0,
            "age": 30,
            "gender": "male",
            "activity_level": "moderately_active",
            "training_goal": "maintenance"
        }),
    );

    // Post-refactor (2b98ba43): schema validation runs before the tool body,
    // so missing required params surface as `Err` rather than
    // `Ok(failed_response)`. The test's intent — "missing weight_kg is
    // rejected" — is preserved with the new contract.
    let result = executor.execute_tool(request).await;
    assert!(
        result.is_err(),
        "Should fail with missing weight_kg parameter"
    );
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("weight_kg"),
        "Error should mention the missing parameter: {err}"
    );

    Ok(())
}

#[tokio::test]
async fn test_calculate_daily_nutrition_invalid_gender() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "weight_kg": 75.0,
            "height_cm": 180.0,
            "age": 30,
            "gender": "invalid",
            "activity_level": "moderately_active",
            "training_goal": "maintenance"
        }),
    );

    // Post-refactor (2b98ba43): schema validation rejects invalid enums as
    // `Err` rather than `Ok(failed_response)`.
    let result = executor.execute_tool(request).await;
    assert!(result.is_err(), "Should fail with invalid gender");

    Ok(())
}

#[tokio::test]
async fn test_calculate_daily_nutrition_invalid_activity_level() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "weight_kg": 75.0,
            "height_cm": 180.0,
            "age": 30,
            "gender": "male",
            "activity_level": "super_duper_active",
            "training_goal": "maintenance"
        }),
    );

    let result = executor.execute_tool(request).await;
    assert!(result.is_err(), "Should fail with invalid activity level");

    Ok(())
}

#[tokio::test]
async fn test_calculate_daily_nutrition_invalid_training_goal() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "weight_kg": 75.0,
            "height_cm": 180.0,
            "age": 30,
            "gender": "male",
            "activity_level": "moderately_active",
            "training_goal": "get_swole"
        }),
    );

    let result = executor.execute_tool(request).await;
    assert!(result.is_err(), "Should fail with invalid training goal");

    Ok(())
}

#[tokio::test]
async fn test_calculate_daily_nutrition_boundary_age() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    // Age too high (>150)
    let request = create_test_request(
        "calculate_daily_nutrition",
        json!({
            "weight_kg": 75.0,
            "height_cm": 180.0,
            "age": 200,
            "gender": "male",
            "activity_level": "moderately_active",
            "training_goal": "maintenance"
        }),
    );

    let result = executor.execute_tool(request).await;
    assert!(result.is_err(), "Should fail with unrealistic age");

    Ok(())
}

// ============================================================================
// get_nutrient_timing Tests
// ============================================================================

#[tokio::test]
async fn test_get_nutrient_timing_high_intensity() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "get_nutrient_timing",
        json!({
            "weight_kg": 75.0,
            "daily_protein_g": 150.0,
            "workout_intensity": "high"
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success, "Tool should succeed");
    let result = response.result.unwrap();

    // Verify pre-workout recommendations
    assert!(result["pre_workout"].is_object(), "Should have pre_workout");
    assert!(
        result["pre_workout"]["carbs_g"].as_f64().unwrap() >= 40.0,
        "High intensity needs substantial pre-workout carbs"
    );

    // Verify post-workout recommendations
    assert!(
        result["post_workout"].is_object(),
        "Should have post_workout"
    );
    assert!(
        result["post_workout"]["protein_g"].as_f64().unwrap() >= 20.0,
        "Post-workout needs protein"
    );

    // Verify protein distribution
    assert!(
        result["daily_protein_distribution"].is_object(),
        "Should have protein distribution"
    );
    assert!(
        result["daily_protein_distribution"]["meals_per_day"]
            .as_u64()
            .unwrap()
            >= 3,
        "Should distribute across 3+ meals"
    );

    // Verify intensity source
    assert_eq!(
        result["intensity_source"].as_str().unwrap(),
        "explicit",
        "Should indicate explicit intensity"
    );

    Ok(())
}

#[tokio::test]
async fn test_get_nutrient_timing_low_intensity() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "get_nutrient_timing",
        json!({
            "weight_kg": 60.0,
            "daily_protein_g": 100.0,
            "workout_intensity": "low"
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success, "Tool should succeed");
    let result = response.result.unwrap();

    // Low intensity should have smaller pre-workout carbs
    let pre_workout_carbs = result["pre_workout"]["carbs_g"].as_f64().unwrap();
    assert!(
        pre_workout_carbs < 40.0,
        "Low intensity should have fewer pre-workout carbs: {pre_workout_carbs}"
    );

    Ok(())
}

#[tokio::test]
async fn test_get_nutrient_timing_moderate_intensity() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "get_nutrient_timing",
        json!({
            "weight_kg": 70.0,
            "daily_protein_g": 140.0,
            "workout_intensity": "moderate"
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success, "Tool should succeed");
    let result = response.result.unwrap();

    // Moderate intensity should have balanced recommendations
    let pre_workout_carbs = result["pre_workout"]["carbs_g"].as_f64().unwrap();
    assert!(
        (50.0..=60.0).contains(&pre_workout_carbs),
        "Moderate intensity should have ~52g carbs: {pre_workout_carbs}"
    );

    Ok(())
}

#[tokio::test]
async fn test_get_nutrient_timing_missing_weight() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "get_nutrient_timing",
        json!({
            "daily_protein_g": 150.0,
            "workout_intensity": "high"
        }),
    );

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail with missing weight_kg");

    Ok(())
}

#[tokio::test]
async fn test_get_nutrient_timing_missing_intensity() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    // Missing the required workout_intensity
    let request = create_test_request(
        "get_nutrient_timing",
        json!({
            "weight_kg": 75.0,
            "daily_protein_g": 150.0
        }),
    );

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail without workout_intensity");

    Ok(())
}

// ============================================================================
// search_food Tests (offline: validation and the no-key refusal)
// ============================================================================

#[tokio::test]
async fn test_search_food_no_api_key() -> Result<()> {
    let executor = create_nutrition_test_executor_without_usda_key().await?;

    let request = create_test_request(
        "search_food",
        json!({
            "query": "chicken breast"
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail without API key");
    assert!(
        response.error.unwrap().contains("USDA API key"),
        "Error should mention API key"
    );

    Ok(())
}

#[tokio::test]
async fn test_search_food_missing_query() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request("search_food", json!({}));

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail with missing query");

    Ok(())
}

#[tokio::test]
async fn test_search_food_page_size_boundary() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    // Page size > 200 should be rejected
    let request = create_test_request(
        "search_food",
        json!({
            "query": "apple",
            "page_size": 300
        }),
    );

    let response = executor.execute_tool(request).await?;

    // A key is configured, so this is the page-size validation, not the
    // missing-key refusal.
    assert!(!response.success, "Should not succeed with page_size > 200");
    let error = response.error.unwrap_or_default();
    assert!(
        error.contains("Page size must be between 1 and 200"),
        "Should be rejected by page-size validation, got: {error}"
    );

    Ok(())
}

// ============================================================================
// get_food_details Tests (offline: validation and the no-key refusal)
// ============================================================================

#[tokio::test]
async fn test_get_food_details_no_api_key() -> Result<()> {
    let executor = create_nutrition_test_executor_without_usda_key().await?;

    let request = create_test_request(
        "get_food_details",
        json!({
            "fdc_id": 171_477
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail without API key");
    assert!(response.error.unwrap().contains("USDA API key"));

    Ok(())
}

#[tokio::test]
async fn test_get_food_details_missing_fdc_id() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request("get_food_details", json!({}));

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail with missing fdc_id");

    Ok(())
}

// ============================================================================
// analyze_meal_nutrition Tests (offline: validation and the no-key refusal)
// ============================================================================

#[tokio::test]
async fn test_analyze_meal_nutrition_no_api_key() -> Result<()> {
    let executor = create_nutrition_test_executor_without_usda_key().await?;

    let request = create_test_request(
        "analyze_meal_nutrition",
        json!({
            "ingredients": [
                {"fdc_id": 171_477, "amount_g": 150.0},
                {"fdc_id": 171_688, "amount_g": 100.0}
            ]
        }),
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail without API key");
    assert!(response.error.unwrap().contains("USDA API key"));

    Ok(())
}

#[tokio::test]
async fn test_analyze_meal_nutrition_missing_foods() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request("analyze_meal_nutrition", json!({}));

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail with missing foods array");

    Ok(())
}

#[tokio::test]
async fn test_analyze_meal_nutrition_empty_foods() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    let request = create_test_request(
        "analyze_meal_nutrition",
        json!({
            "ingredients": []
        }),
    );

    // An empty meal is a valid meal with nothing in it: zero totals, no
    // foods, and no USDA call to make.
    let response = executor.execute_tool(request).await?;
    assert!(
        response.success,
        "an empty meal should succeed: {response:?}"
    );
    let result = response.result.expect("a successful analysis has a result");
    assert!(
        result["total_calories"].as_f64().unwrap().abs() < f64::EPSILON,
        "an empty meal has zero calories"
    );
    assert_eq!(result["foods"].as_array().map(Vec::len), Some(0));

    Ok(())
}

#[tokio::test]
async fn test_analyze_meal_nutrition_invalid_food_entry() -> Result<()> {
    let executor = create_nutrition_test_executor().await?;

    // Missing grams field
    let request = create_test_request(
        "analyze_meal_nutrition",
        json!({
            "ingredients": [
                {"fdc_id": 171_477}
            ]
        }),
    );

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail with missing grams");

    Ok(())
}
